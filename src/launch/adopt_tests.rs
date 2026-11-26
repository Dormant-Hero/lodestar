//! Adopting games that already run (`plan_adoptions`, `Launcher::adopt`), on a fake /proc.
//! The fake pids are above the usual pid_max, and nothing here sends a signal.

use super::*;
use track::fake::{self, P};

const FAKE: i32 = 4_200_000;
const ME: i32 = FAKE + 99;

fn game(id: GameId, source: Source, source_id: &str, launch: LaunchSpec) -> Game {
    Game { id, source, source_id: source_id.into(), launch: Some(launch), ..Game::blank(&format!("Game {id}")) }
}

fn sh() -> LaunchSpec {
    LaunchSpec::Command { program: "sh".into(), args: vec!["-c".into(), "true".into()], cwd: None, env: vec![] }
}

fn tag(lodestar: i32, id: GameId) -> String {
    format!("{}={lodestar}-{id}-1700000000000000000", track::TAG_VAR)
}

fn plan(root: &Path, games: &[Game], skip: impl Fn(GameId, &str) -> bool) -> Vec<Adoption> {
    let procs = track::read_procs_in(root, true);
    let found = track::find_running(&procs, root, ME);
    plan_adoptions(&found, games, &procs, root, skip, Duration::from_secs)
}

#[test]
fn adoption_plan() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proc");
    let dead = FAKE + 1;
    let live = FAKE + 2;
    let osrs_shortcut = 3_123_456_789u32;
    let games = vec![
        game(11, Source::Manual, "a", sh()),
        game(12, Source::Manual, "b", sh()),
        game(13, Source::Steam, "1343370", LaunchSpec::Steam { appid: 1_343_370 }),
        game(
            14,
            Source::Manual,
            "c",
            LaunchSpec::Url(format!("steam://rungameid/{}", ((osrs_shortcut as u64) << 32) | 0x0200_0000)),
        ),
        game(15, Source::Manual, "d", LaunchSpec::Url("https://example.com".into())),
    ];
    let (t11, t12, t13, t15, t99) = (tag(dead, 11), tag(live, 12), tag(dead, 13), tag(dead, 15), tag(dead, 99));
    fake::boot(&root, 1_700_000_000);
    fake::add(&root, P::new(live, 5, &["/usr/bin/lodestar", "--play", "12"]));
    // Game 11, launched by a Lodestar that was closed: its launcher and the game.
    fake::add(&root, P::new(FAKE + 10, 300, &["/g/launcher"]).env(&[&t11]));
    fake::add(&root, P::new(FAKE + 11, 320, &["/g/game"]).env(&[&t11]).parent(FAKE + 10, FAKE + 10, FAKE + 10));
    // Game 12: a desktop shortcut's `lodestar --play` follows it.
    fake::add(&root, P::new(FAKE + 12, 400, &["/g12/run"]).env(&[&t12]));
    // Game 13 (a Steam game) through Steam: the Steam client that `steam -applaunch` started
    // carries that launch's tag, and so does the game's reaper.
    fake::add(&root, P::new(FAKE + 20, 100, &["/s/steam"]).env(&[&t13]));
    fake::add(
        &root,
        P::new(FAKE + 21, 500, &["/s/reaper", "SteamLaunch", "AppId=1343370", "--", "/g/osrs"]).env(&[&t13]),
    );
    // Game 14, a non-Steam shortcut started from Steam.
    fake::add(
        &root,
        P::new(FAKE + 22, 600, &["/s/reaper", "SteamLaunch", &format!("AppId={osrs_shortcut}"), "--", "/x"]),
    );
    // A Steam game that isn't in the library; a URL launch (not followed); a game not in the
    // library; unrelated processes.
    fake::add(&root, P::new(FAKE + 23, 700, &["/s/reaper", "SteamLaunch", "AppId=42", "--", "/y"]));
    fake::add(&root, P::new(FAKE + 24, 710, &["/usr/bin/heroic"]).env(&[&t15]));
    fake::add(&root, P::new(FAKE + 25, 720, &["/z"]).env(&[&t99]));
    fake::add(&root, P::new(FAKE + 26, 730, &["/usr/bin/firefox"]).env(&["HOME=/home/u"]));
    let all = plan(&root, &games, |_, _| false);
    let ids: Vec<GameId> = all.iter().map(|a| a.id).collect();
    assert_eq!(ids, vec![13, 14, 11, 12], "{all:#?}");
    let a13 = &all[0];
    assert_eq!(
        (a13.kind, a13.msg.mode, a13.foreign),
        (ResolvedKind::Steam { appid: 1_343_370 }, track::Mode::Steam { appid: 1_343_370 }, false)
    );
    assert_eq!(a13.msg.known, [(FAKE + 21, 500)].into_iter().collect());
    assert_eq!((a13.msg.since, a13.msg.age), (500, Duration::from_secs(500)));
    assert_eq!(all[1].kind, ResolvedKind::SteamShortcut { appid: osrs_shortcut });
    let a11 = &all[2];
    assert_eq!((a11.kind, a11.msg.mode, a11.foreign), (ResolvedKind::Process, track::Mode::Session, false));
    assert_eq!(a11.msg.known, [(FAKE + 10, 300), (FAKE + 11, 320)].into_iter().collect());
    assert_eq!(a11.msg.tag, t11[track::TAG_VAR.len() + 1..]);
    assert_eq!((a11.msg.since, a11.msg.age), (300, Duration::from_secs(300)), "timed from the earliest process");
    assert!(all[3].foreign, "another Lodestar follows game 12");
    // Games already followed, and sessions that ended, aren't adopted (again).
    let t11v = t11[track::TAG_VAR.len() + 1..].to_string();
    let rest = plan(&root, &games, |id, tag| id == 13 || tag == t11v);
    assert_eq!(rest.iter().map(|a| a.id).collect::<Vec<_>>(), vec![14, 12]);
}

