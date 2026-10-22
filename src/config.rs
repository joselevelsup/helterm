//! User configuration: `~/.config/vibe_term/config.toml` (or `$XDG_CONFIG_HOME`).
//! Every setting is optional; bad entries are skipped with a warning instead of
//! failing the whole file.

use std::collections::HashMap;
use std::path::PathBuf;

use gpui::{Action, App, DummyKeyboardMapper, Global, KeyBinding, Keystroke, SharedString};
use serde::Deserialize;

use crate::workspace::*;

/// Colors of the app chrome (sidebar, tabs, explorer, dialogs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiColors {
    pub background: u32,
    pub panel: u32,
    pub sidebar: u32,
    pub element: u32,
    pub border: u32,
    pub text: u32,
    pub muted: u32,
    pub accent: u32,
    pub error: u32,
    pub warning: u32,
}

impl Default for UiColors {
    fn default() -> Self {
        Self {
            background: 0x1e1e2e,
            panel: 0x181825,
            sidebar: 0x11111b,
            element: 0x313244,
            border: 0x45475a,
            text: 0xcdd6f4,
            muted: 0x7f849c,
            accent: 0x89b4fa,
            error: 0xf38ba8,
            warning: 0xf9e2af,
        }
    }
}

impl UiColors {
    fn slot(&mut self, name: &str) -> Option<&mut u32> {
        Some(match name {
            "background" => &mut self.background,
            "panel" => &mut self.panel,
            "sidebar" => &mut self.sidebar,
            "element" => &mut self.element,
            "border" => &mut self.border,
            "text" => &mut self.text,
            "muted" => &mut self.muted,
            "accent" => &mut self.accent,
            "error" => &mut self.error,
            "warning" => &mut self.warning,
            _ => return None,
        })
    }
}

/// Colors programs in the terminal see. Helix uses its own theme (truecolor).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TermColors {
    pub foreground: u32,
    pub background: u32,
    pub cursor: u32,
    /// Background of selected text.
    pub selection: u32,
    /// black, red, green, yellow, blue, magenta, cyan, white, then the bright ones.
    pub ansi: [u32; 16],
}

const ANSI_NAMES: [&str; 16] = [
    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
    "bright_black", "bright_red", "bright_green", "bright_yellow", "bright_blue",
    "bright_magenta", "bright_cyan", "bright_white",
];

impl Default for TermColors {
    fn default() -> Self {
        Self {
            foreground: 0xcdd6f4,
            background: 0x1e1e2e,
            cursor: 0x89b4fa,
            selection: 0x45475a,
            ansi: [
                0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de,
                0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
            ],
        }
    }
}

impl TermColors {
    fn slot(&mut self, name: &str) -> Option<&mut u32> {
        match name {
            "foreground" => Some(&mut self.foreground),
            "background" => Some(&mut self.background),
            "cursor" => Some(&mut self.cursor),
            "selection" => Some(&mut self.selection),
            _ => {
                let ix = ANSI_NAMES.iter().position(|n| *n == name)?;
                Some(&mut self.ansi[ix])
            }
        }
    }
}

