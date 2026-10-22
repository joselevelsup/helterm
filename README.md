# Vibe Term

> **Heads up:** this is personal software. I built it for my own use, the way I
> like to work. You're welcome to use it at your own leisure, but expect rough
> edges and no promises of support.

## Why?

Because I wanted to, and therefore I did.

## What it is

A project terminal for Linux where terminal editors are first-class citizens.
Instead of one shell in one window, you get:

- **Workspaces**: one per project folder, listed in a sidebar (shrinkable to
  Slack-style initials). The list is saved and reopened on the next start.
- **Tabs** inside each workspace. The tab bar only shows once there are two.
- **Manual, Sway-style splits**: every tab is a tree of containers, each one a
  terminal or a [Helix](https://helix-editor.com) editor.
- **A file explorer** you can toggle next to any editor: open, create, rename
  and delete files from the keyboard.
- **Maximize** a container within its tab, or **float** a terminal over it.
- **A config file** for fonts, colors and every keybinding.

Built in Rust with [GPUI](https://crates.io/crates/gpui) (from the Zed editor)
for the UI and [`alacritty_terminal`](https://crates.io/crates/alacritty_terminal)
for terminal emulation.

## Install

Download `vibe_term-<version>-x86_64-linux.tar.gz` from the
[releases page](https://github.com/joselevelsup/vibe_term/releases), then:

```sh
sha256sum -c vibe_term-*.tar.gz.sha256   # optional: check the download
tar -xzf vibe_term-*-x86_64-linux.tar.gz
cd vibe_term-*-x86_64-linux
./install.sh                # into ~/.local (PREFIX=/usr/local sudo ./install.sh for all users)
./install.sh --uninstall    # to remove it again
```

It shows up in your app launcher as **Vibe Term**.

**Requirements:** x86_64 Linux with glibc 2.35 or newer (Fedora, Arch, Ubuntu
22.04+, Debian 12+, ...) and a Vulkan-capable GPU driver. Editor containers
need Helix (`hx`) on your `PATH`; plain terminals work without it.

### From source

```sh
cargo run --release
```

Needs stable Rust and the X11/Wayland development headers, e.g. on Fedora
`sudo dnf install libxkbcommon-devel libxkbcommon-x11-devel`.

## Getting started

1. On first start there are no workspaces. Press `Alt+Shift+N` (or click
   **+ New workspace**), choose a folder with your desktop's folder picker, and
   press **Create**. With no folder chosen it opens your home folder.
2. A new workspace (or tab) starts empty: press `Alt+Enter` for a terminal or
   `Alt+O` for a Helix editor.
3. Split, open the explorer (`Alt+E` in an editor), add tabs (`Alt+T`), and so on.
   A container closes when its program exits (`exit`, `:q` in Helix) or with
   `Alt+Q`. Close the window to quit.

## Configuration

The config file lives at `$XDG_CONFIG_HOME/vibe_term/config.toml`, usually
`~/.config/vibe_term/config.toml`. Vibe Term creates it on first start with
every setting at its default (the same content as
[`config.example.toml`](config.example.toml)) and never overwrites it after
that. Restart Vibe Term to apply changes.

Every setting is optional: anything you delete goes back to its default.
Mistakes (unknown names, bad colors, invalid keys, a key bound to two actions)
are skipped and listed in a banner at the top of the window, so a typo never
stops the app from starting.

```toml
[font]
family = "JetBrainsMono Nerd Font"
size = 15

# App chrome: sidebar, tabs, explorer, dialogs. Colors are "#rrggbb".
# background, panel, sidebar, element, border, text, muted, accent, error, warning
[colors.ui]
accent = "#7aa2f7"

# What programs in the terminal see: foreground, background, cursor, selection,
# and the 16 ANSI colors (black, red, ..., bright_white). Helix uses its own theme.
[colors.terminal]
background = "#1a1b26"

# action = "key", a list of keys, or "" to unbind.
# Modifiers: ctrl, alt, shift, super. Chords: "ctrl-k ctrl-t".
[keybindings]
new_tab = ["alt-t", "ctrl-shift-t"]
toggle_floating = ""
```

A key can belong to only one action. If two actions share a key, a key you set
yourself wins over a default; between two of yours, the one listed first in
`config.example.toml` keeps it.

Other files Vibe Term keeps:

| File | What |
| ---- | ---- |
| `$XDG_STATE_HOME/vibe_term/workspaces.toml` (usually `~/.local/state/vibe_term/`) | The saved workspace list (folders only, not layouts). |

## Default keybindings

These are the defaults. The **Config name** column is what to use under
`[keybindings]` to change them.

### Workspaces

| Keys | Action | Config name |
| ---- | ------ | ----------- |
| `Alt+Shift+N` | New workspace (asks for a folder) | `new_workspace` |
| `Alt+Shift+Q` | Close the current workspace (asks to confirm) | `close_workspace` |
| `Alt+Tab` | Next workspace | `next_workspace` |
| `Alt+Shift+Tab` | Previous workspace | `prev_workspace` |
| `Alt+B` | Shrink / enlarge the workspace bar | `toggle_sidebar` |

> COSMIC, GNOME and KDE use `Alt+Tab` / `Alt+Shift+Tab` to switch windows, so
> Vibe Term never sees them there. Free them in your desktop's keyboard
> settings, or rebind `next_workspace` / `prev_workspace`.

### Tabs

| Keys | Action | Config name |
| ---- | ------ | ----------- |
| `Alt+T` | New tab | `new_tab` |
| `Alt+]` | Next tab | `next_tab` |
| `Alt+[` | Previous tab | `prev_tab` |

Close a tab with the `×` on its title, or by closing its last container.

### Containers

| Keys | Action | Config name |
| ---- | ------ | ----------- |
| `Alt+Enter` | New terminal, split to the right | `split_right` |
| `Alt+Shift+Enter` | New terminal, split below | `split_down` |
| `Alt+O` | New Helix editor, split to the right | `new_editor` |
| `Alt+Q` | Close the focused container | `close_container` |
| `Alt+H` / `Alt+J` / `Alt+K` / `Alt+L` | Move focus left / down / up / right | `focus_left` / `focus_down` / `focus_up` / `focus_right` |
| `Alt+M` | Maximize / restore the focused container | `toggle_maximize` |
| `Alt+F` | New floating terminal, or hide / show it | `toggle_floating` |
| `Alt+Shift+F` | Float the focused container, or put the floating one back | `float_focused` |
| `Alt+E` | Editor: open / focus its file explorer. Explorer: close it | `toggle_explorer` |

Each tab has at most one floating terminal. Every other key goes to the
focused terminal or editor.

### Copy and paste (in a terminal)

| Keys / mouse | Action | Config name |
| ------------ | ------ | ----------- |
| Drag | Select text (double-click: a word, triple-click: a line) | |
| `Ctrl+Shift+C` | Copy the selection | `copy` |
| `Ctrl+Shift+V` | Paste | `paste` |
| `Alt+V` | Toggle copy mode (see below) | `copy_mode` |
| Middle-click | Paste the last selection (Linux "primary" clipboard) | |

Selecting also fills the middle-click clipboard. Clicking or typing clears the
selection. Pastes into programs that support it (shells, Helix) are bracketed,
so multi-line pastes don't run line by line.

### Scrolling and copy mode (in a terminal)

The mouse wheel / touchpad scrolls back through the terminal's history (10,000
lines). In full-screen programs like Helix or `less` it sends arrow keys
instead. A badge in the corner shows where you are (`12/340`, like tmux); typing
jumps back to the bottom.

`Alt+V` (`copy_mode`) toggles a tmux-style vi copy mode. Its keys never reach the
program:

| Keys | Action |
| ---- | ------ |
| `j` / `k`, `↓` / `↑` | Move down / up (scrolls through history) |
| `h` / `l`, `w` / `b` / `e`, `W` / `B` / `E` | Move left / right, by word |
| `0` / `^` / `$` | Start / first character / end of line |
| `H` / `M` / `L` | Top / middle / bottom of the screen |
| `g` / `G` | Top of history / bottom |
| `Ctrl+U` / `Ctrl+D` | Half a screen up / down |
| `Ctrl+B` / `Ctrl+F`, `PageUp` / `PageDown` | A screen up / down |
| `v` / `V` | Start (or cancel) a character / line selection |
| `y` / `Enter` | Copy the selection and leave copy mode |
| `q` / `Esc` | Leave copy mode |

### File explorer (when focused)

These keys are fixed for now.

| Keys | Action |
| ---- | ------ |
| `j` / `k`, `↓` / `↑` | Move the selection |
| `l` / `→` | Expand a folder, or open a file |
| `h` / `←` | Collapse a folder, or jump to its parent |
| `Enter` / `Space` | Toggle a folder, or open a file in Helix |
| `g` / `G` | Jump to the top / bottom |
| `a` | New file (in the selected folder, or next to the selected file; `dir/` or `a/b/c.rs` work too) |
| `A` | New folder |
| `r` | Rename |
| `d` | Delete (asks `y/n`; permanent, no trash) |
| `.` | Show / hide hidden files |
| `Esc` | Back to the editor |

In a name prompt, `←` / `→` / `Home` / `End` move the cursor, `Backspace` /
`Delete` edit around it, `Enter` confirms and `Esc` cancels. Existing files are
never overwritten.

## Releasing

1. Bump `version` in `Cargo.toml` and commit.
2. `git tag v<version> && git push origin v<version>`

[`.github/workflows/release.yml`](.github/workflows/release.yml) runs the tests,
builds the archive with `packaging/make-release.sh` on Ubuntu 22.04 (so it also
runs on older distros) and publishes it with a checksum as a GitHub release.
The tag must match `Cargo.toml`. Run `packaging/make-release.sh` locally to get
the same archive in `dist/`.

## Code layout

| Path | What |
| ---- | ---- |
| `src/workspace.rs` | Workspaces, tabs, actions, focus, dialogs, layout |
| `src/terminal.rs` | PTY, `alacritty_terminal` grid, rendering, key translation |
| `src/container.rs` | The Sway-style split tree |
| `src/explorer.rs` | File explorer and its file operations |
| `src/config.rs` | Config file: defaults, parsing with warnings, rebindable actions |
| `packaging/` | Launcher, icon, install script, release-archive script |

## License

[MIT](LICENSE)

---

> ⚠️ **Warning:** this project was coded entirely with the
> [Pi coding agent](https://pi.dev). I just need my personal
> software to work, so the code hasn't had the review a hand-written project
> would. Read it before you trust it.
