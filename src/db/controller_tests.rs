use super::*;

/// A library created by the previous version (schema 1, no controller_override column)
/// upgrades in place and keeps its data.
#[test]
fn v1_database_upgrades_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    {
        let mut db = Db::open(&path).unwrap();
        db.merge_scan(&[ScannedGame::new(Source::Steam, "1", "Old Game")], &[Source::Steam]).unwrap();
        let id = db.load_all().unwrap()[0].id;
        db.set_favorite(id, true).unwrap();
        // Turn it back into a v1 database: drop the new column, reset the version.
        db.conn.execute_batch("DROP TRIGGER games_modified; ALTER TABLE games DROP COLUMN controller_override; PRAGMA user_version = 1;").unwrap();
    }
    let db = Db::open(&path).unwrap();
    let v: i32 = db.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(v, SCHEMA_VERSION);
    let g = db.load_all().unwrap().remove(0);
    assert!(g.favorite);
    assert_eq!(g.name, "Old Game");
    assert_eq!(g.controller_override, None);
    db.set_controller_override(g.id, Some(ControllerSupport::None)).unwrap();
    assert_eq!(db.get(g.id).unwrap().unwrap().controller_override, Some(ControllerSupport::None));
    // Opening again is a no-op.
    drop(db);
    Db::open(&path).unwrap();
}

#[test]
fn override_wins_and_survives_rescan() {
    let mut db = Db::open_memory().unwrap();
    let mut g = ScannedGame::new(Source::Steam, "7", "Pad Game");
    g.meta.controller = Some(ControllerSupport::Partial);
    g.meta.deck = Some(DeckRating::Verified);
    db.merge_scan(std::slice::from_ref(&g), &[Source::Steam]).unwrap();
    let id = db.load_all().unwrap()[0].id;
    let loaded = db.get(id).unwrap().unwrap();
    assert_eq!(loaded.controller(), Some(ControllerSupport::Partial));
    assert_eq!(loaded.meta.deck, Some(DeckRating::Verified));
    db.set_controller_override(id, Some(ControllerSupport::Full)).unwrap();
    db.merge_scan(&[g], &[Source::Steam]).unwrap();
    let after = db.get(id).unwrap().unwrap();
    assert_eq!(after.controller(), Some(ControllerSupport::Full));
    db.set_controller_override(id, None).unwrap();
    assert_eq!(db.get(id).unwrap().unwrap().controller(), Some(ControllerSupport::Partial));
}
