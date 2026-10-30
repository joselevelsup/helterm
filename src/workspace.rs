use std::collections::HashMap;
use std::path::PathBuf;

use gpui::{
    App, Context, Div, Entity, EntityId, FocusHandle, Hsla, Focusable, KeyDownEvent, MouseButton,
    MouseDownEvent, PathPromptOptions, Render, SharedString, Stateful, Window, actions, div, hsla,
    prelude::*, px, relative, rgb,
};

use crate::config::{Config, UiColors, ui};
use crate::container::{Axis, FocusDir, Node, Placement};
use crate::explorer::FileExplorer;
use crate::terminal::{Exited, TerminalKind, TerminalView};

actions!(
    workspace,
    [
        SplitRight,
        SplitDown,
        NewEditor,
        CloseContainer,
        FocusLeft,
        FocusRight,
        FocusUp,
        FocusDown,
        ToggleExplorer,
        NewTab,
        NextTab,
        PrevTab,
        ToggleSidebar,
        ToggleMaximize,
        ToggleManage,
        ToggleFloating,
        FloatFocused,
        CloseWorkspace,
        NewWorkspace,
        NextWorkspace,
        PrevWorkspace,
    ]
);

/// Open, show or hide the floating terminal of `[[floating_commands]]` entry `index`.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct RunFloatingCommand {
    pub index: usize,
}

struct Tab {
    /// Stable label; positions change when tabs are moved.
    number: usize,
    /// Split tree of containers; `None` for an empty tab waiting for the user
    /// to pick a terminal or an editor.
    root: Option<Node<Entity<TerminalView>>>,
    /// File explorers are attached to editor containers only.
    explorers: HashMap<EntityId, Entity<FileExplorer>>,
    /// Container to refocus when this tab becomes active again.
    last_focus: Option<EntityId>,
    /// Container filling the whole tab; the split tree underneath is untouched,
    /// so un-maximizing restores the original layout.
    maximized: Option<EntityId>,
    /// Terminals floating over the tab, outside the split tree: at most one per
    /// slot, and at most one visible.
    floats: Vec<Floating>,
}

impl Tab {
    fn visible_float(&self) -> Option<&Floating> {
        self.floats.iter().find(|f| f.visible)
    }

    fn float_index(&self, id: EntityId) -> Option<usize> {
        self.floats.iter().position(|f| f.terminal.entity_id() == id)
    }

    /// Show the float at `ix` (hiding the others), or hide all with `None`.
    fn show_only(&mut self, ix: Option<usize>) {
        for (i, float) in self.floats.iter_mut().enumerate() {
            float.visible = Some(i) == ix;
        }
    }
}

/// What a floating terminal belongs to, i.e. which key toggles it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FloatSlot {
    /// Alt+F's shell (and containers floated with Alt+Shift+F).
    Shell,
    /// Entry of `[[floating_commands]]` in the config.
    Command(usize),
}

struct Floating {
    terminal: Entity<TerminalView>,
    slot: FloatSlot,
    visible: bool,
    /// Its spot in the split tree, if it was floated out of it.
    home: Option<Placement>,
}

/// One entry in the workspace list: a folder with its own tabs.
struct Project {
    /// Folder name.
    base: String,
    /// Tells apart workspaces with the same folder name: "home", "home 2", ...
    dup: usize,
    root: PathBuf,
    tabs: Vec<Tab>,
    active: usize,
    next_tab_number: usize,
}

impl Project {
    fn name(&self) -> String {
        if self.dup > 1 {
            format!("{} {}", self.base, self.dup)
        } else {
            self.base.clone()
        }
    }

    /// Slack-style badge for the collapsed sidebar: initial (+ number if duplicated).
    fn badge(&self) -> String {
        let mut badge: String = self.base.chars().next().into_iter().flat_map(char::to_uppercase).collect();
        if self.dup > 1 {
            badge.push_str(&self.dup.to_string());
        }
        badge
    }
}

/// Management mode (Alt+M): pick a container, then resize or move it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ManageMode {
    Select,
    Resize,
    Move,
}

struct Manage {
    mode: ManageMode,
    /// The container being managed; h/j/k/l moves this in Select mode.
    selected: EntityId,
    /// Holds keyboard focus so keys drive the mode instead of the terminals.
    focus_handle: FocusHandle,
}

/// State of the "Close workspace?" confirmation.
struct ConfirmClose {
    project: usize,
    focus_handle: FocusHandle,
}

/// State of the "New workspace" modal.
struct NewWorkspaceModal {
    folder: Option<PathBuf>,
    error: Option<String>,
    focus_handle: FocusHandle,
}

pub struct Workspace {
    projects: Vec<Project>,
    active: usize,
    /// Slack-style narrow sidebar showing only workspace initials.
    sidebar_collapsed: bool,
    new_workspace: Option<NewWorkspaceModal>,
    confirm_close: Option<ConfirmClose>,
    manage: Option<Manage>,
    /// The user closed the problems banner.
    config_warnings_dismissed: bool,
    /// Problems loading/saving the workspace list, shown in the same banner.
    state_warnings: Vec<String>,
    /// Title last sent to the window manager, to only send changes.
    window_title: String,
    /// Focus target of the empty-tab screen, so keybindings work with no containers.
    empty_focus: FocusHandle,
}

/// How much one resize key press grows / shrinks a pane (share of its split).
const RESIZE_STEP: f32 = 0.05;

/// Space around and between panes (outer padding).
const PANE_GAP: f32 = 6.;

/// Index of the active item once item `closed` is removed and `len` remain.
/// Closing the active item selects its right neighbour, or the new last item.
fn active_after_close(active: usize, closed: usize, len: usize) -> usize {
    if closed < active {
        active - 1
    } else {
        active.min(len - 1)
    }
}

