//! The command line: `lodestar [OPTIONS]` parsed into window options or a print-and-exit command.
//!
//! [`parse`] runs the read-only diagnostics (`--running`, `--emulation-report` …) and `--help` on
//! the spot and exits, as they need nothing else. Commands that use the library or the themes
//! folder come back as a [`Command`] for [`run`], after `main` has set up the paths and the config.
//! Errors in the options print the problem and the help, and exit with status 2.

use crate::app::{LaunchOptions, Mode};
use crate::config::{Config, Paths};
use crate::input::Nav;
use crate::{
    client_launch, db, emulation, game_options, game_options_app, icon, launch, launchtest, media, model, scan, theme,
};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const HELP: &str = "\
Lodestar — game library for Linux

USAGE: lodestar [OPTIONS]

  --controller              start in controller (fullscreen) mode
  --desktop                 start in desktop mode
  --windowed                controller mode in a window (no fullscreen)
  --scan                    scan for games, print a report, exit (no window)
  --list                    print the library (with game ids), exit
  --play ID                 start the game with this id through Lodestar's launcher (launch options,
                            scripts, playtime) without opening a window; exits when the game exits
                            (what desktop shortcuts run)
  --launch NAME             launch one game from the terminal and report what happens, exit
      --direct              bypass Steam / Faugus with the direct command
      --options TEXT        Steam-style launch options for this run
      --stop-after SECS     stop the game SECS after it started
      --timeout SECS        give up after SECS (default 240)
      --dry-run             only print the resolved command
  --import-playnite-theme PATH  import a Playnite theme (.pthm package or theme folder), print
                            what was carried over, exit
  --playnite-look PATH      print what Lodestar reads from a Playnite theme (JSON), exit
      --layout classic|row|rail  the layout a fullscreen theme is laid onto (default: from the
                            theme's game list)
      --media on|off        copy the theme's videos and sounds (default: when under 150 MB)
  --theme-options KEY       list an imported Playnite theme's options, exit
      --set K=V,K2=V2       apply these option values (the others keep the theme's own)
  --emulation-report        print the emulators, ROM folders and BIOS Lodestar finds, exit
  --no-scan                 don't scan on startup
  --size WxH                initial window size (logical px)
  --screenshot FILE.png     render, save a screenshot, exit (for testing)
      --frames N            settle frames before the screenshot (default 30)
      --inject a,b,..       controller inputs to send first (up,down,left,right,accept,back,menu,
                            select,favorite,search,pageup,pagedown,jumpup,jumpdown,guide;
                            right*20 = twenty times)
  -h, --help                this help

ENVIRONMENT
  LODESTAR_HOME=DIR         keep config, library and caches under DIR (testing)
  LODESTAR_FRAME_LOG=1      log every frame with its repaint causes (finding repaint loops)
  LODESTAR_SCREENSHOT_ADOPT=1  screenshot runs also pick up games that run (never stopping them)
  LODESTAR_IDLE_PROBE=SECS  screenshot runs: once settled, count the frames drawn by themselves
                            over SECS seconds (an idle window should draw none)
  LODESTAR_FRAME_STATS=1    screenshot runs: print the time spent per frame (mean, p95, worst)
  LODESTAR_LATENCY_PROBE=1  screenshot runs: time every injected input until its frame and its
                            pictures are on screen, list slow frames and peak memory
  LODESTAR_INJECT_MS=N      screenshot runs: send injected inputs N ms apart (default: every
                            4th frame)
  LODESTAR_UI_TIMING=1      log desktop UI time per frame
  LODESTAR_FAKE_PAD=VID:NAME  act as if this gamepad were in use for button prompts, e.g.
                            054c:Wireless Controller (screenshot runs never look at real pads)
  RUST_LOG=lodestar=debug   more logging
";

/// `--launch`'s default `--timeout`, in seconds.
const LAUNCH_TEST_TIMEOUT_SECS: u64 = 240;
/// `--frames`' default: settle frames before a screenshot.
const SCREENSHOT_FRAMES: u32 = 30;
/// How long `--play` waits for `notify-send` to show its failure notification.
const NOTIFY_WAIT: Duration = Duration::from_secs(3);

/// What the command line asked for.
pub struct Cli {
    pub opts: LaunchOptions,
    /// `--windowed`: controller mode in a window.
    pub windowed: bool,
    /// `--size WxH`: the window's initial size (logical px).
    pub size: Option<[f32; 2]>,
    /// A print-and-exit command; None opens the window.
    pub command: Option<Command>,
}

/// A command that prints its result and exits without opening a window.
pub enum Command {
    ImportPlayniteTheme(PathBuf, theme::playnite::ImportOptions),
    ThemeOptions { key: String, set: Option<String> },
    Scan,
    List,
    Play(String),
    Launch(launchtest::Opts),
}

/// The simple commands; the last one given wins.
#[derive(Clone, Copy)]
enum Action {
    Scan,
    List,
    Play,
    Launch,
}

/// Parse the options (without the program name). Diagnostics and `--help` run here and exit.
pub fn parse(mut args: impl Iterator<Item = String>) -> Cli {
    let mut opts = LaunchOptions { screenshot_frames: SCREENSHOT_FRAMES, ..Default::default() };
    let mut windowed = false;
    let mut size: Option<[f32; 2]> = None;
    let mut playnite_theme: Option<PathBuf> = None;
    let mut playnite_opts = theme::playnite::ImportOptions::default();
    let mut theme_options: Option<String> = None;
    let mut option_set: Option<String> = None;
    let mut action: Option<Action> = None;
    let mut play_id = String::new();
    let mut lt = launchtest::Opts { timeout: LAUNCH_TEST_TIMEOUT_SECS, ..Default::default() };

    while let Some(a) = args.next() {
        // Options that take a value: a missing value is an error, not a silent "ignore".
        let mut value = |what: &str| -> String {
            args.next().unwrap_or_else(|| {
                eprintln!("{a} needs {what}\n\n{HELP}");
                std::process::exit(2);
            })
        };
        match a.as_str() {
            "--controller" => opts.force_mode = Some(Mode::Controller),
            "--desktop" => opts.force_mode = Some(Mode::Desktop),
            "--windowed" => windowed = true,
            "--scan" => action = Some(Action::Scan),
            "--list" => action = Some(Action::List),
            "--play" => {
                action = Some(Action::Play);
                play_id = value("a game id (see --list)");
            }
            "--client-launch" => {
                // Internal: started by Lodestar's launcher for store clients
                // (battlenet, ea, generic).
                let kind = args.next().unwrap_or_default();
                let mut rest = args.by_ref();
                std::process::exit(client_launch::cli(&kind, &mut rest));
            }
            "--launch" => {
                action = Some(Action::Launch);
                lt.query = args.next().unwrap_or_default();
            }
            "--direct" => lt.direct = true,
            "--options" => lt.options = args.next(),
            "--stop-after" => lt.stop_after = args.next().and_then(|v| v.parse().ok()),
            "--timeout" => lt.timeout = args.next().and_then(|v| v.parse().ok()).unwrap_or(LAUNCH_TEST_TIMEOUT_SECS),
            "--dry-run" => lt.dry_run = true,
            "--minimise-pid" => minimise_pid(args.next().and_then(|v| v.parse().ok()).unwrap_or(0)),
            "--running" => print_running(),
            "--game-options-report" => print_game_options_report(),
            "--emulation-report" => print_emulation_report(),
            "--list-windows" => print_windows(),
            "--export-icons" => export_icons(args.next().map(PathBuf::from)),
            "--playnite-look" => print_playnite_look(PathBuf::from(value("a .pthm file or a theme folder"))),
            "--no-scan" => opts.no_scan = true,
            "--import-playnite-theme" => playnite_theme = Some(PathBuf::from(value("a .pthm file or a theme folder"))),
            "--layout" => {
                let v = value("classic, row or rail");
                playnite_opts.family = theme::playnite::Family::parse(&v);
                if playnite_opts.family.is_none() {
                    eprintln!("--layout: expected classic, row or rail");
                    std::process::exit(2);
                }
            }
            "--theme-options" => theme_options = Some(value("an imported theme's key")),
            "--set" => option_set = Some(value("KEY=VALUE pairs")),
            "--media" => {
                playnite_opts.media = Some(!matches!(value("on or off").as_str(), "off" | "no" | "0" | "false"))
            }
            "--screenshot" => opts.screenshot = Some(PathBuf::from(value("a file name"))),
            "--frames" => opts.screenshot_frames = value("a number").parse().unwrap_or(SCREENSHOT_FRAMES),
            "--inject" => opts.inject = parse_navs(&value("a list of inputs")),
            "--size" => {
                size = parse_size(&value("a size like 1600x900"));
                if size.is_none() {
                    eprintln!("--size: expected WIDTHxHEIGHT, e.g. 1600x900");
                    std::process::exit(2);
                }
            }
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown option {other}\n\n{HELP}");
                std::process::exit(2);
            }
        }
    }

    // A theme import and theme options win over the simple commands, in this order.
    let command = if let Some(src) = playnite_theme {
        Some(Command::ImportPlayniteTheme(src, playnite_opts))
    } else if let Some(key) = theme_options {
        Some(Command::ThemeOptions { key, set: option_set })
    } else {
        action.map(|a| match a {
            Action::Scan => Command::Scan,
            Action::List => Command::List,
            Action::Play => Command::Play(play_id),
            Action::Launch => Command::Launch(lt),
        })
    };
    Cli { opts, windowed, size, command }
}

/// Run a print-and-exit command; returns the process exit status.
pub fn run(command: Command, paths: &Paths, config: &Config) -> i32 {
    match command {
        Command::ImportPlayniteTheme(src, opts) => import_playnite_theme(paths, &src, &opts),
        Command::ThemeOptions { key, set } => theme_options(paths, &key, set.as_deref()),
        Command::Scan => scan_and_report(paths, config),
        Command::List => list_library(paths),
        Command::Play(id) => play(paths, config, &id),
        Command::Launch(lt) => launch_test(paths, config, &lt),
    }
}

// ------------------------------------------------------------------------------ diagnostics

/// `--minimise-pid`: minimise one process' windows the way launcher hygiene does.
fn minimise_pid(pid: u32) -> ! {
    let x11 = launch::winhide::iconify_windows_of(None, &[pid]);
    println!("x11: {x11:?}");
    if !matches!(x11, Ok(n) if n > 0) {
        println!("compositor: {:?}", launch::winhide_wayland::minimise(&[pid]));
    }
    std::process::exit(0);
}

/// `--running`: what runs that Lodestar would pick up or keep the controller waiting for, and
/// the controllers connected (read-only).
fn print_running() -> ! {
    // `lodestar --running | head` ends quietly, like the commands `main` runs.
    // SAFETY: plain libc call made before any other thread exists.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let root = std::path::Path::new("/proc");
    let me = std::process::id() as i32;
    let procs = launch::track::read_procs(true);
    let found = launch::track::find_running(&procs, root, me);
    // Read-only, from the folder the app uses; without a library of this version: no names.
    let games = db::Db::open_read_only(&Paths::detect().db_file()).and_then(|d| d.load_all().ok()).unwrap_or_default();
    let dirs: Vec<PathBuf> = games
        .iter()
        .filter(|g| g.installed)
        .filter_map(|g| g.install_dir.clone())
        .filter(|d| launch::is_game_folder(d))
        .collect();
    print!("{}", launch::track::running_report(&found, &procs, root, me, &dirs, &games));
    print_controllers();
    std::process::exit(0);
}

/// The controllers Lodestar can read, Steam Input's virtual one marked.
fn print_controllers() {
    let gilrs = match gilrs::GilrsBuilder::new().with_force_feedback(false).build() {
        Ok(g) => g,
        Err(e) => return println!("Controllers: can't read them ({e})"),
    };
    let pads: Vec<_> = gilrs.gamepads().collect();
    println!("Controllers:");
    if pads.is_empty() {
        println!("  none connected");
    }
    for (id, p) in pads {
        let info = crate::input::style::PadInfo {
            id: usize::from(id),
            name: p.name().to_string(),
            vendor: p.vendor_id(),
            product: p.product_id(),
        };
        let hex = |v: Option<u16>| v.map_or("?".into(), |v| format!("{v:04x}"));
        let steam = if info.is_virtual() { " (Steam Input's virtual pad)" } else { "" };
        println!("  {} [{}:{}]{steam}", info.name, hex(info.vendor), hex(info.product));
    }
}

/// `--game-options-report`: what game options can use here (read-only; one `gamescope --help`).
fn print_game_options_report() -> ! {
    let t = Instant::now();
    let caps = game_options::caps::probe();
    print!("{}", game_options_app::caps_report(&caps));
    println!("(probed in {:?})", t.elapsed());
    std::process::exit(0);
}

/// `--emulation-report`: emulators found and how, ROM folders → consoles, BIOS (read-only).
fn print_emulation_report() -> ! {
    let cfg = Config::load(&Paths::detect().config_file());
    print!("{}", emulation::report::report(&scan::ScanContext::from_config(&cfg)));
    std::process::exit(0);
}

/// `--list-windows`: the X11 windows launcher hygiene can see.
fn print_windows() -> ! {
    match launch::winhide::list_windows(None) {
        Ok(v) => v.iter().for_each(|l| println!("{l}")),
        Err(e) => eprintln!("{e:#}"),
    }
    std::process::exit(0);
}

/// `--export-icons [DIR]`: write the app icon in every size (default: the user's hicolor theme).
fn export_icons(dir: Option<PathBuf>) -> ! {
    let dir = dir.unwrap_or_else(|| dirs::data_dir().unwrap_or_default().join("icons/hicolor"));
    match icon::export(&dir) {
        Ok(()) => println!("icons written to {}", dir.display()),
        Err(e) => {
            eprintln!("icon export failed: {e:#}");
            std::process::exit(1);
        }
    }
    std::process::exit(0);
}

/// `--playnite-look`: what the Playnite theme resolver reads from a theme (JSON).
fn print_playnite_look(src: PathBuf) -> ! {
    match theme::playnite::extract::report(&src) {
        Ok(j) => println!("{j}"),
        Err(e) => {
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    }
    std::process::exit(0);
}

// ------------------------------------------------------------------------------ commands

/// `--import-playnite-theme`: import, then print what came across.
fn import_playnite_theme(paths: &Paths, src: &std::path::Path, opts: &theme::playnite::ImportOptions) -> i32 {
    let started = Instant::now();
    let r = match theme::playnite::import_with(src, &paths.themes_dir(), opts) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("import failed: {e:#}");
            return 1;
        }
    };
    println!("imported Playnite {} theme “{}” -> {}", r.mode.label().to_lowercase(), r.name, r.dir.display());
    println!("theme key: {}", r.key);
    if let Some(f) = r.family {
        let suggested = r.suggested.map(|s| s.key()).unwrap_or("");
        println!("layout: {} (suggested: {suggested})", f.key());
    }
    if r.media_bytes > 0 {
        println!(
            "videos and sounds: {} ({})",
            theme::playnite::report::size_text(r.media_bytes),
            if r.media_included { "copied" } else { "left out" }
        );
    }
    for g in &r.groups {
        println!("{}:", g.title);
        for l in &g.lines {
            println!("  {} {}", if l.ok { "+" } else { "-" }, l.text);
        }
    }
    if r.needs_video {
        println!("note: {}", media::deps::message(media::deps::Need::Video));
    }
    if r.needs_audio {
        println!("note: {}", media::deps::message(media::deps::Need::Audio));
    }
    println!("took {:.2} s", started.elapsed().as_secs_f32());
    0
}

