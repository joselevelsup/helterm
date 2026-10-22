use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use gpui::{
    Context, Entity, FocusHandle, Focusable, KeyDownEvent, MouseButton, MouseDownEvent, Render,
    ScrollHandle, Window, div, prelude::*, px, rgb,
};

use crate::config::{Config, UiColors, ui};
use crate::terminal::TerminalView;

/// A file operation waiting for a name or a confirmation.
enum Op {
    Create { dir: PathBuf, folder: bool },
    Rename { path: PathBuf },
    Delete { path: PathBuf },
}

struct Prompt {
    op: Op,
    input: String,
    /// Byte offset of the cursor in `input`, always on a char boundary.
    cursor: usize,
    error: Option<String>,
}

impl Prompt {
    fn insert(&mut self, text: &str) {
        self.input.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    fn prev_boundary(&self) -> usize {
        self.input[..self.cursor].char_indices().next_back().map_or(0, |(i, _)| i)
    }

    fn next_boundary(&self) -> usize {
        self.cursor + self.input[self.cursor..].chars().next().map_or(0, char::len_utf8)
    }

    fn backspace(&mut self) {
        let start = self.prev_boundary();
        self.input.drain(start..self.cursor);
        self.cursor = start;
    }

    fn delete(&mut self) {
        let end = self.next_boundary();
        self.input.drain(self.cursor..end);
    }
}

/// A non-empty relative name that stays inside its directory.
fn checked_name(input: &str, allow_nested: bool) -> io::Result<&Path> {
    let path = Path::new(input.trim().trim_end_matches('/'));
    let mut components = path.components();
    let ok = components.clone().count() > 0
        && components.all(|c| matches!(c, Component::Normal(_)))
        && (allow_nested || path.components().count() == 1);
    if ok {
        Ok(path)
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid name"))
    }
}

fn already_exists(path: &Path) -> io::Error {
    let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
    io::Error::new(io::ErrorKind::AlreadyExists, format!("{name} already exists"))
}

/// Run a file operation. Returns the path to select afterwards, if any.
/// Never overwrites an existing file or folder.
fn apply(op: &Op, input: &str) -> io::Result<Option<PathBuf>> {
    match op {
        Op::Create { dir, folder } => {
            let path = dir.join(checked_name(input, true)?);
            if path.symlink_metadata().is_ok() {
                return Err(already_exists(&path));
            }
            if *folder || input.trim().ends_with('/') {
                fs::create_dir_all(&path)?;
            } else {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::File::create_new(&path)?;
            }
            Ok(Some(path))
        }
        Op::Rename { path } => {
            let target = path.with_file_name(checked_name(input, false)?);
            if target != *path {
                if target.symlink_metadata().is_ok() {
                    return Err(already_exists(&target));
                }
                fs::rename(path, &target)?;
            }
            Ok(Some(target))
        }
        Op::Delete { path } => {
            // symlink_metadata: a link to a folder removes the link, not the folder.
            if fs::symlink_metadata(path)?.is_dir() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
            Ok(None)
        }
    }
}

struct Row {
    path: PathBuf,
    name: String,
    is_dir: bool,
    depth: usize,
}

/// File tree panel shown next to editor containers.
pub struct FileExplorer {
    root: PathBuf,
    expanded: HashSet<PathBuf>,
    /// Visible tree, flattened. Rebuilt on expand/collapse, never during render.
    // shortcut: no fs watching, new files show up after collapse/expand or reopening
    rows: Vec<Row>,
    selected: usize,
    editor: Entity<TerminalView>,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    prompt: Option<Prompt>,
    /// Show dotfiles (toggled with `.`).
    show_hidden: bool,
}

impl FileExplorer {
    pub fn new(root: PathBuf, editor: Entity<TerminalView>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            root,
            expanded: HashSet::new(),
            rows: vec![],
            selected: 0,
            editor,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            prompt: None,
            show_hidden: false,
        };
        this.rebuild();
        this
    }

    fn rebuild(&mut self) {
        let mut rows = vec![];
        collect(&self.root, 0, &self.expanded, self.show_hidden, &mut rows);
        self.rows = rows;
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }

    fn select(&mut self, ix: usize) {
        self.selected = ix.min(self.rows.len().saturating_sub(1));
        self.scroll.scroll_to_item(self.selected);
    }

