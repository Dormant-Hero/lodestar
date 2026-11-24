//! Schema v5: emulated games (`emu_json`, `EmuRef`).
use super::*;

fn rom(id: &str, name: &str, platform: &str) -> ScannedGame {
    let mut g = ScannedGame::new(Source::Emulator, id, name);
    g.platform = Some("Sony PlayStation 2".into());
    g.emu = Some(EmuRef {
        platform: platform.into(),
        folder: "f1".into(),
        rom: PathBuf::from(format!("/roms/{id}.cue")),
        files: vec![PathBuf::from(format!("/roms/{id}.bin"))],
        emulator: "pcsx2".into(),
        profile: "default".into(),
        serial: Some("SLUS-20062".into()),
        crc: Some("0badf00d".into()),
        dat_name: Some("Some Game (USA)".into()),
        region: Some("USA".into()),
    });
    g
}

#[test]
fn v4_database_gets_the_emu_column() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    {
        let mut db = Db::open(&path).unwrap();
        db.merge_scan(&[ScannedGame::new(Source::Steam, "1", "Old")], &[Source::Steam]).unwrap();
        db.conn.execute_batch("ALTER TABLE games DROP COLUMN emu_json; PRAGMA user_version = 4;").unwrap();
    }
    let mut db = Db::open(&path).unwrap();
    let v: i32 = db.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(v, SCHEMA_VERSION);
    let old = db.load_all().unwrap().remove(0);
    assert_eq!((old.name.as_str(), old.emu.as_ref()), ("Old", None));
    // The new column works after the upgrade.
    db.merge_scan(&[rom("f1/a", "Alpha", "sony_playstation2")], &[Source::Emulator]).unwrap();
    let g = db.load_all().unwrap().into_iter().find(|g| g.source == Source::Emulator).unwrap();
    assert_eq!(g.emu.unwrap().platform, "sony_playstation2");
}

#[test]
fn emu_ref_round_trips_and_updates() {
    let mut db = Db::open_memory().unwrap();
    let a = rom("f1/a", "Alpha", "sony_playstation2");
    db.merge_scan(std::slice::from_ref(&a), &[Source::Emulator]).unwrap();
    let g = db.load_all().unwrap().remove(0);
    assert_eq!(g.emu, a.emu);
    assert_eq!(g.source, Source::Emulator);
    // A rescan rewrites it.
    let mut b = a.clone();
    b.emu.as_mut().unwrap().emulator = "duckstation".into();
    db.merge_scan(std::slice::from_ref(&b), &[Source::Emulator]).unwrap();
    assert_eq!(db.get(g.id).unwrap().unwrap().emu.unwrap().emulator, "duckstation");
    // A scan without one clears it; a broken value reads as None.
    db.merge_scan(&[ScannedGame::new(Source::Emulator, "f1/a", "Alpha")], &[Source::Emulator]).unwrap();
    assert_eq!(db.get(g.id).unwrap().unwrap().emu, None);
    db.conn.execute("UPDATE games SET emu_json = 'nope' WHERE id = ?1", [g.id]).unwrap();
    assert_eq!(db.get(g.id).unwrap().unwrap().emu, None);
}

#[test]
fn per_game_emulator_choice_survives_rescans() {
    let mut db = Db::open_memory().unwrap();
    db.merge_scan(&[rom("f1/a", "Alpha", "sony_playstation2")], &[Source::Emulator]).unwrap();
    let id = db.load_all().unwrap()[0].id;
    let e = UserEdits { emulator: Some("pcsx2".into()), emulator_profile: Some("custom".into()), ..Default::default() };
    db.set_edits(id, &e).unwrap();
    db.merge_scan(&[rom("f1/a", "Alpha", "sony_playstation2")], &[Source::Emulator]).unwrap();
    let g = db.get(id).unwrap().unwrap();
    assert_eq!((g.edits.emulator.as_deref(), g.edits.emulator_profile.as_deref()), (Some("pcsx2"), Some("custom")));
}

