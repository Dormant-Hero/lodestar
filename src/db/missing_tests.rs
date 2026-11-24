//! Games that vanish from their library: removed, missing or only uninstalled (`Db::merge_scan`).

use super::*;

fn sg(src: Source, id: &str, name: &str) -> ScannedGame {
    ScannedGame::new(src, id, name)
}

fn find<'a>(all: &'a [Game], sid: &str) -> Option<&'a Game> {
    all.iter().find(|g| g.source_id == sid)
}

#[test]
fn shortcut_deleted_in_steam_goes_even_with_fetched_art() {
    let mut db = Db::open_memory().unwrap();
    let scan = [
        sg(Source::Steam, "10", "Portal"),
        sg(Source::Steam, "shortcut:1", "Ghost"),
        sg(Source::Steam, "shortcut:2", "Other"),
    ];
    db.merge_scan(&scan, &[Source::Steam]).unwrap();
    let ghost = find(&db.load_all().unwrap(), "shortcut:1").unwrap().id;
    // Online art and metadata were fetched for it: not the user's data.
    let art = Art { cover: Some("/f/c.jpg".into()), ..Default::default() };
    db.set_fetched(ghost, &art, &Metadata { developer: Some("Dev".into()), ..Default::default() }).unwrap();
    // The user deletes the shortcut in Steam.
    let s = db.merge_scan(&[scan[0].clone(), scan[2].clone()], &[Source::Steam]).unwrap();
    assert_eq!((s.removed, s.missing), (1, 0));
    let all = db.load_all().unwrap();
    assert!(find(&all, "shortcut:1").is_none(), "no ghost left");
    assert_eq!(all.len(), 2);
}

#[test]
fn uninstalled_but_owned_steam_game_is_never_missing() {
    let mut db = Db::open_memory().unwrap();
    db.merge_scan(&[sg(Source::Steam, "10", "Portal"), sg(Source::Steam, "20", "Half-Life")], &[Source::Steam])
        .unwrap();
    let id = find(&db.load_all().unwrap(), "10").unwrap().id;
    db.add_playtime(id, 3600).unwrap();
    // Uninstalled: Steam still lists it as owned.
    let mut owned = sg(Source::Steam, "10", "Portal");
    owned.installed = false;
    let s = db.merge_scan(&[owned, sg(Source::Steam, "20", "Half-Life")], &[Source::Steam]).unwrap();
    assert_eq!(s.missing, 0);
    let g = db.get(id).unwrap().unwrap();
    assert!(!g.installed);
    assert_eq!(g.missing, None);
    // Owned games not listed (the setting is off): only "not installed", even without user data.
    let s = db.merge_scan(&[sg(Source::Steam, "99", "Something")], &[Source::Steam]).unwrap();
    assert_eq!((s.removed, s.missing), (0, 0));
    let all = db.load_all().unwrap();
    for sid in ["10", "20"] {
        let g = find(&all, sid).unwrap();
        assert!(!g.installed && g.missing.is_none(), "{sid}");
    }
    // A Battle.net game uninstalled from its Wine prefix isn't missing either.
    db.merge_scan(
        &[sg(Source::BattleNet, "wow", "World of Warcraft"), sg(Source::BattleNet, "d4", "Diablo IV")],
        &[Source::BattleNet],
    )
    .unwrap();
    let wow = find(&db.load_all().unwrap(), "wow").unwrap().id;
    db.set_favorite(wow, true).unwrap();
    db.merge_scan(&[sg(Source::BattleNet, "d4", "Diablo IV")], &[Source::BattleNet]).unwrap();
    let g = db.get(wow).unwrap().unwrap();
    assert!(!g.installed && g.missing.is_none());
}

#[test]
fn played_shortcut_goes_missing_and_stays_until_removed() {
    let mut db = Db::open_memory().unwrap();
    let keep = sg(Source::Steam, "shortcut:2", "Other");
    db.merge_scan(&[sg(Source::Steam, "shortcut:1", "Played"), keep.clone()], &[Source::Steam]).unwrap();
    let id = find(&db.load_all().unwrap(), "shortcut:1").unwrap().id;
    db.add_playtime(id, 600).unwrap();
    let s = db.merge_scan(std::slice::from_ref(&keep), &[Source::Steam]).unwrap();
    assert_eq!((s.removed, s.missing), (0, 1));
    let g = db.get(id).unwrap().unwrap();
    let m = g.missing.expect("missing");
    assert!(!m.kept && m.since > 0);
    assert!(!g.installed);
    assert_eq!(g.playtime_secs, 600);
    // Scan after scan it stays, missing since the first time.
    let s = db.merge_scan(std::slice::from_ref(&keep), &[Source::Steam]).unwrap();
    assert_eq!((s.removed, s.missing), (0, 0));
    assert_eq!(db.get(id).unwrap().unwrap().missing, Some(m));
    // Remove from library… takes it out; Undo puts it back exactly as it was.
    let before = db.get(id).unwrap().unwrap();
    let taken = db.take_games(&[id, 12345]).unwrap();
    assert_eq!(taken.len(), 1);
    assert!(db.get(id).unwrap().is_none());
    assert_eq!(db.restore_games(&taken).unwrap(), 1);
    assert_eq!(db.get(id).unwrap().unwrap(), before);
    db.take_games(&[id]).unwrap();
    assert!(db.load_all().unwrap().iter().all(|g| g.source_id != "shortcut:1"));
}