#[test]
fn store_client_launch_counts_only_while_its_game_runs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proc");
    let folder = dir.path().join("games").join("Diablo IV");
    std::fs::create_dir_all(&folder).unwrap();
    let mut g = game(21, Source::BattleNet, "fenris", sh());
    g.install_dir = Some(folder.clone());
    let games = vec![g];
    let t = tag(FAKE + 1, 21);
    fake::boot(&root, 1_700_000_000);
    // The store client the launch opened stays open after the game.
    fake::add(&root, P::new(FAKE + 30, 100, &["/pfx/Battle.net.exe"]).env(&[&t]).cwd(dir.path()));
    assert!(plan(&root, &games, |_, _| false).is_empty(), "the client alone isn't the game");
    fake::add(&root, P::new(FAKE + 31, 200, &["Z:\\games\\Diablo IV\\Diablo IV.exe"]).env(&[&t]).cwd(&folder));
    let a = plan(&root, &games, |_, _| false);
    assert_eq!(a.len(), 1);
    assert_eq!((a[0].msg.mode, a[0].msg.folder.as_deref()), (track::Mode::GameFolder, Some(folder.as_path())));
}

fn adoption(id: GameId, age: u64, foreign: bool) -> Adoption {
    Adoption {
        id,
        kind: ResolvedKind::Process,
        foreign,
        msg: track::Adopt {
            id,
            mode: track::Mode::Session,
            tag: format!("{}-{id}-1", FAKE + 1),
            folder: None,
            // Not running here: the monitor sees the session over at its first scan.
            known: [(FAKE + 50, 1)].into_iter().collect(),
            since: 1,
            age: Duration::from_secs(age),
        },
    }
}

#[test]
fn adopted_game_shows_running_once_and_ends_timed_from_its_start() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = Launcher::with_log_dir(&egui::Context::default(), dir.path().join("logs"));
    l.poll_every = Duration::from_millis(100);
    l.adopt(&adoption(31, 1800, false));
    l.adopt(&adoption(31, 1800, false));
    assert_eq!(l.active_games(), &[31]);
    assert!(l.is_started(31) && !l.is_foreign(31));
    assert_eq!(l.launch_kind(31), Some(ResolvedKind::Process));
    let ago = now_unix() - l.adopted_start(31).unwrap();
    assert!((1800..1805).contains(&ago), "{ago}");
    let t = std::time::Instant::now();
    let mut evs = vec![];
    while t.elapsed() < Duration::from_secs(10) && evs.is_empty() {
        evs.extend(l.poll());
        std::thread::sleep(Duration::from_millis(50));
    }
    let [LaunchEvent::Exited { id: 31, secs }] = evs.as_slice() else { panic!("{evs:?}") };
    assert!((1800..1815).contains(secs), "{secs}");
    assert!(l.running().is_none());
    // The ended session's tag isn't adopted again (a straggler of it, say).
    assert!(l.ended_tags.contains(&format!("{}-31-1", FAKE + 1)));
}