/// `--theme-options KEY [--set K=V,…]`: apply option values, then list the options.
fn theme_options(paths: &Paths, key: &str, set: Option<&str>) -> i32 {
    let dir = paths.themes_dir().join(key);
    let Some(spec) = theme::playnite::options::read(&dir) else {
        eprintln!(
            "{key}: no theme options Lodestar can apply (not an imported Playnite theme, or none change what Lodestar draws)"
        );
        return 1;
    };
    if let Some(set) = set {
        let mut values = spec.values.clone();
        for kv in set.split(',').filter(|s| !s.trim().is_empty()) {
            let Some((k, v)) = kv.split_once('=') else {
                eprintln!("--set: expected KEY=VALUE, got {kv}");
                return 2;
            };
            values.insert(k.trim().to_string(), v.trim().to_string());
        }
        let started = Instant::now();
        match theme::playnite::apply_options(&paths.themes_dir(), key, &values) {
            Ok(r) => {
                println!(
                    "applied {} option value(s) to {} in {:.2} s",
                    r.option_values.len(),
                    r.key,
                    started.elapsed().as_secs_f32()
                );
                if let Ok(true) = theme::playnite::refresh_option_effects(&paths.themes_dir(), key) {
                    println!("(the options that change something are different with these values)");
                }
            }
            Err(e) => {
                eprintln!("applying options failed: {e:#}");
                return 1;
            }
        }
    }
    let spec = theme::playnite::options::read(&dir).unwrap_or(spec);
    for o in spec.effective() {
        println!(
            "{} = {}  ({}{})",
            o.key,
            o.value_text(&spec.values),
            o.label,
            if o.section.is_empty() { String::new() } else { format!(" — {}", o.section) }
        );
    }
    let others = spec.others();
    println!("{others} more only change{} parts Lodestar draws its own way", if others == 1 { "s" } else { "" });
    0
}

