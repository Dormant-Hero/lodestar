use super::doc::Doc;
use super::files::{self, Asset, Template};
use super::geom::{self, Handle, Snap};
use super::ops::{self, ElPath, Kind, Layer};
use super::validate::{self, Level};
use crate::emulation::zipread::tests::write_zip;
use crate::theme::*;
use egui::{Rect, pos2, vec2};

fn themes_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn every_builtin_theme_duplicated_and_saved_reads_back_the_same() {
    let dir = themes_dir();
    for (key, src) in builtin::SOURCES {
        let k = files::create(dir.path(), &Template::Builtin(key.into()), &format!("Copy of {key}"), "Me").unwrap();
        let doc = Doc::open(&k, dir.path().join(&k)).unwrap();
        // The copy is the built-in with new details.
        let mut expect = ThemeFile::parse(src).unwrap();
        expect.theme = doc.file.theme.clone();
        assert_eq!(doc.file, expect, "{key}");
        assert_eq!(doc.file.theme.name, format!("Copy of {key}"));
        assert_eq!(doc.file.theme.author, "Me");
        // Opening and saving without changes: the same theme.
        let text = doc.text().unwrap();
        files::save(&doc.dir, &text).unwrap();
        let again = Doc::open(&k, dir.path().join(&k)).unwrap();
        assert_eq!(again.file, doc.file, "{key}");
        assert!(!again.dirty());
    }
    // Built-ins are never changed (they are embedded): a second copy is a new folder.
    let a = files::create(dir.path(), &Template::Builtin("Lodestar".into()), "Same", "").unwrap();
    let b = files::create(dir.path(), &Template::Builtin("Lodestar".into()), "Same", "").unwrap();
    assert_ne!(a, b);
    // A name like a built-in's gets another folder name.
    assert_ne!(files::free_key(dir.path(), "Lodestar"), "Lodestar");
}

#[test]
fn a_user_theme_is_copied_with_its_files_but_not_its_download_record() {
    let dir = themes_dir();
    let src = dir.path().join("orig");
    std::fs::create_dir_all(src.join("pictures")).unwrap();
    std::fs::write(src.join("theme.toml"), "[theme]\nname = \"Orig\"\n[games]\n").unwrap();
    std::fs::write(src.join("pictures/bg.png"), b"png").unwrap();
    std::fs::write(src.join(crate::theme::catalog::RECORD), "index = 1").unwrap();
    std::fs::write(src.join("playnite-options.toml"), "x").unwrap();
    let k = files::create(dir.path(), &Template::Folder(src.clone()), "Mine", "").unwrap();
    let new = dir.path().join(&k);
    assert!(new.join("pictures/bg.png").is_file());
    assert!(!new.join(crate::theme::catalog::RECORD).exists());
    assert!(!new.join("playnite-options.toml").exists());
    let t = ThemeFile::parse(&std::fs::read_to_string(new.join("theme.toml")).unwrap()).unwrap();
    assert_eq!((t.theme.name.as_str(), t.theme.origin.as_str()), ("Mine", "lodestar"));
    // The original is untouched.
    assert!(std::fs::read_to_string(src.join("theme.toml")).unwrap().contains("Orig"));
}

#[test]
fn undo_and_redo_step_through_changes_and_typing_is_one_step() {
    let mut d = Doc::new("t".into(), "/tmp/nowhere".into(), "[theme]\nname = \"A\"\n").unwrap();
    assert!(!d.change("nothing", |_| {}));
    assert_eq!(d.undo_label(), None);
    d.change("Rename", |f| f.theme.name = "B".into());
    // Typing in one field: several changes, one step.
    for s in ["C", "Co", "Col"] {
        d.change_merged("field", "Type", |f| f.theme.author = s.into());
    }
    assert_eq!(d.file.theme.author, "Col");
    assert!(d.dirty());
    assert!(d.undo());
    assert_eq!((d.file.theme.name.as_str(), d.file.theme.author.as_str()), ("B", ""));
    assert!(d.undo());
    assert_eq!(d.file.theme.name, "A");
    assert!(!d.undo());
    assert!(!d.dirty());
    assert!(d.redo());
    assert!(d.redo());
    assert_eq!(d.file.theme.author, "Col");
    assert!(!d.redo());
    // A new change after an undo drops the redo steps.
    d.undo();
    d.change("Other", |f| f.theme.version = "2".into());
    assert!(!d.redo());
    // After `break_merge` the same tag starts a new step (a second drag).
    d.change_merged("drag", "Move", |f| f.theme.aspect = 1.0);
    d.break_merge();
    d.change_merged("drag", "Move", |f| f.theme.aspect = 2.0);
    d.undo();
    assert_eq!(d.file.theme.aspect, 1.0);
}

