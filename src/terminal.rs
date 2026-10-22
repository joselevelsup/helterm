use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event as TermEvent, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::{Config as TermConfig, RenderableContent, Term, TermMode};
use alacritty_terminal::vi_mode::ViMotion;
use alacritty_terminal::vte::ansi::{Color as VteColor, CursorShape, Processor, Rgb};
use gpui::{
    actions, relative, rgb, App, Bounds, ClipboardItem, Context, Element, ElementId, Entity,
    EventEmitter, FocusHandle, Focusable, Font, FontStyle, FontWeight, GlobalElementId, Hsla,
    IntoElement, KeyDownEvent, Keystroke, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Render, ScrollDelta, ScrollWheelEvent, ShapedLine, SharedString, Size,
    StrikethroughStyle, Style,
    TextRun, UnderlineStyle, Window, div, point, prelude::*, px, size,
};

actions!(terminal, [Copy, Paste, CopyMode]);
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
    /// Where the grid was last painted, to map mouse positions to cells.
    grid_origin: gpui::Point<Pixels>,
    cell_size: Size<Pixels>,
    /// A mouse-drag selection is in progress.
    selecting: bool,
    /// Fraction of a line left over from smooth (touchpad) scrolling.
    scroll_remainder: f32,
    copy_mode: CopyModeState,
}

impl EventEmitter<Exited> for TerminalView {}

/// Cell under a position measured from the grid's top-left corner, plus which
/// half of the cell it is on. Positions outside the grid clamp to its edges.
fn cell_at(x: f32, y: f32, cell: (f32, f32), grid: (usize, usize)) -> (usize, usize, Side) {
    let (col, row) = (x.max(0.) / cell.0, y.max(0.) / cell.1);
    let (cols, rows) = grid;
    let column = (col as usize).min(cols.saturating_sub(1));
    let side = if col as usize >= cols || col.fract() >= 0.5 { Side::Right } else { Side::Left };
    (column, (row as usize).min(rows.saturating_sub(1)), side)
}

/// What a key did in copy mode.
#[derive(Debug, PartialEq)]
enum CopyModeKey {
    Handled,
    /// Copy this text and leave copy mode.
    Yank(String),
    Exit,
    Ignored,
}

/// Copy mode's own state on top of alacritty's vi mode.
#[derive(Default)]
struct CopyModeState {
    /// Helix select mode (`v`): movement extends the selection.
    extend: bool,
    /// `g` was pressed; the next key picks the goto target.
    pending_goto: bool,
}

enum Move {
    Motion(ViMotion),
    Top,
    Bottom,
}

/// Move the copy-mode cursor the Helix way: in normal mode a move drops the
/// selection, in select mode it extends it (anchored where the cursor was).
fn apply_move<T: EventListener>(term: &mut Term<T>, state: &CopyModeState, mv: Move) {
    if !state.extend {
        term.selection = None;
    } else if term.selection.is_none() {
        let mut selection = Selection::new(SelectionType::Simple, term.vi_mode_cursor.point, Side::Left);
        // An empty selection would not grow as the cursor moves.
        selection.include_all();
        term.selection = Some(selection);
    }
    match mv {
        Move::Motion(motion) => {
            term.vi_motion(motion);
            let point = term.vi_mode_cursor.point;
            // The view follows the cursor into the scrollback.
            term.scroll_to_point(point);
        }
        Move::Top => term.vi_goto_point(GridPoint::new(term.topmost_line(), Column(0))),
        Move::Bottom => term.vi_goto_point(GridPoint::new(term.bottommost_line(), Column(0))),
    }
}