/// Smallest duplicate number (1 = no suffix) not used by a same-named workspace.
fn next_dup(taken: &[usize]) -> usize {
    (1..).find(|n| !taken.contains(n)).unwrap()
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Saved {
    #[serde(default)]
    workspace: Vec<SavedWorkspace>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedWorkspace {
    path: PathBuf,
}

/// Saved workspace folders in sidebar order; a missing file means none.
fn load_saved(path: &std::path::Path) -> Result<Vec<PathBuf>, String> {
    match std::fs::read_to_string(path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(err) => Err(err.to_string()),
        Ok(text) => toml::from_str::<Saved>(&text)
            .map(|saved| saved.workspace.into_iter().map(|w| w.path).collect())
            .map_err(|err| err.message().to_owned()),
    }
}

/// Write via a temp file + rename so a crash never leaves a half-written list.
fn write_saved(path: &std::path::Path, folders: &[PathBuf]) -> std::io::Result<()> {
    let saved = Saved {
        workspace: folders.iter().map(|path| SavedWorkspace { path: path.clone() }).collect(),
    };
    let text = toml::to_string(&saved).map_err(std::io::Error::other)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)
}

fn home_dir() -> PathBuf {
    std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

impl Workspace {
    /// Opens the saved workspaces (each with an empty tab), or none at all.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            projects: vec![],
            active: 0,
            sidebar_collapsed: false,
            new_workspace: None,
            confirm_close: None,
            manage: None,
            config_warnings_dismissed: false,
            state_warnings: vec![],
            window_title: String::new(),
            empty_focus: cx.focus_handle(),
        };
        if let Some(path) = crate::config::state_path() {
            match load_saved(&path) {
                Ok(folders) => {
                    for folder in folders {
                        this.open_project(folder, window, cx);
                    }
                }
                Err(err) => {
                    // Keep the unreadable file instead of overwriting it on the next save.
                    let backup = path.with_extension("toml.broken");
                    let moved = std::fs::rename(&path, &backup).is_ok();
                    this.state_warnings.push(match moved {
                        true => format!("{}: {err} (moved to {})", path.display(), backup.display()),
                        false => format!("{}: {err}", path.display()),
                    });
                }
            }
        }
        this.active = 0;
        this.focus_active_tab(window, cx);
        this
    }

    /// Remember the workspace list (folders in sidebar order) for the next start.
    fn save(&mut self) {
        let Some(path) = crate::config::state_path() else {
            return;
        };
        let folders: Vec<PathBuf> = self.projects.iter().map(|p| p.root.clone()).collect();
        if let Err(err) = write_saved(&path, &folders) {
            self.state_warnings.push(format!("could not save workspaces to {}: {err}", path.display()));
            self.config_warnings_dismissed = false;
        }
    }

    fn project(&self) -> &Project {
        &self.projects[self.active]
    }

    fn project_mut(&mut self) -> &mut Project {
        &mut self.projects[self.active]
    }

    fn tab(&self) -> &Tab {
        let project = self.project();
        &project.tabs[project.active]
    }

    fn tab_mut(&mut self) -> &mut Tab {
        let project = self.project_mut();
        &mut project.tabs[project.active]
    }

    /// The container owning keyboard focus in the active tab. A focused explorer
    /// counts as its editor.
    fn focused_id(&self, window: &Window, cx: &App) -> Option<EntityId> {
        let focused = window.focused(cx)?;
        let project = self.projects.get(self.active)?;
        let tab = project.tabs.get(project.active)?;
        if let Some((id, _)) = tab
            .explorers
            .iter()
            .find(|(_, explorer)| explorer.read(cx).focus_handle(cx) == focused)
        {
            return Some(*id);
        }
        tab.root
            .iter()
            .flat_map(|root| root.leaves())
            .chain(tab.floats.iter().map(|f| &f.terminal))
            .find(|leaf| leaf.read(cx).focus_handle(cx) == focused)
            .map(|leaf| leaf.entity_id())
    }

    /// A container of the active tab, tiled or floating.
    fn find_container(&self, id: EntityId) -> Option<Entity<TerminalView>> {
        let tab = self.tab();
        let tiled = tab.root.as_ref().and_then(|root| root.find_leaf(id.as_u64()));
        tiled
            .or_else(|| tab.floats.iter().map(|f| &f.terminal).find(|t| t.entity_id() == id))
            .cloned()
    }

    /// Whether `id` is tiled in the active tab (splitting/maximizing only apply there).
    fn is_tiled(&self, id: EntityId) -> bool {
        self.tab().root.as_ref().is_some_and(|root| root.contains(id.as_u64()))
    }

    /// Spawn a terminal in the active workspace's folder.
    fn new_terminal(
        &self,
        kind: TerminalKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TerminalView> {
        let cwd = self.project().root.clone();
        let terminal = cx.new(|cx| TerminalView::new(kind, cwd, cx));
        cx.subscribe_in(&terminal, window, |this, terminal, _: &Exited, window, cx| {
            this.remove_container(terminal.entity_id(), window, cx);
        })
        .detach();
        terminal
    }

    fn new_explorer(&self, editor: Entity<TerminalView>, cx: &mut Context<Self>) -> Entity<FileExplorer> {
        let root = self.project().root.clone();
        cx.new(|cx| FileExplorer::new(root, editor, cx))
    }

    /// Store the focused container on its tab so switching back restores it.
    fn remember_focus(&mut self, window: &Window, cx: &App) {
        if let Some(current) = self.focused_id(window, cx).filter(|id| self.is_tiled(*id)) {
            self.tab_mut().last_focus = Some(current);
        }
    }

    /// Focus the visible floating terminal, else the last focused tiled container.
    fn focus_active_tab(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.projects.is_empty() {
            self.empty_focus.focus(window);
            return;
        }
        match self.tab().visible_float() {
            Some(f) => f.terminal.read(cx).focus_handle(cx).focus(window),
            None => self.focus_tiled(window, cx),
        }
    }

    fn focus_tiled(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.tab().root.as_ref() else {
            self.empty_focus.focus(window);
            return;
        };
        let leaf = self
            .tab()
            .last_focus
            .and_then(|id| root.find_leaf(id.as_u64()))
            .or_else(|| root.first_leaf());
        if let Some(leaf) = leaf {
            leaf.read(cx).focus_handle(cx).focus(window);
        }
    }

    // Workspaces ---------------------------------------------------------------

    /// Add a workspace for `root` at the end of the list and switch to it.
    fn open_project(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let base = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        let taken: Vec<usize> = self
            .projects
            .iter()
            .filter(|p| p.base == base)
            .map(|p| p.dup)
            .collect();
        self.remember_focus(window, cx);
        self.projects.push(Project {
            dup: next_dup(&taken),
            base,
            root,
            tabs: vec![],
            active: 0,
            next_tab_number: 1,
        });
        self.active = self.projects.len() - 1;
        self.open_tab(window, cx);
    }

    fn activate_project(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.remember_focus(window, cx);
        self.active = ix;
        self.focus_active_tab(window, cx);
        cx.notify();
    }

    /// Close a workspace, killing its processes. Closing the last one quits.
    fn close_project(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let was_active = ix == self.active;
        // Dropping the workspace drops its terminals, which kills their processes.
        self.projects.remove(ix);
        self.active = match self.projects.len() {
            0 => 0,
            len => active_after_close(self.active, ix, len),
        };
        if was_active {
            self.focus_active_tab(window, cx);
        }
        self.save();
        cx.notify();
    }

    fn new_workspace(&mut self, _: &NewWorkspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_workspace.is_none() && self.confirm_close.is_none() {
            self.open_new_workspace_modal(window, cx);
        }
    }

    fn next_workspace(&mut self, _: &NextWorkspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.projects.len() > 1 {
            self.activate_project((self.active + 1) % self.projects.len(), window, cx);
        }
    }

    fn prev_workspace(&mut self, _: &PrevWorkspace, window: &mut Window, cx: &mut Context<Self>) {
        let len = self.projects.len();
        if len > 1 {
            self.activate_project((self.active + len - 1) % len, window, cx);
        }
    }

    fn close_workspace(&mut self, _: &CloseWorkspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.projects.is_empty() || self.new_workspace.is_some() || self.confirm_close.is_some() {
            return;
        }
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window);
        self.confirm_close = Some(ConfirmClose {
            project: self.active,
            focus_handle,
        });
        cx.notify();
    }

    fn confirm_close_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(modal) = self.confirm_close.take() {
            self.close_project(modal.project, window, cx);
        }
    }

    fn cancel_close_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_close = None;
        self.focus_active_tab(window, cx);
        cx.notify();
    }

    fn open_new_workspace_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window);
        self.new_workspace = Some(NewWorkspaceModal {
            folder: None,
            error: None,
            focus_handle,
        });
        cx.notify();
    }

    /// Ask the desktop's native folder picker (xdg-desktop-portal on Linux).
    fn browse_for_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open workspace".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = picked.await;
            this.update_in(cx, |this, window, cx| {
                let Some(modal) = this.new_workspace.as_mut() else {
                    return;
                };
                match result {
                    Ok(Ok(Some(paths))) => modal.folder = paths.into_iter().next(),
                    Ok(Err(err)) => modal.error = Some(format!("Folder picker unavailable: {err}")),
                    _ => {} // cancelled: keep whatever was chosen before
                }
                modal.focus_handle.focus(window);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn confirm_new_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.new_workspace.take() else {
            return;
        };
        self.open_project(modal.folder.unwrap_or_else(home_dir), window, cx);
        self.save();
    }

    fn cancel_new_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_workspace = None;
        self.focus_active_tab(window, cx);
        cx.notify();
    }

    // Tabs ---------------------------------------------------------------------

    /// Open an empty tab at the end of the active workspace's tabs.
    fn open_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let project = self.project_mut();
        project.tabs.push(Tab {
            number: project.next_tab_number,
            root: None,
            explorers: HashMap::new(),
            last_focus: None,
            maximized: None,
            floats: vec![],
        });
        project.next_tab_number += 1;
        let last = project.tabs.len() - 1;
        self.activate_tab(last, window, cx);
    }

    fn activate_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.project().active < self.project().tabs.len() {
            self.remember_focus(window, cx);
        }
        self.project_mut().active = ix;
        self.focus_active_tab(window, cx);
        cx.notify();
    }

    /// Close a tab, killing its processes. A workspace's only tab is emptied
    /// instead, back to the "terminal or editor?" screen.
    fn close_tab(&mut self, project_ix: usize, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let project = &mut self.projects[project_ix];
        let was_active = project_ix == self.active && ix == project.active;
        // Dropping containers drops their terminals, which kills their processes.
        if project.tabs.len() == 1 {
            let tab = &mut project.tabs[0];
            tab.root = None;
            tab.explorers.clear();
            tab.floats.clear();
            tab.maximized = None;
            tab.last_focus = None;
        } else {
            project.tabs.remove(ix);
            project.active = active_after_close(project.active, ix, project.tabs.len());
        }
        if was_active {
            self.focus_active_tab(window, cx);
        }
        cx.notify();
    }

    fn new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        self.open_tab(window, cx);
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        let project = self.project();
        self.activate_tab((project.active + 1) % project.tabs.len(), window, cx);
    }

    fn prev_tab(&mut self, _: &PrevTab, window: &mut Window, cx: &mut Context<Self>) {
        let project = self.project();
        let len = project.tabs.len();
        self.activate_tab((project.active + len - 1) % len, window, cx);
    }

    // Containers ---------------------------------------------------------------

    /// Remove a container from whichever workspace/tab holds it. Removing a
    /// tab's last container closes the tab.
    fn remove_container(&mut self, id: EntityId, window: &mut Window, cx: &mut Context<Self>) {
        let found = self.projects.iter().enumerate().find_map(|(p, project)| {
            let t = project.tabs.iter().position(|t| {
                t.root.as_ref().is_some_and(|r| r.contains(id.as_u64())) || t.float_index(id).is_some()
            })?;
            Some((p, t))
        });
        let Some((p, t)) = found else {
            return;
        };
        let was_focused = self.focused_id(window, cx) == Some(id);
        let tab = &mut self.projects[p].tabs[t];
        if let Some(ix) = tab.float_index(id) {
            // e.g. lazygit quit: its floating terminal goes away.
            tab.floats.remove(ix);
            tab.explorers.remove(&id);
            if was_focused {
                self.focus_tiled(window, cx);
            }
            cx.notify();
            return;
        }
        if matches!(tab.root, Some(Node::Leaf(_))) {
            if tab.floats.is_empty() {
                self.close_tab(p, t, window, cx);
                return;
            }
            // Floating terminals keep the tab alive, over the empty-tab screen.
            tab.root = None;
            tab.explorers.remove(&id);
            tab.maximized = None;
            if was_focused {
                self.focus_active_tab(window, cx);
            }
            cx.notify();
            return;
        }
        tab.explorers.remove(&id);
        if tab.maximized == Some(id) {
            tab.maximized = None;
        }
        if let Some(root) = tab.root.as_mut() {
            root.remove(id.as_u64());
            if was_focused {
                if let Some(leaf) = root.first_leaf() {
                    leaf.read(cx).focus_handle(cx).focus(window);
                }
            }
        }
        cx.notify();
    }

    /// Add a container next to the focused tiled one, or as the first one of an
    /// empty tab.
    fn add_container(&mut self, kind: TerminalKind, axis: Axis, window: &mut Window, cx: &mut Context<Self>) {
        let anchor = self.focused_id(window, cx).filter(|id| self.is_tiled(*id));
        if self.tab().root.is_some() && anchor.is_none() {
            return; // e.g. the floating terminal is focused
        }
        let container = self.new_terminal(kind, window, cx);
        let tab = self.tab_mut();
        // The new container would be hidden behind a maximized one.
        tab.maximized = None;
        match anchor {
            Some(id) => {
                if let Some(root) = tab.root.as_mut() {
                    root.split_focused(id.as_u64(), axis, container.clone());
                }
            }
            None => tab.root = Some(Node::Leaf(container.clone())),
        }
        container.read(cx).focus_handle(cx).focus(window);
        cx.notify();
    }

    fn split(&mut self, axis: Axis, window: &mut Window, cx: &mut Context<Self>) {
        self.add_container(TerminalKind::Terminal, axis, window, cx);
    }

    fn split_right(&mut self, _: &SplitRight, window: &mut Window, cx: &mut Context<Self>) {
        self.split(Axis::Row, window, cx);
    }

    fn split_down(&mut self, _: &SplitDown, window: &mut Window, cx: &mut Context<Self>) {
        self.split(Axis::Col, window, cx);
    }

    fn new_editor(&mut self, _: &NewEditor, window: &mut Window, cx: &mut Context<Self>) {
        self.add_container(TerminalKind::Editor, Axis::Row, window, cx);
    }

    fn close_focused(&mut self, _: &CloseContainer, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.focused_id(window, cx) {
            self.remove_container(id, window, cx);
        }
    }

    fn focus_in(&mut self, dir: FocusDir, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.focused_id(window, cx) {
            let target = self.tab().root.as_ref().and_then(|root| root.focus_dir(id.as_u64(), dir));
            if let Some(leaf) = target.cloned() {
                // Moving to a hidden container leaves maximized mode (like tmux zoom).
                self.tab_mut().maximized = None;
                leaf.read(cx).focus_handle(cx).focus(window);
                cx.notify();
            }
        }
    }

    // Management mode ------------------------------------------------------------

    fn toggle_manage(&mut self, _: &ToggleManage, window: &mut Window, cx: &mut Context<Self>) {
        if self.manage.is_some() {
            self.exit_manage(window, cx);
            return;
        }
        if self.projects.is_empty() || self.new_workspace.is_some() || self.confirm_close.is_some() {
            return;
        }
        let Some(root) = self.tab().root.as_ref() else {
            return; // nothing tiled to manage
        };
        let first = root.first_leaf().map(|leaf| leaf.entity_id());
        let Some(selected) = self.focused_id(window, cx).filter(|id| self.is_tiled(*id)).or(first) else {
            return;
        };
        // Everything should be visible while rearranging.
        let tab = self.tab_mut();
        tab.maximized = None;
        tab.show_only(None);
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window);
        self.manage = Some(Manage {
            mode: ManageMode::Select,
            selected,
            focus_handle,
        });
        cx.notify();
    }

    /// Leave management mode, focusing the container that was selected.
    fn exit_manage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(manage) = self.manage.take() else {
            return;
        };
        match self.find_container(manage.selected) {
            Some(container) => container.read(cx).focus_handle(cx).focus(window),
            None => self.focus_active_tab(window, cx),
        }
        cx.notify();
    }

    fn manage_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(manage) = self.manage.as_ref() else {
            return;
        };
        let (mode, selected) = (manage.mode, manage.selected);
        let ks = &event.keystroke;
        if ks.modifiers.control || ks.modifiers.alt || ks.modifiers.platform {
            return;
        }
        let dir = match ks.key.as_str() {
            "h" | "left" => Some(FocusDir::Left),
            "j" | "down" => Some(FocusDir::Down),
            "k" | "up" => Some(FocusDir::Up),
            "l" | "right" => Some(FocusDir::Right),
            _ => None,
        };
        match (mode, ks.key.as_str(), dir) {
            (_, "enter", _) | (ManageMode::Select, "escape", _) => self.exit_manage(window, cx),
            (_, "escape", _) => self.set_manage_mode(ManageMode::Select),
            (ManageMode::Select, "r", _) => self.set_manage_mode(ManageMode::Resize),
            (ManageMode::Select, "m", _) => self.set_manage_mode(ManageMode::Move),
            (ManageMode::Select, "z", _) => {
                self.tab_mut().maximized = Some(selected);
                self.exit_manage(window, cx);
            }
            (ManageMode::Move, _, Some(dir)) => {
                // Swap with the neighbour on that side; the selection follows the
                // pane (it's tracked by id), so it can keep moving.
                if let Some(root) = self.tab_mut().root.as_mut() {
                    let neighbour = root.focus_dir(selected.as_u64(), dir).map(|leaf| leaf.entity_id());
                    if let Some(neighbour) = neighbour {
                        root.swap(selected.as_u64(), neighbour.as_u64());
                    }
                }
            }
            (ManageMode::Resize, _, Some(dir)) => {
                // Like dragging the pane's bottom-right corner.
                let (axis, delta) = match dir {
                    FocusDir::Right => (Axis::Row, RESIZE_STEP),
                    FocusDir::Left => (Axis::Row, -RESIZE_STEP),
                    FocusDir::Down => (Axis::Col, RESIZE_STEP),
                    FocusDir::Up => (Axis::Col, -RESIZE_STEP),
                };
                if let Some(root) = self.tab_mut().root.as_mut() {
                    root.resize(selected.as_u64(), axis, delta);
                }
            }
            (ManageMode::Select, _, Some(dir)) => {
                let next = self.tab().root.as_ref().and_then(|root| root.focus_dir(selected.as_u64(), dir));
                if let (Some(next), Some(manage)) = (next.map(|leaf| leaf.entity_id()), self.manage.as_mut()) {
                    manage.selected = next;
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn set_manage_mode(&mut self, mode: ManageMode) {
        if let Some(manage) = self.manage.as_mut() {
            manage.mode = mode;
        }
    }

    fn toggle_maximize(&mut self, _: &ToggleMaximize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.focused_id(window, cx).filter(|id| self.is_tiled(*id)) {
            let tab = self.tab_mut();
            tab.maximized = (tab.maximized != Some(id)).then_some(id);
            cx.notify();
        }
    }

    // Floating -------------------------------------------------------------------

    /// Alt+F: the tab's floating shell.
    fn toggle_floating(&mut self, _: &ToggleFloating, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_float(FloatSlot::Shell, window, cx);
    }

    /// A `[[floating_commands]]` key, e.g. Alt+G for lazygit.
    fn run_floating_command(&mut self, action: &RunFloatingCommand, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_float(FloatSlot::Command(action.index), window, cx);
    }

    /// Show the slot's floating terminal (starting it if needed), or hide it if
    /// it is the one showing. Only one is visible at a time.
    fn toggle_float(&mut self, slot: FloatSlot, window: &mut Window, cx: &mut Context<Self>) {
        self.remember_focus(window, cx);
        match self.tab().floats.iter().position(|f| f.slot == slot) {
            Some(ix) => {
                let tab = self.tab_mut();
                let show = !tab.floats[ix].visible;
                tab.show_only(show.then_some(ix));
            }
            None => {
                let kind = match slot {
                    FloatSlot::Shell => TerminalKind::Terminal,
                    FloatSlot::Command(index) => {
                        let commands = &cx.global::<Config>().floating_commands;
                        let Some(command) = commands.get(index) else {
                            return;
                        };
                        TerminalKind::Command(command.command.clone())
                    }
                };
                let terminal = self.new_terminal(kind, window, cx);
                let tab = self.tab_mut();
                tab.floats.push(Floating {
                    terminal,
                    slot,
                    visible: false,
                    home: None,
                });
                tab.show_only(Some(tab.floats.len() - 1));
            }
        }
        self.focus_active_tab(window, cx);
        cx.notify();
    }

    /// Float the focused tiled container, or put a floating one back.
    fn float_focused(&mut self, _: &FloatFocused, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.focused_id(window, cx) else {
            return;
        };
        if let Some(ix) = self.tab().float_index(id) {
            self.dock_float(ix, window, cx);
            return;
        }
        // It takes the Alt+F slot: whatever floats there goes back first.
        if let Some(ix) = self.tab().floats.iter().position(|f| f.slot == FloatSlot::Shell) {
            self.dock_float(ix, window, cx);
        }
        let tab = self.tab_mut();
        let Some(root) = tab.root.as_mut() else {
            return;
        };
        let Some(terminal) = root.find_leaf(id.as_u64()).cloned() else {
            return;
        };
        let home = root.placement(id.as_u64());
        if !root.remove(id.as_u64()) {
            tab.root = None; // it was the only tiled container
        }
        if tab.maximized == Some(id) {
            tab.maximized = None;
        }
        tab.floats.push(Floating {
            terminal,
            slot: FloatSlot::Shell,
            visible: false,
            home,
        });
        tab.show_only(Some(tab.floats.len() - 1));
        self.focus_active_tab(window, cx);
        cx.notify();
    }

    /// Put a floating terminal back into the split tree: where it came from if
    /// that spot still exists, else on the right edge.
    fn dock_float(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let tab = self.tab_mut();
        let floating = tab.floats.remove(ix);
        let terminal = floating.terminal.clone();
        let restored = match (tab.root.as_mut(), floating.home.as_ref()) {
            (Some(root), Some(home)) => root.restore(home, terminal.clone()),
            _ => false,
        };
        if !restored {
            tab.root = Some(match tab.root.take() {
                Some(old) => Node::Split {
                    axis: Axis::Row,
                    ratio: 0.5,
                    first: Box::new(old),
                    second: Box::new(Node::Leaf(terminal)),
                },
                None => Node::Leaf(terminal),
            });
        }
        // A maximized container would hide the one just docked.
        tab.maximized = None;
        floating.terminal.read(cx).focus_handle(cx).focus(window);
        cx.notify();
    }

    fn focus_left(&mut self, _: &FocusLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_in(FocusDir::Left, window, cx);
    }

    fn focus_right(&mut self, _: &FocusRight, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_in(FocusDir::Right, window, cx);
    }

    fn focus_up(&mut self, _: &FocusUp, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_in(FocusDir::Up, window, cx);
    }

    fn focus_down(&mut self, _: &FocusDown, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_in(FocusDir::Down, window, cx);
    }

    /// Editor focused: open (or jump to) its explorer. Explorer focused: close it.
    fn toggle_explorer(&mut self, _: &ToggleExplorer, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.focused_id(window, cx) else {
            return;
        };
        let Some(editor) = self.find_container(id) else {
            return;
        };
        if editor.read(cx).kind != TerminalKind::Editor {
            return;
        }
        match self.tab().explorers.get(&id).cloned() {
            Some(explorer) if explorer.read(cx).focus_handle(cx).is_focused(window) => {
                self.tab_mut().explorers.remove(&id);
                editor.update(cx, |editor, _| editor.explorer_open = false);
                editor.read(cx).focus_handle(cx).focus(window);
            }
            Some(explorer) => explorer.read(cx).focus_handle(cx).focus(window),
            None => {
                editor.update(cx, |editor, _| editor.explorer_open = true);
                let explorer = self.new_explorer(editor, cx);
                explorer.read(cx).focus_handle(cx).focus(window);
                self.tab_mut().explorers.insert(id, explorer);
            }
        }
        cx.notify();
    }

    // Rendering ----------------------------------------------------------------

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> Div {
        let t = ui(cx);
        let collapsed = self.sidebar_collapsed;

        let toggle = div()
            .id("toggle-sidebar")
            .flex_none()
            .px_1()
            .rounded_sm()
            .text_color(rgb(t.muted))
            .hover(|el| el.bg(rgb(t.element)).text_color(rgb(t.text)))
            .child(if collapsed { "»" } else { "«" })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    this.toggle_sidebar(&ToggleSidebar, window, cx)
                }),
            );

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .text_size(px(14.))
            .when(!collapsed, |el| {
                el.child(div().text_color(rgb(t.accent)).child("WORKSPACES"))
            })
            .child(toggle);

        let entries = self.projects.iter().enumerate().map(|(ix, project)| {
            let active = ix == self.active;
            let el = div()
                .id(("workspace", ix))
                .rounded_md()
                .bg(rgb(if active { t.element } else { t.sidebar }))
                .text_color(rgb(if active { t.text } else { t.muted }))
                .hover(|el| el.bg(rgb(t.element)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        this.activate_project(ix, window, cx)
                    }),
                );
            if collapsed {
                el.size(px(32.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .text_size(px(if project.dup > 1 { 12. } else { 16. }))
                    .child(project.badge())
            } else {
                el.text_size(px(13.)).px_2().py_1().child(project.name())
            }
        });

        let add = div()
            .id("new-workspace")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded_md()
            .text_color(rgb(t.muted))
            .border_1()
            .border_dashed()
            .border_color(rgb(t.border))
            .hover(|el| el.bg(rgb(t.element)).text_color(rgb(t.text)))
            .map(|el| {
                if collapsed {
                    el.size(px(32.)).text_size(px(16.)).child("+")
                } else {
                    el.py_1().text_size(px(13.)).whitespace_nowrap().child("+ New workspace")
                }
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    this.open_new_workspace_modal(window, cx)
                }),
            );

        div()
            .flex_none()
            .w(px(if collapsed { 48. } else { 180. }))
            .h_full()
            .bg(rgb(t.sidebar))
            .border_r_1()
            .border_color(rgb(t.element))
            .p(px(if collapsed { 8. } else { 12. }))
            .flex()
            .flex_col()
            .when(collapsed, |el| el.items_center())
            .gap_2()
            .child(header)
            .child(
                div()
                    .id("workspace-list")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .gap_1()
                    .overflow_y_scroll()
                    .children(entries),
            )
            .child(add)
    }

    fn render_new_workspace_modal(&self, modal: &NewWorkspaceModal, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = ui(cx);
        let folder = match &modal.folder {
            Some(folder) => folder.display().to_string(),
            None => format!("No folder chosen: opens your home folder ({})", home_dir().display()),
        };
        modal_overlay("new-workspace-modal", &modal.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "enter" => this.confirm_new_workspace(window, cx),
                    "escape" => this.cancel_new_workspace(window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(
                modal_panel("New workspace", t)
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .bg(rgb(t.sidebar))
                            .text_color(rgb(if modal.folder.is_some() { t.text } else { t.muted }))
                            .child(folder),
                    )
                    .when_some(modal.error.clone(), |el, error| {
                        el.child(div().text_color(rgb(t.error)).child(error))
                    })
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(modal_button("modal-browse", "Choose folder…", false, t).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                    this.browse_for_folder(window, cx)
                                }),
                            ))
                            .child(div().flex_1())
                            .child(modal_button("modal-cancel", "Cancel", false, t).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                    this.cancel_new_workspace(window, cx)
                                }),
                            ))
                            .child(modal_button("modal-create", "Create", true, t).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                    this.confirm_new_workspace(window, cx)
                                }),
                            )),
                    ),
            )
    }

    fn render_confirm_close(&self, modal: &ConfirmClose, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = ui(cx);
        let project = &self.projects[modal.project];
        let containers: usize = project
            .tabs
            .iter()
            .map(|tab| tab.root.as_ref().map_or(0, |root| root.leaves().len()) + tab.floats.len())
            .sum();
        let detail = match containers {
            0 => "Nothing is running in it.".to_owned(),
            1 => "1 running terminal or editor will be ended.".to_owned(),
            n => format!("{n} running terminals or editors will be ended."),
        };
        modal_overlay("close-workspace-modal", &modal.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "enter" => this.confirm_close_workspace(window, cx),
                    "escape" => this.cancel_close_workspace(window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(
                modal_panel(format!("Close workspace \"{}\"?", project.name()), t)
                    .child(div().text_color(rgb(t.text)).child(detail))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(div().flex_1())
                            .child(modal_button("close-cancel", "Cancel", false, t).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                    this.cancel_close_workspace(window, cx)
                                }),
                            ))
                            .child(modal_button("close-confirm", "Close workspace", true, t).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                    this.confirm_close_workspace(window, cx)
                                }),
                            )),
                    ),
            )
    }

    fn render_tabbar(&self, cx: &mut Context<Self>) -> Div {
        let t = ui(cx);
        let project = self.project();
        let tabs = project.tabs.iter().enumerate().map(|(ix, tab)| {
            let active = ix == project.active;
            div()
                .id(("tab", tab.number))
                .flex()
                .items_center()
                .gap_2()
                .pl_3()
                .pr_1()
                .py_1()
                .rounded_sm()
                .text_size(px(13.))
                .bg(rgb(if active { t.element } else { t.panel }))
                .text_color(rgb(if active { t.text } else { t.muted }))
                .hover(|el| el.bg(rgb(t.element)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        this.activate_tab(ix, window, cx)
                    }),
                )
                .child(format!("Tab {}", tab.number))
                .child(
                    div()
                        .id(("close-tab", tab.number))
                        .px_1()
                        .rounded_sm()
                        .hover(|el| el.bg(rgb(t.border)).text_color(rgb(t.error)))
                        .child("×")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                // Don't let the tab underneath activate itself as well.
                                cx.stop_propagation();
                                this.close_tab(this.active, ix, window, cx);
                            }),
                        ),
                )
        });

        div()
            .flex_none()
            .h(px(32.))
            .flex()
            .items_center()
            .px_2()
            .gap_1()
            .bg(rgb(t.panel))
            .border_b_1()
            .border_color(rgb(t.element))
            .children(tabs)
            .child(
                div()
                    .id("new-tab")
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .text_size(px(13.))
                    .text_color(rgb(t.muted))
                    .hover(|el| el.bg(rgb(t.element)).text_color(rgb(t.text)))
                    .child("+")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, window, cx| {
                            this.open_tab(window, cx)
                        }),
                    ),
            )
    }

    /// Banner listing config problems; nothing when the config loaded cleanly.
    fn render_config_warnings(&self, cx: &mut Context<Self>) -> Option<Div> {
        let t = ui(cx);
        let config_warnings = &cx.global::<Config>().warnings;
        if config_warnings.is_empty() && self.state_warnings.is_empty() {
            return None;
        }
        let path = crate::config::path().map(|p| p.display().to_string()).unwrap_or_default();
        let config_lines = config_warnings.iter().map(|w| format!("{path}: {w}"));
        let lines: Vec<String> = config_lines.chain(self.state_warnings.iter().cloned()).collect();
        Some(
            div()
                .flex_none()
                .flex()
                .gap_2()
                .px_3()
                .py_2()
                .bg(rgb(t.panel))
                .border_b_1()
                .border_color(rgb(t.warning))
                .text_size(px(12.))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(div().text_color(rgb(t.warning)).child("Helterm found problems:"))
                        .children(lines.into_iter().map(|w| div().text_color(rgb(t.text)).child(format!("• {w}")))),
                )
                .child(
                    div()
                        .id("dismiss-config-warnings")
                        .flex_none()
                        .px_1()
                        .rounded_sm()
                        .text_color(rgb(t.muted))
                        .hover(|el| el.bg(rgb(t.element)).text_color(rgb(t.text)))
                        .child("×")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, _, cx| {
                                this.config_warnings_dismissed = true;
                                cx.notify();
                            }),
                        ),
                ),
        )
    }

    /// The active workspace: its tab bar (once there are 2+ tabs), containers
    /// and floating terminal.
    fn render_active_tab(&self, cx: &mut Context<Self>) -> Div {
        let t = ui(cx);
        let explorer_left = cx.global::<Config>().explorer_left;
        let tab = self.tab();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            // Tabs stay out of the way until there is more than one.
            .when(self.project().tabs.len() > 1, |el| el.child(self.render_tabbar(cx)))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    // Outer padding: panes sit like cards on a slightly darker backdrop.
                    .p(px(PANE_GAP))
                    .bg(rgb(t.panel))
                    .when_some(self.manage.as_ref(), |el, manage| {
                        el.track_focus(&manage.focus_handle)
                            .key_context("ManageMode")
                            .on_key_down(cx.listener(Self::manage_key))
                    })
                    .child(match (&tab.root, tab.maximized) {
                        (None, _) => self.render_empty_tab(cx),
                        (Some(root), maximized) => {
                            match maximized.and_then(|id| root.find_leaf(id.as_u64())) {
                                Some(leaf) => self.render_node(tab, &Node::Leaf(leaf.clone()), t, explorer_left),
                                None => self.render_node(tab, root, t, explorer_left),
                            }
                        }
                    })
                    .when_some(tab.visible_float(), |el, floating| {
                        el.child(
                            div()
                                .id("floating")
                                .absolute()
                                .top(relative(0.1))
                                .left(relative(0.1))
                                .w(relative(0.8))
                                .h(relative(0.8))
                                .flex()
                                .rounded_md()
                                .border_2()
                                .border_color(rgb(t.accent))
                                .shadow_lg()
                                // Clicks must not fall through to the tiles underneath.
                                .occlude()
                                .child(self.render_node(tab, &Node::Leaf(floating.terminal.clone()), t, explorer_left)),
                        )
                    })
                    .when_some(self.manage.as_ref(), |el, manage| el.child(manage_pill(manage.mode, t))),
            )
    }

    /// No workspace open (first start, or all closed).
    fn render_no_workspaces(&self, cx: &mut Context<Self>) -> Div {
        let t = ui(cx);
        let label = match cx.global::<Config>().key_label("new_workspace") {
            Some(key) => format!("+ New workspace  {key}"),
            None => "+ New workspace".to_owned(),
        };
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .track_focus(&self.empty_focus)
            .child(div().text_size(px(15.)).text_color(rgb(t.muted)).child("No workspaces open"))
            .child(
                div().text_size(px(13.)).child(modal_button("start-workspace", label, true, t).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, window, cx| this.open_new_workspace_modal(window, cx)),
                )),
            )
    }

    /// Empty tab: let the user pick what to start with.
    fn render_empty_tab(&self, cx: &mut Context<Self>) -> Div {
        let t = ui(cx);
        let config = cx.global::<Config>();
        let with_key = |label: &str, action: &str| match config.key_label(action) {
            Some(key) => format!("{label}  {key}"),
            None => label.to_owned(),
        };
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .track_focus(&self.empty_focus)
            .child(
                div()
                    .text_size(px(15.))
                    .text_color(rgb(t.muted))
                    .child(format!("{}: start with", self.project().name())),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .text_size(px(13.))
                    .child(modal_button("start-terminal", with_key("Terminal", "split_right"), false, t).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, window, cx| {
                            this.add_container(TerminalKind::Terminal, Axis::Row, window, cx)
                        }),
                    ))
                    .child(modal_button("start-editor", with_key("Editor", "new_editor"), false, t).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, window, cx| {
                            this.add_container(TerminalKind::Editor, Axis::Row, window, cx)
                        }),
                    )),
            )
    }

    fn render_node(&self, tab: &Tab, node: &Node<Entity<TerminalView>>, t: UiColors, explorer_left: bool) -> Div {
        match node {
            Node::Leaf(terminal) => {
                // No gap: the explorer is joined to its editor's edge.
                let mut el = div().relative().flex().flex_1().min_w_0().min_h_0().child(terminal.clone());
                if explorer_left {
                    el = el.flex_row_reverse();
                }
                if let Some(explorer) = tab.explorers.get(&terminal.entity_id()) {
                    el = el.child(explorer.clone());
                }
                let selected = self.manage.as_ref().is_some_and(|m| m.selected == terminal.entity_id());
                if selected {
                    // Light tint so the terminal underneath stays readable.
                    let accent: Hsla = rgb(t.accent).into();
                    el = el.child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .rounded_md()
                            .bg(Hsla { a: 0.15, ..accent })
                            .border_2()
                            .border_color(accent),
                    );
                }
                el
            }
            Node::Split { axis, ratio, first, second } => {
                let mut el = div().flex_1().min_w_0().min_h_0().flex().gap(px(PANE_GAP));
                el = match axis {
                    Axis::Row => el.flex_row(),
                    Axis::Col => el.flex_col(),
                };
                // Children split the space by the ratio (flex basis 0, grow by share).
                el.child(grow(self.render_node(tab, first, t, explorer_left), *ratio))
                    .child(grow(self.render_node(tab, second, t, explorer_left), 1. - *ratio))
            }
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let stale = self.manage.as_ref().is_some_and(|m| self.projects.is_empty() || !self.is_tiled(m.selected));
        if stale {
            self.manage = None;
            self.focus_active_tab(window, cx);
        }
        let title = match self.projects.get(self.active) {
            Some(project) => format!("{} - Helterm", project.name()),
            None => "Helterm".to_owned(),
        };
        if title != self.window_title {
            window.set_window_title(&title);
            self.window_title = title;
        }
        let t = ui(cx);
        div()
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .bg(rgb(t.background))
            .text_color(rgb(t.text))
            .font_family(cx.global::<Config>().font_family.clone())
            // Workspace-level actions work even with no workspace open.
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::toggle_manage))
            .on_action(cx.listener(Self::close_workspace))
            .on_action(cx.listener(Self::new_workspace))
            .on_action(cx.listener(Self::next_workspace))
            .on_action(cx.listener(Self::prev_workspace))
            .when(!self.projects.is_empty(), |el| {
                el.on_action(cx.listener(Self::split_right))
                    .on_action(cx.listener(Self::split_down))
                    .on_action(cx.listener(Self::new_editor))
                    .on_action(cx.listener(Self::close_focused))
                    .on_action(cx.listener(Self::focus_left))
                    .on_action(cx.listener(Self::focus_right))
                    .on_action(cx.listener(Self::focus_up))
                    .on_action(cx.listener(Self::focus_down))
                    .on_action(cx.listener(Self::toggle_explorer))
                    .on_action(cx.listener(Self::new_tab))
                    .on_action(cx.listener(Self::next_tab))
                    .on_action(cx.listener(Self::prev_tab))
                    .on_action(cx.listener(Self::toggle_maximize))
                    .on_action(cx.listener(Self::toggle_floating))
                    .on_action(cx.listener(Self::run_floating_command))
                    .on_action(cx.listener(Self::float_focused))
            })
            .child(self.render_sidebar(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .when(!self.config_warnings_dismissed, |el| el.children(self.render_config_warnings(cx)))
                    .child(match self.projects.is_empty() {
                        true => self.render_no_workspaces(cx),
                        false => self.render_active_tab(cx),
                    }),
            )
            .when_some(self.new_workspace.as_ref(), |el, modal| {
                el.child(self.render_new_workspace_modal(modal, cx))
            })
            .when_some(self.confirm_close.as_ref(), |el, modal| {
                el.child(self.render_confirm_close(modal, cx))
            })
    }
}