#[test]
fn dragging_snaps_edges_and_centres_and_shows_guides() {
    let start = Rect::from_min_size(pos2(0.1, 0.1), vec2(0.2, 0.1));
    let snap = Snap::new(&[Rect::from_min_size(pos2(0.6, 0.6), vec2(0.1, 0.1))], vec2(0.01, 0.01));
    // Moved so its centre lands 0.004 from the screen centre: it snaps there.
    let (r, g) = geom::drag(start, Handle::Move, vec2(0.304, 0.0), Some(&snap));
    assert!((r.center().x - 0.5).abs() < 1e-6, "{r:?}");
    assert_eq!(g.xs, vec![0.5]);
    // Free: exactly where the pointer put it.
    let (r, g) = geom::drag(start, Handle::Move, vec2(0.304, 0.0), None);
    assert!((r.center().x - 0.504).abs() < 1e-6);
    assert!(g.xs.is_empty());
    // Left edge to another element's left edge.
    let (r, g) = geom::drag(start, Handle::Move, vec2(0.495, 0.0), Some(&snap));
    assert!((r.left() - 0.6).abs() < 1e-6, "{r:?}");
    assert_eq!(g.xs, vec![0.6]);
    // Resizing moves only the dragged edges, snaps them, and never turns the box inside out.
    let (r, _) = geom::drag(start, Handle::SE, vec2(0.1, 0.05), None);
    assert_eq!(r.min, start.min);
    assert!((r.max.x - 0.4).abs() < 1e-6 && (r.max.y - 0.25).abs() < 1e-6, "{r:?}");
    let (r, g) = geom::drag(start, Handle::E, vec2(0.198, 0.0), Some(&snap));
    assert!((r.right() - 0.5).abs() < 1e-6, "{r:?}");
    assert_eq!(g.xs, vec![0.5]);
    let (r, _) = geom::drag(start, Handle::W, vec2(0.5, 0.0), None);
    assert!(r.width() >= geom::MIN_SIZE - 1e-6 && r.right() == start.right());
    let (r, _) = geom::drag(start, Handle::N, vec2(0.0, -0.05), None);
    assert!((r.top() - 0.05).abs() < 1e-6 && r.bottom() == start.bottom());
}

#[test]
fn boxes_follow_the_anchor_and_automatic_sizes() {
    let mut c = Common { pos: [0.5, 0.5], size: [0.2, 0.1], origin: [0.5, 0.5], ..Default::default() };
    let b = geom::element_box(&c, vec2(0.0, 0.0), true);
    assert!((b.min.x - 0.4).abs() < 1e-6 && (b.min.y - 0.45).abs() < 1e-6);
    // Write a moved box back: the anchor point lands at the new position.
    geom::apply_box(&mut c, Rect::from_min_size(pos2(0.1, 0.2), vec2(0.3, 0.2)), [true, true]);
    assert_eq!(c.pos, [0.25, 0.3]);
    assert_eq!(c.size, [0.3, 0.2]);
    // An automatic height stays automatic when only moved; a picture keeps its aspect.
    let mut t = Common { pos: [0.0, 0.0], size: [0.4, 0.0], ..Default::default() };
    let b = geom::element_box(&t, vec2(0.2, 0.1), true);
    assert!((b.height() - 0.2).abs() < 1e-6);
    geom::apply_box(&mut t, b.translate(vec2(0.1, 0.0)), [false, false]);
    assert_eq!(t.size, [0.4, 0.0]);
    assert_eq!(t.pos, [0.1, 0.0]);
    // Resizing that side makes it fixed.
    geom::apply_box(&mut t, Rect::from_min_size(pos2(0.1, 0.0), vec2(0.4, 0.3)), [false, true]);
    assert_eq!(t.size, [0.4, 0.3]);
    // Values are tidy.
    geom::nudge(&mut t, vec2(0.0010001, 0.0));
    assert_eq!(t.pos[0], 0.101);
}