#[test]
fn app_records_playtime_of_adopted_games_but_not_of_foreign_ones() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = crate::app::AppState::for_test(dir.path());
    let add = |st: &mut crate::app::AppState, name: &str| {
        st.add_manual_game(name, sh(), Art::default());
        st.library.games.iter().find(|g| g.name == name).unwrap().id
    };
    let mine = add(&mut st, "Mine");
    let theirs = add(&mut st, "Theirs");
    st.launcher.adopt(&adoption(mine, 600, false));
    st.launcher.adopt(&adoption(theirs, 600, true));
    // Only the one this Lodestar follows keeps the window from closing.
    assert_eq!(st.close_guard_game(), Some(mine));
    assert!(st.poll_until(Duration::from_secs(10), |s| s.launcher.running().is_none()), "never exited");
    let p = |st: &crate::app::AppState, id| st.db.get(id).unwrap().unwrap().playtime_secs;
    assert!((600..615).contains(&p(&st, mine)), "{}", p(&st, mine));
    assert_eq!(p(&st, theirs), 0, "the other Lodestar records that one");
    assert_eq!(st.library.get(mine).unwrap().playtime_secs, p(&st, mine));
}

#[test]
fn process_tracking_adopts_only_the_launchs_own_process() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proc");
    let mut g = game(41, Source::Manual, "p", sh());
    g.edits.actions = vec![GameAction {
        name: "Play".into(),
        path: "/g41/run".into(),
        tracking: TrackingMode::Process,
        play: true,
        ..Default::default()
    }];
    let games = vec![g];
    let t = tag(FAKE + 1, 41);
    fake::boot(&root, 1_700_000_000);
    // The launch's process (a session leader: setsid() at spawn) and a helper it started.
    fake::add(&root, P::new(FAKE + 40, 300, &["/g41/run"]).env(&[&t]));
    fake::add(&root, P::new(FAKE + 41, 310, &["/g41/updater"]).env(&[&t]).parent(FAKE + 40, FAKE + 40, FAKE + 40));
    let a = plan(&root, &games, |_, _| false);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert_eq!(a[0].msg.mode, track::Mode::Process);
    assert_eq!(a[0].msg.known, [(FAKE + 40, 300)].into_iter().collect());
    assert_eq!((a[0].msg.since, a[0].msg.age), (300, Duration::from_secs(300)));
    // The launch's process is gone: what it left behind isn't the game (its session ended).
    let dir2 = tempfile::tempdir().unwrap();
    let root2 = dir2.path().join("proc");
    fake::boot(&root2, 1_700_000_000);
    fake::add(&root2, P::new(FAKE + 41, 310, &["/g41/updater"]).env(&[&t]).parent(1, FAKE + 40, FAKE + 40));
    assert!(plan(&root2, &games, |_, _| false).is_empty());
}

fn scan_of(root: &Path) -> RunningScan {
    let procs = track::read_procs_in(root, true);
    let found = track::find_running(&procs, root, ME);
    let others = track::other_game(&found, &procs, root, ME, &[]);
    RunningScan {
        found,
        btime: track::boot_time(root),
        root: root.to_path_buf(),
        others,
        procs,
        started: std::time::Instant::now(),
    }
}

/// What a finished look for running games on `root` says: something game-like runs.
fn others_seen_on(root: &Path, games: &[Game]) -> bool {
    let dir = tempfile::tempdir().unwrap();
    let mut l = Launcher::with_log_dir(&egui::Context::default(), dir.path().join("logs"));
    let (tx, rx) = crossbeam_channel::unbounded();
    l.running_scan = Some(rx);
    tx.send(scan_of(root)).unwrap();
    l.finish_running_scan(games).expect("the scan was there");
    assert_ne!(l.others_seen(), l.others_clear(Duration::from_secs(60)));
    l.others_seen()
}

