//! Reads the Helix theme named in Helix's config.toml and turns it into
//! Helterm colors (`use_helix_theme = true`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use toml::{Table, Value};

/// A color for a Helterm slot: ("ui" | "terminal", slot name, 0xrrggbb).
pub type Color = (&'static str, &'static str, u32);

#[derive(Default)]
pub struct HelixTheme {
    pub colors: Vec<Color>,
    /// Helix's config plus every theme file read, to notice edits.
    pub files: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

/// `$XDG_CONFIG_HOME/helix`, else `~/.config/helix`, like Helix itself.
fn config_dir() -> Option<PathBuf> {
    crate::config::xdg_base(std::env::var_os("XDG_CONFIG_HOME"), std::env::home_dir(), ".config")
        .map(|base| base.join("helix"))
}

/// Load the theme Helix uses in `dark` (or light) mode. No theme set, or one
/// of Helix's built-in ones, means no colors: Helterm keeps its own.
pub fn load(dark: bool) -> HelixTheme {
    let Some(dir) = config_dir() else {
        return HelixTheme::default();
    };
    let config_path = dir.join("config.toml");
    let mut theme = HelixTheme {
        files: vec![config_path.clone()],
        ..Default::default()
    };
    let text = match std::fs::read_to_string(&config_path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return theme,
        Err(err) => {
            theme.warnings.push(format!("{}: {err}", config_path.display()));
            return theme;
        }
    };
    let name = match theme_name(&text, dark) {
        Ok(Some(name)) => name,
        Ok(None) => return theme,
        Err(err) => {
            theme.warnings.push(format!("{}: {err}", config_path.display()));
            return theme;
        }
    };
    let mut search = vec![dir.join("themes")];
    search.extend(runtime_dirs(&dir).into_iter().map(|d| d.join("themes")));
    match read_chain(&name, &search, &mut theme.files) {
        Ok(tables) => theme.colors = colors(&merge(tables)),
        Err(err) => theme.warnings.push(format!("helix theme: {err}")),
    }
    theme
}

/// `theme = "name"`, or `[theme]` with `light` / `dark` (and `fallback`,
/// which Helix only uses when the terminal doesn't say; Helterm always does).
fn theme_name(config: &str, dark: bool) -> Result<Option<String>, String> {
    let table: Table = toml::from_str(config).map_err(|err| err.message().to_owned())?;
    let name = match table.get("theme") {
        None => None,
        Some(Value::String(name)) => Some(name.clone()),
        Some(Value::Table(modes)) => {
            let pick = |key: &str| modes.get(key).and_then(Value::as_str).map(str::to_owned);
            match dark {
                true => pick("dark"),
                false => pick("light").or_else(|| pick("dark")),
            }
        }
        Some(_) => return Err("theme must be a name or a table with light/dark".into()),
    };
    // Built into the Helix binary, there is no file to read.
    Ok(name.filter(|n| n != "default" && n != "base16_default"))
}

/// Where Helix's bundled themes may live. Distros pick the folder at build time.
/// shortcut: a list of known install layouts, add one when a distro is missed.
fn runtime_dirs(config_dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("HELIX_RUNTIME").map(PathBuf::from).into_iter().collect();
    dirs.push(config_dir.join("runtime"));
    let hx = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|dir| dir.join("hx"))
        .find(|hx| hx.is_file())
        .and_then(|hx| std::fs::canonicalize(hx).ok());
    if let Some(bin) = hx.as_deref().and_then(Path::parent) {
        // Next to the binary (release tarballs), or under its prefix (distros, Homebrew).
        dirs.push(bin.join("runtime"));
        for rel in ["../libexec/runtime", "../lib/helix/runtime", "../lib64/helix/runtime", "../share/helix/runtime"] {
            dirs.push(bin.join(rel));
        }
    }
    for dir in ["/usr/lib/helix/runtime", "/usr/lib64/helix/runtime", "/usr/share/helix/runtime"] {
        dirs.push(PathBuf::from(dir));
    }
    dirs
}

/// The theme and the themes it inherits from, base first.
fn read_chain(name: &str, search: &[PathBuf], files: &mut Vec<PathBuf>) -> Result<Vec<Table>, String> {
    let mut chain = vec![];
    let mut name = name.to_owned();
    loop {
        // A theme may inherit one of the same name (a user tweak of a bundled
        // theme): skip files already read. This also ends inheritance loops.
        let path = search
            .iter()
            .map(|dir| dir.join(format!("{name}.toml")))
            .find(|path| path.is_file() && !files.contains(path))
            .ok_or_else(|| format!("theme \"{name}\" not found"))?;
        let text = std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
        let table: Table = toml::from_str(&text).map_err(|err| format!("{}: {}", path.display(), err.message()))?;
        files.push(path);
        let parent = table.get("inherits").and_then(Value::as_str).map(str::to_owned);
        chain.push(table);
        match parent {
            Some(parent) if parent != "default" && parent != "base16_default" => name = parent,
            _ => break,
        }
    }
    chain.reverse();
    Ok(chain)
}