/// Helix-style copy mode: keys move a cursor through the scrollback, `x`/`v`
/// select, `y` copies.
fn copy_mode_key<T: EventListener>(term: &mut Term<T>, state: &mut CopyModeState, ks: &Keystroke) -> CopyModeKey {
    let ctrl = ks.modifiers.control;
    // Prefer the typed character ("W", "%"); fall back to the key name.
    let key = match ks.key_char.as_deref() {
        Some(typed) if !ctrl => typed.to_owned(),
        _ if ks.modifiers.shift && ks.key.chars().count() == 1 => ks.key.to_uppercase(),
        _ => ks.key.clone(),
    };
    let key = key.as_str();

    if std::mem::take(&mut state.pending_goto) {
        let mv = match key {
            "g" => Move::Top,
            "e" => Move::Bottom,
            "h" => Move::Motion(ViMotion::First),
            "l" => Move::Motion(ViMotion::Last),
            "s" => Move::Motion(ViMotion::FirstOccupied),
            "t" => Move::Motion(ViMotion::High),
            "c" => Move::Motion(ViMotion::Middle),
            "b" => Move::Motion(ViMotion::Low),
            _ => return CopyModeKey::Handled, // unknown goto: cancel, like Helix
        };
        apply_move(term, state, mv);
        return CopyModeKey::Handled;
    }

    let motion = match (ctrl, key) {
        (false, "j" | "down") => Some(ViMotion::Down),
        (false, "k" | "up") => Some(ViMotion::Up),
        (false, "h" | "left") => Some(ViMotion::Left),
        (false, "l" | "right") => Some(ViMotion::Right),
        (false, "w") => Some(ViMotion::SemanticRight),
        (false, "b") => Some(ViMotion::SemanticLeft),
        (false, "e") => Some(ViMotion::SemanticRightEnd),
        (false, "W") => Some(ViMotion::WordRight),
        (false, "B") => Some(ViMotion::WordLeft),
        (false, "E") => Some(ViMotion::WordRightEnd),
        (false, "home") => Some(ViMotion::First),
        (false, "end") => Some(ViMotion::Last),
        _ => None,
    };
    if let Some(motion) = motion {
        apply_move(term, state, Move::Motion(motion));
        return CopyModeKey::Handled;
    }

    let half_page = (term.screen_lines() / 2).max(1) as i32;
    let scroll = match (ctrl, key) {
        (true, "u") => Some(Scroll::Delta(half_page)),
        (true, "d") => Some(Scroll::Delta(-half_page)),
        (true, "b") | (false, "pageup") => Some(Scroll::PageUp),
        (true, "f") | (false, "pagedown") => Some(Scroll::PageDown),
        (true, "y") => Some(Scroll::Delta(1)),
        (true, "e") => Some(Scroll::Delta(-1)),
        _ => None,
    };
    if let Some(scroll) = scroll {
        // Keeps the cursor inside the view.
        term.scroll_display(scroll);
        return CopyModeKey::Handled;
    }

    match (ctrl, key) {
        (false, "g") => state.pending_goto = true,
        (false, "x") => {
            if term.selection.as_ref().is_some_and(|s| s.ty == SelectionType::Lines) {
                // Already on whole lines: grow by one more, like Helix.
                term.vi_motion(ViMotion::Down);
                let point = term.vi_mode_cursor.point;
                term.scroll_to_point(point);
            } else {
                let mut selection = Selection::new(SelectionType::Lines, term.vi_mode_cursor.point, Side::Left);
                selection.include_all();
                term.selection = Some(selection);
            }
        }
        (false, "v") => state.extend = !state.extend,
        (false, "%") => {
            let mut all = Selection::new(SelectionType::Lines, GridPoint::new(term.topmost_line(), Column(0)), Side::Left);
            all.update(GridPoint::new(term.bottommost_line(), term.last_column()), Side::Right);
            term.selection = Some(all);
        }
        (false, ";") => term.selection = None,
        (false, "y") => match term.selection_to_string().filter(|text| !text.is_empty()) {
            Some(text) => return CopyModeKey::Yank(text),
            None => {} // nothing selected: stay, so a stray y doesn't drop you out
        },
        (false, "escape") | (true, "c") => {
            // Like Helix: first Esc leaves select mode / drops the selection.
            if state.extend || term.selection.is_some() {
                state.extend = false;
                term.selection = None;
            } else {
                return CopyModeKey::Exit;
            }
        }
        _ => return CopyModeKey::Ignored,
    }
    CopyModeKey::Handled
}