/// Steam as a launch that found it closed starts it, all carrying the launch's `tag` (as measured
/// on a real Steam: the launcher script, the client and its helpers, the runtime's logger left to
/// run on its own). `under`: the process that ran Steam (None: the launch command was Steam).
fn steam_tree(root: &Path, tag: &str, under: Option<i32>) {
    let home = "/home/u/.local/share/Steam";
    let env = [tag];
    let script = P::new(FAKE + 9, 99, &["bash", "/home/u/.local/share/Steam/steam.sh", "-silent"]);
    let script = match under {
        Some(pid) => script.parent(pid, pid, pid),
        None => script,
    };
    fake::add(root, script.env(&env));
    let client = format!("{home}/steamrt64/steam");
    let fossilize = format!("{home}/steamrt64/../ubuntu12_64/fossilize_replay");
    let (sh, cl, wh) = (FAKE + 9, FAKE + 10, FAKE + 11);
    fake::add(root, P::new(cl, 100, &[&client, "-silent", "steam://rungameid/1"]).parent(sh, sh, sh).env(&env));
    fake::add(root, P::new(FAKE + 11, 101, &["./steamwebhelper", "-nocrashdialog"]).parent(cl, sh, sh).env(&env));
    fake::add(root, P::new(FAKE + 12, 102, &["/proc/self/exe", "--type=utility"]).parent(wh, sh, sh).env(&env));
    fake::add(root, P::new(FAKE + 14, 103, &[&fossilize]).parent(cl, sh, sh).env(&env));
    let logger = format!("{home}/steamrt64/pv-runtime/steam-runtime-steamrt/pressure-vessel/libexec/srt-logger");
    fake::add(root, P::new(FAKE + 15, 98, &[&logger, "--sh-syntax"]).env(&env));
}

/// Lodestar launched a Steam game while Steam was closed, so its launch started the Steam client,
/// and Steam and everything it runs keep that launch's tag until Steam quits. A later Lodestar
/// run (closed and opened again, updated) must not take that for a game running: it kept the
/// controller out of desktop mode (and the background) until Steam was restarted. A game
/// running in that Steam still counts, by its reaper.
#[test]
fn a_steam_client_an_earlier_lodestar_started_is_no_game() {
    let earlier = FAKE + 1; // that Lodestar run is gone
    let games = vec![
        game(61, Source::Steam, "1086940", LaunchSpec::Steam { appid: 1086940 }),
        game(62, Source::Manual, "s", LaunchSpec::Url("steam://rungameid/12345678901234567890".into())),
        game(63, Source::Manual, "n", sh()),
    ];
    // 63: an action of a native game that goes through Steam started it as well.
    for id in [61, 62, 63] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proc");
        fake::boot(&root, 1_700_000_000);
        let t = tag(earlier, id);
        steam_tree(&root, &t, None);
        assert!(!others_seen_on(&root, &games), "Steam alone, started for game {id}");
        assert!(plan(&root, &games, |_, _| false).is_empty(), "and nothing to adopt");
        // A game starts in that Steam: its reaper is seen.
        let reaper = ["/home/u/.local/share/Steam/steamrt64/reaper", "SteamLaunch", "AppId=1086940", "--", "/g/bg3"];
        fake::add(&root, P::new(FAKE + 13, 200, &reaper).parent(FAKE + 10, FAKE + 13, FAKE + 13).env(&[&t]));
        assert!(others_seen_on(&root, &games), "a game in that Steam, for game {id}");
    }
}