#[test]
fn every_kind_of_element_starts_valid_and_survives_a_save() {
    let pal = Palette::default();
    let mut f = ThemeFile::default();
    for (_, kinds) in Kind::GROUPS {
        for k in kinds.iter().copied() {
            for view in [ViewKind::Collections, ViewKind::Games] {
                let v = if view == ViewKind::Games { &mut f.games } else { &mut f.collections };
                let name = ops::unique_name(&v.elements, k.base_name());
                v.elements.push(ops::new_element(k, view, &pal, name));
            }
        }
    }
    let text = f.to_theme_toml(None).unwrap();
    assert_eq!(ThemeFile::parse(&text).unwrap(), f);
    // Names are unique, also counting the parts inside stacks.
    let mut names = std::collections::HashSet::new();
    for e in &f.games.elements {
        e.for_each_nested(&mut |x| assert!(names.insert(x.common().name.clone()), "{}", x.common().name));
    }
    // A fresh theme with a list on both screens has no problems.
    let found = validate::check(&f, None);
    assert!(found.iter().all(|x| x.level != Level::Problem), "{found:?}");
}

#[test]
fn reordering_changes_only_the_moved_element() {
    let el = |name: &str, z: f32| Element::Rect {
        common: Common { name: name.into(), z, ..Default::default() },
        color: Color::BLACK,
        corner_radius: 0.0,
        border_color: None,
        border_width: 0.0,
        gradient: None,
        fill: None,
        track: None,
    };
    let names =
        |els: &[Element]| ops::draw_order(els).into_iter().map(|i| els[i].common().name.clone()).collect::<Vec<_>>();
    let mut els = vec![el("bg", 0.0), el("a", 1.0), el("b", 1.0), el("top", 5.0)];
    assert_eq!(names(&els), ["bg", "a", "b", "top"]);
    // "top" to the back.
    let at = ops::move_in_draw_order(&mut els, 3, 0);
    assert_eq!(names(&els), ["top", "bg", "a", "b"]);
    assert_eq!(els[at].common().name, "top");
    // "bg" (now second) to the front.
    let i = els.iter().position(|e| e.common().name == "bg").unwrap();
    ops::move_in_draw_order(&mut els, i, 3);
    assert_eq!(names(&els), ["top", "a", "b", "bg"]);
    // Between two of the same z.
    let i = els.iter().position(|e| e.common().name == "bg").unwrap();
    ops::move_in_draw_order(&mut els, i, 2);
    assert_eq!(names(&els), ["top", "a", "bg", "b"]);
    // The others kept their z.
    assert_eq!(els.iter().find(|e| e.common().name == "b").unwrap().common().z, 1.0);
}

#[test]
fn duplicate_delete_and_nested_parts() {
    let pal = Palette::default();
    let mut els = vec![ops::new_element(Kind::Stack, ViewKind::Games, &pal, "info".into())];
    let p = ElPath::top(0).child(ops::Nest::Children, 0);
    assert!(ops::get(&els, &p).is_some());
    let np = ops::duplicate(&mut els, &p).unwrap();
    assert_eq!(np, ElPath::top(0).child(ops::Nest::Children, 1));
    assert_eq!(ops::get(&els, &np).unwrap().common().name, "info_title_copy");
    let moved = ops::shift_nested(&mut els, &np, 1).unwrap();
    assert_eq!(moved.sub[0].1, 2);
    assert!(ops::delete(&mut els, &moved).is_some());
    let Element::Stack { children, .. } = &els[0] else { panic!() };
    assert_eq!(children.len(), 2);
    let top = ops::duplicate(&mut els, &ElPath::top(0)).unwrap();
    assert_eq!(top, ElPath::top(1));
    assert_eq!(els[1].common().pos, [0.07, 0.32]);
}

