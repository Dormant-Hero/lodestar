//! Whole-scan integration test over one fixture home that holds every store at once: Heroic
//! (Epic + GOG + Amazon + sideload, installed + owned), the Legendary / nile CLIs, Wine prefixes
//! with Battle.net / EA / Ubisoft / Rockstar / Riot / HoYoPlay, Prism / MultiMC / ATLauncher
//! instances, Moonlight hosts and desktop shortcuts that duplicate store games.
//! Checks sources, cross-scanner dedupe, launch specs and that a re-merge adds nothing.

use super::desktop::test_support::{copy_tree, fake_png, fixture};
use super::*;
use crate::model::LaunchSpec;
use std::collections::HashSet;
use std::path::Path;

fn replace_in_json(dir: &Path, from: &str, to: &str) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            replace_in_json(&p, from, to);
        } else if p.extension().is_some_and(|x| x == "json") {
            let t = std::fs::read_to_string(&p).unwrap();
            if t.contains(from) {
                std::fs::write(&p, t.replace(from, to)).unwrap();
            }
        }
    }
}

fn stores(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stores").join(rel)
}

/// The combined fixture home (wine prefixes first; Heroic's library merged into its config dir).
fn everything_home() -> (tempfile::TempDir, std::path::PathBuf) {
    let (tmp, home) = super::winprefix::tests::fixture_home();
    let hs = home.to_string_lossy().into_owned();
    // Heroic: keep the wine fixture's GamesConfig (prefix settings), add the library files.
    let heroic = home.join(".config/heroic");
    let staging = tmp.path().join("heroic-staging");
    copy_tree(&fixture("heroic/heroic"), &staging);
    std::fs::remove_dir_all(staging.join("GamesConfig")).unwrap();
    copy_tree(&staging, &heroic);
    replace_in_json(&heroic, "/HOME", &hs);
    for d in ["Games/Heroic/Fortnite", "Games/Heroic/Cyberpunk 2077", "Games/Heroic/Tomb Raider", "Games/itchy"] {
        std::fs::create_dir_all(home.join(d)).unwrap();
    }
    std::fs::write(home.join("Games/itchy/run.sh"), "#!/bin/sh").unwrap();
    // The CLIs (Rocket League only in Legendary; nile's Hitman only in nile).
    copy_tree(&stores("legendary"), &home.join(".config/legendary"));
    copy_tree(&stores("nile"), &home.join(".config/nile"));
    replace_in_json(&home.join(".config/legendary"), "/HOME", &hs);
    replace_in_json(&home.join(".config/nile"), "/HOME", &hs);
    for d in ["Games/legendary/RocketLeague", "Games/nile/Hitman"] {
        std::fs::create_dir_all(home.join(d)).unwrap();
    }
    // Minecraft (Prism) and Moonlight.
    let prism = home.join(".local/share/PrismLauncher");
    copy_tree(&stores("minecraft/PrismLauncher"), &prism);
    fake_png(&prism.join("icons/fabulously-optimized.png"), 128, 128);
    let ml = home.join(".config/Moonlight Game Streaming Project/Moonlight.conf");
    std::fs::create_dir_all(ml.parent().unwrap()).unwrap();
    std::fs::copy(stores("moonlight/Moonlight.conf"), &ml).unwrap();
    // Desktop shortcuts that duplicate store games (Heroic, the Legendary CLI).
    let apps = home.join(".local/share/applications");
    std::fs::create_dir_all(&apps).unwrap();
    std::fs::write(
        apps.join("Fortnite.desktop"),
        "[Desktop Entry]\nType=Application\nName=Fortnite\nExec=xdg-open heroic://launch?appName=Fortnite&runner=legendary\nCategories=Game;\n",
    )
    .unwrap();
    std::fs::write(
        apps.join("rocket.desktop"),
        "[Desktop Entry]\nType=Application\nName=Rocket League\nExec=legendary launch Sugar\nCategories=Game;\n",
    )
    .unwrap();
    std::fs::write(
        apps.join("tux.desktop"),
        "[Desktop Entry]\nType=Application\nName=Tux Racer\nExec=/usr/bin/true\nCategories=Game;\n",
    )
    .unwrap();
    (tmp, home)
}

