use super::*;
use crate::library::UserCollection;
use crate::model::{Art, EmuRef, Source};
use crate::theme::render::{Binder, CollectionItem};

fn coll(kind: CollectionKind) -> CollectionItem {
    CollectionItem {
        collection: Collection { kind, name: "x".into() },
        name: "x".into(),
        count: 3,
        art: Art::default(),
    }
}

/// Rasterise like the image loader does (resvg, longest side `px`); returns the pixels.
fn rasterise(svg: &str, px: u32) -> resvg::tiny_skia::Pixmap {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_data(svg.as_bytes(), &usvg::Options::default()).expect("parses");
    let s = tree.size();
    let scale = px as f32 / s.width().max(s.height());
    let (w, h) = ((s.width() * scale).floor() as u32, (s.height() * scale).floor() as u32);
    let mut pm = tiny_skia::Pixmap::new(w.max(1), h.max(1)).unwrap();
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pm.as_mut());
    pm
}

#[test]
fn every_console_and_collection_has_all_its_pictures() {
    for p in crate::emulation::platforms::all() {
        for k in ArtKind::ALL {
            assert!(has(p.id, k), "{} has no {}", p.id, k.name());
        }
    }
    for s in Source::ALL {
        assert!(has(s.key(), ArtKind::Card), "store {} has no art", s.key());
    }
    for kind in [
        CollectionKind::All,
        CollectionKind::Installed,
        CollectionKind::Controller,
        CollectionKind::Pc,
        CollectionKind::Favorites,
        CollectionKind::Recent,
        CollectionKind::Source(Source::Steam),
        CollectionKind::Platform("nintendo_gamecube".into()),
    ] {
        let c = Collection { kind, name: String::new() };
        let key = key_for(&c).unwrap_or_else(|| panic!("{c:?} has no art"));
        for k in ArtKind::ALL {
            assert!(has(key, k), "{key} {}", k.name());
        }
    }
    // The user's own collections and tags show their names instead.
    assert_eq!(key_for(&Collection { kind: CollectionKind::Tag("RPG".into()), name: String::new() }), None);
    assert_eq!(
        key_for(&Collection { kind: CollectionKind::User(UserCollection::default()), name: String::new() }),
        None
    );
    assert_eq!(key_for(&Collection { kind: CollectionKind::Platform("nope".into()), name: String::new() }), None);
    assert!(keys().len() >= crate::emulation::platforms::all().len() + 6);
}

#[test]
fn every_picture_renders_at_two_sizes() {
    for key in keys() {
        for k in ArtKind::ALL {
            let svg = svg(key, k).unwrap_or_else(|| panic!("{key} {}", k.name()));
            for px in [64, 360] {
                let pm = rasterise(&svg, px);
                let lit = pm.pixels().iter().filter(|p| p.alpha() > 0).count();
                let all = pm.pixels().len();
                // A wordmark is thin type, a device (a slim tower) may be narrow; cards and
                // backgrounds fill their frame.
                let share = match k {
                    ArtKind::Mark => 25,
                    ArtKind::Device => 12,
                    ArtKind::Card | ArtKind::Hero => 2,
                };
                assert!(lit * share > all, "{key} {} at {px}px: only {lit} of {all} pixels drawn", k.name());
                if matches!(k, ArtKind::Card | ArtKind::Hero) {
                    // Full-bleed backgrounds: no transparent corners.
                    assert_eq!(pm.pixels()[0].alpha(), 255, "{key} {}", k.name());
                    assert_eq!(pm.pixels()[all - 1].alpha(), 255, "{key} {}", k.name());
                }
            }
        }
    }
}

#[test]
fn shapes_are_as_documented() {
    let size = |key: &str, k: ArtKind| {
        let tree = resvg::usvg::Tree::from_data(svg(key, k).unwrap().as_bytes(), &Default::default()).unwrap();
        (tree.size().width(), tree.size().height())
    };
    assert_eq!(size("sony_playstation2", ArtKind::Card), (600.0, 800.0));
    assert_eq!(size("sony_playstation2", ArtKind::Hero), (1920.0, 1080.0));
    assert_eq!(size("sony_playstation2", ArtKind::Device), (800.0, 600.0));
    let (w, h) = size("sony_playstation2", ArtKind::Mark);
    assert!(w > 2.0 * h, "a wordmark is wide: {w}x{h}");
}

#[test]
fn the_embedded_set_matches_the_generated_files() {
    // tools/gen-console-art.py writes both; a stale console-art.bin would ship old pictures.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/consoles");
    for key in keys() {
        for k in ArtKind::ALL {
            let on_disk = std::fs::read_to_string(dir.join(file_name(key, k))).unwrap_or_default();
            assert!(on_disk == svg(key, k).unwrap(), "{} differs: run tools/gen-console-art.py", file_name(key, k));
        }
    }
}