/// Bytes a paste sends to the program.
fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        // Drop ESC so pasted text can't end the paste early and run as commands.
        format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', "")).into_bytes()
    } else {
        // Without bracketed paste, newlines are typed as Enter, like other terminals.
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

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
            grid_origin: gpui::Point::default(),
            cell_size: Size::default(),
            selecting: false,
            scroll_remainder: 0.,
            copy_mode: CopyModeState::default(),
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.parser.advance(&mut self.term, *b);
        }
    }

    fn grid_point(&self, position: gpui::Point<Pixels>) -> (GridPoint, Side) {
        let grid = self.term.grid();
        let offset = position - self.grid_origin;
        let cell = (self.cell_size.width / px(1.), self.cell_size.height / px(1.));
        if cell.0 <= 0. || cell.1 <= 0. {
            return (GridPoint::new(Line(0), Column(0)), Side::Left); // not painted yet
        }
        let (column, row, side) = cell_at(
            offset.x / px(1.),
            offset.y / px(1.),
            cell,
            (grid.columns(), grid.screen_lines()),
        );
        let line = Line(row as i32 - grid.display_offset() as i32);
        (GridPoint::new(line, Column(column)), side)
    }

    /// Click starts a selection: 1 click by character, 2 by word, 3 by line.
    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window);
        let (point, side) = self.grid_point(event.position);
        let ty = match event.click_count {
            2 => SelectionType::Semantic,
            n if n >= 3 => SelectionType::Lines,
            _ => SelectionType::Simple,
        };
        self.term.selection = Some(Selection::new(ty, point, side));
        self.selecting = true;
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let (point, side) = self.grid_point(event.position);
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side);
            cx.notify();
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.selecting) {
            return;
        }
        // Linux convention: selecting copies to the PRIMARY selection (middle-click paste).
        if let Some(text) = self.selected_text() {
            cx.write_to_primary(ClipboardItem::new_string(text));
        }
    }

    fn in_copy_mode(&self) -> bool {
        self.term.mode().contains(TermMode::VI)
    }

    fn toggle_copy_mode(&mut self, _: &CopyMode, _: &mut Window, cx: &mut Context<Self>) {
        if self.in_copy_mode() {
            self.exit_copy_mode();
        } else {
            self.term.selection = None;
            self.copy_mode = CopyModeState::default();
            self.term.toggle_vi_mode();
        }
        cx.notify();
    }

    fn exit_copy_mode(&mut self) {
        if self.in_copy_mode() {
            self.term.toggle_vi_mode();
        }
        self.copy_mode = CopyModeState::default();
        self.term.selection = None;
        self.term.scroll_display(Scroll::Bottom);
    }

    /// Wheel/touchpad: scroll the history. Full-screen programs (Helix, less)
    /// have no history, so they get arrow keys instead, like Alacritty does.
    fn scroll_wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let line_height = self.cell_size.height / px(1.);
        if line_height <= 0. {
            return;
        }
        self.scroll_remainder += match event.delta {
            ScrollDelta::Lines(delta) => delta.y * 3.,
            ScrollDelta::Pixels(delta) => delta.y / px(1.) / line_height,
        };
        let lines = self.scroll_remainder.trunc();
        self.scroll_remainder -= lines;
        let lines = lines as i32;
        if lines == 0 {
            return;
        }
        let mode = *self.term.mode();
        if mode.contains(TermMode::ALT_SCREEN) && !mode.contains(TermMode::VI) {
            if mode.contains(TermMode::ALTERNATE_SCROLL) {
                let arrow: &[u8] = if lines > 0 { b"\x1b[A" } else { b"\x1b[B" };
                self.write(&arrow.repeat(lines.unsigned_abs() as usize));
            }
        } else {
            self.term.scroll_display(Scroll::Delta(lines));
            cx.notify();
        }
    }

    fn selected_text(&self) -> Option<String> {
        self.term.selection_to_string().filter(|text| !text.is_empty())
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.paste_text(&text, cx);
        }
    }

    fn paste_primary(&mut self, _: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window);
        if let Some(text) = cx.read_from_primary().and_then(|item| item.text()) {
            self.paste_text(&text, cx);
        }
    }

    fn paste_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.term.selection = None;
        let bracketed = self.term.mode().contains(TermMode::BRACKETED_PASTE);
        self.write(&paste_bytes(text, bracketed));
        cx.notify();
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
            selection,
            ..
        } = term.renderable_content();
        let selection_bg: Hsla = rgb(colors.selection).into();
        let columns = term.grid().columns();
        let screen_lines = term.grid().screen_lines();

        let mut rows: Vec<ShapedLine> = Vec::with_capacity(screen_lines);
        let mut row_text = String::new();
        let mut row_runs: Vec<TextRun> = Vec::new();
        let mut run_start = 0usize;
        let mut cur: Option<CellStyle> = None;
        let mut col = 0usize;

        for cell in display_iter {
            let mut cell_style = CellStyle::of(&cell, &colors);
            if selection.is_some_and(|s| s.contains(cell.point)) {
                cell_style.bg = Some(selection_bg);
            }
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
        self.view.update(cx, |view, cx| {
            view.grid_origin = bounds.origin;
            view.cell_size = size(cell_width, line_height);
            if view.cols != cols || view.rows != rows {
                view.resize(cols, rows);
                cx.notify();
            }
        });

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
        // Where we are in the history, like tmux's [12/340]; only when not at the bottom.
        let grid = self.term.grid();
        let (offset, history) = (grid.display_offset(), grid.history_size());
        let badge = match self.in_copy_mode() {
            true => {
                let mode = if self.copy_mode.extend { "COPY SEL" } else { "COPY" };
                let goto = if self.copy_mode.pending_goto { "  g…" } else { "" };
                Some(format!("{mode}  {offset}/{history}{goto}"))
            }
            false => (offset > 0).then(|| format!("{offset}/{history}")),
        };
        let (badge_bg, badge_fg) = {
            let ui = cx.global::<Config>().ui;
            (ui.accent, ui.background)
        };
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
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::paste_primary))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::toggle_copy_mode))
            .on_scroll_wheel(cx.listener(Self::scroll_wheel))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                if this.in_copy_mode() {
                    // Copy mode keys never reach the program.
                    match copy_mode_key(&mut this.term, &mut this.copy_mode, &event.keystroke) {
                        CopyModeKey::Ignored => return,
                        CopyModeKey::Handled => {}
                        CopyModeKey::Yank(text) => {
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                            this.exit_copy_mode();
                        }
                        CopyModeKey::Exit => this.exit_copy_mode(),
                    }
                    cx.notify();
                    return;
                }
                if let Some(bytes) = keystroke_to_bytes(&event.keystroke) {
                    // Typing clears the selection and jumps back to the bottom,
                    // like other terminals.
                    let scrolled = this.term.grid().display_offset() != 0;
                    if this.term.selection.take().is_some() || scrolled {
                        this.term.scroll_display(Scroll::Bottom);
                        cx.notify();
                    }
                    this.write(&bytes);
                }
            }))
            .font_family(family)
            .text_size(px(size))
            .line_height(px((size * 1.4).round()))
            .bg(rgb(background))
            .border_1()
            .border_color(rgb(border))
            .rounded_md()
            // Inner padding between the border and the text; the grid shrinks to fit.
            .p(px(6.))
            .relative()
            .child(TerminalElement { view: cx.entity() })
            .when_some(badge, |el, badge| {
                el.child(
                    div()
                        .absolute()
                        .top(px(4.))
                        .right(px(8.))
                        .px_2()
                        .rounded_sm()
                        .bg(rgb(badge_bg))
                        .text_color(rgb(badge_fg))
                        .text_size(px(11.))
                        .line_height(px(16.))
                        .child(badge),
                )
            })
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
    fn mouse_positions_map_to_cells() {
        let cell = (10., 20.);
        assert_eq!(cell_at(0., 0., cell, (80, 24)), (0, 0, Side::Left));
        assert_eq!(cell_at(14., 0., cell, (80, 24)), (1, 0, Side::Left));
        assert_eq!(cell_at(16., 45., cell, (80, 24)), (1, 2, Side::Right));
        // Outside the grid clamps to the edges (drags past the pane).
        assert_eq!(cell_at(-50., -50., cell, (80, 24)), (0, 0, Side::Left));
        assert_eq!(cell_at(5000., 5000., cell, (80, 24)), (79, 23, Side::Right));
    }

    #[test]
    fn paste_is_bracketed_and_cannot_escape() {
        assert_eq!(paste_bytes("ls\nrm -rf x", false), b"ls\rrm -rf x");
        assert_eq!(paste_bytes("a\r\nb", false), b"a\rb");
        // A pasted end-marker must not end the paste early.
        assert_eq!(
            paste_bytes("echo hi\x1b[201~rm -rf /\n", true),
            b"\x1b[200~echo hi[201~rm -rf /\n\x1b[201~"
        );
    }

    #[test]
    fn selections_copy_the_right_text() {
        let mut term = Term::new(
            TermConfig::default(),
            &Dims { cols: 40, rows: 5 },
            PtyListener {
                writer: Arc::new(Mutex::new(Box::new(std::io::sink()))),
                colors: TermColors::default(),
            },
        );
        let mut parser: Processor = Processor::new();
        for b in b"hello world\r\nsecond line" {
            parser.advance(&mut term, *b);
        }
        let at = |line, col| GridPoint::new(Line(line), Column(col));

        // Drag from "w" to "d".
        let mut drag = Selection::new(SelectionType::Simple, at(0, 6), Side::Left);
        drag.update(at(0, 10), Side::Right);
        term.selection = Some(drag);
        assert_eq!(term.selection_to_string().as_deref(), Some("world"));

        // Double-click a word, triple-click a line.
        term.selection = Some(Selection::new(SelectionType::Semantic, at(1, 2), Side::Left));
        assert_eq!(term.selection_to_string().as_deref(), Some("second"));
        term.selection = Some(Selection::new(SelectionType::Lines, at(0, 3), Side::Left));
        assert_eq!(term.selection_to_string().as_deref(), Some("hello world\n"));

        // A plain click selects nothing.
        term.selection = Some(Selection::new(SelectionType::Simple, at(0, 3), Side::Left));
        assert_eq!(term.selection_to_string().filter(|s| !s.is_empty()), None);
    }

    fn term_with_lines(lines: usize) -> Term<PtyListener> {
        let mut term = Term::new(
            TermConfig::default(),
            &Dims { cols: 20, rows: 5 },
            PtyListener {
                writer: Arc::new(Mutex::new(Box::new(std::io::sink()))),
                colors: TermColors::default(),
            },
        );
        let mut parser: Processor = Processor::new();
        let text: String = (0..lines).map(|i| format!("line {i}\r\n")).collect();
        for b in text.as_bytes() {
            parser.advance(&mut term, *b);
        }
        term
    }

    fn press(term: &mut Term<PtyListener>, state: &mut CopyModeState, keys: &str) -> CopyModeKey {
        let mut last = CopyModeKey::Ignored;
        for key in keys.split_whitespace() {
            last = copy_mode_key(term, state, &Keystroke::parse(key).unwrap());
        }
        last
    }

    #[test]
    fn copy_mode_uses_helix_keys() {
        let mut term = term_with_lines(30);
        let st = &mut CopyModeState::default();
        term.toggle_vi_mode();

        // k walks up past the top of the screen: the view follows into history.
        press(&mut term, st, "k k k k k k k k k k");
        assert!(term.grid().display_offset() > 0);

        // gg: top of history, ge: last line.
        press(&mut term, st, "g g");
        assert_eq!(term.grid().display_offset(), term.grid().history_size());
        press(&mut term, st, "g e");
        assert_eq!(term.grid().display_offset(), 0);

        // x selects the line, x again extends it, y copies.
        press(&mut term, st, "g g j j j x x");
        assert_eq!(press(&mut term, st, "y"), CopyModeKey::Yank("line 3\nline 4\n".into()));

        // Normal mode: moving drops the selection.
        press(&mut term, st, "g g x j");
        assert!(term.selection.is_none());

        // v: select mode, movement extends from where the cursor was.
        press(&mut term, st, "g g j v l l l");
        assert_eq!(press(&mut term, st, "y"), CopyModeKey::Yank("line".into()));
        press(&mut term, st, "v"); // back to normal mode

        // gl / gh: end of the line's text ("line 0" ends at column 5) / start.
        press(&mut term, st, "g g g l");
        assert_eq!(term.vi_mode_cursor.point.column, Column(5));
        press(&mut term, st, "g h");
        assert_eq!(term.vi_mode_cursor.point.column, Column(0));

        // %: everything; ; collapses.
        press(&mut term, st, "%->%");
        assert!(term.selection_to_string().unwrap().starts_with("line 0\nline 1\n"));
        press(&mut term, st, ";->;");
        assert!(term.selection.is_none());

        // Ctrl+U / Ctrl+D scroll by half a screen.
        press(&mut term, st, "g e ctrl-u");
        assert_eq!(term.grid().display_offset(), 2);
        press(&mut term, st, "ctrl-d");
        assert_eq!(term.grid().display_offset(), 0);

        // y with nothing selected stays in copy mode; unknown keys do nothing.
        assert_eq!(press(&mut term, st, "y"), CopyModeKey::Handled);
        assert_eq!(press(&mut term, st, "q"), CopyModeKey::Ignored);

        // Esc: first drops the selection / select mode, then leaves.
        press(&mut term, st, "x");
        assert_eq!(press(&mut term, st, "escape"), CopyModeKey::Handled);
        assert!(term.selection.is_none());
        assert_eq!(press(&mut term, st, "escape"), CopyModeKey::Exit);
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