    fn set_expanded(&mut self, ix: usize, expanded: bool) {
        let Some(row) = self.rows.get(ix).filter(|r| r.is_dir) else {
            return;
        };
        let path = row.path.clone();
        if expanded {
            self.expanded.insert(path);
        } else {
            self.expanded.remove(&path);
        }
        self.rebuild();
    }

    /// Enter / click: toggle a directory, or open a file in the editor and focus it.
    fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(ix) else {
            return;
        };
        if row.is_dir {
            let open = self.expanded.contains(&row.path);
            self.set_expanded(ix, !open);
        } else {
            let editor = self.editor.read(cx);
            // shortcut: keystroke injection into hx; breaks if hx is mid-prompt or in
            // insert mode, switch to a proper channel when needed
            editor.send_line(&format!(":open {}", row.path.display()));
            editor.focus_handle(cx).focus(window);
        }
    }

    fn start(&mut self, op: Op, input: String) {
        self.prompt = Some(Prompt {
            op,
            cursor: input.len(),
            input,
            error: None,
        });
    }

    /// Folder new entries go into: the selected folder, or the selected file's folder.
    fn target_dir(&self) -> PathBuf {
        match self.rows.get(self.selected) {
            Some(row) if row.is_dir => row.path.clone(),
            Some(row) => row.path.parent().unwrap_or(&self.root).to_path_buf(),
            None => self.root.clone(),
        }
    }

    fn confirm(&mut self) {
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        match apply(&prompt.op, &prompt.input) {
            Err(err) => prompt.error = Some(err.to_string()),
            Ok(select) => {
                self.prompt = None;
                // Reveal the result: expand every folder above it.
                if let Some(path) = &select {
                    let mut parent = path.parent();
                    while let Some(dir) = parent.filter(|d| d.starts_with(&self.root) && *d != self.root) {
                        self.expanded.insert(dir.to_path_buf());
                        parent = dir.parent();
                    }
                }
                self.rebuild();
                if let Some(ix) = select.and_then(|path| self.rows.iter().position(|r| r.path == path)) {
                    self.select(ix);
                }
            }
        }
    }

    fn on_prompt_key(&mut self, event: &KeyDownEvent) {
        let ks = &event.keystroke;
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        if matches!(prompt.op, Op::Delete { .. }) {
            match ks.key.as_str() {
                "y" => self.confirm(),
                "n" | "escape" => self.prompt = None,
                _ => {}
            }
            return;
        }
        match ks.key.as_str() {
            "escape" => self.prompt = None,
            "enter" => self.confirm(),
            "backspace" => prompt.backspace(),
            "delete" => prompt.delete(),
            "left" => prompt.cursor = prompt.prev_boundary(),
            "right" => prompt.cursor = prompt.next_boundary(),
            "home" => prompt.cursor = 0,
            "end" => prompt.cursor = prompt.input.len(),
            "space" => prompt.insert(" "),
            _ if !(ks.modifiers.control || ks.modifiers.alt || ks.modifiers.platform) => {
                if let Some(text) = ks.key_char.as_deref() {
                    prompt.insert(text);
                }
            }
            _ => {}
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.prompt.is_some() {
            self.on_prompt_key(event);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        let ks = &event.keystroke;
        if ks.modifiers.control || ks.modifiers.alt || ks.modifiers.platform {
            return;
        }
        let ix = self.selected;
        match ks.key.as_str() {
            "a" | "A" => {
                let folder = ks.modifiers.shift || ks.key == "A";
                self.start(Op::Create { dir: self.target_dir(), folder }, String::new())
            }
            "r" | "d" => {
                let Some(row) = self.rows.get(ix) else { return };
                let path = row.path.clone();
                match ks.key.as_str() {
                    "r" => self.start(Op::Rename { path }, row.name.clone()),
                    _ => self.start(Op::Delete { path }, String::new()),
                }
            }
            "." | "period" => {
                // Keep the selection on the same entry when rows appear/disappear.
                let selected = self.rows.get(ix).map(|row| row.path.clone());
                self.show_hidden = !self.show_hidden;
                self.rebuild();
                if let Some(ix) = selected.and_then(|path| self.rows.iter().position(|r| r.path == path)) {
                    self.select(ix);
                }
            }
            "j" | "down" => self.select(ix + 1),
            "k" | "up" => self.select(ix.saturating_sub(1)),
            "g" if ks.modifiers.shift => self.select(usize::MAX),
            "g" | "home" => self.select(0),
            "end" => self.select(usize::MAX),
            "l" | "right" => match self.rows.get(ix) {
                Some(row) if row.is_dir => self.set_expanded(ix, true),
                Some(_) => self.activate(ix, window, cx),
                None => {}
            },
            "h" | "left" => {
                let Some(row) = self.rows.get(ix) else { return };
                if row.is_dir && self.expanded.contains(&row.path) {
                    self.set_expanded(ix, false);
                } else if let Some(parent) = (0..ix)
                    .rev()
                    .find(|&i| self.rows[i].depth + 1 == row.depth)
                {
                    self.select(parent);
                }
            }
            "enter" | "space" => self.activate(ix, window, cx),
            "escape" => self.editor.read(cx).focus_handle(cx).focus(window),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Focusable for FileExplorer {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// Dirs first, then files, skipping hidden and build junk.
fn collect(dir: &Path, depth: usize, expanded: &HashSet<PathBuf>, show_hidden: bool, out: &mut Vec<Row>) {
    let mut entries: Vec<Row> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?.to_owned();
            let hidden = name.starts_with('.') && !show_hidden;
            let skip = hidden || matches!(name.as_str(), "target" | "node_modules");
            (!skip).then(|| Row {
                is_dir: path.is_dir(),
                path,
                name,
                depth,
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    for row in entries {
        let recurse = row.is_dir && expanded.contains(&row.path);
        let path = row.path.clone();
        out.push(row);
        if recurse {
            collect(&path, depth + 1, expanded, show_hidden, out);
        }
    }
}

impl Render for FileExplorer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = ui(cx);
        let focused = self.focus_handle.is_focused(window);
        let rows = self.rows.iter().enumerate().map(|(ix, row)| {
            let icon = match (row.is_dir, self.expanded.contains(&row.path)) {
                (true, true) => "▾",
                (true, false) => "▸",
                _ => " ",
            };
            let selected = ix == self.selected;
            div()
                .id(ix)
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .h(px(22.))
                .pr_2()
                .pl(px(8. + row.depth as f32 * 12.))
                .text_color(rgb(if row.is_dir { t.accent } else { t.text }))
                .when(selected, |el| {
                    el.bg(rgb(if focused { t.border } else { t.element }))
                })
                .hover(|el| el.bg(rgb(t.element)))
                .child(div().flex_none().w(px(12.)).child(icon))
                .child(row.name.clone())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        this.selected = ix;
                        this.focus_handle.focus(window);
                        this.activate(ix, window, cx);
                        cx.notify();
                    }),
                )
        });

        div()
            .flex()
            .flex_none()
            .flex_col()
            .w(px(240.))
            .h_full()
            .bg(rgb(t.panel))
            .border_1()
            .border_color(rgb(if focused { t.accent } else { t.element }))
            .rounded_md()
            .font_family(cx.global::<Config>().font_family.clone())
            .text_size(px(13.))
            .key_context("FileExplorer")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key))
            .child(
                div()
                    .flex_none()
                    .px_2()
                    .py_2()
                    .flex()
                    .justify_between()
                    .text_color(rgb(t.accent))
                    .child("EXPLORER")
                    .when(self.show_hidden, |el| {
                        el.child(div().text_color(rgb(t.muted)).child("+hidden"))
                    }),
            )
            .child(
                div()
                    .id("explorer-rows")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(rows),
            )
            .child(self.render_footer(t))
    }
}

