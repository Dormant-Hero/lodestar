use super::*;
use crate::model::ScannedGame;

fn scanned(st: &mut AppState, games: &[ScannedGame], sources: &[Source]) {
    let mut db = Db::open(&st.paths.db_file()).unwrap();
    db.merge_scan(games, sources).unwrap();
    st.reload_library();
}

fn id_of(st: &AppState, name: &str) -> GameId {
    st.library.games.iter().find(|g| g.name == name).unwrap().id
}

/// Scripted screenshot runs must never start a real game (one once did, via a stray gamepad
/// press reaching a visible test window).
#[test]
fn screenshot_runs_never_launch_games() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = AppState::for_test(dir.path());
    let marker = dir.path().join("launched");
    let mut g = ScannedGame::new(Source::Desktop, "x.desktop", "X");
    g.launch = Some(LaunchSpec::Command {
        program: "sh".into(),
        args: vec!["-c".into(), format!("touch '{}'", marker.display())],
        cwd: None,
        env: vec![],
    });
    scanned(&mut st, &[g], &[Source::Desktop]);
    let id = id_of(&st, "X");
    st.no_launch = true;
    st.launch(id);
    std::thread::sleep(std::time::Duration::from_millis(800));
    assert!(!marker.exists(), "a screenshot run launched a game");
    let g = st.library.get(id).unwrap();
    assert_eq!((g.play_count, g.last_played), (0, None));
}

#[test]
fn scan_done_keeps_ui_edits_made_during_the_scan() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = AppState::for_test(dir.path());
    scanned(&mut st, &[ScannedGame::new(Source::Steam, "1", "A")], &[Source::Steam]);
    let id = id_of(&st, "A");
    // What the scan thread does: merge on its own connection, then report.
    let mut scan_db = Db::open(&st.paths.db_file()).unwrap();
    let mut b = ScannedGame::new(Source::Steam, "2", "B");
    b.installed = true;
    scan_db.merge_scan(&[ScannedGame::new(Source::Steam, "1", "A"), b], &[Source::Steam]).unwrap();
    // The user stars A while the message is in flight.
    st.scan.running = true;
    st.toggle_favorite(id);
    st.bg_tx.send(BgMsg::ScanDone { report: ScanReport::default(), added: 1 }).unwrap();
    st.poll_background();
    assert!(!st.scan.running);
    assert!(st.library.get(id).unwrap().favorite, "favourite reverted by ScanDone");
    assert!(st.library.games.iter().any(|g| g.name == "B"), "scan result missing");
}

#[test]
fn new_games_count_survives_a_partial_reload_and_skips_excluded_ones() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = AppState::for_test(dir.path());
    scanned(&mut st, &[ScannedGame::new(Source::Lutris, "1", "A")], &[Source::Lutris]);
    // What a scan with library add-ons does: the scan starts, the built-in libraries are
    // merged and reported early (ScanPartial reloads), then ScanDone arrives.
    st.scan.running = true;
    st.scan.known_before = Some(st.library.games.iter().map(|g| g.id).collect());
    let games = [
        ScannedGame::new(Source::Lutris, "1", "A"),
        ScannedGame::new(Source::Lutris, "2", "B"),
        ScannedGame::new(Source::Lutris, "3", "Proton 9.0"),
    ];
    Db::open(&st.paths.db_file()).unwrap().merge_scan(&games, &[Source::Lutris]).unwrap();
    st.bg_tx.send(BgMsg::ScanPartial { per_source: vec![], total: 3, elapsed: Duration::ZERO }).unwrap();
    st.poll_background();
    st.bg_tx.send(BgMsg::ScanDone { report: ScanReport::default(), added: 2 }).unwrap();
    st.poll_background();
    assert_eq!(st.scan.last.as_ref().unwrap().added, 1, "B is new; the Proton entry doesn't count");
}

