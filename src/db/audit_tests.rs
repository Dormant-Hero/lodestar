use super::*;

fn sg(src: Source, id: &str, name: &str) -> ScannedGame {
    ScannedGame::new(src, id, name)
}

#[test]
fn rescan_keeps_custom_name_sort_key() {
    let mut db = Db::open_memory().unwrap();
    db.merge_scan(&[ScannedGame::new(Source::Steam, "1", "Alpha")], &[Source::Steam]).unwrap();
    let id = db.load_all().unwrap()[0].id;
    db.set_user_name(id, Some("Zulu")).unwrap();
    db.merge_scan(&[ScannedGame::new(Source::Steam, "1", "Alpha")], &[Source::Steam]).unwrap();
    assert_eq!(db.get(id).unwrap().unwrap().sort_name, "zulu");
}

#[test]
fn empty_source_result_does_not_delete() {
    let mut db = Db::open_memory().unwrap();
    db.merge_scan(&[sg(Source::Faugus, "a", "A"), sg(Source::Faugus, "b", "B")], &[Source::Faugus]).unwrap();
    // games.json caught mid-rewrite (empty): nothing may be deleted.
    let s = db.merge_scan(&[], &[Source::Faugus]).unwrap();
    assert_eq!(s.removed, 0);
    assert_eq!(db.load_all().unwrap().len(), 2);
}

#[test]
fn renamed_desktop_entry_keeps_user_data() {
    let mut db = Db::open_memory().unwrap();
    db.merge_scan(
        &[sg(Source::Desktop, "old.desktop", "Game"), sg(Source::Desktop, "x.desktop", "X")],
        &[Source::Desktop],
    )
    .unwrap();
    let id = db.load_all().unwrap().iter().find(|g| g.name == "Game").unwrap().id;
    db.set_favorite(id, true).unwrap();
    db.merge_scan(
        &[sg(Source::Desktop, "new.desktop", "Game"), sg(Source::Desktop, "x.desktop", "X")],
        &[Source::Desktop],
    )
    .unwrap();
    let all = db.load_all().unwrap();
    assert_eq!(all.len(), 2, "{all:#?}");
    let g = all.iter().find(|g| g.name == "Game").unwrap();
    assert_eq!((g.id, g.favorite, g.source_id.as_str(), g.installed), (id, true, "new.desktop", true));
}

#[test]
fn adoption_only_when_unambiguous_and_allowed() {
    let mut db = Db::open_memory().unwrap();
    // Two vanished rows with the same name: nobody adopts, the new entry is a new row.
    db.merge_scan(
        &[sg(Source::Faugus, "a1", "Same"), sg(Source::Faugus, "a2", "Same"), sg(Source::Faugus, "k", "Keep")],
        &[Source::Faugus],
    )
    .unwrap();
    for g in db.load_all().unwrap() {
        db.set_favorite(g.id, true).unwrap();
    }
    db.merge_scan(&[sg(Source::Faugus, "a3", "Same"), sg(Source::Faugus, "k", "Keep")], &[Source::Faugus]).unwrap();
    let all = db.load_all().unwrap();
    assert_eq!(all.len(), 4);
    assert!(!all.iter().find(|g| g.source_id == "a3").unwrap().favorite);
    // A different Steam appid with the same title is a different product.
    db.merge_scan(&[sg(Source::Steam, "100", "Remaster")], &[Source::Steam]).unwrap();
    let old = db.load_all().unwrap().into_iter().find(|g| g.source_id == "100").unwrap().id;
    db.set_favorite(old, true).unwrap();
    db.merge_scan(&[sg(Source::Steam, "200", "Remaster")], &[Source::Steam]).unwrap();
    let g100 = db.get(old).unwrap().unwrap();
    assert_eq!((g100.source_id.as_str(), g100.installed, g100.favorite), ("100", false, true));
    // …but a Steam shortcut whose id changed (exe edited) is the same game.
    db.merge_scan(&[sg(Source::Steam, "shortcut:1", "Emu"), sg(Source::Steam, "200", "Remaster")], &[Source::Steam])
        .unwrap();
    let emu = db.load_all().unwrap().into_iter().find(|g| g.source_id == "shortcut:1").unwrap().id;
    db.set_tags(emu, &["retro".into()]).unwrap();
    db.merge_scan(&[sg(Source::Steam, "shortcut:2", "Emu"), sg(Source::Steam, "200", "Remaster")], &[Source::Steam])
        .unwrap();
    let g = db.get(emu).unwrap().unwrap();
    assert_eq!((g.source_id.as_str(), g.tags.clone()), ("shortcut:2", vec!["retro".to_string()]));
}

