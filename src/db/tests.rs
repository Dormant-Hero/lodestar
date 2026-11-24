use super::*;

fn sg(src: Source, id: &str, name: &str) -> ScannedGame {
    ScannedGame::new(src, id, name)
}

#[test]
fn merge_preserves_user_data() {
    let mut db = Db::open_memory().unwrap();
    let s = db
        .merge_scan(
            &[sg(Source::Steam, "1", "Alpha"), sg(Source::Desktop, "d1", "Beta")],
            &[Source::Steam, Source::Desktop],
        )
        .unwrap();
    assert_eq!(s.added, 2);
    let games = db.load_all().unwrap();
    let alpha = games.iter().find(|g| g.name == "Alpha").unwrap().id;
    db.set_favorite(alpha, true).unwrap();
    db.set_user_name(alpha, Some("Alpha Custom")).unwrap();
    // Rescan: alpha renamed upstream, beta gone.
    let mut a2 = sg(Source::Steam, "1", "Alpha 2");
    a2.playtime_secs = Some(100);
    let s = db.merge_scan(&[a2, sg(Source::Desktop, "d2", "Gamma")], &[Source::Steam, Source::Desktop]).unwrap();
    assert_eq!(s.updated, 1);
    assert_eq!(s.removed, 1);
    let g = db.get(alpha).unwrap().unwrap();
    assert!(g.favorite);
    assert_eq!(g.display_name(), "Alpha Custom");
    assert_eq!(g.name, "Alpha 2");
    assert_eq!(g.playtime_secs, 100);
    // Steam game disappears -> uninstalled, not deleted.
    let s = db.merge_scan(&[], &[Source::Steam]).unwrap();
    assert_eq!(s.uninstalled, 1);
    assert!(!db.get(alpha).unwrap().unwrap().installed);
}

#[test]
fn heroic_rows_move_to_their_store_source() {
    let mut db = Db::open_memory().unwrap();
    db.merge_scan(
        &[
            sg(Source::Heroic, "Fortnite", "Fortnite"),
            sg(Source::Heroic, "1207658924", "Rayman"),
            sg(Source::Heroic, "side1", "Side"),
        ],
        &[Source::Heroic],
    )
    .unwrap();
    let rayman = db.load_all().unwrap().into_iter().find(|g| g.name == "Rayman").unwrap().id;
    db.set_favorite(rayman, true).unwrap();
    db.add_playtime(rayman, 600).unwrap();
    // The new Heroic scanner reports per-store sources with the same ids.
    let s = db
        .merge_scan(
            &[
                sg(Source::Epic, "Fortnite", "Fortnite"),
                sg(Source::Gog, "1207658924", "Rayman"),
                sg(Source::Heroic, "side1", "Side"),
            ],
            &[Source::Heroic, Source::Epic, Source::Gog, Source::Amazon],
        )
        .unwrap();
    assert_eq!(s.added, 0);
    let all = db.load_all().unwrap();
    assert_eq!(all.len(), 3, "no duplicates: {all:?}");
    let r = db.get(rayman).unwrap().unwrap();
    assert_eq!(r.source, Source::Gog);
    assert!(r.favorite && r.installed);
    assert_eq!(r.playtime_secs, 600);
    assert!(all.iter().any(|g| g.source == Source::Epic && g.source_id == "Fortnite"));
    assert!(all.iter().any(|g| g.source == Source::Heroic && g.source_id == "side1" && g.installed));
}

#[test]
fn failed_source_untouched() {
    let mut db = Db::open_memory().unwrap();
    db.merge_scan(&[sg(Source::Desktop, "d1", "Beta")], &[Source::Desktop]).unwrap();
    db.merge_scan(&[], &[Source::Steam]).unwrap();
    assert_eq!(db.load_all().unwrap().len(), 1);
}

#[test]
fn art_layers() {
    let mut db = Db::open_memory().unwrap();
    let mut g = sg(Source::Steam, "1", "A");
    g.art.cover = Some("/scan/cover.jpg".into());
    db.merge_scan(&[g], &[Source::Steam]).unwrap();
    let id = db.load_all().unwrap()[0].id;
    let fetched = Art { cover: Some("/f/c.jpg".into()), hero: Some("/f/h.jpg".into()), ..Default::default() };
    db.set_fetched(id, &fetched, &Metadata::default()).unwrap();
    let g = db.get(id).unwrap().unwrap();
    assert_eq!(g.art.cover.as_deref(), Some(Path::new("/scan/cover.jpg")));
    assert_eq!(g.art.hero.as_deref(), Some(Path::new("/f/h.jpg")));
    db.set_user_art(id, &Art { cover: Some("/u/c.png".into()), ..Default::default() }).unwrap();
    assert_eq!(db.get(id).unwrap().unwrap().art.cover.as_deref(), Some(Path::new("/u/c.png")));
}

#[test]
fn fetched_wikipedia_text_wins_over_the_launchers() {
    let mut db = Db::open_memory().unwrap();
    let mut g = sg(Source::Heroic, "h1", "Doom");
    g.meta.description = Some("Launcher text".into());
    db.merge_scan(&[g], &[Source::Heroic]).unwrap();
    let id = db.load_all().unwrap()[0].id;
    // A store description fetched later doesn't replace the launcher's.
    db.set_fetched(id, &Art::default(), &Metadata { description: Some("Store".into()), ..Default::default() }).unwrap();
    assert_eq!(db.get(id).unwrap().unwrap().meta.description.as_deref(), Some("Launcher text"));
    // Wikipedia's (fetched only when preferred) does, with its credit.
    let wiki = Metadata {
        description: Some("Wiki text".into()),
        wikipedia: Some("https://en.wikipedia.org/wiki/Doom".into()),
        ..Default::default()
    };
    db.set_fetched(id, &Art::default(), &wiki).unwrap();
    let g = db.get(id).unwrap().unwrap();
    assert_eq!((g.meta.description.as_deref(), g.meta.wikipedia.is_some()), (Some("Wiki text"), true));
}

#[test]
fn launch_and_playtime() {
    let mut db = Db::open_memory().unwrap();
    db.merge_scan(&[sg(Source::Faugus, "x", "X")], &[Source::Faugus]).unwrap();
    let id = db.load_all().unwrap()[0].id;
    db.record_launch(id, 1000).unwrap();
    db.add_playtime(id, 60).unwrap();
    let g = db.get(id).unwrap().unwrap();
    assert_eq!((g.play_count, g.last_played, g.playtime_secs), (1, Some(1000), 60));
    let o = LaunchOverride { options: "gamemoderun %command%".into(), ..Default::default() };
    db.set_launch_override(id, Some(&o)).unwrap();
    assert_eq!(db.get(id).unwrap().unwrap().launch_override, Some(o));
    db.set_launch_override(id, Some(&LaunchOverride::default())).unwrap();
    assert_eq!(db.get(id).unwrap().unwrap().launch_override, None);
    // Faugus game with playtime is kept as uninstalled when it disappears.
    db.merge_scan(&[], &[Source::Faugus]).unwrap();
    assert!(!db.get(id).unwrap().unwrap().installed);
}
