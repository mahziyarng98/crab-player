mod app;
mod command;
mod config;
mod dialogs;
mod media;
mod player;
mod playlist;
mod startup;
mod ui;
mod wayland_dnd;

use clap::Parser;
use tracing_subscriber::EnvFilter;

/// Command line surface: files, folders or URLs, as passed by a file
/// manager or typed in a shell.
#[derive(Debug, Parser)]
#[command(
    name = "crab-player",
    version,
    about = "Crab Player: a fast, native media player for Linux"
)]
struct Cli {
    /// Files, folders or URLs to open
    #[arg(value_name = "MEDIA")]
    media: Vec<String>,
}

fn main() -> eframe::Result {
    startup::begin();
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("crab_player=info,warn")),
        )
        .try_init();

    let cli = Cli::parse();
    // libmpv is opened at runtime, so a missing library is reported here
    // (and in the window) instead of by the dynamic loader.
    #[cfg(feature = "mpv")]
    if let Err(reason) = player::mpv_dl::load() {
        eprintln!("{}", player::mpv_dl::missing_help(&reason));
    }
    // Settings written under the project's earlier working name are carried
    // over once, before anything reads them.
    config::migrate_legacy();
    let (config, config_warning) = config::AppConfig::load();
    startup::mark("config loaded");

    // Shown by X11 window managers; Wayland compositors take the icon from
    // the desktop entry that matches the app id.
    let icon = eframe::icon_data::from_png_bytes(app::ICON_PNG)
        .inspect_err(|err| tracing::warn!(%err, "could not decode the window icon"))
        .unwrap_or_default();

    let native_options = eframe::NativeOptions {
        // glow/OpenGL is deliberate: it is the only backend that lets libmpv
        // render into an FBO owned by the egui painter on X11 *and* Wayland.
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_app_id(app::APP_ID)
            .with_title(app::APP_NAME)
            .with_icon(icon)
            // The caption strip is the title bar (drag, maximize, close);
            // see `ui::caption`.
            .with_decorations(false)
            .with_inner_size([1120.0, 700.0])
            .with_min_inner_size(app::MIN_WINDOW_SIZE),
        // Window state stays in `~/.local/share/crab-player/app.ron`, next
        // to the other data, instead of a folder named after the app id.
        persistence_path: eframe::storage_dir(app::APP_SLUG).map(|dir| dir.join("app.ron")),
        ..Default::default()
    };

    eframe::run_native(
        app::APP_NAME,
        native_options,
        Box::new(move |cc| {
            Ok(Box::new(app::PlayerApp::new(
                cc,
                config,
                config_warning,
                cli.media,
            )))
        }),
    )
}