/// `--scan`: scan every library, print per-source counts, merge the result into the library.
fn scan_and_report(paths: &Paths, config: &Config) -> i32 {
    let ctx = scan::ScanContext::from_config(config);
    let report = scan::run_all(&ctx);
    for (src, r) in &report.per_source {
        match &r.error {
            Some(e) => println!("{:<8} ERROR {e}", src.key()),
            None => println!("{:<8} {:>4} games  ({:?})", src.key(), r.found, r.elapsed),
        }
    }
    println!("{} duplicates dropped, total {} in {:?}", report.duplicates_dropped, report.games.len(), report.elapsed);
    match db::Db::open(&paths.db_file())
        .and_then(|mut d| d.merge_scan_with_shortcuts(&report.games, &report.ok_sources, &report.dropped_shortcuts))
    {
        Ok(stats) => println!("library: {stats:?}"),
        Err(e) => eprintln!("library update failed: {e:#}"),
    }
    0
}

/// `--list`: the library with game ids.
fn list_library(paths: &Paths) -> i32 {
    match db::Db::open(&paths.db_file()).and_then(|d| d.load_all()) {
        Ok(games) => {
            for g in games {
                println!(
                    "{:>5} {:<8} {:<10} {:<50} {} cover={} hero={} {}",
                    g.id,
                    g.source.key(),
                    g.source_id.chars().take(10).collect::<String>(),
                    g.display_name(),
                    if g.installed { "installed" } else { "-" },
                    g.art.cover.is_some() as u8,
                    g.art.hero.is_some() as u8,
                    model::format_playtime(g.playtime_secs)
                );
            }
        }
        Err(e) => eprintln!("{e:#}"),
    }
    0
}