/// Every rebindable action: config name, default key, constructor.
const ACTIONS: &[(&str, &str, fn() -> Box<dyn Action>)] = &[
    ("split_right", "alt-enter", || Box::new(SplitRight)),
    ("split_down", "alt-shift-enter", || Box::new(SplitDown)),
    ("new_editor", "alt-o", || Box::new(NewEditor)),
    ("close_container", "alt-q", || Box::new(CloseContainer)),
    ("close_workspace", "alt-shift-q", || Box::new(CloseWorkspace)),
    ("new_workspace", "alt-shift-n", || Box::new(NewWorkspace)),
    ("next_workspace", "alt-tab", || Box::new(NextWorkspace)),
    ("prev_workspace", "alt-shift-tab", || Box::new(PrevWorkspace)),
    ("focus_left", "alt-h", || Box::new(FocusLeft)),
    ("focus_down", "alt-j", || Box::new(FocusDown)),
    ("focus_up", "alt-k", || Box::new(FocusUp)),
    ("focus_right", "alt-l", || Box::new(FocusRight)),
    ("toggle_explorer", "alt-e", || Box::new(ToggleExplorer)),
    ("new_tab", "alt-t", || Box::new(NewTab)),
    ("next_tab", "alt-]", || Box::new(NextTab)),
    ("prev_tab", "alt-[", || Box::new(PrevTab)),
    ("toggle_sidebar", "alt-b", || Box::new(ToggleSidebar)),
    ("toggle_maximize", "alt-m", || Box::new(ToggleMaximize)),
    ("toggle_floating", "alt-f", || Box::new(ToggleFloating)),
    ("float_focused", "alt-shift-f", || Box::new(FloatFocused)),
    ("copy", "ctrl-shift-c", || Box::new(crate::terminal::Copy)),
    ("paste", "ctrl-shift-v", || Box::new(crate::terminal::Paste)),
];

pub struct Config {
    pub ui: UiColors,
    pub terminal: TermColors,
    pub font_family: SharedString,
    pub font_size: f32,
    /// Action name -> key strings (validated). Empty means unbound.
    keys: HashMap<&'static str, Vec<String>>,
    /// Problems found while loading, shown to the user.
    pub warnings: Vec<String>,
}

impl Global for Config {}

impl Default for Config {
    fn default() -> Self {
        Self {
            ui: UiColors::default(),
            terminal: TermColors::default(),
            font_family: "JetBrainsMono Nerd Font".into(),
            font_size: 15.,
            keys: ACTIONS
                .iter()
                .map(|(name, key, _)| (*name, vec![key.to_string()]))
                .collect(),
            warnings: vec![],
        }
    }
}

// The file format. Everything optional.
#[derive(Deserialize, Default)]
#[serde(default)]
struct File {
    font: FontFile,
    colors: ColorsFile,
    keybindings: HashMap<String, Keys>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct FontFile {
    family: Option<String>,
    size: Option<f32>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ColorsFile {
    ui: HashMap<String, String>,
    terminal: HashMap<String, String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Keys {
    One(String),
    Many(Vec<String>),
}

fn parse_hex(value: &str) -> Option<u32> {
    let hex = value.trim().trim_start_matches('#');
    (hex.len() == 6).then(|| u32::from_str_radix(hex, 16).ok()).flatten()
}

const NAMED_KEYS: &[&str] = &[
    "enter", "tab", "space", "escape", "backspace", "delete", "insert", "up", "down", "left",
    "right", "home", "end", "pageup", "pagedown",
];

/// A key string like "alt-shift-f" or a chord like "ctrl-k ctrl-t".
/// gpui accepts any word as a key name, so typos like "alt-entr" are caught here.
fn valid_key(key: &str) -> bool {
    let known = |name: &str| {
        name.chars().count() == 1
            || NAMED_KEYS.contains(&name)
            || name.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=24).contains(&n))
    };
    !key.trim().is_empty()
        && key
            .split_whitespace()
            .all(|part| Keystroke::parse(part).is_ok_and(|ks| known(&ks.key)))
}

/// Canonical spelling, so "shift-alt-f" and "alt-shift-f" compare equal.
fn normalize(key: &str) -> String {
    key.split_whitespace()
        .map(|part| Keystroke::parse(part).map_or(part.to_owned(), |ks| ks.unparse()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Written on first start: every setting with its default value.
const DEFAULT_CONFIG: &str = include_str!("../config.example.toml");

/// The config file: `$XDG_CONFIG_HOME/vibe_term/config.toml`, else `~/.config/...`.
pub fn path() -> Option<PathBuf> {
    xdg_file(std::env::var_os("XDG_CONFIG_HOME"), std::env::home_dir(), ".config", "config.toml")
}

/// Saved workspaces: `$XDG_STATE_HOME/vibe_term/workspaces.toml`, else `~/.local/state/...`.
pub fn state_path() -> Option<PathBuf> {
    xdg_file(std::env::var_os("XDG_STATE_HOME"), std::env::home_dir(), ".local/state", "workspaces.toml")
}

/// `$XDG_*/vibe_term/<file>`, else `~/<fallback>/vibe_term/<file>`. Per the XDG
/// spec an empty or relative variable is ignored, so files never land relative
/// to wherever the app was launched from.
fn xdg_file(xdg: Option<std::ffi::OsString>, home: Option<PathBuf>, fallback: &str, file: &str) -> Option<PathBuf> {
    let base = xdg
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| home.map(|home| home.join(fallback)))?;
    Some(base.join("vibe_term").join(file))
}

/// Create the config file with defaults; never overwrites an existing one.
fn write_default(path: &std::path::Path) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::File::create_new(path)?.write_all(DEFAULT_CONFIG.as_bytes())
}

impl Config {
    /// Load the user's config; a missing file just means defaults.
    pub fn load() -> Self {
        let Some(path) = path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                // First start: leave a config with every default to edit.
                let mut config = Self::default();
                if let Err(err) = write_default(&path) {
                    config.warnings.push(format!("could not create {}: {err}", path.display()));
                }
                config
            }
            Err(err) => Self {
                warnings: vec![format!("{}: {err}", path.display())],
                ..Self::default()
            },
        }
    }