/// Only Steam's own processes are left out of a tagged launch: whatever else carries the tag
/// keeps it a game that runs (and one to pick up), whatever game the tag names.
#[test]
fn a_tagged_program_besides_steam_is_still_a_game() {
    let earlier = FAKE + 1;
    let games = vec![
        game(61, Source::Steam, "1086940", LaunchSpec::Steam { appid: 1086940 }),
        game(63, Source::Manual, "n", sh()),
    ];
    let fresh = || {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proc");
        fake::boot(&root, 1_700_000_000);
        (dir, root)
    };
    // A Steam game's extra action running a native source port; a native game; a game no longer
    // in the library; game binaries that happen to be called `steam` or `steamwebhelper`, or sit
    // on a drive someone called Steam.
    let programs = [
        (61, "/usr/bin/gzdoom"),
        (63, "/g/run"),
        (64, "/g/old"),
        (63, "/g/steam"),
        (63, "/g/steamwebhelper"),
        (63, "/run/media/u/Steam/Games/Foo/foo.x86_64"),
        (63, "/home/u/.steam/games/foo"),
    ];
    for (id, program) in programs {
        let (_d, root) = fresh();
        fake::add(&root, P::new(FAKE + 20, 100, &[program]).env(&[&tag(earlier, id)]));
        assert!(others_seen_on(&root, &games), "game {id}: {program}");
    }
    // A launch script that started Steam and runs the game beside it: the game counts and is
    // picked up from its own processes. (Following it afterwards goes by the tag, which Steam
    // carries too, so it may show as running until Steam quits, as before: `adopt_tagged`.)
    let (_d, root) = fresh();
    let t = tag(earlier, 63);
    fake::add(&root, P::new(FAKE + 30, 90, &["bash", "/home/u/bin/play.sh"]).env(&[&t]));
    steam_tree(&root, &t, Some(FAKE + 30));
    fake::add(&root, P::new(FAKE + 31, 110, &["/usr/bin/gzdoom"]).parent(FAKE + 30, FAKE + 30, FAKE + 30).env(&[&t]));
    assert!(others_seen_on(&root, &games));
    let a = plan(&root, &games, |_, _| false);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert_eq!(a[0].id, 63);
    assert_eq!(
        a[0].msg.known,
        [(FAKE + 30, 90), (FAKE + 31, 110)].into_iter().collect(),
        "picked up from none of Steam's"
    );
    // A game a launch started in the background before it became Steam's launcher
    // (`sh -c 'gzdoom & steam -silent'`: the shell execs the last command), or Flatpak's Steam
    // (`flatpak run` execs into the sandbox's bwrap, in Flatpak Steam's scope): its parent is that
    // launcher, not the Steam client, and it is outside that scope, so it counts.
    let scope =
        "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-flatpak-com.valvesoftware.Steam-7.scope\n";
    let launchers = [
        (&["/bin/sh", "/usr/bin/steam", "-silent"][..], false),
        (&["bwrap", "--args", "40", "/app/bin/steam-wrapper", "-silent"][..], true),
    ];
    for (launcher, flatpak) in launchers {
        let (_d, root) = fresh();
        fake::add(&root, P::new(FAKE + 30, 90, launcher).env(&[&t]));
        if flatpak {
            std::fs::write(root.join((FAKE + 30).to_string()).join("cgroup"), scope).unwrap();
        }
        steam_tree(&root, &t, Some(FAKE + 30));
        assert!(!others_seen_on(&root, &games), "that launcher and Steam alone: {launcher:?}");
        fake::add(
            &root,
            P::new(FAKE + 31, 95, &["/usr/bin/gzdoom"]).parent(FAKE + 30, FAKE + 30, FAKE + 30).env(&[&t]),
        );
        assert!(others_seen_on(&root, &games), "{launcher:?}");
    }
    // A reused pid: a process that started after the "child" isn't its parent.
    let (_d, root) = fresh();
    steam_tree(&root, &t, None);
    fake::add(&root, P::new(FAKE + 31, 50, &["/usr/bin/gzdoom"]).parent(FAKE + 10, FAKE + 31, FAKE + 31).env(&[&t]));
    assert!(others_seen_on(&root, &games), "the Steam client started after it");
    // Steam's web helper alone (its client gone) beside a game: the game counts.
    let (_d, root) = fresh();
    fake::add(&root, P::new(FAKE + 11, 101, &["./steamwebhelper", "-nocrashdialog"]).env(&[&t]));
    fake::add(&root, P::new(FAKE + 31, 110, &["/usr/bin/gzdoom"]).env(&[&t]));
    assert!(others_seen_on(&root, &games));
}