/// `--play ID`: headless, never a window. Errors also go to a desktop notification when there is
/// no terminal to see them (started from a desktop shortcut).
fn play(paths: &Paths, config: &Config, id: &str) -> i32 {
    match launch::play::run(paths, config, id) {
        Ok(r) => {
            if let Some(f) = r.failed {
                notify_failure(&f);
                return 1;
            }
            0
        }
        Err(e) => {
            eprintln!("lodestar --play: {e:#}");
            notify_failure(&format!("{e:#}"));
            1
        }
    }
}

/// `--launch NAME`: launch one game from the terminal and report what happens.
fn launch_test(paths: &Paths, config: &Config, lt: &launchtest::Opts) -> i32 {
    let res = db::Db::open(&paths.db_file()).and_then(|d| launchtest::run(&d, config, lt));
    if let Err(e) = res {
        eprintln!("launch test failed: {e:#}");
        return 1;
    }
    0
}

/// `--play` failed and nobody sees stderr (desktop shortcut): show a desktop notification,
/// best effort, waiting at most a moment for `notify-send`.
fn notify_failure(text: &str) {
    // SAFETY: plain libc call.
    if unsafe { libc::isatty(2) } == 1 {
        return;
    }
    let Some(ns) = launch::find_in_path("notify-send") else { return };
    if let Ok(mut c) = std::process::Command::new(ns)
        .args(["--app-name=Lodestar", "--icon=lodestar", "Couldn't start the game", text])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        let t = Instant::now();
        while t.elapsed() < NOTIFY_WAIT {
            if let Ok(Some(_)) = c.try_wait() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

// ------------------------------------------------------------------------------ option values

/// `1600x900` -> [1600, 900] (logical px; both at least 1).
fn parse_size(s: &str) -> Option<[f32; 2]> {
    let (w, h) = s.trim().split_once(['x', 'X'])?;
    let (w, h): (f32, f32) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    (w.is_finite() && h.is_finite() && w >= 1.0 && h >= 1.0).then_some([w, h])
}

/// `--inject` inputs; `right*20` is the same input 20 times (a held direction).
pub(crate) fn parse_navs(s: &str) -> Vec<Nav> {
    s.split(',')
        .flat_map(|t| {
            let (t, n) = match t.split_once('*') {
                Some((t, n)) => (t, n.trim().parse::<usize>().unwrap_or(1).min(1000)),
                None => (t, 1),
            };
            std::iter::repeat_n(t, n)
        })
        .filter_map(|t| match t.trim().to_lowercase().as_str() {
            "up" => Some(Nav::Up),
            "down" => Some(Nav::Down),
            "left" => Some(Nav::Left),
            "right" => Some(Nav::Right),
            "accept" | "a" => Some(Nav::Accept),
            "back" | "b" => Some(Nav::Back),
            "menu" | "start" => Some(Nav::Menu),
            "select" => Some(Nav::Select),
            "favorite" | "x" => Some(Nav::Favorite),
            "search" | "y" => Some(Nav::Search),
            "pageup" | "lb" => Some(Nav::PageUp),
            "pagedown" | "rb" => Some(Nav::PageDown),
            "jumpup" | "lt" => Some(Nav::JumpUp),
            "jumpdown" | "rt" => Some(Nav::JumpDown),
            "guide" => Some(Nav::Guide),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injected_inputs_repeat_with_a_count() {
        assert_eq!(parse_navs("right*3,a"), vec![Nav::Right, Nav::Right, Nav::Right, Nav::Accept]);
        assert_eq!(parse_navs("down*x,up"), vec![Nav::Down, Nav::Up]);
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("1600x900"), Some([1600.0, 900.0]));
        assert_eq!(parse_size("800X600"), Some([800.0, 600.0]));
        assert_eq!(parse_size("1600"), None);
        assert_eq!(parse_size("0x900"), None);
        assert_eq!(parse_size("NaNx900"), None);
    }

    #[test]
    fn navs() {
        assert_eq!(parse_navs("right, accept,bogus,LB,guide"), vec![Nav::Right, Nav::Accept, Nav::PageUp, Nav::Guide]);
        assert!(parse_navs("").is_empty());
    }

    fn parsed(args: &[&str]) -> Cli {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn a_theme_import_wins_over_the_simple_commands() {
        let c = parsed(&["--scan", "--theme-options", "k", "--import-playnite-theme", "/p"]);
        assert!(matches!(c.command, Some(Command::ImportPlayniteTheme(..))));
        let c = parsed(&["--scan", "--theme-options", "k"]);
        assert!(matches!(c.command, Some(Command::ThemeOptions { .. })));
    }

    #[test]
    fn the_last_simple_command_wins_and_launch_options_apply_in_any_order() {
        let c = parsed(&["--play", "7", "--list"]);
        assert!(matches!(c.command, Some(Command::List)));
        let c = parsed(&["--direct", "--launch", "Celeste", "--timeout", "9"]);
        let Some(Command::Launch(lt)) = c.command else { panic!("expected --launch") };
        assert_eq!((lt.query.as_str(), lt.direct, lt.timeout), ("Celeste", true, 9));
        let c = parsed(&["--launch", "x", "--timeout", "soon"]);
        let Some(Command::Launch(lt)) = c.command else { panic!("expected --launch") };
        assert_eq!(lt.timeout, LAUNCH_TEST_TIMEOUT_SECS);
    }

    #[test]
    fn window_options_without_a_command() {
        let c = parsed(&["--controller", "--windowed", "--size", "800x600", "--frames", "x"]);
        assert!(c.command.is_none());
        assert!(c.windowed);
        assert_eq!(c.size, Some([800.0, 600.0]));
        assert_eq!(c.opts.force_mode, Some(Mode::Controller));
        assert_eq!(c.opts.screenshot_frames, SCREENSHOT_FRAMES);
    }
}
