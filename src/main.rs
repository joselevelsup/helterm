mod config;
mod container;
mod explorer;
mod helix_theme;
mod terminal;
mod workspace;

use gpui::{App, AppContext, Application, Bounds, TitlebarOptions, WindowAppearance, WindowBounds, WindowOptions, px, size};

use crate::config::Config;
use crate::workspace::Workspace;

/// Must match the .desktop file name and icon name in packaging/, so launchers
/// and docks can pair the window with its icon.
const APP_ID: &str = "io.github.joselevelsup.Helterm";

fn main() {
    Application::new().run(|cx: &mut App| {
        let mut config = Config::load();
        config.dark = matches!(cx.window_appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark);
        if config.use_helix_theme {
            config.apply_helix_theme(helix_theme::load(config.dark));
        }
        for warning in &config.warnings {
            eprintln!("helterm config: {warning}");
        }
        cx.bind_keys(config.bindings());
        cx.set_global(config);

        let bounds = Bounds::centered(None, size(px(1100.), px(750.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                app_id: Some(APP_ID.to_owned()),
                titlebar: Some(TitlebarOptions {
                    title: Some("Helterm".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| Workspace::new(window, cx))
            },
        )
        .expect("failed to open window");
    });
}