fn ctx_for(home: &Path, data: &Path) -> ScanContext {
    let mut c = super::winprefix::tests::ctx(home);
    c.data_dir = data.to_path_buf();
    c
}

#[test]
fn every_store_in_one_home() {
    let (tmp, home) = everything_home();
    let data = tmp.path().join("lodestar-data");
    std::fs::create_dir_all(&data).unwrap();
    let ctx = ctx_for(&home, &data);
    let report = run_all(&ctx);
    for g in &report.games {
        eprintln!("{:<10} {:<32} {:<40} {:?}", g.source.key(), g.source_id, g.name, g.launch);
    }

    for (src, r) in &report.per_source {
        assert!(r.error.is_none(), "{src:?} failed: {:?}", r.error);
    }
    let found = |src: Source| report.per_source.iter().find(|(s, _)| *s == src).map(|(_, r)| r.found).unwrap();
    let ids = |src: Source| -> Vec<&str> {
        let mut v: Vec<&str> = report.games.iter().filter(|g| g.source == src).map(|g| g.source_id.as_str()).collect();
        v.sort();
        v
    };

    // Heroic → its stores; the CLIs add only what Heroic doesn't know.
    assert_eq!(ids(Source::Epic), vec!["Fortnite", "Sugar"], "{:#?}", report.games);
    assert_eq!(ids(Source::Gog), vec!["1423049311"]);
    assert!(ids(Source::Amazon).contains(&"amzn1.adg.product.tombraider"));
    assert_eq!(ids(Source::Heroic), vec!["sl_itchy"]);
    for src in [Source::BattleNet, Source::Ea, Source::Ubisoft, Source::Rockstar, Source::Riot, Source::HoYoPlay] {
        assert!(found(src) > 0, "no {src:?} games: {:#?}", report.games);
    }
    assert!(found(Source::Minecraft) >= 1);
    assert!(found(Source::Moonlight) >= 2);

    // Nothing reported twice, and the store-duplicating desktop shortcuts are gone.
    let mut seen = HashSet::new();
    for g in &report.games {
        assert!(seen.insert((g.source, g.source_id.clone())), "duplicate {:?} {}", g.source, g.source_id);
    }
    let desktop: Vec<&str> =
        report.games.iter().filter(|g| g.source == Source::Desktop).map(|g| g.name.as_str()).collect();
    assert_eq!(desktop, vec!["Tux Racer"], "store shortcuts must be deduped");
    assert!(report.duplicates_dropped >= 2);

    // Launch specs: Heroic games through heroic://, CLI-only through the CLI, prefix stores
    // through the launcher in their prefix. All are specs only; nothing was started.
    let game = |src: Source, id: &str| report.games.iter().find(|g| g.source == src && g.source_id == id).unwrap();
    assert_eq!(
        game(Source::Epic, "Fortnite").launch,
        Some(LaunchSpec::Url("heroic://launch?appName=Fortnite&runner=legendary".into()))
    );
    match &game(Source::Epic, "Sugar").launch {
        Some(LaunchSpec::Command { program, args, .. }) => {
            assert!(program.ends_with("legendary"), "{program}");
            assert_eq!(args.first().map(String::as_str), Some("launch"));
            assert!(args.iter().any(|a| a == "Sugar"));
        }
        other => panic!("Sugar launch: {other:?}"),
    }
    for g in report.games.iter().filter(|g| {
        matches!(
            g.source,
            Source::BattleNet | Source::Ea | Source::Ubisoft | Source::Rockstar | Source::Riot | Source::HoYoPlay
        )
    }) {
        match &g.launch {
            // Every store-client game goes through Lodestar's helper (client_launch.rs), which
            // runs the client with the prefix's runner (umu-run / proton / wine).
            Some(LaunchSpec::Command { program, args, .. }) => {
                assert_eq!(program, crate::client_launch::SELF_PROGRAM, "{}", g.name);
                assert!(args.iter().any(|a| a == "--client-launch"), "{args:?}");
                assert!(args.iter().any(|a| a == "--prefix"), "{} has no prefix: {args:?}", g.name);
            }
            Some(LaunchSpec::Url(u)) => panic!("{} launches a URL {u}", g.name),
            other => panic!("{} launch: {other:?}", g.name),
        }
        assert_eq!(g.platform.as_deref(), Some("windows"));
    }

    // Into the DB twice: the second merge adds nothing and removes nothing.
    let mut db = crate::db::Db::open_memory().unwrap();
    let s1 = db.merge_scan(&report.games, &report.ok_sources).unwrap();
    assert_eq!(s1.added, report.games.len());
    let again = run_all(&ctx);
    let s2 = db.merge_scan(&again.games, &again.ok_sources).unwrap();
    assert_eq!((s2.added, s2.removed, s2.uninstalled), (0, 0, 0), "{s2:?}");
    assert_eq!(db.load_all().unwrap().len(), report.games.len());

    // Owned-but-not-installed games appear only when asked for, as uninstalled.
    let mut c2 = ctx.clone();
    c2.config.include_uninstalled = true;
    let owned = run_all(&c2);
    let hades =
        owned.games.iter().find(|g| g.source == Source::Epic && g.source_id == "Hades").expect("owned Epic game");
    assert!(!hades.installed);
    assert!(!report.games.iter().any(|g| g.source_id == "Hades"));
}