/// Child themes override their parents' keys; palettes merge.
fn merge(chain: Vec<Table>) -> Table {
    let mut merged = Table::new();
    let mut palette = Table::new();
    for table in chain {
        for (key, value) in table {
            match (key.as_str(), value) {
                ("palette", Value::Table(p)) => palette.extend(p),
                (_, value) => {
                    merged.insert(key, value);
                }
            }
        }
    }
    merged.insert("palette".into(), Value::Table(palette));
    merged
}

/// Helix's names for the terminal's 16 colors, as Helterm's default palette.
fn ansi_name(name: &str) -> Option<u32> {
    let ix = match name {
        "black" => 0,
        "red" => 1,
        "green" => 2,
        "yellow" => 3,
        "blue" => 4,
        "magenta" => 5,
        "cyan" => 6,
        "light-gray" => 7,
        "gray" => 8,
        "light-red" => 9,
        "light-green" => 10,
        "light-yellow" => 11,
        "light-blue" => 12,
        "light-magenta" => 13,
        "light-cyan" => 14,
        "white" => 15,
        _ => return None,
    };
    Some(crate::config::TermColors::default().ansi[ix])
}

fn hex(value: &str) -> Option<u32> {
    let hex = value.strip_prefix('#')?;
    (hex.len() == 6).then(|| u32::from_str_radix(hex, 16).ok()).flatten()
}

struct Lookup<'a> {
    theme: &'a Table,
    palette: HashMap<&'a str, &'a str>,
}

impl Lookup<'_> {
    fn color(&self, value: &str) -> Option<u32> {
        hex(value)
            .or_else(|| self.palette.get(value).and_then(|v| hex(v).or_else(|| ansi_name(v))))
            .or_else(|| ansi_name(value))
    }

    /// `attr` ("fg" / "bg") of `scope`, falling back to shorter scopes like
    /// Helix does: ui.cursor.primary, then ui.cursor, then ui.
    fn get(&self, scope: &str, attr: &str) -> Option<u32> {
        let mut scope = scope;
        loop {
            let value = match self.theme.get(scope) {
                Some(Value::String(fg)) if attr == "fg" => Some(fg.as_str()),
                Some(Value::Table(style)) => style.get(attr).and_then(Value::as_str),
                _ => None,
            };
            if let Some(color) = value.and_then(|v| self.color(v)) {
                return Some(color);
            }
            scope = &scope[..scope.rfind('.')?];
        }
    }

    /// First of `scopes` that sets `attr`.
    fn first(&self, attr: &str, scopes: &[&str]) -> Option<u32> {
        scopes.iter().find_map(|scope| self.get(scope, attr))
    }
}