#[test]
fn changes_for_a_variant_shape_or_console_are_stored_as_small_patches() {
    let src = r##"
[theme]
name = "L"
[games]
[[games.elements]]
type = "text"
name = "title"
text = "{game.name}"
pos = [0.1, 0.1]
style = { font_size = 0.05 }
[[games.elements]]
type = "grid"
pos = [0.0, 0.3]
size = [1.0, 0.7]
"##;
    let mut f = ThemeFile::parse(src).unwrap();
    let base = f.games.clone();
    let layer = Layer::Variant("Big".into());
    let lost = ops::edit_in_layer(&mut f, &layer, ViewKind::Games, &ElPath::top(0), |e| {
        e.common_mut().pos[1] = 0.2;
        if let Element::Text { style, .. } = e {
            style.font_size = 0.08;
        }
    })
    .unwrap();
    assert!(lost.is_empty());
    // The theme itself didn't change; the variant has only what changed.
    assert_eq!(f.games, base);
    let patch = &f.variants["Big"].games["title"];
    assert_eq!(patch.len(), 2, "{patch:?}");
    assert_eq!(patch["style"]["font_size"].as_float().map(|v| v as f32), Some(0.08));
    // The renderer shows it with the variant.
    let p = crate::theme::render::prepare_view_for(&f, ViewKind::Games, None, Some("Big"), None).unwrap();
    assert_eq!(p.view.elements[0].common().pos, [0.1, 0.2]);
    // Editing again builds on the variant's version.
    ops::edit_in_layer(&mut f, &layer, ViewKind::Games, &ElPath::top(0), |e| e.common_mut().opacity = 0.5).unwrap();
    assert_eq!(f.variants["Big"].games["title"].len(), 3);
    // An element without a name is named for its patch.
    ops::edit_in_layer(&mut f, &Layer::Aspect("4:3".into()), ViewKind::Games, &ElPath::top(1), |e| {
        e.common_mut().size[1] = 0.6
    })
    .unwrap();
    assert_eq!(f.games.elements[1].common().name, "grid");
    assert!(f.aspects["4:3"].games.contains_key("grid"));
    // Taking a value away can't be a patch: it is reported.
    let lost = ops::edit_in_layer(&mut f, &Layer::Console("steam".into()), ViewKind::Games, &ElPath::top(0), |e| {
        if let Element::Text { style, .. } = e {
            style.shadow = None;
        }
        e.common_mut().visible_if = None;
    })
    .unwrap();
    assert!(lost.is_empty(), "nothing to take away here: {lost:?}");
    // Back to the theme's own.
    assert!(ops::reset_in_layer(&mut f, &layer, ViewKind::Games, "title"));
    assert!(!f.variants["Big"].games.contains_key("title"));
    // And it all saves.
    assert_eq!(ThemeFile::parse(&f.to_theme_toml(None).unwrap()).unwrap(), f);
}

#[test]
fn a_picture_is_copied_into_the_theme_folder() {
    let dir = themes_dir();
    let theme = dir.path().join("t");
    std::fs::create_dir_all(&theme).unwrap();
    let outside = dir.path().join("My Sunset!.PNG");
    std::fs::write(&outside, b"\x89PNG fake").unwrap();
    let rel = files::bring_in(&theme, &outside, Asset::Picture).unwrap();
    assert!(rel.starts_with("pictures/my-sunset-") && rel.ends_with(".png"), "{rel}");
    assert_eq!(std::fs::read(theme.join(&rel)).unwrap(), b"\x89PNG fake");
    // The same file again: the same name; a changed file: a new one.
    assert_eq!(files::bring_in(&theme, &outside, Asset::Picture).unwrap(), rel);
    std::fs::write(&outside, b"\x89PNG other").unwrap();
    assert_ne!(files::bring_in(&theme, &outside, Asset::Picture).unwrap(), rel);
    // Already in the theme: used where it is.
    assert_eq!(files::bring_in(&theme, &theme.join(&rel), Asset::Picture).unwrap(), rel);
    // Not a picture: refused in plain words.
    let txt = dir.path().join("notes.txt");
    std::fs::write(&txt, "x").unwrap();
    let e = files::bring_in(&theme, &txt, Asset::Picture).unwrap_err().to_string();
    assert!(e.contains("isn't a picture"), "{e}");
    let font = dir.path().join("Nice.ttf");
    std::fs::write(&font, b"font").unwrap();
    assert!(files::bring_in(&theme, &font, Asset::Font).unwrap().starts_with("fonts/nice-"));
}