#[test]
fn moved_rom_takes_over_its_row() {
    let mut db = Db::open_memory().unwrap();
    db.merge_scan(&[rom("f1/ps2/Alpha.iso", "Alpha", "sony_playstation2")], &[Source::Emulator]).unwrap();
    let id = db.load_all().unwrap()[0].id;
    db.set_favorite(id, true).unwrap();
    let s = db.merge_scan(&[rom("f2/Alpha.iso", "Alpha", "sony_playstation2")], &[Source::Emulator]).unwrap();
    assert_eq!((s.added, s.removed), (0, 0));
    let g = db.get(id).unwrap().unwrap();
    assert_eq!(g.source_id, "f2/Alpha.iso");
    assert!(g.favorite);
}

/// A second copy of a ROM (an older scan listed it twice) folds into the copy that stays: its
/// place in a collection goes to that copy, so the collection neither loses the game nor later
/// picks up whatever new game SQLite gives the deleted row's id.
#[test]
fn folded_duplicate_keeps_its_collection_place() {
    use crate::library::user::{self, UserCollection};
    let mut db = Db::open_memory().unwrap();
    let (a, b) = (rom("f1/a", "Alpha", "sony_playstation2"), rom("f2/a", "Alpha", "sony_playstation2"));
    db.merge_scan(&[a.clone(), b], &[Source::Emulator]).unwrap();
    let all = db.load_all().unwrap();
    let id = |sid: &str| all.iter().find(|g| g.source_id == sid).unwrap().id;
    let (keep, dup) = (id("f1/a"), id("f2/a"));
    db.set_favorite(dup, true).unwrap();
    user::save(&db, &[UserCollection { id: 7, name: "PS2".into(), games: vec![dup], rule: None }]);
    db.merge_scan(std::slice::from_ref(&a), &[Source::Emulator]).unwrap();
    let all = db.load_all().unwrap();
    assert_eq!(all.len(), 1);
    assert!(all[0].favorite);
    assert_eq!(user::load(&db)[0].games, vec![keep]);
}

/// A RetroAchievements game id set by hand (Edit → Installation) is a user edit: it shows as
/// `meta.ra_game_id`, survives rescans (even one whose scanner metadata changed) and goes
/// with "back to the source's values" (empty edits).
#[test]
fn hand_set_retroachievements_id_survives_rescans() {
    let mut db = Db::open_memory().unwrap();
    let a = rom("f1/a", "Alpha", "nintendo_nes");
    db.merge_scan(std::slice::from_ref(&a), &[Source::Emulator]).unwrap();
    let g = db.load_all().unwrap().remove(0);
    assert_eq!(g.meta.ra_game_id, None);
    let e = UserEdits { ra_game_id: Some(1234), ..g.edits.clone() };
    db.set_edits(g.id, &e).unwrap();
    assert_eq!(db.get(g.id).unwrap().unwrap().meta.ra_game_id, Some(1234));
    // Rescans (the second with new scanner metadata) keep it.
    db.merge_scan(std::slice::from_ref(&a), &[Source::Emulator]).unwrap();
    let mut b = a.clone();
    b.meta.description = Some("New text".into());
    db.merge_scan(std::slice::from_ref(&b), &[Source::Emulator]).unwrap();
    let g = db.get(g.id).unwrap().unwrap();
    assert_eq!((g.meta.ra_game_id, g.edits.ra_game_id), (Some(1234), Some(1234)));
    assert_eq!(g.meta.description.as_deref(), Some("New text"));
    // Fetched metadata doesn't touch it either.
    db.set_fetched(g.id, &Art::default(), &Metadata { ra_game_id: Some(9), ..Default::default() }).unwrap();
    assert_eq!(db.get(g.id).unwrap().unwrap().meta.ra_game_id, Some(1234));
    // Cleared: back to automatic (id 0 counts as none).
    db.set_edits(g.id, &UserEdits { ra_game_id: Some(0), ..g.edits.clone() }).unwrap();
    assert_eq!(db.get(g.id).unwrap().unwrap().meta.ra_game_id, Some(9), "the fetched one shows again");
    db.set_edits(g.id, &UserEdits::default()).unwrap();
    assert_eq!(db.get(g.id).unwrap().unwrap().edits.ra_game_id, None);
}