#[test]
fn pictures_are_written_once_and_found_by_path() {
    let p = path("nintendo_snes_does_not_exist", ArtKind::Card);
    assert!(p.is_none());
    let p = path("sega_saturn", ArtKind::Hero).expect("a path");
    assert!(p.is_file(), "{}", p.display());
    assert!(p.ends_with("hero/sega_saturn.svg"));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), svg("sega_saturn", ArtKind::Hero).unwrap());
    assert!(p.parent().unwrap().parent().unwrap().join(MARKER).is_file());
}

#[test]
fn bindings_resolve_to_lodestars_art() {
    let ps2 = coll(CollectionKind::Platform("sony_playstation2".into()));
    let b = Binder { collection: Some(&ps2), ..Default::default() };
    for kind in ["card", "hero", "mark", "device"] {
        let v = b.resolve(&format!("{{collection.art.{kind}}}"), false).into_owned();
        assert!(v.ends_with(&format!("{kind}/sony_playstation2.svg")), "{v}");
        assert!(Path::new(&v).is_file(), "{v}");
    }
    assert_eq!(b.resolve("{collection.art.nope}", false), "");
    let all = coll(CollectionKind::All);
    let b = Binder { collection: Some(&all), ..Default::default() };
    assert!(b.resolve("{collection.art.card}", false).ends_with("card/all.svg"));
    // No art for the user's own collections: "" (themes show the name).
    let tag = coll(CollectionKind::Tag("RPG".into()));
    let b = Binder { collection: Some(&tag), ..Default::default() };
    assert_eq!(b.resolve("{collection.art.card}", false), "");
    // A game's console: its emulated platform, else the platform set on it, else its store.
    let mut g = Game::blank("x");
    g.source = Source::Steam;
    let b = |g: &Game| Binder::default().with_game(Some(g)).resolve("{game.consoleart.mark}", false).into_owned();
    assert!(b(&g).ends_with("mark/steam.svg"), "{}", b(&g));
    g.platform = Some("windows".into());
    assert!(b(&g).ends_with("mark/pc_windows.svg"));
    g.emu = Some(EmuRef { platform: "nintendo_64".into(), ..Default::default() });
    assert!(b(&g).ends_with("mark/nintendo_64.svg"));
    assert_eq!(Binder::default().resolve("{game.consoleart.card}", false), "");
}

#[test]
fn a_themes_own_picture_wins_and_falls_back_to_lodestars() {
    let t = crate::theme::ThemeFile::parse(
        r#"
[collection_art.sony_playstation2]
card = "images/ps2.png"
[collection_art.all]
hero = "/abs/all-wide.jpg"
[collection_art."platform:nintendo_64"]
mark = "n64.svg"
[collection_art."tag:RPG"]
card = "rpg.png"
"#,
    )
    .unwrap();
    assert_eq!(t.collection_art.len(), 4);
    let again = crate::theme::ThemeFile::parse(&t.to_toml().unwrap()).unwrap();
    assert_eq!(again.collection_art, t.collection_art);
    let map = Some(&t.collection_art);
    let ps2 = coll(CollectionKind::Platform("sony_playstation2".into()));
    let b = Binder { collection: Some(&ps2), collection_art: map, ..Default::default() };
    assert_eq!(b.resolve("{collection.art.card}", false), "images/ps2.png");
    // Not set by the theme: Lodestar's.
    assert!(b.resolve("{collection.art.hero}", false).ends_with("hero/sony_playstation2.svg"));
    // A picture path tries the theme's file first, then Lodestar's (a missing file falls back).
    let mut v = vec![];
    b.path_candidates("{collection.art.card}", &mut v);
    assert_eq!(v.len(), 2, "{v:?}");
    assert_eq!(v[0], "images/ps2.png");
    assert!(v[1].ends_with("card/sony_playstation2.svg"));
    let all = coll(CollectionKind::All);
    let b = Binder { collection: Some(&all), collection_art: map, ..Default::default() };
    assert_eq!(b.resolve("{collection.art.hero}", false), "/abs/all-wide.jpg");
    // Keyed by the collection key too, and for games by their console.
    let mut g = Game::blank("x");
    g.emu = Some(EmuRef { platform: "nintendo_64".into(), ..Default::default() });
    let b = Binder { game: Some(&g), collection_art: map, ..Default::default() };
    assert_eq!(b.resolve("{game.consoleart.mark}", false), "n64.svg");
    // The user's own collections and tags get pictures only from the theme.
    let tag = coll(CollectionKind::Tag("RPG".into()));
    let b = Binder { collection: Some(&tag), collection_art: map, ..Default::default() };
    assert_eq!(b.resolve("{collection.art.card}", false), "rpg.png");
    assert_eq!(b.resolve("{collection.art.hero}", false), "");
}