/// Give a flex child `share` of its parent's free space.
fn grow(mut el: Div, share: f32) -> Div {
    el.style().flex_grow = Some(share);
    el
}

/// The pill showing which management mode is active and its keys.
fn manage_pill(mode: ManageMode, t: UiColors) -> Div {
    let text = match mode {
        ManageMode::Select => "MANAGE  ·  hjkl select  ·  r resize  ·  m move  ·  z zoom  ·  esc exit",
        ManageMode::Resize => "RESIZE  ·  h/l width  ·  j/k height  ·  esc back",
        ManageMode::Move => "MOVE  ·  hjkl swap  ·  esc back",
    };
    div()
        .absolute()
        .top(px(PANE_GAP + 6.))
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(
            div()
                .px_3()
                .py_1()
                .rounded_full()
                .bg(rgb(t.accent))
                .text_color(rgb(t.background))
                .text_size(px(12.))
                .shadow_lg()
                .child(text),
        )
}

/// Dimmed full-window backdrop that owns focus while a dialog is open.
fn modal_overlay(id: &'static str, focus_handle: &FocusHandle) -> Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(hsla(0., 0., 0., 0.5))
        // Keep clicks and keys away from the terminals underneath.
        .occlude()
        .track_focus(focus_handle)
}

fn modal_panel(title: impl Into<SharedString>, t: UiColors) -> Div {
    div()
        .w(px(460.))
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded_lg()
        .bg(rgb(t.background))
        .border_1()
        .border_color(rgb(t.border))
        .text_size(px(13.))
        .child(div().text_size(px(15.)).text_color(rgb(t.accent)).child(title.into()))
}

