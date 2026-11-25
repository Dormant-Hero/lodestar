//! Rows of shortcuts that a scan now drops as duplicates (`Db::merge_scan_with_shortcuts`).
//! Before 0.17.2 a `.desktop` shortcut that starts a Faugus game was listed as a game of its own;
//! those rows must fold into the real game instead of staying behind as missing games.

use super::*;
use crate::library::user::{self, UserCollection};

fn sg(src: Source, id: &str, name: &str) -> ScannedGame {
    ScannedGame::new(src, id, name)
}

fn shortcut(id: &str, name: &str, into: (Source, &str)) -> ScannedGame {
    let mut g = sg(Source::Desktop, id, name);
    g.duplicate_of = Some((into.0, into.1.to_string()));
    g
}

fn row<'a>(all: &'a [Game], src: Source, sid: &str) -> Option<&'a Game> {
    all.iter().find(|g| g.source == src && g.source_id == sid)
}

/// What a scan hands the library: the scanned games with the duplicates dropped, and the
/// dropped shortcuts.
fn merge(db: &mut Db, mut games: Vec<ScannedGame>, ok: &[Source]) -> MergeStats {
    let dropped = crate::scan::dedupe(&mut games);
    db.merge_scan_with_shortcuts(&games, ok, &dropped).unwrap()
}

const BOTH: [Source; 2] = [Source::Faugus, Source::Desktop];

/// The library as the bug left it: the Faugus game and its shortcut, both as games.
fn bug_era(db: &mut Db) -> (GameId, GameId) {
    let scan = [
        sg(Source::Faugus, "fl-2026", "Football Life 2026"),
        sg(Source::Desktop, "fl-2026.desktop", "Football Life 2026"),
        sg(Source::Desktop, "runelite.desktop", "RuneLite"),
    ];
    db.merge_scan(&scan, &BOTH).unwrap();
    let all = db.load_all().unwrap();
    (row(&all, Source::Faugus, "fl-2026").unwrap().id, row(&all, Source::Desktop, "fl-2026.desktop").unwrap().id)
}

fn fixed_scan() -> Vec<ScannedGame> {
    vec![
        sg(Source::Faugus, "fl-2026", "Football Life 2026"),
        shortcut("fl-2026.desktop", "Football Life 2026", (Source::Faugus, "fl-2026")),
        sg(Source::Desktop, "runelite.desktop", "RuneLite"),
    ]
}

#[test]
fn shortcut_row_folds_into_the_game_it_starts() {
    let mut db = Db::open_memory().unwrap();
    let (game, dup) = bug_era(&mut db);
    db.set_play_stats(game, 3600, 3, Some(1_700_000_000)).unwrap();
    db.set_tags(game, &["sport".into()]).unwrap();
    // What the user did on the shortcut's row while it was listed.
    db.set_play_stats(dup, 600, 1, Some(1_700_500_000)).unwrap();
    db.set_favorite(dup, true).unwrap();
    db.set_tags(dup, &["sport".into(), "online".into()]).unwrap();
    db.set_rating(dup, Some(4)).unwrap();
    db.set_user_art_slot(dup, |a| a.hero = Some("/art/user/2/hero.jpg".into())).unwrap();
    db.set_edits(dup, &UserEdits { developer: Some("PES Patch Team".into()), ..Default::default() }).unwrap();
    user::save(&db, &[UserCollection { id: 1, name: "Football".into(), games: vec![dup], rule: None }]);

    let s = merge(&mut db, fixed_scan(), &BOTH);
    assert_eq!((s.removed, s.missing), (1, 0), "{s:?}");
    let all = db.load_all().unwrap();
    assert!(row(&all, Source::Desktop, "fl-2026.desktop").is_none(), "no missing copy left");
    let g = row(&all, Source::Faugus, "fl-2026").unwrap();
    assert_eq!(g.id, game);
    assert_eq!((g.playtime_secs, g.play_count, g.last_played), (4200, 4, Some(1_700_500_000)));
    assert!(g.favorite);
    assert_eq!(g.tags, ["sport", "online"]);
    assert_eq!(g.user_rating, Some(4));
    assert_eq!(g.art.hero.as_deref(), Some(std::path::Path::new("/art/user/2/hero.jpg")));
    assert_eq!(g.meta.developer.as_deref(), Some("PES Patch Team"));
    assert_eq!(user::load(&db)[0].games, [game], "the collection follows");
    assert!(row(&all, Source::Desktop, "runelite.desktop").is_some_and(|g| g.missing.is_none()));

    // The next scan finds nothing left to fold.
    let s = merge(&mut db, fixed_scan(), &BOTH);
    assert_eq!((s.removed, s.missing), (0, 0));
    assert_eq!(db.load_all().unwrap().len(), 2);
}

/// The 0.17.2 scan already marked the shortcut's row missing: the following scan folds it.
#[test]
fn shortcut_row_already_marked_missing_folds_too() {
    let mut db = Db::open_memory().unwrap();
    let (game, dup) = bug_era(&mut db);
    db.set_user_art_slot(dup, |a| a.logo = Some("/art/user/2/logo.png".into())).unwrap();
    // A scan that drops the shortcut but doesn't fold it (as 0.17.2 did).
    let mut games = fixed_scan();
    crate::scan::dedupe(&mut games);
    db.merge_scan(&games, &BOTH).unwrap();
    assert!(db.get(dup).unwrap().unwrap().missing.is_some());

    merge(&mut db, fixed_scan(), &BOTH);
    assert!(db.get(dup).unwrap().is_none());
    let g = db.get(game).unwrap().unwrap();
    assert_eq!(g.art.logo.as_deref(), Some(std::path::Path::new("/art/user/2/logo.png")));
}