/// A library from before shortcuts were matched: the Heroic shortcut `Fortnite.desktop` was a
/// game of its own. The next scan folds its row into the Epic game, played time and favourite
/// included, and leaves no missing copy.
#[test]
fn old_rows_of_store_shortcuts_fold_into_the_store_game() {
    let (tmp, home) = everything_home();
    let data = tmp.path().join("lodestar-data");
    std::fs::create_dir_all(&data).unwrap();
    let report = run_all(&ctx_for(&home, &data));
    assert!(
        report.dropped_shortcuts.contains(&DroppedShortcut {
            shortcut: (Source::Desktop, "Fortnite.desktop".into()),
            target: (Source::Epic, "Fortnite".into()),
        }),
        "{:?}",
        report.dropped_shortcuts
    );

    let mut db = crate::db::Db::open_memory().unwrap();
    let mut old = report.games.clone();
    old.push(ScannedGame::new(Source::Desktop, "Fortnite.desktop", "Fortnite"));
    db.merge_scan(&old, &report.ok_sources).unwrap();
    let id_of = |db: &crate::db::Db, src: Source, sid: &str| {
        db.load_all().unwrap().into_iter().find(|g| g.source == src && g.source_id == sid).map(|g| g.id)
    };
    let shortcut = id_of(&db, Source::Desktop, "Fortnite.desktop").unwrap();
    let epic = id_of(&db, Source::Epic, "Fortnite").unwrap();
    let heroic_playtime = db.get(epic).unwrap().unwrap().playtime_secs;
    db.set_favorite(shortcut, true).unwrap();
    db.add_playtime(shortcut, 1200).unwrap();

    let s = db.merge_scan_with_shortcuts(&report.games, &report.ok_sources, &report.dropped_shortcuts).unwrap();
    assert_eq!((s.removed, s.missing), (1, 0), "{s:?}");
    assert_eq!(id_of(&db, Source::Desktop, "Fortnite.desktop"), None);
    let epic = db.get(epic).unwrap().unwrap();
    assert!(epic.favorite);
    assert_eq!(epic.playtime_secs, heroic_playtime + 1200);
}
