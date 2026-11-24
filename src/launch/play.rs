//! `lodestar --play <id>` (what desktop shortcuts run): start one game through the same launcher
//! as the library window (Play action, launch options, HDR, scripts, tracking), wait for it to
//! end, record play count / last played / playtime in the library, exit. Never opens a window.

use super::{LaunchEvent, LaunchPlan, Launcher, ResolvedKind};
use crate::config::{Config, Paths};
use crate::db::Db;
use crate::game_options::{self, Caps, LaunchEnv};
use crate::model::*;
use anyhow::anyhow;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Completion statuses a launch moves to Playing (`config.auto_playing`).
pub fn marks_playing(c: Option<CompletionStatus>) -> bool {
    matches!(c, None | Some(CompletionStatus::NotPlayed | CompletionStatus::PlanToPlay))
}

/// The game's edits with completion set to Playing, when a launch should do that.
pub fn auto_playing_edits(game: &Game, config: &Config) -> Option<UserEdits> {
    (config.auto_playing && marks_playing(game.completion())).then(|| {
        let mut e = game.edits.clone();
        e.completion = Some(CompletionStatus::Playing);
        e
    })
}

/// Does Lodestar add the session length to a game's playtime, given how it was launched?
/// Not for Steam apps the Steam client ran: Steam counts those itself (`localconfig.vdf`, read by
/// the rescan after they exit). Everything else Lodestar timed is ours to count: processes it
/// spawned (including Steam games started with a custom command), Steam's non-Steam shortcuts
/// (Steam keeps no playtime for them that we could read) and URL launches followed by folder
/// tracking (plain URL launches aren't followed, so no session ever ends for them).
pub fn records_own_playtime(kind: Option<ResolvedKind>) -> bool {
    matches!(kind, Some(ResolvedKind::Process | ResolvedKind::SteamShortcut { .. } | ResolvedKind::OpenUrl))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayReport {
    /// The game was seen running.
    pub started: bool,
    /// Session length as tracked (0: not followed / Steam counted it).
    pub secs: u64,
    /// Added to the library's playtime.
    pub recorded_secs: u64,
    pub failed: Option<String>,
    /// Not followed (URL launch, tracking None): returned once it was handed off.
    pub untracked: bool,
}

/// The launch environment a headless launch (`--play`, `--launch`) resolves with: the saved
/// global and per-library game options, the Proton log folder, and `caps`. No monitor (there is
/// no window): gamescope's Auto output size falls back to `caps.preferred_mode`.
pub fn headless_env(paths: &Paths, config: &Config, caps: Caps) -> LaunchEnv {
    LaunchEnv {
        defaults: config.game_options.clone(),
        by_source: config.game_options_by_source.clone(),
        caps: Arc::new(caps),
        monitor: None,
        log_dir: Some(paths.data_dir.join("proton-logs")),
    }
}

/// Publish [`headless_env`] process-wide so `launch::resolve` (and the `Launcher`) see what the
/// library window would: without this a desktop shortcut ignores Settings → Games and every
/// wrapper option reports "isn't installed". Probes capabilities synchronously (a few ms plus a
/// cached `gamescope --help`; fine without a UI thread). Call once, before resolving.
pub fn publish_headless(paths: &Paths, config: &Config) {
    let t = Instant::now();
    let env = headless_env(paths, config, game_options::caps::probe_for_launch());
    log::debug!("headless launch: capabilities probed in {:?}", t.elapsed());
    if let Some(dir) = env.log_dir.clone() {
        game_options::set_log_dir(dir);
    }
    game_options::set_defaults(env.defaults);
    game_options::set_source_defaults(env.by_source);
    game_options::set_caps(Arc::unwrap_or_clone(env.caps));
    // Emulated games build their command from the emulators found (no UI thread here).
    crate::emulation::registry::refresh(&crate::scan::ScanContext::from_config(config));
}

/// `lodestar --play <id_arg>`.
pub fn run(paths: &Paths, config: &Config, id_arg: &str) -> anyhow::Result<PlayReport> {
    let id: GameId = id_arg
        .trim()
        .parse()
        .map_err(|_| anyhow!("--play needs a game id (a number; `lodestar --list` shows them), not {id_arg:?}"))?;
    let db = Db::open(&paths.db_file())?;
    let game = db.get(id)?.ok_or_else(|| {
        anyhow!("there is no game with id {id} in the library (removed?); `lodestar --list` shows the ids")
    })?;
    publish_headless(paths, config);
    let ctx = egui::Context::default();
    let mut l = Launcher::with_log_dir(&ctx, paths.cache_dir.join("logs"));
    // Store clients: minimised while the game runs, closed after it when Lodestar started them.
    l.set_hygiene(super::hygiene::HygieneConfig::from_settings(&config.launchers));
    let report = play(&db, config, &game, &mut l, &mut |line| eprintln!("{line}"));
    // Long enough for a Steam Lodestar started to finish its cloud sync and be closed (at most 2 min).
    l.finish_hygiene(Duration::from_secs(150));
    report
}

/// Launch `game` and follow it until it ends, recording the session like the library window
/// does. `say` gets progress lines.
pub fn play(
    db: &Db,
    config: &Config,
    game: &Game,
    l: &mut Launcher,
    say: &mut dyn FnMut(&str),
) -> anyhow::Result<PlayReport> {
    let id = game.id;
    let name = game.display_name().to_string();
    let r = super::resolve(game)?;
    say(&format!("Starting {name}: {}", r.display));
    for n in &r.notes {
        say(&format!("note: {n}"));
    }
    let mut plan = LaunchPlan::for_game(game, game.play_action(), &config.scripts, true);
    if let Some(note) = crate::cloud::launch_note(game, game.play_action(), &mut plan) {
        say(&note);
    }
    l.launch_planned(id, &name, &r, &plan)?;
    let prev = db.record_launch(id, now_unix())?;
    if let Some(e) = auto_playing_edits(game, config) {
        db.set_edits(id, &e)?;
    }
    let mut report = PlayReport { untracked: l.running() != Some(id), ..Default::default() };
    loop {
        for ev in l.poll() {
            match ev {
                LaunchEvent::Started { .. } => {
                    report.started = true;
                    say(&format!("{name} is running"));
                }
                LaunchEvent::Failed { error, .. } => {
                    // Failed at once: not a play.
                    db.undo_launch(id, prev)?;
                    say(&format!("{name} failed: {error}"));
                    report.failed = Some(error);
                }
                LaunchEvent::Exited { secs, .. } => {
                    report.secs = secs;
                    let kind = l.launch_kind(id);
                    if records_own_playtime(kind) && report.failed.is_none() && secs > 0 {
                        db.add_playtime(id, secs)?;
                        report.recorded_secs = secs;
                    }
                    if matches!(kind, Some(ResolvedKind::Steam { .. })) {
                        say("Steam counts this session's play time (Lodestar picks it up on its next scan).");
                    }
                    say(&format!("{name} exited after {}", session_length(secs)));
                    return Ok(finish(l, report, say));
                }
            }
        }
        say_notices(l, say);
        // Not followed: done once it was handed off and its scripts ran.
        if report.untracked && l.running() != Some(id) && !l.scripts_busy() {
            return Ok(finish(l, report, say));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn say_notices(l: &mut Launcher, say: &mut dyn FnMut(&str)) {
    for n in l.script_notices() {
        if n.should_report() {
            say(&n.text());
        }
    }
}

/// "42s", "12m", "1h 5m".
fn session_length(secs: u64) -> String {
    if secs < 60 { format!("{secs}s") } else { format_playtime(secs) }
}

/// Let the after-exit scripts finish (bounded: they time out after 30 s themselves).
fn finish(l: &mut Launcher, report: PlayReport, say: &mut dyn FnMut(&str)) -> PlayReport {
    let t = Instant::now();
    // Scripts started by the last poll() may not have counted themselves yet: give them a tick.
    std::thread::sleep(Duration::from_millis(50));
    while l.scripts_busy() && t.elapsed() < super::scripts::SCRIPT_TIMEOUT + Duration::from_secs(5) {
        say_notices(l, say);
        std::thread::sleep(Duration::from_millis(50));
    }
    say_notices(l, say);
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh_game(db: &mut Db, name: &str, script: &str) -> Game {
        let id = db
            .add_manual(
                name,
                LaunchSpec::Command {
                    program: "sh".into(),
                    args: vec!["-c".into(), script.into()],
                    cwd: None,
                    env: vec![],
                },
                Art::default(),
            )
            .unwrap();
        db.get(id).unwrap().unwrap()
    }

    fn launcher(dir: &std::path::Path) -> Launcher {
        let mut l = Launcher::with_log_dir(&egui::Context::default(), dir.join("logs"));
        l.poll_every = Duration::from_millis(100);
        l
    }

    #[test]
    fn headless_play_records_the_session_and_runs_scripts() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Db::open(&dir.path().join("lib.db")).unwrap();
        let mut g = sh_game(&mut db, "Sleeper", "sleep 1.2");
        let before = dir.path().join("before");
        let after = dir.path().join("after");
        g.edits.scripts = GameScripts {
            before_start: format!("echo \"$LODESTAR_GAME_NAME\" > '{}'", before.display()),
            after_exit: format!("touch '{}'", after.display()),
            ..Default::default()
        };
        db.set_edits(g.id, &g.edits).unwrap();
        let g = db.get(g.id).unwrap().unwrap();
        let config = Config::default();
        let mut lines = vec![];
        let rep = play(&db, &config, &g, &mut launcher(dir.path()), &mut |s| lines.push(s.to_string())).unwrap();
        assert!(rep.started && rep.failed.is_none(), "{rep:?} {lines:?}");
        assert!(rep.recorded_secs >= 1, "{rep:?}");
        assert_eq!(std::fs::read_to_string(&before).unwrap().trim(), "Sleeper");
        assert!(after.exists(), "after-exit script didn't run before --play returned");
        let g = db.get(g.id).unwrap().unwrap();
        assert_eq!(g.play_count, 1);
        assert!(g.last_played.is_some());
        assert_eq!(g.playtime_secs, rep.recorded_secs);
        assert_eq!(g.completion(), Some(CompletionStatus::Playing), "auto Playing");
    }

    #[test]
    fn headless_play_failure_is_not_a_play_and_keeps_completion_choice() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Db::open(&dir.path().join("lib.db")).unwrap();
        let g = sh_game(&mut db, "Broken", "echo boom; exit 4");
        let mut e = g.edits.clone();
        e.completion = Some(CompletionStatus::Beaten);
        db.set_edits(g.id, &e).unwrap();
        let g = db.get(g.id).unwrap().unwrap();
        let rep = play(&db, &Config::default(), &g, &mut launcher(dir.path()), &mut |_| {}).unwrap();
        assert!(rep.failed.as_deref().is_some_and(|e| e.contains("code 4") && e.contains("boom")), "{rep:?}");
        let g = db.get(g.id).unwrap().unwrap();
        assert_eq!((g.play_count, g.last_played, g.playtime_secs), (0, None, 0));
        assert_eq!(g.completion(), Some(CompletionStatus::Beaten), "a set status isn't touched");
    }

    #[test]
    fn unknown_or_bad_ids_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: dir.path().join("home"),
            config_dir: dir.path().join("config"),
            data_dir: dir.path().join("data"),
            cache_dir: dir.path().join("cache"),
        };
        paths.ensure();
        let e = run(&paths, &Config::default(), "999").unwrap_err().to_string();
        assert!(e.contains("no game with id 999"), "{e}");
        let e = run(&paths, &Config::default(), "half-life").unwrap_err().to_string();
        assert!(e.contains("needs a game id"), "{e}");
    }

    #[test]
    fn headless_env_carries_global_and_library_options() {
        use crate::game_options::{GameOptions, OptionId, OptionState, Tri};
        use crate::launch::{Host, SteamCmd, resolve_full};
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: dir.path().join("home"),
            config_dir: dir.path().join("config"),
            data_dir: dir.path().join("data"),
            cache_dir: dir.path().join("cache"),
        };
        let mut config =
            Config { game_options: GameOptions { mangohud: Tri::On, ..Default::default() }, ..Default::default() };
        config
            .game_options_by_source
            .insert(Source::Manual.key().to_string(), GameOptions { gamemode: Tri::On, ..Default::default() });
        // Hand-built caps (every tool installed), standing in for the probe.
        let caps = (*crate::game_options::tests::fixture().caps).clone();
        let env = headless_env(&paths, &config, caps);
        assert_eq!(env.defaults, config.game_options);
        assert_eq!(env.by_source, config.game_options_by_source);
        assert_eq!(env.log_dir, Some(paths.data_dir.join("proton-logs")));
        assert!(env.caps.probed && env.caps.bins.mangohud.is_some());
        assert_eq!(env.monitor, None, "no window: Auto size uses caps.preferred_mode");

        let mut db = Db::open_memory().unwrap();
        let g = sh_game(&mut db, "Shortcut", "true");
        let host = Host { steam: SteamCmd::Native, home: None, vars: Default::default() };
        let r = resolve_full(&g, &host, &env).unwrap();
        let state =
            |r: &super::super::Resolved, id| r.options.iter().find(|(o, _, _)| *o == id).map(|(_, _, s)| s.clone());
        assert_eq!(state(&r, OptionId::Mangohud), Some(OptionState::Applied), "global: {r:?}");
        assert_eq!(state(&r, OptionId::Gamemode), Some(OptionState::Applied), "library: {r:?}");
        assert!(r.display.contains("gamemoderun"), "{}", r.display);
        // What --play resolved with before: neither option.
        let r = resolve_full(&g, &host, &LaunchEnv::none()).unwrap();
        assert_ne!(state(&r, OptionId::Mangohud), Some(OptionState::Applied), "{r:?}");
        assert!(!r.display.contains("gamemoderun"), "{}", r.display);
    }

    #[test]
    fn completion_rules() {
        assert!(marks_playing(None));
        assert!(marks_playing(Some(CompletionStatus::NotPlayed)));
        assert!(marks_playing(Some(CompletionStatus::PlanToPlay)));
        assert!(!marks_playing(Some(CompletionStatus::Playing)));
        assert!(!marks_playing(Some(CompletionStatus::Completed)));
        let mut c = Config::default();
        let mut db = Db::open_memory().unwrap();
        let g = sh_game(&mut db, "X", "true");
        assert_eq!(auto_playing_edits(&g, &c).unwrap().completion, Some(CompletionStatus::Playing));
        c.auto_playing = false;
        assert!(auto_playing_edits(&g, &c).is_none());
    }
}
