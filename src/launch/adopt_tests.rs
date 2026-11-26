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

/// `--running` says it plainly.
#[test]
fn running_report_in_plain_words() {
    let earlier = FAKE + 1;
    let games = vec![game(63, Source::Manual, "n", sh())];
    let report = |root: &Path| {
        let procs = track::read_procs_in(root, true);
        let found = track::find_running(&procs, root, ME);
        track::running_report(&found, &procs, root, ME, &[], &games)
    };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proc");
    fake::boot(&root, 1_700_000_000);
    assert_eq!(report(&root), "Started elsewhere:\n  nothing\nController: may act, nothing looks like a game\n");
    fake::add(&root, P::new(FAKE + 31, 110, &["/usr/bin/gzdoom"]).env(&[&tag(earlier, 63)]));
    assert_eq!(
        report(&root),
        "Started elsewhere:\n  Game 63 (started by an earlier Lodestar), 1 process\n\
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