#[test]
fn console_pictures_are_matched_by_the_usual_file_names() {
    let colls = vec![
        crate::library::Collection {
            kind: crate::library::CollectionKind::Source(crate::model::Source::Steam),
            name: "Steam".into(),
        },
        crate::library::Collection { kind: crate::library::CollectionKind::Favorites, name: "Favourites".into() },
    ];
    let m = |s: &str| super::pages::match_console(s, &colls);
    assert_eq!(m("ps2").as_deref(), Some("sony_playstation2"));
    assert_eq!(m("PS2").as_deref(), Some("sony_playstation2"));
    assert_eq!(m("sony_playstation2").as_deref(), Some("sony_playstation2"));
    assert_eq!(m("snes").as_deref(), Some("nintendo_super_nes"));
    assert!(m("megadrive").is_some());
    assert!(m("n64").is_some());
    assert_eq!(m("Nintendo 64"), m("n64"));
    assert_eq!(m("steam").as_deref(), Some("steam"));
    assert_eq!(m("favourites").as_deref(), Some("favorites"));
    assert_eq!(m("auto-allgames").as_deref(), Some("all"));
    assert_eq!(m("not a console"), None);
    // A folder of them: matched ones copied into the theme, the rest listed for review.
    let dir = themes_dir();
    let theme = dir.path().join("t");
    let pics = dir.path().join("pics");
    std::fs::create_dir_all(&theme).unwrap();
    std::fs::create_dir_all(&pics).unwrap();
    for f in ["ps2.png", "snes.jpg", "readme.png", "notes.txt"] {
        std::fs::write(pics.join(f), f.as_bytes()).unwrap();
    }
    let (ok, rest) = super::pages::import_console_folder(&theme, "card", &pics, &colls);
    assert_eq!(ok.len(), 2, "{ok:?}");
    assert_eq!(rest, vec![pics.join("readme.png")]);
    for (_, rel) in &ok {
        assert!(rel.starts_with("consoles/card/") && theme.join(rel).is_file(), "{rel}");
    }
    // Written into [collection_art] and read back by the renderer's model.
    let mut f = ThemeFile::default();
    for (k, rel) in &ok {
        super::pages::set_art(&mut f, k, "card", Some(rel.clone()));
    }
    assert!(f.collection_art["sony_playstation2"].card.as_deref().unwrap().ends_with(".png"));
    let back = ThemeFile::parse(&f.to_theme_toml(None).unwrap()).unwrap();
    assert_eq!(back.collection_art, f.collection_art);
    super::pages::set_art(&mut f, "sony_playstation2", "card", None);
    assert!(!f.collection_art.contains_key("sony_playstation2"));
    // Not a picture.
    assert!(files::copy_console_picture(&theme, "card", "x", &pics.join("notes.txt")).is_err());
}