#[test]
fn a_missing_game_that_comes_back_is_not_missing_any_more() {
    let mut db = Db::open_memory().unwrap();
    let scan = [sg(Source::Faugus, "a", "Alpha"), sg(Source::Faugus, "b", "Beta")];
    db.merge_scan(&scan, &[Source::Faugus]).unwrap();
    let id = find(&db.load_all().unwrap(), "a").unwrap().id;
    db.set_tags(id, &["rpg".to_string()]).unwrap();
    db.merge_scan(&scan[1..], &[Source::Faugus]).unwrap();
    assert!(db.get(id).unwrap().unwrap().missing.is_some());
    db.set_missing_kept(&[id], true).unwrap();
    assert!(db.get(id).unwrap().unwrap().missing.unwrap().kept);
    db.merge_scan(&scan, &[Source::Faugus]).unwrap();
    let g = db.get(id).unwrap().unwrap();
    assert_eq!(g.missing, None);
    assert!(g.installed);
}

#[test]
fn next_scan_removes_a_missing_game_without_user_data() {
    let mut db = Db::open_memory().unwrap();
    let other = sg(Source::Desktop, "o", "Other");
    db.merge_scan(&[sg(Source::Desktop, "a", "Alpha"), other.clone()], &[Source::Desktop]).unwrap();
    let id = find(&db.load_all().unwrap(), "a").unwrap().id;
    db.set_favorite(id, true).unwrap();
    db.merge_scan(std::slice::from_ref(&other), &[Source::Desktop]).unwrap();
    assert!(db.get(id).unwrap().unwrap().missing.is_some());
    // The favourite was the only thing keeping it.
    db.set_favorite(id, false).unwrap();
    let s = db.merge_scan(std::slice::from_ref(&other), &[Source::Desktop]).unwrap();
    assert_eq!(s.removed, 1);
    assert!(db.get(id).unwrap().is_none());
}

#[test]
fn kept_counts_as_user_data() {
    let mut db = Db::open_memory().unwrap();
    let other = sg(Source::Lutris, "o", "Other");
    db.merge_scan(&[sg(Source::Lutris, "a", "Alpha"), other.clone()], &[Source::Lutris]).unwrap();
    let id = find(&db.load_all().unwrap(), "a").unwrap().id;
    db.record_launch(id, 1000).unwrap();
    db.merge_scan(std::slice::from_ref(&other), &[Source::Lutris]).unwrap();
    db.set_missing_kept(&[id], true).unwrap();
    db.set_play_stats(id, 0, 0, None).unwrap();
    db.merge_scan(std::slice::from_ref(&other), &[Source::Lutris]).unwrap();
    assert!(db.get(id).unwrap().unwrap().missing.unwrap().kept);
}

#[test]
fn a_source_that_lists_nothing_deletes_nothing() {
    let mut db = Db::open_memory().unwrap();
    db.merge_scan(&[sg(Source::Faugus, "a", "Alpha")], &[Source::Faugus]).unwrap();
    let id = db.load_all().unwrap()[0].id;
    db.add_playtime(id, 5).unwrap();
    db.merge_scan(&[sg(Source::Faugus, "a", "Alpha"), sg(Source::Faugus, "b", "Beta")], &[Source::Faugus]).unwrap();
    let beta = find(&db.load_all().unwrap(), "b").unwrap().id;
    // Nothing listed (games.json mid-rewrite, or every game deleted): nothing is deleted, both
    // are missing (so the user can still remove them), and a scan that lists them again clears it.
    let s = db.merge_scan(&[], &[Source::Faugus]).unwrap();
    assert_eq!((s.removed, s.missing), (0, 2));
    let g = db.get(id).unwrap().unwrap();
    assert!(!g.installed && g.missing.is_some());
    assert!(db.get(beta).unwrap().unwrap().missing.is_some());
    db.merge_scan(&[sg(Source::Faugus, "a", "Alpha"), sg(Source::Faugus, "b", "Beta")], &[Source::Faugus]).unwrap();
    assert!(db.load_all().unwrap().iter().all(|g| g.missing.is_none()));
    // A failed scan (source not listed) changes nothing either.
    db.merge_scan(&[], &[]).unwrap();
    assert!(db.get(id).unwrap().is_some());
}

