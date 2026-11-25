//! Lodestar — fast game library and controller-mode frontend for Linux.

mod achievements;
mod addgame;
mod addons;
mod app;
mod app_locate;
mod art;
mod art_app;
mod cli;
mod client_launch;
mod cloud;
mod config;
mod db;
mod edit_app;
mod emu_app;
mod emulation;
mod exclude;
mod exclude_app;
mod fsutil;
mod game_options;
mod game_options_app;
mod game_options_bulk;
mod icon;
mod igdb;
mod images;
mod input;
mod instance;
mod launch;
mod launchtest;
mod library;
mod main_window;
mod media;
mod metadata;
mod missing_app;
mod model;
mod procfs;
mod romm;
mod romm_app;
mod running_app;
mod scan;
mod search;
mod steam_names;
mod steam_names_app;
mod steam_owned;
mod store_app;
#[cfg(test)]
mod test_support;
mod theme;
mod tray;
mod tray_app;
mod ui;
mod update;
mod update_app;
mod welcome_app;
mod winctl;

use app::{LaunchOptions, Mode};
use config::{Config, Paths};

fn main() {
    images::tune_allocator();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("lodestar=info,warn"))
        .format_timestamp_millis()
        .init();

    let cli::Cli { mut opts, windowed, size, command } = cli::parse(std::env::args().skip(1));
    if opts.screenshot.is_some() {
        rerun_headless();
    }

    let paths = Paths::detect();
    paths.ensure();
    let config = Config::load(&paths.config_file());
    // Session-only overrides: never written back to config.toml.
    opts.windowed = windowed || opts.screenshot.is_some();
    opts.no_minimize = opts.screenshot.is_some();

    if let Some(command) = command {
        // Command-line output: `lodestar --list | head` should end quietly, not panic with
        // "failed printing to stdout: Broken pipe". Only for these print-and-exit actions: the
        // GUI keeps Rust's default (SIGPIPE ignored), so a dropped network connection can never
        // kill it.
        // SAFETY: plain libc call made before any other thread exists.
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        }
        std::process::exit(cli::run(command, &paths, &config));
    }
    run_window(paths, config, opts, size);
}

/// Screenshot test runs draw on an invisible virtual display when one is available, so test
/// windows never pop up on the user's desktop or take focus. LODESTAR_VISIBLE=1 shows them.
/// Returns only when the run stays in this process (no `xvfb-run`, or already its child).
fn rerun_headless() {
    if std::env::var_os("LODESTAR_VISIBLE").is_some() || std::env::var_os("LODESTAR_HEADLESS_CHILD").is_some() {
        return;
    }
    let (Some(xvfb), Ok(me)) = (launch::find_in_path("xvfb-run"), std::env::current_exe()) else { return };
    let status = std::process::Command::new(xvfb)
        .args(["-a", "-s", "-screen 0 2560x1600x24 -nolisten tcp"])
        .arg(me)
        .args(std::env::args_os().skip(1))
        .env("LODESTAR_HEADLESS_CHILD", "1")
        .env("XDG_SESSION_TYPE", "x11")
        .env("LIBGL_ALWAYS_SOFTWARE", "1")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .status();
    match status {
        Ok(s) => std::process::exit(s.code().unwrap_or(1)),
        Err(e) => eprintln!("headless run failed ({e}); showing the window instead"),
    }
}

/// Open the window (desktop or controller mode) and run until it closes.
fn run_window(paths: Paths, config: Config, opts: LaunchOptions, size: Option<[f32; 2]>) {
    // One window per library: a second start shows the running one and exits (instance.rs).
    // Screenshot runs are always their own.
    let instance = if opts.screenshot.is_none() {
        let message = match opts.force_mode {
            Some(Mode::Controller) => "activate controller",
            Some(Mode::Desktop) => "activate desktop",
            None => "activate",
        };
        let key = instance::key(&paths.data_dir);
        match instance::acquire(&instance::runtime_dir(), &key, message, std::time::Duration::from_secs(4)) {
            instance::Start::First(i) => Some(i),
            instance::Start::HandedOver => {
                eprintln!("Lodestar is already running: showing its window");
                return;
            }
            instance::Start::Alone => None,
        }
    } else {
        None
    };

    let db = match db::Db::open(&paths.db_file()) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("cannot open library database {}: {e:#}", paths.db_file().display());
            std::process::exit(1);
        }
    };

    let controller_start = opts.force_mode == Some(Mode::Controller)
        || (opts.force_mode.is_none() && config.start_mode == config::StartMode::Controller);
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Lodestar")
        .with_app_id("lodestar")
        .with_inner_size(size.map_or(main_window::DEFAULT, egui::Vec2::from))
        .with_min_inner_size(main_window::MIN);
    // As it was left (size, maximised; on X11 its place too): not for --size / screenshot runs
    // (unless LODESTAR_RESTORE_WINDOW=1, a testing hook).
    if size.is_none() && (opts.screenshot.is_none() || std::env::var_os("LODESTAR_RESTORE_WINDOW").is_some()) {
        let saved = config.desktop.windows.get(main_window::KEY);
        let wayland = app::is_wayland(|k| std::env::var_os(k));
        let monitors = if wayland { vec![] } else { main_window::x11_monitors() };
        viewport = main_window::restore(viewport, saved, wayland, &monitors);
        main_window::set_monitors(monitors);
    }
    if let Some(ic) = icon::window_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(ic));
    }
    if controller_start && config.controller.fullscreen && !opts.windowed {
        viewport = viewport.with_fullscreen(true);
    }
    let native = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Glow, ..Default::default() };
    let result = eframe::run_native(
        "lodestar",
        native,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, paths, config, db, opts, instance)))),
    );
    // A library add-on may still be answering a scan: it doesn't outlive the app.
    addons::run::kill_running();
    // "Restart now" after an update: this process becomes the new AppImage.
    if result.is_ok()
        && let Some(appimage) = update::take_restart()
    {
        let e = update::exec_restart(&appimage);
        eprintln!("lodestar: couldn't start the updated Lodestar ({}): {e}", appimage.display());
    }
    if let Err(e) = result {
        eprintln!("lodestar: {e}");
        std::process::exit(1);
    }
}