    pub fn parse(text: &str) -> Self {
        let mut config = Self::default();
        let file: File = match toml::from_str(text) {
            Ok(file) => file,
            Err(err) => {
                config.warnings.push(format!("config not loaded, using defaults: {}", err.message()));
                return config;
            }
        };

        if let Some(family) = file.font.family.filter(|f| !f.trim().is_empty()) {
            config.font_family = family.into();
        }
        match file.font.size {
            Some(size) if (6.0..=72.0).contains(&size) => config.font_size = size,
            Some(size) => config.warnings.push(format!("font.size {size} ignored (allowed: 6 to 72)")),
            None => {}
        }

        for (section, colors) in [("ui", &file.colors.ui), ("terminal", &file.colors.terminal)] {
            for (name, value) in colors {
                let slot = match section {
                    "ui" => config.ui.slot(name),
                    _ => config.terminal.slot(name),
                };
                match (slot, parse_hex(value)) {
                    (None, _) => config.warnings.push(format!("unknown color colors.{section}.{name}")),
                    (Some(_), None) => config
                        .warnings
                        .push(format!("colors.{section}.{name} = \"{value}\" is not a #rrggbb color")),
                    (Some(slot), Some(color)) => *slot = color,
                }
            }
        }

        let mut explicit = std::collections::HashSet::new();
        for (name, keys) in file.keybindings {
            let Some((action, _, _)) = ACTIONS.iter().find(|(n, _, _)| *n == name) else {
                config.warnings.push(format!("unknown action keybindings.{name}"));
                continue;
            };
            let keys = match keys {
                Keys::One(key) => vec![key],
                Keys::Many(keys) => keys,
            };
            let mut valid = vec![];
            for key in keys {
                if key.trim().is_empty() {
                    continue; // "" unbinds
                }
                if valid_key(&key) {
                    valid.push(key);
                } else {
                    config.warnings.push(format!("keybindings.{name}: invalid key \"{key}\""));
                }
            }
            config.keys.insert(action, valid);
            explicit.insert(*action);
        }
        config.drop_duplicate_keys(&explicit);
        config
    }

    /// One key, one action. Keys you set beat defaults; between two of yours,
    /// the action listed first (see config.example.toml) keeps the key.
    fn drop_duplicate_keys(&mut self, explicit: &std::collections::HashSet<&'static str>) {
        let names = ACTIONS.iter().map(|(name, _, _)| *name);
        let order: Vec<&'static str> = names
            .clone()
            .filter(|n| explicit.contains(n))
            .chain(names.filter(|n| !explicit.contains(n)))
            .collect();
        let mut owner: HashMap<String, &'static str> = HashMap::new();
        for name in order {
            let mut kept = vec![];
            for key in std::mem::take(self.keys.get_mut(name).unwrap()) {
                match owner.get(&normalize(&key)) {
                    Some(winner) if *winner == name => {} // listed twice for the same action
                    Some(winner) => self.warnings.push(format!(
                        "\"{key}\" is bound to both {winner} and {name}; {winner} keeps it"
                    )),
                    None => {
                        owner.insert(normalize(&key), name);
                        kept.push(key);
                    }
                }
            }
            self.keys.insert(name, kept);
        }
    }