#[test]
fn a_game_in_a_collection_goes_missing() {
    let mut db = Db::open_memory().unwrap();
    let other = sg(Source::Bottles, "o", "Other");
    db.merge_scan(&[sg(Source::Bottles, "a", "Alpha"), other.clone()], &[Source::Bottles]).unwrap();
    let id = find(&db.load_all().unwrap(), "a").unwrap().id;
    let colls = vec![crate::library::UserCollection { id: 1, name: "Couch".into(), games: vec![id], rule: None }];
    crate::library::user::save(&db, &colls);
    db.merge_scan(std::slice::from_ref(&other), &[Source::Bottles]).unwrap();
    assert!(db.get(id).unwrap().unwrap().missing.is_some());
}

#[test]
fn unreadable_shortcuts_file_deletes_no_shortcuts() {
    let mut db = Db::open_memory().unwrap();
    let portal = sg(Source::Steam, "10", "Portal");
    db.merge_scan(&[portal.clone(), sg(Source::Steam, "shortcut:1", "Ghost")], &[Source::Steam]).unwrap();
    let id = find(&db.load_all().unwrap(), "shortcut:1").unwrap().id;
    // Steam scanned fine but listed no shortcuts at all (shortcuts.vdf caught mid-rewrite).
    let s = db.merge_scan(std::slice::from_ref(&portal), &[Source::Steam]).unwrap();
    assert_eq!((s.removed, s.missing), (0, 1));
    assert!(db.get(id).unwrap().unwrap().missing.is_some());
    // Listed again: the same row, no longer missing.
    db.merge_scan(&[portal, sg(Source::Steam, "shortcut:1", "Ghost")], &[Source::Steam]).unwrap();
    let g = db.get(id).unwrap().unwrap();
    assert!(g.missing.is_none() && g.installed);
}

/// A RomM game the user favourited, tagged, played and put in a collection is downloaded: the
/// scan finds the ROM and drops the RomM entry (`absorbs`). Everything moves to the ROM's row and
/// no "No longer in RomM" ghost is left.
#[test]
fn downloaded_romm_game_takes_its_data_along() {
    use crate::library::user::{self, UserCollection};
    let mut db = Db::open_memory().unwrap();
    db.merge_scan(
        &[sg(Source::Romm, "42", "Chrono Trigger"), sg(Source::Romm, "43", "Earthbound")],
        &[Source::Romm, Source::Emulator],
    )
    .unwrap();
    let all = db.load_all().unwrap();
    let romm = find(&all, "42").unwrap().id;
    db.set_favorite(romm, true).unwrap();
    db.set_tags(romm, &["rpg".into()]).unwrap();
    db.set_play_stats(romm, 600, 2, Some(1_700_000_000)).unwrap();
    let coll = UserCollection { id: 1, name: "Best".into(), games: vec![romm], rule: None };
    user::save(&db, &[coll]);

    let mut rom = sg(Source::Emulator, "snes:/roms/snes/Chrono Trigger.sfc", "Chrono Trigger");
    rom.absorbs.push((Source::Romm, "42".into()));
    let s = db.merge_scan(&[rom, sg(Source::Romm, "43", "Earthbound")], &[Source::Romm, Source::Emulator]).unwrap();
    assert_eq!(s.missing, 0, "no ghost");
    let all = db.load_all().unwrap();
    assert!(find(&all, "42").is_none(), "the RomM row went");
    let g = find(&all, "snes:/roms/snes/Chrono Trigger.sfc").unwrap();
    assert!(g.favorite);
    assert_eq!(g.tags, vec!["rpg".to_string()]);
    assert_eq!((g.playtime_secs, g.play_count, g.last_played), (600, 2, Some(1_700_000_000)));
    assert_eq!(user::load(&db)[0].games, vec![g.id], "the collection follows");
    // The next scan (same result) changes nothing.
    let mut rom = sg(Source::Emulator, "snes:/roms/snes/Chrono Trigger.sfc", "Chrono Trigger");
    rom.absorbs.push((Source::Romm, "42".into()));
    db.merge_scan(&[rom], &[Source::Romm, Source::Emulator]).unwrap();
    let g2 = find(&db.load_all().unwrap(), "snes:/roms/snes/Chrono Trigger.sfc").unwrap().clone();
    assert!(g2.favorite && g2.playtime_secs == 600);
}
