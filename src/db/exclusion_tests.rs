use super::*;
use crate::exclude::Mark;

fn sg(src: Source, id: &str, name: &str) -> ScannedGame {
    ScannedGame::new(src, id, name)
}

#[test]
fn v3_database_gets_the_exclusions_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    {
        let mut db = Db::open(&path).unwrap();
        db.merge_scan(&[sg(Source::Steam, "1", "Old")], &[Source::Steam]).unwrap();
        db.conn.execute_batch("DROP TABLE exclusions; PRAGMA user_version = 3;").unwrap();
    }
    let db = Db::open(&path).unwrap();
    let v: i32 = db.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(v, SCHEMA_VERSION);
    assert_eq!(db.load_all().unwrap()[0].name, "Old");
    assert!(db.exclusions().unwrap().is_empty());
    db.set_exclusions(&[(Source::Steam, "1".into(), "Old".into())], Mark::NotGame).unwrap();
    drop(db);
    let db = Db::open(&path).unwrap();
    let e = db.exclusions().unwrap();
    assert_eq!((e.len(), e[0].mark, e[0].name.as_str()), (1, Mark::NotGame, "Old"));
}

#[test]
fn marks_upsert_clear_and_survive_rescans() {
    let mut db = Db::open_memory().unwrap();
    db.merge_scan(
        &[sg(Source::Desktop, "a.desktop", "Tool"), sg(Source::Desktop, "b.desktop", "Game")],
        &[Source::Desktop],
    )
    .unwrap();
    db.set_exclusions(&[(Source::Desktop, "a.desktop".into(), "Tool".into())], Mark::NotGame).unwrap();
    // The entry disappears (row deleted: no user data) and comes back: still marked.
    db.merge_scan(&[sg(Source::Desktop, "b.desktop", "Game")], &[Source::Desktop]).unwrap();
    assert_eq!(db.load_all().unwrap().len(), 1);
    db.merge_scan(
        &[sg(Source::Desktop, "a.desktop", "Tool 2"), sg(Source::Desktop, "b.desktop", "Game")],
        &[Source::Desktop],
    )
    .unwrap();
    let e = db.exclusions().unwrap();
    assert_eq!((e.len(), e[0].source_id.as_str()), (1, "a.desktop"));
    // Marking again replaces the kind.
    db.set_exclusions(&[(Source::Desktop, "a.desktop".into(), "Tool 2".into())], Mark::Keep).unwrap();
    let e = db.exclusions().unwrap();
    assert_eq!((e.len(), e[0].mark, e[0].name.as_str()), (1, Mark::Keep, "Tool 2"));
    db.clear_exclusions(&[(Source::Desktop, "a.desktop".into())]).unwrap();
    assert!(db.exclusions().unwrap().is_empty());
}

#[test]
fn a_mark_follows_an_adopted_row() {
    let mut db = Db::open_memory().unwrap();
    db.merge_scan(&[sg(Source::Lutris, "old", "Junk"), sg(Source::Lutris, "k", "Keep")], &[Source::Lutris]).unwrap();
    db.set_exclusions(&[(Source::Lutris, "old".into(), "Junk".into())], Mark::NotGame).unwrap();
    // Reinstalled under a new id, same name: the row (and the mark) move over.
    db.merge_scan(&[sg(Source::Lutris, "new", "Junk"), sg(Source::Lutris, "k", "Keep")], &[Source::Lutris]).unwrap();
    let e = db.exclusions().unwrap();
    assert_eq!(e[0].source_id, "new");
}

#[test]
fn bulk_hidden_and_edits() {
    let mut db = Db::open_memory().unwrap();
    db.merge_scan(
        &[sg(Source::Steam, "1", "A"), sg(Source::Steam, "2", "B"), sg(Source::Steam, "3", "C")],
        &[Source::Steam],
    )
    .unwrap();
    let ids: Vec<GameId> = db.load_all().unwrap().iter().map(|g| g.id).collect();
    db.set_hidden_many(&ids[..2], true).unwrap();
    let hidden: Vec<bool> = db.load_all().unwrap().iter().map(|g| g.hidden).collect();
    assert_eq!(hidden, [true, true, false]);
    let e = UserEdits { notes: "n".into(), ..Default::default() };
    db.set_edits_many(&[(ids[0], e.clone()), (ids[2], UserEdits::default())]).unwrap();
    assert_eq!(db.get(ids[0]).unwrap().unwrap().edits, e);
    assert!(db.get(ids[2]).unwrap().unwrap().edits.is_empty());
}
