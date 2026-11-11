# Helterm

> **Heads up:** this is personal software. I built it for my own use, the way I
> like to work. You're welcome to use it at your own leisure, but expect rough
> edges and no promises of support.

## Platform support

Helterm is built and used on **Fedora Linux**. It has **not been tested on
macOS or Windows**, and releases only ship a Linux (x86_64) build.

## Why?

Because I wanted to, and therefore I did.

## What it is

Helterm is a Helix-centric project workspace manager: each workspace is a
project folder with its editors, shells and tools arranged around it. It isn't
trying to be a plain terminal emulator or a multiplexer; the terminal and the
panes are just how the project is laid out.

- **Workspaces are projects**: one per folder, picked from a switcher
  (`Alt+W`) that also pops up while you hold the next/previous workspace keys,
  like Zed's tab switcher. The list is saved and reopened on the next start.
- **[Helix](https://helix-editor.com) is first-class**: editor containers open
  Helix in the project, with a file explorer you can toggle next to them to
  open, create, rename and delete files from the keyboard.
- **Tools around the editor**: shells in a main-stack layout (main on the
  left, the rest stacked on the right, side by side when you want), tabs per
  workspace, and floating terminals, including ones that run a command on a
  key (lazygit on `Alt+G`, say).
- **Coding agents next to the editor**: agent containers run your preferred
  agent (OpenCode, Claude Code, Codex, pi, ...) in the project on `Alt+A`.
- **Arrange it your way**: management mode to select and resize containers,
  maximize one, scroll back with a Helix-style copy mode.
- **A config file** for fonts, colors and every keybinding.

Built in Rust with [GPUI](https://crates.io/crates/gpui) (from the Zed editor)
for the UI and [`alacritty_terminal`](https://crates.io/crates/alacritty_terminal)
for terminal emulation.

## Install

Download `helterm-<version>-x86_64-linux.tar.gz` from the
[releases page](https://github.com/joselevelsup/helterm/releases), then:

```sh
sha256sum -c helterm-*.tar.gz.sha256   # optional: check the download
tar -xzf helterm-*-x86_64-linux.tar.gz
cd helterm-*-x86_64-linux
./install.sh                # into ~/.local (PREFIX=/usr/local sudo ./install.sh for all users)
./install.sh --uninstall    # to remove it again
```

It shows up in your app launcher as **Helterm**.

**Requirements:** x86_64 Linux with glibc 2.35 or newer (Fedora, Arch, Ubuntu
22.04+, Debian 12+, ...) and a Vulkan-capable GPU driver. Editor containers
need Helix (`hx`) on your `PATH`; plain terminals work without it. Editors
start through your shell (`$SHELL -l -i -c hx`) in the workspace folder, so
tools from mise, asdf, nvm, ... (and the LSPs they provide) are found.

### From source

```sh
git clone https://github.com/joselevelsup/helterm && cd helterm
./install.sh                # builds the release binary, then installs it (same as above)
./install.sh --uninstall    # to remove it again
```

Run `./install.sh` again after pulling changes to update; it tells you when a
running Helterm needs a restart. To try it without installing:
`cargo run --release`.

Needs stable Rust and the X11/Wayland development headers, e.g. on Fedora
`sudo dnf install libxkbcommon-devel libxkbcommon-x11-devel`.

## Getting started

1. On first start there are no workspaces. Press `Alt+Shift+N` (or click
   **+ New workspace**), choose a folder with your desktop's folder picker, and
   press **Create**. With no folder chosen it opens your home folder.
2. A new workspace (or tab) starts empty: press `Alt+Enter` for a terminal or
   `Alt+O` for a Helix editor.
3. Add more containers, open the explorer (`Alt+E` in an editor), add tabs (`Alt+T`), and so on.
   A container closes when its program exits (`exit`, `:q` in Helix) or with
   `Alt+Q`. Close the window to quit.

## Configuration

The config file lives at `$XDG_CONFIG_HOME/helterm/config.toml`, usually
`~/.config/helterm/config.toml`. Helterm creates it on first start with
every setting at its default (the same content as
[`config.example.toml`](config.example.toml)) and never overwrites it after
that. Restart Helterm to apply changes.

Every setting is optional: anything you delete goes back to its default.
Mistakes (unknown names, bad colors, invalid keys, a key bound to two actions)
are skipped and listed in a banner at the top of the window, so a typo never
stops the app from starting. A file that can't be read at all (broken TOML, a
misspelled section or setting like `[explorr]`, a wrong type) is ignored in
favour of the defaults, and the banner says which line is wrong.

```toml
[font]
family = "JetBrainsMono Nerd Font"
size = 15

# Which side of its editor the file explorer opens on: "right" or "left".
[explorer]
side = "left"

# Use the colors of the theme set in Helix's config.toml (default false). It
# follows Helix's [theme] light / dark themes with your desktop's mode, and
# updates when you change the theme. Colors set below still win over it.
[colors]
use_helix_theme = true

# App chrome: tabs, explorer, dialogs. Colors are "#rrggbb".
# background, panel, sidebar, element, border, text, muted, accent, error, warning, success
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

Floating terminals that run a command get a key each, e.g. lazygit on `Alt+G`:

```toml
[[floating_commands]]
key = "alt-g"
command = "lazygit"
```

Press the key to open it, again to hide / show it (it keeps running); it
closes when the command exits. Add as many as you like. Commands run through
your shell (`$SHELL -l -i -c`), so your `PATH`, aliases and arguments work.

Agent containers run your coding agent (OpenCode, Claude Code, Codex, pi, ...)
in the workspace folder, on `Alt+A` or the **Agent** button of an empty tab:

```toml
[agent]
command = "opencode"   # any command with arguments, e.g. "claude", "opencode --continue"
```

They run through your shell like floating commands, and come back with the
workspace on the next start (as a new run of the command; use the agent's own
resume flag, like `--continue`, to pick up where it left off). Every container
has `HELTERM=1` set, so scripts and agent hooks can tell they run in Helterm.

### When an agent is done or needs you

Any container can ask for your attention: by ringing the terminal bell, or
with a desktop notification sequence (OSC 9 or OSC 777). Helterm's own signal
also says which it is:

```sh
printf '\033]777;notify;helterm;done\007'    # finished: green border
printf '\033]777;notify;helterm;input\007'   # needs you: thick yellow border
```

A bell or any other notification counts as "needs you". Unless you're already
looking at the container, its border changes color, its tab and workspace (in
the switcher) get a dot, and, when the container is out of sight or Helterm
isn't the active window, a desktop notification pops up (through
`notify-send`). It all clears once you focus the container.

Hook your agent up to send them. Hooks don't write to the terminal on their
own, so the snippets print to `/dev/tty`:

**Claude Code** (`~/.claude/settings.json`):

```json
{
  "hooks": {
    "Stop": [{ "hooks": [{ "type": "command",
      "command": "[ -n \"$HELTERM\" ] && printf '\\033]777;notify;helterm;done\\007' > /dev/tty || true" }] }],
    "Notification": [{ "matcher": "permission_prompt|elicitation_dialog", "hooks": [{ "type": "command",
      "command": "[ -n \"$HELTERM\" ] && printf '\\033]777;notify;helterm;input\\007' > /dev/tty || true" }] }]
  }
}
```

**OpenCode** (`~/.config/opencode/plugins/helterm.js`):

```js
import { writeFileSync } from "node:fs"

const signal = (what) => {
  if (!process.env.HELTERM) return
  try { writeFileSync("/dev/tty", `\x1b]777;notify;helterm;${what}\x07`) } catch {}
}

export const Helterm = async () => ({
  event: async ({ event }) => {
    if (event.type === "session.idle") signal("done")
    if (event.type === "permission.asked") signal("input")
  },
})
```

**Codex** (`~/.codex/config.toml`) sends notifications itself; they show as
"needs you":

```toml
[tui]
notifications = true
notification_method = "osc9"
notification_condition = "always"   # Helterm doesn't report focus to programs
```

**Other agents** (pi, ...): anything that rings the bell or sends OSC 9 works;
for "done" vs "needs you", have its hooks or extensions print the lines above.

A key can belong to only one action. If two actions share a key, a key you set
yourself wins over a default; between two of yours, the one listed first in
`config.example.toml` keeps it, then `[[floating_commands]]` in file order.

Other files Helterm keeps:

| File | What |
| ---- | ---- |
| `$XDG_STATE_HOME/helterm/workspaces.toml` (usually `~/.local/state/helterm/`) | The saved workspace list (folders only, not layouts). |

## Default keybindings

These are the defaults. The **Config name** column is what to use under
`[keybindings]` to change them.

### Workspaces

| Keys | Action | Config name |
| ---- | ------ | ----------- |
| `Alt+Shift+N` | New workspace (asks for a folder) | `new_workspace` |
| `Alt+Shift+Q` | Close the current workspace, or the one selected in the switcher (asks to confirm) | `close_workspace` |
| `Alt+Tab` | Next workspace | `next_workspace` |
| `Alt+Shift+Tab` | Previous workspace | `prev_workspace` |
| `Alt+W` | Workspace switcher (↑/↓ or j/k, Enter, Esc) | `toggle_workspaces` |

> COSMIC, GNOME and KDE use `Alt+Tab` / `Alt+Shift+Tab` to switch windows, so
> Helterm never sees them there. Free them in your desktop's keyboard
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
| `Alt+Enter` | New terminal at the bottom of the stack | `new_terminal` |
| `Alt+Shift+Enter` | New terminal right of the focused one (side by side) | `split_side` |
| `Alt+O` | New Helix editor at the bottom of the stack | `new_editor` |
| `Alt+A` | New coding agent (`[agent] command`) at the bottom of the stack | `new_agent` |
| `Alt+Q` | Close the focused container | `close_container` |
| `Alt+H` / `Alt+J` / `Alt+K` / `Alt+L` | Move focus left / down / up / right | `focus_left` / `focus_down` / `focus_up` / `focus_right` |
| `Alt+Z` | Maximize / restore the focused container (like tmux's zoom) | `toggle_maximize` |
| `Alt+M` | Management mode: select, resize and move containers (see below) | `manage_mode` |
| `Alt+F` | New floating terminal, or hide / show it | `toggle_floating` |
| `Alt+Shift+F` | Float the focused container, or put the floating one back | `float_focused` |
| `Alt+E` | Editor: open / focus its file explorer. Explorer: close it | `toggle_explorer` |

Containers use a main-stack layout: the first one is **main**, on the left;
every new one goes to the bottom of the **stack** on the right. Any slot (main
or a stack entry) can hold several containers side by side (`Alt+Shift+Enter`).
Closing main moves the top of the stack into its place. The old `split_right` /
`split_down` config names still work as `new_terminal` / `split_side`.

Each tab can have one floating shell (`Alt+F`) plus one per floating command;
only one shows at a time. Every other key goes to the focused terminal or editor.

### Copy and paste (in a terminal)

| Keys / mouse | Action | Config name |
| ------------ | ------ | ----------- |
| Drag | Select text (double-click: a word, triple-click: a line) | |
| `Ctrl+Shift+C` | Copy the selection | `copy` |
| `Ctrl+Shift+V` | Paste | `paste` |
| `Alt+V` | Toggle copy mode (see below) | `copy_mode` |
| Middle-click | Paste the last selection (Linux "primary" clipboard) | |
| `Alt++` / `Alt+-` / `Alt+0` | Zoom the focused terminal in / out / back to the configured size | `zoom_in` / `zoom_out` / `zoom_reset` |

Selecting also fills the middle-click clipboard. Clicking or typing clears the
selection. Pastes into programs that support it (shells, Helix) are bracketed,
so multi-line pastes don't run line by line.

### Scrolling and copy mode (in a terminal)

The mouse wheel / touchpad scrolls back through the terminal's history (10,000
lines), like any terminal. Programs that handle the mouse themselves (Pi's
fullscreen mode, Helix, htop, ...) get the wheel events and scroll their own
view; full-screen programs without mouse support (`less`, `man`) get arrow
keys. A badge in the corner shows where you are in the history (`12/340`, like
tmux); typing jumps back to the bottom.

`Alt+V` (`copy_mode`) toggles a tmux-style copy mode with **Helix keys**. Like
Helix there is a normal mode (moving drops the selection) and a select mode
(`v`, moving extends it). Its keys never reach the program:

| Keys | Action |
| ---- | ------ |
| `j` / `k`, `↓` / `↑` | Move down / up (scrolls through history) |
| `h` / `l`, `←` / `→` | Move left / right |
| `w` / `b` / `e`, `W` / `B` / `E` | Next word / previous word / word end (`W`: whitespace-separated) |
| `x` | Select the line; again to extend it down |
| `v` | Toggle select mode |
| `%` | Select the whole history |
| `;` | Collapse the selection |
| `gg` / `ge` | Top of history / last line |
| `gh` / `gl` / `gs`, `Home` / `End` | Line start / line end / first non-blank |
| `gt` / `gc` / `gb` | Top / middle / bottom of the screen |
| `Ctrl+U` / `Ctrl+D` | Half a screen up / down |
| `Ctrl+B` / `Ctrl+F`, `PageUp` / `PageDown` | A screen up / down |
| `Ctrl+Y` / `Ctrl+E` | Scroll one line up / down |
| `y` | Copy the selection and leave copy mode |
| `Esc` | Drop the selection / select mode; again to leave copy mode |

The corner badge shows `COPY` (or `COPY SEL` in select mode) and `g…` while a
`g` key is waiting for its second key.

### Management mode

`Alt+M` (`manage_mode`) lets you rearrange the containers of a tab, a bit like
Pop Shell's adjustment mode. The selected container gets a light tint and a pill
at the top shows the mode and its keys. Keys go to management mode, not the
terminals.

| Mode | Keys |
| ---- | ---- |
| Manage | `h` / `j` / `k` / `l` select a container, `p` swap its slot with main, `r` resize, `m` move, `z` maximize it, `Esc` / `Enter` / `Alt+M` leave |
| Resize | `l` / `h` move the main / stack edge right / left, `j` / `k` make a stack entry taller / shorter, `Esc` back |
| Move | `h` / `j` / `k` / `l` swap with the neighbour on that side, `Esc` back |

Leaving focuses the selected container. Entering un-maximizes and hides
floating terminals so everything is visible.

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
| `/` | Search files by name with [fzf](https://github.com/junegunn/fzf) (needs `fzf` installed): results replace the tree as you type; `↑` / `↓` (or `Ctrl+P` / `Ctrl+N`) to move, `Enter` to open, `Esc` back to the tree |
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
| `src/container.rs` | The main-stack layout |
| `src/explorer.rs` | File explorer and its file operations |
| `src/config.rs` | Config file: defaults, parsing with warnings, rebindable actions |
| `src/helix_theme.rs` | Reads the Helix theme (`use_helix_theme`) and maps it to Helterm colors |
| `packaging/` | Launcher, icon, install script (`./install.sh` links to it), release-archive script |

## License

[MIT](LICENSE)

---

> ⚠️ **Warning:** this project was coded entirely with the
> [Pi coding agent](https://pi.dev). I just need my personal
> software to work, so the code hasn't had the review a hand-written project
> would. Read it before you trust it.
