use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event as TermEvent, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config as TermConfig, RenderableContent, Term};
use alacritty_terminal::vte::ansi::{Color as VteColor, CursorShape, Processor, Rgb};
use gpui::{
    relative, rgb, App, Bounds, Context, Element, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, Font, FontStyle, FontWeight, GlobalElementId, Hsla, IntoElement, KeyDownEvent,
    Keystroke, LayoutId, MouseDownEvent, Pixels, Render, ShapedLine, SharedString,
    StrikethroughStyle, Style, TextRun, UnderlineStyle, Window, div, point, prelude::*, px, size,
};
use crate::config::{Config, TermColors};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TerminalKind {
    /// A plain shell.
    Terminal,
    /// Helix running in the PTY.
    Editor,
}

/// Emitted once the process in the PTY has exited.
pub struct Exited;

struct Dims {
    cols: usize,
    rows: usize,
}

impl Dimensions for Dims {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// RGB for an xterm color index, including alacritty's named slots (256+).
fn index_rgb(i: usize, colors: &TermColors) -> u32 {
    match i {
        0..=15 => colors.ansi[i],
        16..=231 => {
            let c = i - 16;
            let step = [0u32, 95, 135, 175, 215, 255];
            step[c / 36] << 16 | step[(c / 6) % 6] << 8 | step[c % 6]
        }
        232..=255 => {
            let gray = (8 + 10 * (i - 232)) as u32;
            gray << 16 | gray << 8 | gray
        }
        257 => colors.background,
        258 => colors.cursor,
        // shortcut: dim colors reuse the normal palette, add a dim palette if it matters
        259..=266 => colors.ansi[i - 259],
        _ => colors.foreground,
    }
}

fn color_of(color: VteColor, colors: &TermColors) -> Hsla {
    match color {
        VteColor::Named(named) => rgb(index_rgb(named as usize, colors)).into(),
        VteColor::Indexed(i) => rgb(index_rgb(i as usize, colors)).into(),
        VteColor::Spec(spec) => {
            rgb((spec.r as u32) << 16 | (spec.g as u32) << 8 | spec.b as u32).into()
        }
    }
}

type PtyWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// Answers terminal queries (cursor position, device attributes, colors).
/// Programs like Helix block on these replies at startup.
#[derive(Clone)]
struct PtyListener {
    writer: PtyWriter,
    colors: TermColors,
}

impl EventListener for PtyListener {
    fn send_event(&self, event: TermEvent) {
        let reply = match event {
            TermEvent::PtyWrite(text) => text,
            TermEvent::ColorRequest(index, format) => {
                let c = index_rgb(index, &self.colors);
                format(Rgb {
                    r: (c >> 16) as u8,
                    g: (c >> 8) as u8,
                    b: c as u8,
                })
            }
            _ => return,
        };
        let _ = self.writer.lock().unwrap().write_all(reply.as_bytes());
    }
}

pub struct TerminalView {
    pub kind: TerminalKind,
    focus_handle: FocusHandle,
    term: Term<PtyListener>,
    writer: PtyWriter,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    exited: Arc<AtomicBool>,
    parser: Processor,
    cols: usize,
    rows: usize,
}

impl EventEmitter<Exited> for TerminalView {}

impl TerminalView {
    pub fn new(kind: TerminalKind, cwd: PathBuf, cx: &mut Context<Self>) -> Self {
        let pty = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("failed to open pty");

        let mut cmd = match kind {
            TerminalKind::Editor => CommandBuilder::new("hx"),
            TerminalKind::Terminal => CommandBuilder::new_default_prog(),
        };
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.cwd(&cwd);
        let mut child = pty
            .slave
            .spawn_command(cmd)
            .expect("failed to spawn pty child");
        let killer = child.clone_killer();

        let writer: PtyWriter = Arc::new(Mutex::new(pty.master.take_writer().expect("pty writer")));
        let mut reader = pty.master.try_clone_reader().expect("pty reader");
        let term = Term::new(
            TermConfig::default(),
            &Dims { cols: 80, rows: 24 },
            PtyListener {
                writer: writer.clone(),
                colors: cx.global::<Config>().terminal,
            },
        );

        let (tx, rx) = async_channel::unbounded::<Vec<u8>>();

        // Closing the channel when the child exits ends the UI loop below even if
        // a background job still holds the pty open.
        let exited = Arc::new(AtomicBool::new(false));
        {
            let tx = tx.clone();
            let exited = exited.clone();
            std::thread::spawn(move || {
                let _ = child.wait();
                exited.store(true, Ordering::SeqCst);
                tx.close();
            });
        }

        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send_blocking(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        cx.spawn(async move |this, cx| {
            while let Ok(mut bytes) = rx.recv().await {
                // Coalesce everything already queued into one parse + one redraw.
                while let Ok(more) = rx.try_recv() {
                    bytes.extend_from_slice(&more);
                }
                let fed = this.update(cx, |view, cx| {
                    view.feed(&bytes);
                    cx.notify();
                });
                if fed.is_err() {
                    return;
                }
            }
            this.update(cx, |_, cx| cx.emit(Exited)).ok();
        })
        .detach();

        Self {
            kind,
            focus_handle: cx.focus_handle(),
            term,
            writer,
            master: pty.master,
            killer,
            exited,
            parser: Processor::new(),
            cols: 80,
            rows: 24,
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.parser.advance(&mut self.term, *b);
        }
    }

    pub fn write(&self, bytes: &[u8]) {
        let _ = self.writer.lock().unwrap().write_all(bytes);
    }

    /// Send a line of text to the PTY, as if typed followed by Enter.
    pub fn send_line(&self, line: &str) {
        let mut bytes = line.as_bytes().to_vec();
        bytes.push(b'\r');
        self.write(&bytes);
    }

    fn resize(&mut self, cols: usize, rows: usize) {
        if cols == 0 || rows == 0 || (cols == self.cols && rows == self.rows) {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        self.term.resize(Dims { cols, rows });
        let _ = self.master.resize(PtySize {
            rows: rows as u16,
            cols: cols as u16,
            pixel_width: 0,
            pixel_height: 0,
        });
    }
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        // The reader thread holds a dup of the pty fd, so dropping the master alone
        // would not hang up the child. Skip if already reaped (pid could be reused).
        if !self.exited.load(Ordering::SeqCst) {
            let _ = self.killer.kill();
        }
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// Translate a gpui keystroke into PTY bytes. Covers what shells and Helix need
/// day to day; exotic combos are dropped rather than sent as garbage.
fn keystroke_to_bytes(ks: &Keystroke) -> Option<Vec<u8>> {
    let key = ks.key.as_str();
    let mods = &ks.modifiers;

    // Never forward bare modifier presses.
    if matches!(
        key,
        "shift" | "control" | "alt" | "meta" | "platform" | "function" | "capslock"
    ) {
        return None;
    }

    if mods.control && !mods.alt && !mods.shift && !mods.platform {
        let byte = match key {
            "space" => Some(0u8),
            _ => match key.as_bytes() {
                [b @ b'a'..=b'z'] => Some(b - b'a' + 1),
                [b'@'] => Some(0),
                [b'['] => Some(27),
                [b'\\'] => Some(28),
                [b']'] => Some(29),
                [b'^'] => Some(30),
                [b'_'] => Some(31),
                _ => None,
            },
        };
        return byte.map(|b| vec![b]);
    }

    // Alt-modified keys are sent as ESC + key, unless a workspace binding
    // (alt-enter etc.) already consumed the keystroke.
    if mods.alt && !mods.control && !mods.platform {
        if let Some(seq) = named_key_sequence(key) {
            return Some([vec![0x1b], seq].concat());
        }
        if let Some(c) = ks.key_char.as_deref().and_then(|s| s.chars().next()) {
            let mut out = vec![0x1b];
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            return Some(out);
        }
        if let Some(b) = key.as_bytes().first() {
            return Some(vec![0x1b, *b]);
        }
        return None;
    }

    if mods.shift && key == "tab" {
        return Some(b"\x1b[Z".to_vec());
    }
    if let Some(seq) = named_key_sequence(key) {
        return Some(seq);
    }

    if let Some(c) = ks.key_char.as_deref() {
        return Some(c.as_bytes().to_vec());
    }

    // Fallback for layouts where key_char is missing: single ascii key name.
    match key.as_bytes() {
        [b] if b.is_ascii_graphic() => Some(vec![*b]),
        _ => None,
    }
}

fn named_key_sequence(key: &str) -> Option<Vec<u8>> {
    let seq: &[u8] = match key {
        "enter" => b"\r",
        "escape" => b"\x1b",
        "backspace" => b"\x7f",
        "tab" => b"\t",
        "space" => b" ",
        "up" => b"\x1b[A",
        "down" => b"\x1b[B",
        "right" => b"\x1b[C",
        "left" => b"\x1b[D",
        "home" => b"\x1b[H",
        "end" => b"\x1b[F",
        "pageup" => b"\x1b[5~",
        "pagedown" => b"\x1b[6~",
        "insert" => b"\x1b[2~",
        "delete" => b"\x1b[3~",
        _ => return None,
    };
    Some(seq.to_vec())
}

// Rendering -------------------------------------------------------------------

/// Everything about a cell that affects how its run is shaped and painted.
#[derive(Clone, Copy, PartialEq)]
struct CellStyle {
    fg: Hsla,
    bg: Option<Hsla>,
    flags: Flags,
    underline_color: Option<Hsla>,
}

const STYLE_FLAGS: Flags = Flags::BOLD
    .union(Flags::ITALIC)
    .union(Flags::ALL_UNDERLINES)
    .union(Flags::STRIKEOUT);

impl CellStyle {
    fn of(cell: &alacritty_terminal::term::cell::Cell, colors: &TermColors) -> Self {
        let default_bg = matches!(
            cell.bg,
            VteColor::Named(alacritty_terminal::vte::ansi::NamedColor::Background)
        );
        let mut fg = color_of(cell.fg, colors);
        let mut bg = (!default_bg).then(|| color_of(cell.bg, colors));
        if cell.flags.contains(Flags::INVERSE) {
            let new_fg = bg.unwrap_or(rgb(colors.background).into());
            bg = Some(fg);
            fg = new_fg;
        }
        if cell.flags.contains(Flags::DIM) {
            fg.a *= 0.66;
        }
        if cell.flags.contains(Flags::HIDDEN) {
            fg.a = 0.;
        }
        Self {
            fg,
            bg,
            flags: cell.flags & STYLE_FLAGS,
            underline_color: cell.underline_color().map(|c| color_of(c, colors)),
        }
    }

    fn run(&self, len: usize, font: &Font) -> TextRun {
        let mut font = font.clone();
        if self.flags.contains(Flags::BOLD) {
            font.weight = FontWeight::BOLD;
        }
        if self.flags.contains(Flags::ITALIC) {
            font.style = FontStyle::Italic;
        }
        let underline = self.flags.intersects(Flags::ALL_UNDERLINES).then(|| UnderlineStyle {
            color: Some(self.underline_color.unwrap_or(self.fg)),
            thickness: px(1.),
            wavy: self.flags.contains(Flags::UNDERCURL),
        });
        let strikethrough = self.flags.contains(Flags::STRIKEOUT).then(|| StrikethroughStyle {
            thickness: px(1.),
            color: Some(self.fg),
        });
        TextRun {
            len,
            font,
            color: self.fg,
            background_color: self.bg,
            underline,
            strikethrough,
        }
    }
}

struct TerminalElement {
    view: Entity<TerminalView>,
}

struct TerminalPrepaint {
    rows: Vec<ShapedLine>,
    cursor: Option<(usize, usize, CursorShape)>,
    cell_width: Pixels,
}

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = TerminalPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let colors = cx.global::<Config>().terminal;
        let view = self.view.read(cx);
        let style = window.text_style();
        let font = style.font();
        let font_size = style.font_size.to_pixels(window.rem_size());

        // Measure the cell width from a probe glyph.
        let probe = window.text_system().shape_line(
            SharedString::from("M"),
            font_size,
            &[CellStyle {
                fg: style.color,
                bg: None,
                flags: Flags::empty(),
                underline_color: None,
            }
            .run(1, &font)],
            None,
        );
        let cell_width = probe.x_for_index(1).max(px(1.));

        let term = &view.term;
        let RenderableContent {
            display_iter,
            cursor,
            display_offset,
            ..
        } = term.renderable_content();
        let columns = term.grid().columns();
        let screen_lines = term.grid().screen_lines();

        let mut rows: Vec<ShapedLine> = Vec::with_capacity(screen_lines);
        let mut row_text = String::new();
        let mut row_runs: Vec<TextRun> = Vec::new();
        let mut run_start = 0usize;
        let mut cur: Option<CellStyle> = None;
        let mut col = 0usize;

        for cell in display_iter {
            let cell_style = CellStyle::of(&cell, &colors);
            if cur != Some(cell_style) {
                if let Some(prev) = cur.filter(|_| row_text.len() > run_start) {
                    row_runs.push(prev.run(row_text.len() - run_start, &font));
                }
                run_start = row_text.len();
                cur = Some(cell_style);
            }
            // Wide-char spacer cells stay as ' ' so every cell maps to one glyph.
            row_text.push(cell.c);
            col += 1;
            if col == columns {
                if let Some(prev) = cur.filter(|_| row_text.len() > run_start) {
                    row_runs.push(prev.run(row_text.len() - run_start, &font));
                }
                // force_width snaps every glyph to the cell grid.
                rows.push(window.text_system().shape_line(
                    SharedString::from(std::mem::take(&mut row_text)),
                    font_size,
                    &row_runs,
                    Some(cell_width),
                ));
                row_runs.clear();
                run_start = 0;
                cur = None;
                col = 0;
            }
        }

        let cursor = {
            let row = cursor.point.line.0 + display_offset as i32;
            let column = cursor.point.column.0;
            (row >= 0
                && (row as usize) < screen_lines
                && column < columns
                && cursor.shape != CursorShape::Hidden)
                .then_some((row as usize, column, cursor.shape))
        };

        TerminalPrepaint {
            rows,
            cursor,
            cell_width,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let line_height = window.line_height();
        let cell_width = prepaint.cell_width;

        // Resize the terminal to fit the available bounds.
        let cols = ((bounds.size.width / cell_width).floor() as usize).max(1);
        let rows = ((bounds.size.height / line_height).floor() as usize).max(1);
        let needs_resize = {
            let view = self.view.read(cx);
            view.cols != cols || view.rows != rows
        };
        if needs_resize {
            self.view.update(cx, |view, cx| {
                view.resize(cols, rows);
                cx.notify();
            });
        }

        // Backgrounds are a separate pass in gpui; all of them go under all text.
        for (i, line) in prepaint.rows.iter().enumerate() {
            let origin = bounds.origin + point(px(0.), line_height * i as f32);
            let _ = line.paint_background(origin, line_height, window, cx);
        }
        for (i, line) in prepaint.rows.iter().enumerate() {
            let origin = bounds.origin + point(px(0.), line_height * i as f32);
            let _ = line.paint(origin, line_height, window, cx);
        }

        if let Some((row, column, shape)) = prepaint.cursor {
            let origin =
                bounds.origin + point(cell_width * column as f32, line_height * row as f32);
            let (origin, cursor_size) = match shape {
                CursorShape::Beam => (origin, size(px(2.), line_height)),
                CursorShape::Underline => (
                    origin + point(px(0.), line_height - px(2.)),
                    size(cell_width, px(2.)),
                ),
                _ => (origin, size(cell_width, line_height)),
            };
            let mut color: Hsla = rgb(cx.global::<Config>().terminal.cursor).into();
            color.a = 0.7; // let the glyph underneath show through
            window.paint_quad(gpui::fill(Bounds::new(origin, cursor_size), color));
        }
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);
        let config = cx.global::<Config>();
        let (family, size) = (config.font_family.clone(), config.font_size);
        let (background, border) = (
            config.terminal.background,
            if focused { config.ui.accent } else { config.ui.element },
        );
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .size_full()
            .key_context("Terminal")
            .track_focus(&self.focus_handle)
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, _cx| {
                    this.focus_handle.focus(window);
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, _cx| {
                if let Some(bytes) = keystroke_to_bytes(&event.keystroke) {
                    this.write(&bytes);
                }
            }))
            .font_family(family)
            .text_size(px(size))
            .line_height(px((size * 1.4).round()))
            .bg(rgb(background))
            .border_1()
            .border_color(rgb(border))
            .child(TerminalElement { view: cx.entity() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SharedBuf(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn feed_and_capture(input: &[u8]) -> String {
        let out = Arc::new(Mutex::new(Vec::new()));
        let writer: PtyWriter = Arc::new(Mutex::new(Box::new(SharedBuf(out.clone()))));
        let mut term = Term::new(
            TermConfig::default(),
            &Dims { cols: 80, rows: 24 },
            PtyListener {
                writer,
                colors: TermColors::default(),
            },
        );
        let mut parser: Processor = Processor::new();
        for b in input {
            parser.advance(&mut term, *b);
        }
        let bytes = out.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn answers_device_attributes_and_cursor_queries() {
        assert!(feed_and_capture(b"\x1b[c").starts_with("\x1b[?"));
        assert_eq!(feed_and_capture(b"\x1b[6n"), "\x1b[1;1R");
    }

    #[test]
    fn answers_background_color_query() {
        assert_eq!(
            feed_and_capture(b"\x1b]11;?\x07"),
            "\x1b]11;rgb:1e1e/1e1e/2e2e\x07"
        );
    }
}