/// `--running` says it plainly: a Steam an earlier launch opened is no game; a game beside it is.
#[test]
fn running_report_in_plain_words() {
    let earlier = FAKE + 1;
    let games = vec![
        game(61, Source::Steam, "1086940", LaunchSpec::Steam { appid: 1086940 }),
        game(63, Source::Manual, "n", sh()),
    ];
    let report = |root: &Path| {
        let procs = track::read_procs_in(root, true);
        let found = track::find_running(&procs, root, ME);
        track::running_report(&found, &procs, root, ME, &[], &games)
    };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proc");
    fake::boot(&root, 1_700_000_000);
    assert_eq!(report(&root), "Started elsewhere:\n  nothing\nController: may act, nothing looks like a game\n");
    steam_tree(&root, &tag(earlier, 61), None);
    assert_eq!(
        report(&root),
        "Started elsewhere:\n  Steam (opened by an earlier Lodestar's launch of Game 61; not counted as a game)\n\
         Controller: may act, nothing looks like a game\n"
    );
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proc");
    fake::boot(&root, 1_700_000_000);
    let t = tag(earlier, 63);
    fake::add(&root, P::new(FAKE + 30, 90, &["bash", "/home/u/bin/play.sh"]).env(&[&t]));
    steam_tree(&root, &t, Some(FAKE + 30));
    fake::add(&root, P::new(FAKE + 31, 110, &["/usr/bin/gzdoom"]).parent(FAKE + 30, FAKE + 30, FAKE + 30).env(&[&t]));
    assert_eq!(
        report(&root),
        "Started elsewhere:\n  Game 63 (started by an earlier Lodestar), 2 processes\n\
         Controller: waits, this looks like a game: Game 63 (started by an earlier Lodestar)\n"
    );
}

#[test]
fn a_steam_appid_a_launch_follows_isnt_adopted_for_another_entry() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proc");
    let games = vec![
        game(51, Source::Manual, "a", LaunchSpec::Url("steam://rungameid/570".into())),
        game(52, Source::Steam, "570", LaunchSpec::Steam { appid: 570 }),
    ];
    fake::boot(&root, 1_700_000_000);
    fake::add(&root, P::new(FAKE + 60, 100, &["/s/reaper", "SteamLaunch", "AppId=570", "--", "/g/dota"]));
    let mut l = Launcher::with_log_dir(&egui::Context::default(), dir.path().join("logs"));
    // This run launched entry 51 through Steam (no monitor involved here).
    l.active.push(51);
    l.kinds.insert(51, ResolvedKind::Steam { appid: 570 });
    assert!(l.adopt_found(&scan_of(&root), &games).is_empty());
    assert_eq!(l.active_games(), &[51]);
    // Not launched here: the Steam game itself is picked up.
    let mut l = Launcher::with_log_dir(&egui::Context::default(), dir.path().join("logs2"));
    l.poll_every = Duration::from_secs(3600);
    let a = l.adopt_found(&scan_of(&root), &games);
    assert_eq!(a.iter().map(|a| a.id).collect::<Vec<_>>(), vec![52]);
    // Found again (focus came back): already followed.
    assert!(l.adopt_found(&scan_of(&root), &games).is_empty());
}

#[test]
fn the_running_scan_runs_off_the_ui_thread_and_reports_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = Launcher::with_log_dir(&egui::Context::default(), dir.path().join("logs"));
    assert!(l.finish_running_scan(&[]).is_none(), "nothing started");
    l.start_running_scan(vec![]);
    l.start_running_scan(vec![]); // one at a time
    let t = std::time::Instant::now();
    let mut got = None;
    while got.is_none() && t.elapsed() < Duration::from_secs(10) {
        got = l.finish_running_scan(&[]);
        std::thread::sleep(Duration::from_millis(10));
    }
    // No library games: nothing is adopted whatever runs on this machine.
    assert_eq!(got, Some(vec![]));
    assert!(l.finish_running_scan(&[]).is_none());
    // The look is fresh now: either something game-like runs here or the background controller
    // may act, never both.
    assert_ne!(l.others_clear(Duration::from_secs(60)), l.others_seen());
    assert!(!l.others_clear(Duration::ZERO), "a look counts only while it is fresh");
}