#[test]
fn a_theme_exported_as_zip_imports_as_a_new_theme() {
    let dir = themes_dir();
    let k = files::create(dir.path(), &Template::Builtin("Shelf".into()), "Zip me", "").unwrap();
    let tdir = dir.path().join(&k);
    std::fs::create_dir_all(tdir.join("pictures")).unwrap();
    std::fs::write(tdir.join("pictures/a.png"), vec![7u8; 5000]).unwrap();
    std::fs::write(tdir.join(".hidden"), b"x").unwrap();
    let zip = dir.path().join("out.zip");
    assert_eq!(super::zip::export(&tdir, &k, &zip).unwrap(), 2);
    let entries = crate::emulation::zipread::list_uncached(&zip).unwrap();
    let mut names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    names.sort();
    assert_eq!(names, [format!("{k}/pictures/a.png"), format!("{k}/theme.toml")]);
    let other = themes_dir();
    let imp = super::zip::import(&zip, other.path()).unwrap();
    assert_eq!(imp.name, "Zip me");
    let back = other.path().join(&imp.key);
    assert_eq!(std::fs::read(back.join("pictures/a.png")).unwrap(), vec![7u8; 5000]);
    assert_eq!(
        ThemeFile::parse(&std::fs::read_to_string(back.join("theme.toml")).unwrap()).unwrap(),
        Doc::open(&k, tdir).unwrap().file
    );
    // Importing again never replaces it: a second folder.
    let again = super::zip::import(&zip, other.path()).unwrap();
    assert_ne!(again.key, imp.key);
    // No work folders are left behind.
    let left: Vec<String> = std::fs::read_dir(other.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(left.iter().all(|n| !n.starts_with('.')), "{left:?}");
}

#[test]
fn hostile_or_wrong_zips_are_refused_and_leave_nothing() {
    let dir = themes_dir();
    let themes = dir.path().join("themes");
    let try_zip = |name: &str, entries: &[(&str, &[u8], bool)]| {
        let z = dir.path().join(name);
        write_zip(&z, entries);
        super::zip::import(&z, &themes)
    };
    let good = b"[theme]\nname = \"Evil\"\n" as &[u8];
    // A path leading out of the folder is never written (the zip reader leaves such entries out).
    let t = try_zip(
        "trav.zip",
        &[("theme.toml", good, false), ("../escaped.txt", b"x", false), ("a/../../up.txt", b"x", false)],
    );
    assert!(!dir.path().join("escaped.txt").exists() && !dir.path().join("up.txt").exists());
    if let Ok(t) = t {
        assert!(!themes.join(&t.key).join("escaped.txt").exists());
        std::fs::remove_dir_all(themes.join(&t.key)).unwrap();
    }
    let abs = dir.path().join("abs-evil.txt");
    let t = try_zip("abs.zip", &[("theme.toml", good, false), (abs.to_str().unwrap(), b"x", false)]);
    assert!(!abs.exists(), "an absolute path is never written");
    if let Ok(t) = t {
        std::fs::remove_dir_all(themes.join(&t.key)).unwrap();
    }
    for bad in ["/etc/x", "../x", "a/../../x", "C:/x", "C:\\x", ""] {
        assert_eq!(super::zip::safe_rel(bad), None, "{bad}");
    }
    assert_eq!(super::zip::safe_rel("./a/b.png"), Some(std::path::PathBuf::from("a/b.png")));
    let e = try_zip("none.zip", &[("readme.txt", b"hi", false)]).unwrap_err().to_string();
    assert!(e.contains("isn't a Lodestar theme"), "{e}");
    let e = try_zip("other.zip", &[("mytheme/theme.xml", b"<theme/>", false)]).unwrap_err().to_string();
    assert!(e.contains("another program"), "{e}");
    let e = try_zip("bad.zip", &[("theme.toml", b"this is = = not toml", false)]).unwrap_err().to_string();
    assert!(e.contains("mistake"), "{e}");
    let e = try_zip("deep.zip", &[("a/theme.toml", good, false), ("b/x.png", b"x", false)]).unwrap_err().to_string();
    assert!(e.contains("isn't at the top"), "{e}");
    let not_zip = dir.path().join("plain.zip");
    std::fs::write(&not_zip, b"not a zip at all").unwrap();
    assert!(super::zip::import(&not_zip, &themes).is_err());
    // Nothing was added, no work folder left.
    let left: Vec<_> = std::fs::read_dir(&themes).map(|r| r.flatten().collect()).unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");
    // A good one, with an executable inside: added, but nothing is executable.
    let ok =
        try_zip("ok.zip", &[("Evil/theme.toml", good, true), ("Evil/run.sh", b"#!/bin/sh\nrm -rf ~", false)]).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let m = std::fs::metadata(themes.join(&ok.key).join("run.sh")).unwrap().permissions().mode();
    assert_eq!(m & 0o111, 0);
}

#[test]
fn the_check_explains_problems_in_plain_words() {
    let src = r##"
[theme]
name = ""
[games]
[[games.elements]]
type = "text"
name = "t"
text = "{gmae.name}"
style = { font = "fancy" }
[[games.elements]]
type = "image"
name = "t"
path = "pictures/missing.png"
pos = [1.5, 0.0]
size = [0.2, 0.2]
sparkle = 3
[variants.V.games.nope]
opacity = 0.5
[aspects.wide.games.t]
opacity = 0.5
"##;
    let f = ThemeFile::parse(src).unwrap();
    let dir = themes_dir();
    let found = validate::check(&f, Some(dir.path()));
    let texts: Vec<&str> = found.iter().map(|x| x.text.as_str()).collect();
    let has = |s: &str| texts.iter().any(|t| t.contains(s));
    assert!(has("needs a name"), "{texts:#?}");
    assert!(has("no list to pick games"));
    assert!(has("no list to pick collections"));
    assert!(has("{gmae.name}"));
    assert!(has("font “fancy”"));
    assert!(has("missing.png"));
    assert!(has("outside the screen"));
    assert!(has("called “t”"));
    assert!(has("sparkle"));
    assert!(has("no element with that name"));
    assert!(has("“wide” isn't a shape"));
    // Problems come first.
    assert_eq!(found[0].level, Level::Problem);
}

#[test]
fn an_element_edited_as_toml_reads_back() {
    let pal = Palette::default();
    for k in [Kind::Stack, Kind::Carousel, Kind::Title, Kind::Backdrop] {
        let e = ops::new_element(k, ViewKind::Games, &pal, "x".into());
        let t = super::inspector::element_toml(&e);
        assert!(t.starts_with("type = "), "{t}");
        assert_eq!(super::inspector::parse_element(&t).unwrap(), e, "{t}");
    }
    assert!(super::inspector::parse_element("type = \"nonsense\"").is_err());
}

#[test]
fn sharing_details_are_checked_and_suggested() {
    assert_eq!(super::pages::suggest_id("Marko K.", "Night Shelf!"), "marko-k-night-shelf");
    assert!(super::pages::is_semver("1.10.0"));
    assert!(!super::pages::is_semver("1.0"));
    let mut f = ThemeFile::default();
    assert_eq!(super::pages::sharing_gaps(&f).len(), 6);
    f.theme.author = "Me".into();
    f.theme.id = "me-theme".into();
    f.theme.version = "1.0.0".into();
    f.theme.license = "CC-BY-4.0".into();
    f.theme.summary = "Nice".into();
    f.theme.previews = vec!["previews/games.png".into()];
    assert!(super::pages::sharing_gaps(&f).is_empty());
    // The details are part of the format and survive a save.
    let back = ThemeFile::parse(&f.to_theme_toml(None).unwrap()).unwrap();
    assert_eq!(back.theme, f.theme);
    // `licence` is read too.
    let t = ThemeFile::parse("[theme]\nlicence = \"MIT\"\n").unwrap();
    assert_eq!(t.theme.license, "MIT");
}

#[test]
fn a_light_scheme_recolours_a_dark_theme() {
    let (_, src) = builtin::SOURCES[0];
    let mut f = ThemeFile::parse(src).unwrap();
    f.color_schemes.insert("light".into(), super::pages::light_scheme(&f.palette));
    let l = f.with_color_scheme("light");
    assert!(!l.palette.dark);
    assert_ne!(l.games, f.games, "element colours that were palette colours follow");
    assert_eq!(ThemeFile::parse(&f.to_theme_toml(None).unwrap()).unwrap(), f);
}