    pub fn bindings(&self) -> Vec<KeyBinding> {
        ACTIONS
            .iter()
            .flat_map(|(name, _, action)| {
                self.keys[name].iter().filter_map(move |key| {
                    // Keys were validated while parsing, so this only skips the impossible.
                    KeyBinding::load(key, action(), None, false, None, &DummyKeyboardMapper).ok()
                })
            })
            .collect()
    }

    /// Human-readable first key for an action, e.g. "Alt+Shift+F".
    pub fn key_label(&self, action: &str) -> Option<String> {
        let key = self.keys.get(action)?.first()?;
        let label = key
            .split_whitespace()
            .map(|chord| {
                chord
                    .split('-')
                    .map(|part| {
                        let mut chars = part.chars();
                        chars.next().map_or(String::new(), |c| c.to_uppercase().chain(chars).collect())
                    })
                    .collect::<Vec<_>>()
                    .join("+")
            })
            .collect::<Vec<_>>()
            .join(" ");
        Some(label)
    }
}

/// App chrome colors, for render code.
pub fn ui(cx: &App) -> UiColors {
    cx.global::<Config>().ui
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_is_all_defaults() {
        let config = Config::parse("");
        assert!(config.warnings.is_empty());
        assert_eq!(config.ui, UiColors::default());
        assert_eq!(config.keys["new_tab"], vec!["alt-t"]);
    }

    #[test]
    fn overrides_merge_with_defaults() {
        let config = Config::parse(
            r##"
            [font]
            size = 17

            [colors.ui]
            accent = "#7aa2f7"

            [colors.terminal]
            bright_red = "ff0000"

            [keybindings]
            new_tab = ["alt-t", "ctrl-shift-t"]
            toggle_floating = ""
            split_right = "ctrl-k ctrl-l"
            "##,
        );
        assert!(config.warnings.is_empty(), "{:?}", config.warnings);
        assert_eq!(config.font_size, 17.);
        assert_eq!(config.ui.accent, 0x7aa2f7);
        assert_eq!(config.ui.text, UiColors::default().text);
        assert_eq!(config.terminal.ansi[9], 0xff0000);
        assert_eq!(config.keys["new_tab"], vec!["alt-t", "ctrl-shift-t"]);
        assert!(config.keys["toggle_floating"].is_empty());
        assert_eq!(config.keys["close_container"], vec!["alt-q"]);
        assert_eq!(config.key_label("split_right").as_deref(), Some("Ctrl+K Ctrl+L"));
        assert_eq!(config.key_label("toggle_floating"), None);
    }

    #[test]
    fn bad_entries_warn_and_keep_defaults() {
        let config = Config::parse(
            r##"
            [font]
            size = 500

            [colors.ui]
            accent = "blue"
            sparkle = "#ffffff"

            [keybindings]
            teleport = "alt-x"
            new_tab = ["alt-t", "alt-notakey"]
            "##,
        );
        assert_eq!(config.warnings.len(), 5, "{:?}", config.warnings);
        assert_eq!(config.font_size, 15.);
        assert_eq!(config.ui.accent, UiColors::default().accent);
        assert_eq!(config.keys["new_tab"], vec!["alt-t"]);
    }

    #[test]
    fn broken_toml_falls_back_to_defaults() {
        let config = Config::parse("[colors.ui\naccent = ");
        assert_eq!(config.warnings.len(), 1);
        assert_eq!(config.ui, UiColors::default());
    }

    #[test]
    fn config_path_follows_xdg_and_ignores_bad_values() {
        let home = Some(PathBuf::from("/home/u"));
        let at = |xdg: Option<&str>| xdg_file(xdg.map(Into::into), home.clone(), ".config", "config.toml").unwrap();
        assert_eq!(at(None), PathBuf::from("/home/u/.config/vibe_term/config.toml"));
        assert_eq!(at(Some("/xdg")), PathBuf::from("/xdg/vibe_term/config.toml"));
        assert_eq!(at(Some("")), PathBuf::from("/home/u/.config/vibe_term/config.toml"));
        assert_eq!(at(Some("rel/dir")), PathBuf::from("/home/u/.config/vibe_term/config.toml"));
        assert_eq!(xdg_file(None, None, ".config", "config.toml"), None);
        assert_eq!(
            xdg_file(None, home.clone(), ".local/state", "workspaces.toml").unwrap(),
            PathBuf::from("/home/u/.local/state/vibe_term/workspaces.toml")
        );
    }

    #[test]
    fn first_start_writes_defaults_but_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("vibe_term_cfg_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("vibe_term").join("config.toml");

        write_default(&path).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(written, DEFAULT_CONFIG);
        assert!(Config::parse(&written).warnings.is_empty());

        std::fs::write(&path, "# mine").unwrap();
        assert!(write_default(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# mine");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn example_config_matches_defaults() {
        // Keeps config.example.toml honest: every name valid, every value the default.
        let example = Config::parse(include_str!("../config.example.toml"));
        let defaults = Config::default();
        assert!(example.warnings.is_empty(), "{:?}", example.warnings);
        assert_eq!(example.ui, defaults.ui);
        assert_eq!(example.terminal, defaults.terminal);
        assert_eq!(example.font_family, defaults.font_family);
        assert_eq!(example.font_size, defaults.font_size);
        assert_eq!(example.keys, defaults.keys);
    }

    #[test]
    fn duplicate_keys_are_flagged() {
        // Yours beats a default: new_tab loses its default alt-t.
        let config = Config::parse("[keybindings]\nsplit_right = \"alt-t\"");
        assert_eq!(config.warnings.len(), 1, "{:?}", config.warnings);
        assert!(config.warnings[0].contains("split_right keeps it"));
        assert_eq!(config.keys["split_right"], vec!["alt-t"]);
        assert!(config.keys["new_tab"].is_empty());

        // Two of yours: the earlier action in the list wins, whatever the file order.
        // Different spellings of the same key still count as duplicates.
        let config = Config::parse(
            "[keybindings]\nnew_tab = \"shift-alt-x\"\nsplit_down = \"alt-shift-x\"",
        );
        assert_eq!(config.warnings.len(), 1, "{:?}", config.warnings);
        assert_eq!(config.keys["split_down"], vec!["alt-shift-x"]);
        assert!(config.keys["new_tab"].is_empty());

        // Repeating a key for the same action is harmless, not a conflict.
        let config = Config::parse("[keybindings]\nnew_tab = [\"alt-t\", \"alt-t\"]");
        assert!(config.warnings.is_empty(), "{:?}", config.warnings);
        assert_eq!(config.keys["new_tab"], vec!["alt-t"]);
        assert_eq!(config.bindings().len(), ACTIONS.len());
    }

    #[test]
    fn key_names_are_checked() {
        for good in ["alt-enter", "ctrl-shift-t", "super-1", "alt-[", "f5", "ctrl-k ctrl-t"] {
            assert!(valid_key(good), "{good}");
        }
        for bad in ["", "alt-entr", "ctrl-", "alt-f99", "ctrl-k ctrl-nope"] {
            assert!(!valid_key(bad), "{bad}");
        }
    }

    #[test]
    fn every_default_key_is_valid() {
        let config = Config::default();
        for (name, key, _) in ACTIONS {
            assert!(valid_key(key), "{name}: {key}");
        }
        assert_eq!(config.bindings().len(), ACTIONS.len());
    }
}