#[test]
fn game_moving_between_libraries_and_failed_sources() {
    let mut db = Db::open_memory().unwrap();
    let mut a = sg(Source::Steam, "1", "A");
    a.install_dir = Some("/mnt/a/steamapps/common/A".into());
    db.merge_scan(&[a.clone(), sg(Source::Faugus, "f", "F")], &[Source::Steam, Source::Faugus]).unwrap();
    let id = db.load_all().unwrap().into_iter().find(|g| g.source_id == "1").unwrap().id;
    db.set_launch_override(id, Some(&LaunchOverride { options: "%command% -x".into(), ..Default::default() })).unwrap();
    db.add_playtime(id, 30).unwrap();
    a.install_dir = Some("/mnt/b/steamapps/common/A".into());
    // Steam moved the game; Faugus failed this time (not in ok sources) and must stay as it was.
    db.merge_scan(&[a], &[Source::Steam]).unwrap();
    let g = db.get(id).unwrap().unwrap();
    assert_eq!(g.install_dir.as_deref(), Some(Path::new("/mnt/b/steamapps/common/A")));
    assert!(g.launch_override.is_some());
    assert_eq!(g.playtime_secs, 30);
    assert!(db.load_all().unwrap().iter().any(|g| g.source_id == "f" && g.installed));
}

#[test]
fn merge_is_fast_for_big_libraries() {
    let mut db = Db::open_memory().unwrap();
    let games: Vec<ScannedGame> =
        (0..5000).map(|i| sg(Source::Desktop, &format!("g{i}.desktop"), &format!("Game {i}"))).collect();
    let t = std::time::Instant::now();
    db.merge_scan(&games, &[Source::Desktop]).unwrap();
    // Rescan where every id changed (worst case for the rename matching).
    let renamed: Vec<ScannedGame> =
        (0..5000).map(|i| sg(Source::Desktop, &format!("n{i}.desktop"), &format!("Game {i}"))).collect();
    let s = db.merge_scan(&renamed, &[Source::Desktop]).unwrap();
    assert_eq!((s.added, s.updated, s.removed), (0, 5000, 0));
    assert!(t.elapsed() < std::time::Duration::from_secs(3), "{:?}", t.elapsed());
}

#[test]
fn merge_survives_concurrent_ui_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let mut scan_db = Db::open(&path).unwrap();
    let ui_db = Db::open(&path).unwrap();
    scan_db.merge_scan(&[sg(Source::Steam, "1", "A")], &[Source::Steam]).unwrap();
    let id = ui_db.load_all().unwrap()[0].id;
    let t = std::thread::spawn(move || {
        for i in 0..200 {
            ui_db.set_favorite(id, i % 2 == 0).unwrap();
        }
        ui_db.set_favorite(id, true).unwrap();
    });
    for _ in 0..200 {
        scan_db.merge_scan(&[sg(Source::Steam, "1", "A"), sg(Source::Steam, "2", "B")], &[Source::Steam]).unwrap();
    }
    t.join().unwrap();
    assert!(scan_db.get(id).unwrap().unwrap().favorite);
}

#[test]
fn steam_last_played_counts_only_with_playtime_or_a_launch() {
    let mut db = Db::open_memory().unwrap();
    // What Steam's localconfig has after installs: LastPlayed without Playtime.
    let mut installed_only = sg(Source::Steam, "960910", "Heavy Rain");
    installed_only.last_played = Some(1_792_188_688);
    let mut played = sg(Source::Steam, "252950", "Rocket League");
    played.last_played = Some(1_785_710_411);
    played.playtime_secs = Some(40399 * 60);
    let mut shortcut = sg(Source::Steam, "shortcut:7", "Emu");
    shortcut.last_played = Some(1_790_000_000);
    let mut lutris = sg(Source::Lutris, "x", "L");
    lutris.last_played = Some(1_790_000_001);
    db.merge_scan(&[installed_only, played, shortcut], &[Source::Steam]).unwrap();
    db.merge_scan(&[lutris], &[Source::Lutris]).unwrap();
    let lp = |db: &Db, name: &str| db.load_all().unwrap().into_iter().find(|g| g.name == name).unwrap();
    assert_eq!(lp(&db, "Heavy Rain").last_played, None);
    assert_eq!(lp(&db, "Rocket League").last_played, Some(1_785_710_411));
    assert_eq!(lp(&db, "Emu").last_played, Some(1_790_000_000));
    assert_eq!(lp(&db, "L").last_played, Some(1_790_000_001));
    // Launched from Lodestar: it counts (the newer time wins) …
    let id = lp(&db, "Heavy Rain").id;
    let prev = db.record_launch(id, 1_800_000_000).unwrap();
    assert_eq!(prev, Some(1_792_188_688));
    assert_eq!(db.get(id).unwrap().unwrap().last_played, Some(1_800_000_000));
    // … unless that launch failed at once: back to how it was.
    db.undo_launch(id, prev).unwrap();
    let g = db.get(id).unwrap().unwrap();
    assert_eq!((g.play_count, g.last_played), (0, None));
    // Steam's value is still stored: playtime later makes it count.
    db.add_playtime(id, 120).unwrap();
    assert_eq!(db.get(id).unwrap().unwrap().last_played, Some(1_792_188_688));
}

#[test]
fn duplicate_ids_in_one_scan_first_wins() {
    let mut db = Db::open_memory().unwrap();
    let s = db
        .merge_scan(&[sg(Source::Faugus, "x", "First"), sg(Source::Faugus, "x", "Second")], &[Source::Faugus])
        .unwrap();
    assert_eq!((s.added, s.updated), (1, 0));
    let all = db.load_all().unwrap();
    assert_eq!((all.len(), all[0].name.as_str()), (1, "First"));
}