fn modal_button(id: &'static str, label: impl Into<SharedString>, primary: bool, t: UiColors) -> Stateful<Div> {
    div()
        .id(id)
        .px_3()
        .py_1()
        .rounded_sm()
        .bg(rgb(if primary { t.accent } else { t.element }))
        .text_color(rgb(if primary { t.sidebar } else { t.text }))
        .hover(|el| el.opacity(0.85))
        .child(label.into())
}

#[cfg(test)]
mod tests {
    use super::{active_after_close, load_saved, next_dup, write_saved};
    use std::path::PathBuf;

    #[test]
    fn saved_workspaces_round_trip_in_order() {
        let dir = std::env::temp_dir().join(format!("helterm_state_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("helterm").join("workspaces.toml");

        assert_eq!(load_saved(&path).unwrap(), Vec::<PathBuf>::new()); // first start
        let folders = vec![PathBuf::from("/home/u/b"), PathBuf::from("/home/u/a"), PathBuf::from("/home/u")];
        write_saved(&path, &folders).unwrap();
        assert_eq!(load_saved(&path).unwrap(), folders);
        write_saved(&path, &[]).unwrap(); // all closed
        assert_eq!(load_saved(&path).unwrap(), Vec::<PathBuf>::new());

        std::fs::write(&path, "[[workspace]\npath = ").unwrap();
        assert!(load_saved(&path).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn duplicate_workspace_names_get_numbers() {
        assert_eq!(next_dup(&[]), 1); // "home"
        assert_eq!(next_dup(&[1]), 2); // "home 2"
        assert_eq!(next_dup(&[1, 2]), 3); // "home 3"
        // A closed workspace frees its number for reuse.
        assert_eq!(next_dup(&[1, 3]), 2);
        assert_eq!(next_dup(&[2]), 1);
    }

    #[test]
    fn closing_tabs_keeps_a_sensible_active_tab() {
        // Closing a tab left of the active one shifts the active index left.
        assert_eq!(active_after_close(2, 0, 3), 1);
        // Closing the active tab selects its right neighbour...
        assert_eq!(active_after_close(1, 1, 3), 1);
        // ...or the new last tab when it was the last one.
        assert_eq!(active_after_close(3, 3, 3), 2);
        // Closing a tab right of the active one leaves it alone.
        assert_eq!(active_after_close(0, 2, 3), 0);
    }
}
