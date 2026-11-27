use super::*;

#[test]
fn edits_override_source_and_survive_rescans() {
    let mut db = Db::open_memory().unwrap();
    let mut g = ScannedGame::new(Source::Steam, "5", "Game");
    g.meta.developer = Some("Source Dev".into());
    g.meta.genres = vec!["Action".into()];
    g.platform = Some("windows".into());
    db.merge_scan(std::slice::from_ref(&g), &[Source::Steam]).unwrap();
    let id = db.load_all().unwrap()[0].id;
    let e = UserEdits {
        developer: Some("My Dev".into()),
        genres: Some(vec!["RPG".into()]),
        sort_name: Some("Aaa".into()),
        completion: Some(CompletionStatus::Beaten),
        links: vec![GameLink { name: "Wiki".into(), url: "https://w".into() }],
        ..Default::default()
    };
    db.set_edits(id, &e).unwrap();
    db.merge_scan(&[g], &[Source::Steam]).unwrap();
    let got = db.get(id).unwrap().unwrap();
    assert_eq!(got.meta.developer.as_deref(), Some("My Dev"));
    assert_eq!(got.source_meta.developer.as_deref(), Some("Source Dev"));
    assert_eq!(got.meta.genres, vec!["RPG".to_string()]);
    assert_eq!(got.sort_name, "aaa");
    assert_eq!(got.completion(), Some(CompletionStatus::Beaten));
    assert_eq!(got.links().len(), 5, "4 automatic Steam links + 1 own");
    db.set_edits(id, &UserEdits::default()).unwrap();
    assert_eq!(db.get(id).unwrap().unwrap().meta.developer.as_deref(), Some("Source Dev"));
}

#[test]
fn v2_database_upgrades_to_v3() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    {
        let mut db = Db::open(&path).unwrap();
        db.merge_scan(&[ScannedGame::new(Source::Steam, "1", "Old")], &[Source::Steam]).unwrap();
        db.conn
            .execute_batch(
                "DROP TRIGGER games_modified; ALTER TABLE games DROP COLUMN edits_json; PRAGMA user_version = 2;",
            )
            .unwrap();
    }
    let db = Db::open(&path).unwrap();
    let g = db.load_all().unwrap().remove(0);
    assert_eq!(g.name, "Old");
    assert!(g.edits.is_empty());
    db.set_edits(g.id, &UserEdits { notes: "hi".into(), ..Default::default() }).unwrap();
    assert_eq!(db.get(g.id).unwrap().unwrap().edits.notes, "hi");
}

#[test]
fn one_bad_edit_value_keeps_the_other_edits() {
    use crate::game_options::Tri;
    let mut db = Db::open_memory().unwrap();
    db.merge_scan(&[ScannedGame::new(Source::Steam, "1", "Alpha")], &[Source::Steam]).unwrap();
    let id = db.load_all().unwrap()[0].id;
    let raw = |json: &str| {
        db.conn.execute("UPDATE games SET edits_json = ?2 WHERE id = ?1", params![id, json]).unwrap();
        db.get(id).unwrap().unwrap()
    };
    // A newer build's enum value / an out-of-range number inside game_options.
    let g = raw(
        r#"{"sort_name":"Renamed","notes":"keep me","game_options":{"hdr":"on","gs_filter":"bicubic","gs_sharpness":300,"layers":{"VK_LAYER_a":"on","VK_LAYER_b":"sometimes"}}}"#,
    );
    assert_eq!(g.edits.notes, "keep me");
    assert_eq!(g.edits.sort_name.as_deref(), Some("Renamed"));
    let o = &g.edits.game_options;
    assert_eq!((o.hdr, o.gs_filter, o.gs_sharpness), (Tri::On, None, None));
    assert_eq!(o.layers.get("VK_LAYER_a"), Some(&Tri::On));
    assert!(!o.layers.contains_key("VK_LAYER_b"));
    // A bad value outside game_options drops only that field too.
    let g = raw(
        r#"{"notes":"still here","completion":"abandoned_forever","user_score":900,"game_options":{"mangohud":"on"}}"#,
    );
    assert_eq!(g.edits.notes, "still here");
    assert_eq!((g.edits.completion, g.edits.user_score), (None, None));
    assert_eq!(g.edits.game_options.mangohud, Tri::On);
    // One action with a newer build's kind drops only that action.
    let g = raw(
        r#"{"notes":"n","actions":[{"name":"Setup","path":"/a"},{"name":"New","kind":"from_the_future"},{"name":"Docs","path":"/c"}]}"#,
    );
    let names: Vec<&str> = g.edits.actions.iter().map(|a| a.name.as_str()).collect();
    assert_eq!((names, g.edits.notes.as_str()), (vec!["Setup", "Docs"], "n"));
    // Not JSON at all: nothing to keep.
    assert!(raw("{nope").edits.is_empty());
}

#[test]
fn a_hidden_logo_is_saved_with_the_placement() {
    let mut db = Db::open_memory().unwrap();
    db.merge_scan(&[ScannedGame::new(Source::Steam, "1", "Alpha")], &[Source::Steam]).unwrap();
    let id = db.load_all().unwrap()[0].id;
    let json = |db: &Db| -> String {
        db.conn.query_row("SELECT edits_json FROM games WHERE id = ?1", [id], |r| r.get(0)).unwrap()
    };
    // A row from before the choice existed: shown.
    db.conn
        .execute(
            "UPDATE games SET edits_json = ?2 WHERE id = ?1",
            params![id, r#"{"logo_placement":{"cx":0.3,"cy":0.6,"h":0.25}}"#],
        )
        .unwrap();
    let saved = db.get(id).unwrap().unwrap().edits;
    assert!(!saved.logo_hidden);
    db.set_edits(id, &UserEdits { logo_hidden: true, ..saved.clone() }).unwrap();
    let got = db.get(id).unwrap().unwrap().edits;
    assert!(got.logo_hidden);
    assert_eq!(got.logo_placement, saved.logo_placement);
    // Shown again: nothing is written for it.
    db.set_edits(id, &UserEdits { logo_hidden: false, ..got }).unwrap();
    assert!(!db.get(id).unwrap().unwrap().edits.logo_hidden);
    assert!(!json(&db).contains("logo_hidden"), "{}", json(&db));
}