/// Helix scopes -> Helterm slots. Helix themes have no terminal palette, so the
/// 16 terminal colors borrow syntax colors of the same hue.
fn colors(theme: &Table) -> Vec<Color> {
    let palette = match theme.get("palette") {
        Some(Value::Table(p)) => p.iter().filter_map(|(k, v)| Some((k.as_str(), v.as_str()?))).collect(),
        _ => HashMap::new(),
    };
    let t = Lookup { theme, palette };
    let text = t.get("ui.text", "fg");
    let background = t.get("ui.background", "bg");
    let red = t.first("fg", &["error", "diagnostic.error"]);
    let green = t.first("fg", &["string", "diff.plus"]);
    let yellow = t.first("fg", &["warning", "diagnostic.warning"]);
    let blue = t.first("fg", &["function", "ui.text.focus"]);
    let magenta = t.first("fg", &["keyword"]);
    let cyan = t.first("fg", &["type", "constant"]);
    let black = t.first("bg", &["ui.menu", "ui.statusline", "ui.cursorline.primary"]);
    let comment = t.first("fg", &["comment", "ui.linenr"]);
    let slots: [(&'static str, &'static str, Option<u32>); 31] = [
        ("ui", "background", background),
        ("ui", "panel", t.first("bg", &["ui.statusline", "ui.menu"])),
        ("ui", "sidebar", t.first("bg", &["ui.popup", "ui.menu"])),
        ("ui", "element", t.first("bg", &["ui.selection", "ui.cursorline.primary"])),
        ("ui", "border", t.first("fg", &["ui.window", "ui.virtual.ruler"])),
        ("ui", "text", text),
        ("ui", "muted", comment),
        ("ui", "accent", t.first("fg", &["function", "ui.text.focus"])),
        ("ui", "error", red),
        ("ui", "warning", yellow),
        ("ui", "success", green),
        ("terminal", "foreground", text),
        ("terminal", "background", background),
        ("terminal", "cursor", t.first("bg", &["ui.cursor.primary", "ui.cursor"])),
        ("terminal", "selection", t.get("ui.selection", "bg")),
        ("terminal", "black", black),
        ("terminal", "red", red),
        ("terminal", "green", green),
        ("terminal", "yellow", yellow),
        ("terminal", "blue", blue),
        ("terminal", "magenta", magenta),
        ("terminal", "cyan", cyan),
        ("terminal", "white", text),
        ("terminal", "bright_black", comment),
        ("terminal", "bright_red", red),
        ("terminal", "bright_green", green),
        ("terminal", "bright_yellow", yellow),
        ("terminal", "bright_blue", blue),
        ("terminal", "bright_magenta", magenta),
        ("terminal", "bright_cyan", cyan),
        ("terminal", "bright_white", text),
    ];
    slots.into_iter().filter_map(|(section, slot, color)| Some((section, slot, color?))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_name_follows_light_and_dark() {
        assert_eq!(theme_name("theme = \"tokyonight\"", false).unwrap().as_deref(), Some("tokyonight"));
        let modes = "[theme]\ndark = \"night\"\nlight = \"day\"\nfallback = \"other\"";
        assert_eq!(theme_name(modes, true).unwrap().as_deref(), Some("night"));
        assert_eq!(theme_name(modes, false).unwrap().as_deref(), Some("day"));
        // No light theme: Helix uses the dark one.
        assert_eq!(theme_name("[theme]\ndark = \"night\"", false).unwrap().as_deref(), Some("night"));
        assert_eq!(theme_name("[editor]\nline-number = \"relative\"", true).unwrap(), None);
        assert_eq!(theme_name("theme = \"default\"", true).unwrap(), None);
        assert!(theme_name("theme = 3", true).is_err());
    }

    #[test]
    fn themes_inherit_resolve_palettes_and_fall_back_to_shorter_scopes() {
        let dir = std::env::temp_dir().join(format!("helterm_hxtheme_{}", std::process::id()));
        let (user, runtime) = (dir.join("user"), dir.join("runtime"));
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::write(
            runtime.join("base.toml"),
            r##"
            "ui.background" = { bg = "bg" }
            "ui.text" = "fg"
            "ui.cursor" = { fg = "bg", bg = "#ff00ff" }
            "error" = "red"
            [palette]
            bg = "#000001"
            fg = "#000002"
            "##,
        )
        .unwrap();
        // Same name as a user theme: the user's inherits the bundled one.
        std::fs::write(runtime.join("mine.toml"), "inherits = \"base\"\n\"string\" = \"#00ff00\"").unwrap();
        std::fs::write(
            user.join("mine.toml"),
            "inherits = \"mine\"\n\"ui.text\" = { fg = \"accent\" }\n[palette]\nbg = \"#000003\"\naccent = \"#000004\"",
        )
        .unwrap();

        let mut files = vec![];
        let chain = read_chain("mine", &[user.clone(), runtime.clone()], &mut files).unwrap();
        assert_eq!(files.len(), 3);
        let colors = colors(&merge(chain));
        let get = |section, slot| colors.iter().find(|c| c.0 == section && c.1 == slot).map(|c| c.2);
        assert_eq!(get("ui", "background"), Some(0x000003)); // child palette wins
        assert_eq!(get("ui", "text"), Some(0x000004)); // child key wins
        assert_eq!(get("terminal", "cursor"), Some(0xff00ff)); // ui.cursor.primary -> ui.cursor
        assert_eq!(get("terminal", "green"), Some(0x00ff00));
        assert_eq!(get("terminal", "red"), Some(crate::config::TermColors::default().ansi[1])); // ANSI name
        assert_eq!(get("ui", "panel"), None); // not in the theme: Helterm's stays

        std::fs::write(runtime.join("loop.toml"), "inherits = \"loop2\"").unwrap();
        std::fs::write(runtime.join("loop2.toml"), "inherits = \"loop\"").unwrap();
        assert!(read_chain("loop", &[runtime.clone()], &mut vec![]).is_err());
        assert!(read_chain("missing", &[runtime], &mut vec![]).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