#[test]
fn remove_sticks_for_scanned_games_and_deletes_manual_ones() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = AppState::for_test(dir.path());
    let games = [ScannedGame::new(Source::Steam, "239140", "Dying Light"), ScannedGame::new(Source::Steam, "1", "X")];
    scanned(&mut st, &games, &[Source::Steam]);
    let id = id_of(&st, "Dying Light");
    st.set_tags(id, vec!["Zombies".into()]);
    st.remove_game(id);
    // Gone from the normal views at once…
    let visible = st.library.filter(&crate::library::Filter::default());
    assert!(!visible.iter().any(|&i| st.library.games[i].id == id));
    // …and the next scan doesn't bring it back (user data kept for a restore).
    scanned(&mut st, &games, &[Source::Steam]);
    let g = st.library.get(id).unwrap();
    assert!(g.hidden);
    assert_eq!(g.tags, vec!["Zombies".to_string()]);
    st.restore_game(id);
    assert!(!st.library.get(id).unwrap().hidden);
    // A game added by hand is really deleted.
    st.add_manual_game(
        "Mine",
        LaunchSpec::Command { program: "/bin/true".into(), args: vec![], cwd: None, env: vec![] },
        Art::default(),
    );
    let m = id_of(&st, "Mine");
    st.remove_game(m);
    assert!(st.library.get(m).is_none());
    assert!(st.db.get(m).unwrap().is_none());
}

#[test]
fn steam_game_with_custom_command_records_playtime_and_failed_launch_is_not_a_play() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = AppState::for_test(dir.path());
    scanned(
        &mut st,
        &[ScannedGame::new(Source::Steam, "42", "Direct"), ScannedGame::new(Source::Steam, "43", "Broken")],
        &[Source::Steam],
    );
    let id = id_of(&st, "Direct");
    st.set_launch_override(id, Some(LaunchOverride { command: Some("/bin/sleep 1".into()), ..Default::default() }));
    st.launch(id);
    assert_eq!(st.launcher.launch_kind(id), Some(ResolvedKind::Process));
    assert!(st.poll_until(Duration::from_secs(15), |s| s.launcher.running().is_none()), "game never exited");
    let g = st.db.get(id).unwrap().unwrap();
    assert!(g.playtime_secs >= 1, "playtime not recorded: {}", g.playtime_secs);
    assert_eq!(g.play_count, 1);
    assert_eq!(st.library.get(id).unwrap().playtime_secs, g.playtime_secs);
    assert!(st.library.get(id).unwrap().last_played.is_some());
    // Not started through Steam: no rescan of the real Steam data was asked for.
    assert!(!st.scan.running && st.rescan_at.is_none());

    let bad = id_of(&st, "Broken");
    // Fails after a moment (crash at start): no play count, no playtime.
    st.set_launch_override(
        bad,
        Some(LaunchOverride { command: Some("sh -c 'sleep 1.2; exit 3'".into()), ..Default::default() }),
    );
    st.launch(bad);
    assert_eq!(st.library.get(bad).unwrap().play_count, 1);
    assert!(st.poll_until(Duration::from_secs(15), |s| s.launcher.running().is_none()), "never exited");
    let g = st.db.get(bad).unwrap().unwrap();
    assert_eq!((g.play_count, g.last_played, g.playtime_secs), (0, None, 0));
    let g = st.library.get(bad).unwrap();
    assert_eq!((g.play_count, g.last_played), (0, None));
    assert!(st.toasts.iter().any(|t| t.error), "failure toast");
}

#[test]
fn playtime_is_counted_by_how_the_game_ran() {
    assert!(records_own_playtime(Some(ResolvedKind::Process)));
    assert!(records_own_playtime(Some(ResolvedKind::SteamShortcut { appid: 1 })));
    assert!(!records_own_playtime(Some(ResolvedKind::Steam { appid: 1 })));
    // URL launches only end a session (and so count) when folder tracking followed them.
    assert!(records_own_playtime(Some(ResolvedKind::OpenUrl)));
    assert!(!records_own_playtime(None));
}

#[test]
fn toast_width_is_bounded() {
    assert_eq!(toast_width(50.0, 1600.0), 240.0);
    assert_eq!(toast_width(300.4, 1600.0), 301.0);
    assert_eq!(toast_width(2000.0, 1600.0), 460.0);
    // Narrow window: never wider than the screen.
    assert!(toast_width(2000.0, 360.0) <= 360.0 - 48.0);
}

#[test]
fn an_unknown_focus_counts_as_left_for_the_game() {
    assert!(focus_left_for_game(true, Some(true), Some(false)));
    assert!(focus_left_for_game(true, Some(true), None), "known → unknown: gone, as for the gate");
    assert!(!focus_left_for_game(true, Some(true), Some(true)));
    assert!(!focus_left_for_game(true, None, Some(false)), "never had it");
    assert!(!focus_left_for_game(false, Some(true), Some(false)), "no game: nothing to protect");
}