impl FileExplorer {
    fn relative(&self, path: &Path) -> String {
        match path.strip_prefix(&self.root) {
            Ok(rel) if rel.as_os_str().is_empty() => "./".to_owned(),
            Ok(rel) => format!("{}/", rel.display()),
            Err(_) => path.display().to_string(),
        }
    }

    fn render_footer(&self, t: UiColors) -> gpui::Div {
        let footer = div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .border_t_1()
            .border_color(rgb(t.element));
        let Some(prompt) = &self.prompt else {
            return footer
                .text_size(px(11.))
                .text_color(rgb(t.muted))
                .child("a new file    A new folder")
                .child("r rename      d delete")
                .child(". hidden files");
        };
        let name = |path: &Path| path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let label = match &prompt.op {
            Op::Create { dir, folder: false } => format!("New file in {}", self.relative(dir)),
            Op::Create { dir, folder: true } => format!("New folder in {}", self.relative(dir)),
            Op::Rename { path } => format!("Rename {}", name(path)),
            Op::Delete { path } => format!("Delete {} permanently? y/n", name(path)),
        };
        footer
            .child(div().text_color(rgb(t.accent)).child(label))
            .when(!matches!(prompt.op, Op::Delete { .. }), |el| {
                el.child(
                    div()
                        .px_1()
                        .rounded_sm()
                        .bg(rgb(t.sidebar))
                        .border_1()
                        .border_color(rgb(t.border))
                        .flex()
                        .items_center()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(prompt.input[..prompt.cursor].to_owned())
                        // A thin bar instead of a glyph, so it takes no text width.
                        .child(div().flex_none().w(px(1.5)).h(px(15.)).bg(rgb(t.accent)))
                        .child(prompt.input[prompt.cursor..].to_owned()),
                )
            })
            .when_some(prompt.error.clone(), |el, error| {
                el.child(div().text_color(rgb(t.error)).child(error))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vibe_term_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn create(dir: &Path, input: &str, folder: bool) -> io::Result<Option<PathBuf>> {
        apply(&Op::Create { dir: dir.to_path_buf(), folder }, input)
    }

    #[test]
    fn prompt_cursor_edits_in_place() {
        let mut p = Prompt {
            op: Op::Rename { path: PathBuf::new() },
            input: "café.rs".into(),
            cursor: "café.rs".len(),
            error: None,
        };
        for _ in 0..3 {
            p.cursor = p.prev_boundary(); // left past ".rs"
        }
        p.backspace(); // removes the multi-byte "é"
        assert_eq!((p.input.as_str(), p.cursor), ("caf.rs", 3));
        p.insert("e");
        p.delete(); // removes "."
        assert_eq!(p.input, "cafers");
        p.cursor = 0;
        p.backspace(); // no-op at the start
        p.cursor = p.input.len();
        p.delete(); // no-op at the end
        p.cursor = p.next_boundary(); // stays at the end
        assert_eq!((p.input.as_str(), p.cursor), ("cafers", 6));
    }

    #[test]
    fn create_files_and_folders_without_overwriting() {
        let dir = temp_dir("create");
        create(&dir, "a.txt", false).unwrap();
        fs::write(dir.join("a.txt"), "keep me").unwrap();
        assert!(create(&dir, "a.txt", false).is_err());
        assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "keep me");

        create(&dir, "folder", true).unwrap();
        create(&dir, "slash/", false).unwrap();
        create(&dir, "deep/er/file.rs", false).unwrap();
        assert!(dir.join("folder").is_dir());
        assert!(dir.join("slash").is_dir());
        assert!(dir.join("deep/er/file.rs").is_file());

        for bad in ["", "  ", "../escape", "/abs", "a/../../b"] {
            assert!(create(&dir, bad, false).is_err(), "{bad:?}");
        }
        assert!(!dir.parent().unwrap().join("escape").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rename_refuses_to_clobber_or_move() {
        let dir = temp_dir("rename");
        fs::write(dir.join("a"), "a").unwrap();
        fs::write(dir.join("b"), "b").unwrap();
        let rename = |input: &str| apply(&Op::Rename { path: dir.join("a") }, input);

        assert!(rename("b").is_err());
        assert_eq!(fs::read_to_string(dir.join("b")).unwrap(), "b");
        assert!(rename("sub/c").is_err());
        assert!(rename("../c").is_err());
        assert_eq!(rename("c").unwrap(), Some(dir.join("c")));
        assert_eq!(fs::read_to_string(dir.join("c")).unwrap(), "a");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn delete_removes_folders_but_not_symlink_targets() {
        let dir = temp_dir("delete");
        create(&dir, "tree/inner/file", false).unwrap();
        let target = dir.join("target_dir");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("precious"), "x").unwrap();
        std::os::unix::fs::symlink(&target, dir.join("link")).unwrap();

        apply(&Op::Delete { path: dir.join("tree") }, "").unwrap();
        apply(&Op::Delete { path: dir.join("link") }, "").unwrap();
        assert!(!dir.join("tree").exists());
        assert!(!dir.join("link").exists());
        assert!(target.join("precious").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