/// Hiding the extra copy was how the user tidied the list: the real game stays visible.
#[test]
fn hiding_the_shortcut_does_not_hide_the_game() {
    let mut db = Db::open_memory().unwrap();
    let (game, dup) = bug_era(&mut db);
    db.set_hidden(dup, true).unwrap();
    merge(&mut db, fixed_scan(), &BOTH);
    assert!(db.get(dup).unwrap().is_none());
    assert!(!db.get(game).unwrap().unwrap().hidden);
}

/// A desktop game whose `.desktop` file is really gone stays a missing game (the user decides in
/// Settings → Libraries → Missing games), even when it was a shortcut once.
#[test]
fn shortcut_whose_file_is_gone_stays_missing() {
    let mut db = Db::open_memory().unwrap();
    let (game, dup) = bug_era(&mut db);
    db.set_favorite(dup, true).unwrap();
    let s = merge(
        &mut db,
        vec![sg(Source::Faugus, "fl-2026", "Football Life 2026"), sg(Source::Desktop, "runelite.desktop", "RuneLite")],
        &BOTH,
    );
    assert_eq!((s.removed, s.missing), (0, 1));
    let g = db.get(dup).unwrap().unwrap();
    assert!(g.missing.is_some() && g.favorite);
    assert!(!db.get(game).unwrap().unwrap().favorite, "nothing moved");
}

/// The game's library failed to scan: its old rows stay as they were and nothing is matched
/// against them, so the shortcut's row isn't folded.
#[test]
fn shortcut_into_a_library_that_failed_is_left_alone() {
    let mut db = Db::open_memory().unwrap();
    let (game, dup) = bug_era(&mut db);
    db.set_favorite(dup, true).unwrap();
    let dropped = [DroppedShortcut {
        shortcut: (Source::Desktop, "fl-2026.desktop".into()),
        target: (Source::Faugus, "fl-2026".into()),
    }];
    let games = [sg(Source::Desktop, "runelite.desktop", "RuneLite")];
    db.merge_scan_with_shortcuts(&games, &[Source::Desktop], &dropped).unwrap();
    assert!(db.get(dup).unwrap().is_some_and(|g| g.favorite));
    assert!(!db.get(game).unwrap().unwrap().favorite);
}

/// Heroic shortcuts name the app; the game is stored as an Epic (GOG, Amazon) game.
#[test]
fn heroic_shortcut_folds_into_the_epic_game() {
    let mut db = Db::open_memory().unwrap();
    let stores = [Source::Epic, Source::Gog, Source::Amazon, Source::Heroic, Source::Desktop];
    db.merge_scan(&[sg(Source::Epic, "Hades", "Hades"), sg(Source::Desktop, "hades.desktop", "Hades")], &stores)
        .unwrap();
    let all = db.load_all().unwrap();
    let (epic, dup) =
        (row(&all, Source::Epic, "Hades").unwrap().id, row(&all, Source::Desktop, "hades.desktop").unwrap().id);
    db.set_favorite(dup, true).unwrap();

    merge(
        &mut db,
        vec![sg(Source::Epic, "Hades", "Hades"), shortcut("hades.desktop", "Hades", (Source::Heroic, "Hades"))],
        &stores,
    );
    assert!(db.get(dup).unwrap().is_none());
    assert!(db.get(epic).unwrap().unwrap().favorite);
}

/// A shortcut into a game its library knows only as not installed (left out of the scan): the
/// shortcut's row still folds into that game's row.
#[test]
fn shortcut_into_an_uninstalled_game_folds_into_its_row() {
    let mut db = Db::open_memory().unwrap();
    let stores = [Source::Epic, Source::Gog, Source::Amazon, Source::Heroic, Source::Desktop];
    db.merge_scan(
        &[
            sg(Source::Epic, "Hades", "Hades"),
            sg(Source::Epic, "Sugar", "Rocket League"),
            sg(Source::Desktop, "hades.desktop", "Hades"),
        ],
        &stores,
    )
    .unwrap();
    let all = db.load_all().unwrap();
    let (epic, dup) =
        (row(&all, Source::Epic, "Hades").unwrap().id, row(&all, Source::Desktop, "hades.desktop").unwrap().id);
    db.add_playtime(dup, 90).unwrap();
    // Hades was uninstalled: Heroic no longer lists it, the scan drops its shortcut as stale.
    let dropped = [DroppedShortcut {
        shortcut: (Source::Desktop, "hades.desktop".into()),
        target: (Source::Heroic, "Hades".into()),
    }];
    db.merge_scan_with_shortcuts(&[sg(Source::Epic, "Sugar", "Rocket League")], &stores, &dropped).unwrap();
    assert!(db.get(dup).unwrap().is_none());
    let g = db.get(epic).unwrap().unwrap();
    assert_eq!(g.playtime_secs, 90);
    assert!(!g.installed && g.missing.is_none());
}
