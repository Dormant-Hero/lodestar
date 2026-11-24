//! Tests for the desktop UI's pure logic (+ an ignored helper that writes a large fake library).

use super::*;
use crate::library::Library;

pub(crate) fn fake_game(id: GameId, name: &str, source: Source) -> Game {
    Game {
        id,
        source,
        source_id: id.to_string(),
        installed: id % 3 != 0,
        playtime_secs: (id as u64 % 17) * 3600,
        last_played: if id % 4 == 0 { Some(1_700_000_000 + id) } else { None },
        added: id,
        favorite: id % 7 == 0,
        hidden: id % 50 == 0,
        tags: if id % 5 == 0 { vec!["RPG".into()] } else { vec![] },
        ..Game::blank(name)
    }
}

#[test]
fn counts_skip_hidden_and_group() {
    let games: Vec<Game> = (1..=100)
        .map(|i| fake_game(i, &format!("Game {i}"), if i % 2 == 0 { Source::Steam } else { Source::Desktop }))
        .collect();
    let lib = Library::new(games);
    let c = Counts::compute(&lib);
    assert_eq!(c.hidden, 2);
    assert_eq!(c.all, 98);
    assert_eq!(c.sources.iter().map(|s| s.1).sum::<usize>(), 98);
    assert_eq!(c.tags.len(), 1);
    assert_eq!(c.tags[0].0, "RPG");
    assert_eq!(c.generation, lib.generation);
}

#[test]
fn strip_html_basic() {
    assert_eq!(strip_html("plain text"), "plain text");
    assert_eq!(strip_html("<p>Hello &amp; <b>welcome</b></p><p>Second</p>"), "Hello & welcome\nSecond");
    assert_eq!(strip_html("<ul><li>One</li><li>Two</li></ul>"), "• One\n• Two");
    assert_eq!(strip_html("a < b"), "a < b");
    assert_eq!(strip_html("x &unknown; y"), "x &unknown; y");
    assert_eq!(strip_html("<br><br><br><br>z"), "z");
}

#[test]
fn play_state_uses_the_store_caps() {
    use details::{PlayState, play_state};
    let mut g = fake_game(1, "A", Source::Steam);
    g.installed = false;
    let caps = |can_install| StoreCaps { id: 1, can_install, can_uninstall: false, store_page: None };
    assert_eq!(play_state(&g, None, None), PlayState::NotInstalled);
    assert_eq!(play_state(&g, None, Some(&caps(false))), PlayState::NotInstalled);
    assert_eq!(play_state(&g, None, Some(&caps(true))), PlayState::Install);
    assert_eq!(play_state(&g, Some(1), Some(&caps(true))), PlayState::Running);
    g.installed = true;
    assert_eq!(play_state(&g, None, Some(&caps(true))), PlayState::Play);
}

#[test]
fn every_library_source_has_a_settings_row() {
    let listed = libraries::listed_sources();
    for s in Source::ALL {
        if s == Source::Manual {
            continue;
        }
        assert!(listed.contains(&s), "{s:?} has no row in Settings → Libraries");
    }
    // No duplicates.
    let mut keys: Vec<&str> = listed.iter().map(|s| s.key()).collect();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), listed.len());
    // Built-in cards in the Add-ons window: everything but the add-ons row itself.
    assert_eq!(libraries::builtin_libraries().count(), listed.len() - 1);
}

#[test]
fn library_row_status() {
    use libraries::{Status, status_for};
    let last = vec![(Source::Steam, 12, None), (Source::Epic, 0, None), (Source::Gog, 0, Some("bad json".to_string()))];
    let lib = vec![(Source::Lutris, 3)];
    assert_eq!(status_for(Source::Steam, false, false, Some(&last), &lib), Status::Off);
    assert_eq!(status_for(Source::Steam, true, true, Some(&last), &lib), Status::Scanning);
    assert_eq!(status_for(Source::Steam, true, false, Some(&last), &lib), Status::Found(12));
    assert_eq!(status_for(Source::Epic, true, false, Some(&last), &lib), Status::NotFound);
    assert_eq!(status_for(Source::Gog, true, false, Some(&last), &lib), Status::Error("bad json".into()));
    // Switched on after the last scan.
    assert_eq!(status_for(Source::Amazon, true, false, Some(&last), &lib), Status::NotScanned);
    // No scan this session: what the library holds.
    assert_eq!(status_for(Source::Lutris, true, false, None, &lib), Status::InLibrary(3));
    assert_eq!(status_for(Source::Riot, true, false, None, &lib), Status::NotScanned);
}

#[test]
fn protondb_tiers() {
    use details::ProtonTier;
    assert_eq!(ProtonTier::parse("platinum"), Some(ProtonTier::Platinum));
    assert_eq!(ProtonTier::parse(" Gold "), Some(ProtonTier::Gold));
    assert_eq!(ProtonTier::parse("BORKED"), Some(ProtonTier::Borked));
    assert_eq!(ProtonTier::parse("native"), Some(ProtonTier::Native));
    assert_eq!(ProtonTier::parse("pending"), None);
    assert_eq!(ProtonTier::parse(""), None);
    let pal = Pal::default();
    // Distinct medal colours.
    let all = [
        ProtonTier::Platinum,
        ProtonTier::Gold,
        ProtonTier::Silver,
        ProtonTier::Bronze,
        ProtonTier::Borked,
        ProtonTier::Native,
    ];
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert_ne!(a.colour(&pal), b.colour(&pal));
        }
    }
}

#[test]
fn explorer_sources_are_alphabetical() {
    let games = vec![
        fake_game(1, "A", Source::Steam),
        fake_game(2, "B", Source::Epic),
        fake_game(3, "C", Source::BattleNet),
        fake_game(4, "D", Source::Gog),
    ];
    let mut lib = Library::default();
    lib.replace(games);
    let c = Counts::compute(&lib);
    let names: Vec<&str> = c.sources.iter().map(|(s, _)| s.label()).collect();
    assert_eq!(names, ["Battle.net", "Epic Games", "GOG", "Steam"]);
}

#[test]
fn addons_sections_from_keys() {
    use crate::addons::AddonKind;
    use addons::Section;
    assert_eq!(Section::from_key("libraries"), Section::Kind(AddonKind::Library));
    assert_eq!(Section::from_key("metadata"), Section::Kind(AddonKind::Metadata));
    assert_eq!(Section::from_key("builtin"), Section::BuiltIn);
    assert_eq!(Section::from_key("browse"), Section::Browse);
    assert_eq!(Section::from_key(""), Section::All);
}

#[test]
fn view_follows_filter_and_keeps_selection() {
    let games: Vec<Game> = (1..=40).map(|i| fake_game(i, &format!("Game {i:02}"), Source::Steam)).collect();
    let mut st_lib = Library::new(games);
    let mut ui = DesktopUi::new();
    // select a game that survives the "installed only" filter
    ui.selected = Some(8);
    ui.refresh_view_for(&st_lib);
    assert_eq!(ui.selected, Some(8));
    let pos = ui.sel_pos.unwrap();
    assert_eq!(st_lib.games[ui.view[pos]].id, 8);
    ui.filter.installed_only = true;
    ui.refresh_view_for(&st_lib);
    assert_eq!(ui.selected, Some(8), "selection persists when still visible");
    // selected game filtered out -> falls back to the first visible game
    ui.filter.search = "Game 1".into();
    ui.refresh_view_for(&st_lib);
    assert!(ui.selected.is_some());
    assert!(ui.view.iter().all(|&i| st_lib.games[i].installed));
    // library change re-filters
    st_lib.update(10, |g| g.installed = false);
    ui.refresh_view_for(&st_lib);
    assert!(!ui.view.iter().any(|&i| st_lib.games[i].id == 10));
}

#[test]
fn empty_view_has_no_selected_position() {
    // The details panel relies on this: with no results it shows nothing instead of the
    // (still remembered) selected game that does not match the search.
    let games: Vec<Game> = (1..=10).map(|i| fake_game(i, &format!("Game {i:02}"), Source::Steam)).collect();
    let lib = Library::new(games);
    let mut ui = DesktopUi::new();
    ui.selected = Some(4);
    ui.refresh_view_for(&lib);
    assert!(ui.sel_pos.is_some());
    ui.filter.search = "no such game zzz".into();
    ui.refresh_view_for(&lib);
    assert!(ui.view.is_empty());
    assert_eq!(ui.sel_pos, None);
    assert_eq!(ui.selected, Some(4), "selection is remembered for when the search clears");
    ui.filter.search.clear();
    ui.refresh_view_for(&lib);
    assert_eq!(ui.selected, Some(4));
    assert_eq!(lib.games[ui.view[ui.sel_pos.unwrap()]].id, 4);
}

fn companies(s: &str) -> Vec<String> {
    let mut v = vec![];
    split_companies(Some(s), |c| v.push(c.to_string()));
    v
}

#[test]
fn split_companies_keeps_suffixes() {
    assert_eq!(companies("Larian Studios, Feral Interactive"), ["Larian Studios", "Feral Interactive"]);
    assert_eq!(companies("Square Enix, Inc., Eidos-Montréal"), ["Square Enix, Inc.", "Eidos-Montréal"]);
    assert_eq!(companies(" CD PROJEKT RED "), ["CD PROJEKT RED"]);
    assert_eq!(companies("A,,B,"), ["A", "B"]);
    assert!(companies("").is_empty());
}

fn facet_lib() -> Library {
    let mut games: Vec<Game> = (1..=6).map(|i| fake_game(i, &format!("Game {i}"), Source::Steam)).collect();
    for g in &mut games {
        g.hidden = false;
        g.installed = true;
    }
    games[0].meta.genres = vec!["RPG".into(), "Action".into(), "rpg".into()];
    games[1].meta.genres = vec!["rpg".into()];
    games[2].meta.genres = vec!["Strategy".into()];
    games[0].platform = Some("windows".into());
    games[1].platform = Some("linux".into());
    games[2].platform = Some("windows".into());
    games[0].meta.developer = Some("Larian Studios, Feral Interactive".into());
    games[1].meta.developer = Some("Larian Studios".into());
    games[3].meta.publisher = Some("ドラゴン社".into());
    Library::new(games)
}

#[test]
fn facet_counts_group_case_insensitively() {
    let lib = facet_lib();
    let c = Counts::compute(&lib);
    let genres = &c.facets[Facet::Genre.index()];
    assert_eq!(
        genres.iter().map(|(k, d, n)| (k.as_str(), d.as_str(), *n)).collect::<Vec<_>>(),
        [("action", "Action", 1), ("rpg", "RPG", 2), ("strategy", "Strategy", 1)]
    );
    let plats = &c.facets[Facet::Platform.index()];
    assert_eq!(plats.len(), 4);
    assert_eq!(plats[0], ("pc".to_string(), "PC".to_string(), 6), "every game here is a PC game");
    assert!(plats.iter().any(|(_, d, n)| d == "Other" && *n == 3), "the three PC games with no platform: {plats:?}");
    assert!(plats.iter().any(|(_, d, n)| d == "Windows" && *n == 2));
    assert!(plats.iter().any(|(_, d, n)| d == "Linux" && *n == 1));
    let devs = &c.facets[Facet::Developer.index()];
    assert!(devs.iter().any(|(_, d, n)| d == "Larian Studios" && *n == 2));
    assert!(devs.iter().any(|(_, d, n)| d == "Feral Interactive" && *n == 1));
    assert_eq!(c.facets[Facet::Publisher.index()][0].1, "ドラゴン社");
}

#[test]
fn facet_filter_any_within_all_across() {
    let lib = facet_lib();
    let mut ui = DesktopUi::new();
    let ids = |ui: &DesktopUi| {
        let mut v: Vec<GameId> = ui.view.iter().map(|&i| lib.games[i].id).collect();
        v.sort();
        v
    };
    ui.refresh_view_for(&lib);
    assert_eq!(ui.view.len(), 6);
    ui.facet_sel[Facet::Genre.index()] = vec!["rpg".into()];
    ui.refresh_view_for(&lib);
    assert_eq!(ids(&ui), [1, 2], "genre matches case-insensitively");
    ui.facet_sel[Facet::Genre.index()].push("strategy".into());
    ui.refresh_view_for(&lib);
    assert_eq!(ids(&ui), [1, 2, 3], "any value within a field");
    ui.facet_sel[Facet::Platform.index()] = vec!["windows".into()];
    ui.refresh_view_for(&lib);
    assert_eq!(ids(&ui), [1, 3], "all fields together");
    ui.facet_sel = Default::default();
    ui.facet_sel[Facet::Publisher.index()] = vec!["ドラゴン社".into()];
    ui.refresh_view_for(&lib);
    assert_eq!(ids(&ui), [4]);
    ui.set_lib_sel(LibSel::Installed);
    assert!(ui.no_extra_filters(), "picking a library entry clears facets");
}

#[test]
fn feature_facet_counts_and_filters() {
    let mut games: Vec<Game> = (1..=6).map(|i| fake_game(i, &format!("G{i}"), Source::Steam)).collect();
    for g in &mut games {
        g.hidden = false;
    }
    games[0].meta.controller = Some(ControllerSupport::Full);
    games[0].meta.deck = Some(DeckRating::Verified);
    games[1].meta.controller = Some(ControllerSupport::Partial);
    games[1].meta.deck = Some(DeckRating::Playable);
    games[2].meta.controller = Some(ControllerSupport::Full);
    games[2].controller_override = Some(ControllerSupport::None); // the user says no
    games[3].controller_override = Some(ControllerSupport::Full); // unknown to Steam
    games[4].meta.deck = Some(DeckRating::Unsupported);
    let lib = Library::new(games);
    let c = Counts::compute(&lib);
    let feats: Vec<(&str, usize)> = c.facets[Facet::Feature.index()].iter().map(|(_, d, n)| (d.as_str(), *n)).collect();
    assert_eq!(
        feats,
        [
            ("Full controller support", 2),
            ("Partial controller support", 1),
            ("Steam Deck Verified", 1),
            ("Steam Deck Playable", 1),
        ],
        "natural order, overrides respected, no Unsupported / None rows"
    );
    let mut ui = DesktopUi::new();
    ui.facet_sel[Facet::Feature.index()] = vec!["full controller support".into()];
    ui.refresh_view_for(&lib);
    let mut ids: Vec<GameId> = ui.view.iter().map(|&i| lib.games[i].id).collect();
    ids.sort();
    assert_eq!(ids, [1, 4]);
}

#[test]
fn controller_menu_labels() {
    let mut g = fake_game(1, "G", Source::Steam);
    assert_eq!(controller_auto_label(&g), "Automatic (not known)");
    g.meta.controller = Some(ControllerSupport::Full);
    assert_eq!(controller_auto_label(&g), "Automatic (Steam says: Full)");
    g.source = Source::Heroic;
    g.meta.controller = Some(ControllerSupport::Partial);
    assert_eq!(controller_auto_label(&g), "Automatic (Partial)");
}

#[test]
fn explorer_state_round_trips() {
    let mut ui = DesktopUi::new();
    ui.lib_sel = LibSel::Recent;
    ui.src_sel = vec![SrcKey::source(Source::Steam), SrcKey::source(Source::Heroic)];
    ui.filter.tags = vec!["Co-op".into()];
    ui.facet_sel[Facet::Genre.index()] = vec!["rpg".into()];
    ui.facet_sel[Facet::Platform.index()] = vec!["linux".into()];
    ui.facet_sel[Facet::Feature.index()] = vec!["steam deck verified".into()];
    ui.collapsed = vec!["tags".into()];
    let h = ui.explorer_hash();
    let json = serde_json::to_string(&ui.explorer_state()).unwrap();
    let mut back = DesktopUi::new();
    assert_ne!(back.explorer_hash(), h);
    back.apply_explorer_state(&serde_json::from_str(&json).unwrap());
    assert_eq!(back.lib_sel, LibSel::Recent);
    assert_eq!(back.src_sel, ui.src_sel);
    assert_eq!(back.filter.tags, ui.filter.tags);
    assert_eq!(back.facet_sel, ui.facet_sel);
    assert_eq!(back.collapsed, ui.collapsed);
    assert_eq!(back.explorer_hash(), h);
    // Unknown / old data degrades gracefully.
    let old: ExplorerState = serde_json::from_str(r#"{"lib":"bogus","sources":["nope"]}"#).unwrap();
    back.apply_explorer_state(&old);
    assert_eq!(back.lib_sel, LibSel::All);
    assert!(back.src_sel.is_empty());
}

#[test]
fn recently_played_reports_its_real_sort() {
    let mut ui = DesktopUi::new();
    let d = crate::config::DesktopConfig { sort: SortBy::Name, sort_descending: true, ..Default::default() };
    assert_eq!(ui.effective_sort(&d), (SortBy::Name, true, false));
    ui.lib_sel = LibSel::Recent;
    assert_eq!(ui.effective_sort(&d), (SortBy::LastPlayed, false, true));
}

#[test]
fn offset_to_show_moves_minimally() {
    // Viewport 0..800 of 3000 px content.
    assert_eq!(offset_to_show(0.0, 800.0, 100.0, 400.0, 3000.0), None);
    // Below: bottom edge lands on the viewport bottom.
    assert_eq!(offset_to_show(0.0, 800.0, 1000.0, 1300.0, 3000.0), Some(500.0));
    // Above: top edge lands on the viewport top.
    assert_eq!(offset_to_show(1500.0, 800.0, 1000.0, 1300.0, 3000.0), Some(1000.0));
    // Clamped to the scrollable range; never past the end (the startup overshoot).
    assert_eq!(offset_to_show(0.0, 800.0, 2900.0, 3000.0, 3000.0), Some(2200.0));
    assert_eq!(offset_to_show(2200.0, 800.0, 2900.0, 3000.0, 3000.0), None);
    // Taller than the viewport: show its top.
    assert_eq!(offset_to_show(0.0, 200.0, 500.0, 900.0, 3000.0), Some(500.0));
    // Content shorter than the viewport: stays at 0.
    assert_eq!(offset_to_show(0.0, 800.0, 100.0, 200.0, 300.0), None);
}

/// Writes a synthetic library (default 3000 games, every source, some with real art paths
/// taken from `LODESTAR_FAKE_ART` dir if set) into `$LODESTAR_FAKE_DB`.
/// Run: LODESTAR_FAKE_DB=/tmp/x/data/library.db cargo test make_fake_library -- --ignored
#[test]
#[ignore]
fn make_fake_library() {
    let Some(db_path) = std::env::var_os("LODESTAR_FAKE_DB") else {
        return;
    };
    let n: usize = std::env::var("LODESTAR_FAKE_N").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
    let covers: Vec<std::path::PathBuf> = std::env::var_os("LODESTAR_FAKE_ART")
        .and_then(|d| std::fs::read_dir(d).ok())
        .map(|rd| {
            rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "jpg" || e == "png")).collect()
        })
        .unwrap_or_default();
    let words = [
        "Star", "Shadow", "Iron", "Last", "Crystal", "Dark", "Neon", "Lost", "Galactic", "Silent", "Broken", "Hollow",
        "Crimson",
    ];
    let nouns = [
        "Odyssey",
        "Frontier",
        "Legacy",
        "Knight",
        "Protocol",
        "Empire",
        "Saga",
        "Horizon",
        "Dungeon",
        "Racer",
        "Tactics",
        "Chronicles",
    ];
    let srcs =
        [Source::Steam, Source::Heroic, Source::Lutris, Source::Faugus, Source::Desktop, Source::Itch, Source::Bottles];
    let mut scanned = vec![];
    for i in 0..n {
        let name = format!(
            "{} {} {}",
            words[i % words.len()],
            nouns[(i / words.len()) % nouns.len()],
            i / (words.len() * nouns.len()) + 1
        );
        let src = srcs[i % srcs.len()];
        let mut sg = crate::model::ScannedGame::new(src, format!("fake{i}"), name);
        sg.installed = i % 4 != 0;
        sg.playtime_secs = Some((i as u64 * 7919) % 400_000);
        sg.last_played = if i % 3 == 0 { Some(1_760_000_000 - i as i64 * 3600) } else { None };
        if !covers.is_empty() && i % 5 != 0 {
            sg.art.cover = Some(covers[i % covers.len()].clone());
        }
        sg.meta.description = Some(format!("Synthetic test game number {i}."));
        sg.meta.release_year = Some(1990 + (i % 35) as i32);
        sg.launch = Some(LaunchSpec::Command {
            program: "sh".into(),
            args: vec!["-c".into(), "sleep 1".into()],
            cwd: None,
            env: vec![],
        });
        scanned.push(sg);
    }
    let mut db = crate::db::Db::open(std::path::Path::new(&db_path)).expect("open db");
    let stats = db.merge_scan(&scanned, &srcs).expect("merge");
    eprintln!("fake library: {stats:?}");
}

#[test]
fn completion_and_year_facets_and_detail_filters() {
    let mut games: Vec<Game> = (1..=5).map(|i| fake_game(i, &format!("G{i}"), Source::Steam)).collect();
    for g in &mut games {
        g.hidden = false;
    }
    games[0].edits.completion = Some(CompletionStatus::Beaten);
    games[1].edits.completion = Some(CompletionStatus::Playing);
    games[2].edits.completion = Some(CompletionStatus::Playing);
    games[0].meta.release_year = Some(2017);
    games[1].meta.release_year = Some(2023);
    games[3].edits.features = Some(vec!["Co-op".into()]);
    let lib = Library::new(games);
    let c = Counts::compute(&lib);
    let comp: Vec<(&str, usize)> =
        c.facets[Facet::Completion.index()].iter().map(|(_, d, n)| (d.as_str(), *n)).collect();
    assert_eq!(comp, [("Playing", 2), ("Beaten", 1)], "natural status order");
    assert_eq!(c.facets[Facet::Year.index()][0].1, "2023", "newest year first");
    assert!(
        c.facets[Facet::Feature.index()].iter().any(|(_, d, n)| d == "Co-op" && *n == 1),
        "the user's own features are filterable"
    );

    let mut ui = DesktopUi::new();
    ui.counts = c;
    let ids = |ui: &DesktopUi| {
        let mut v: Vec<GameId> = ui.view.iter().map(|&i| lib.games[i].id).collect();
        v.sort();
        v
    };
    ui.filter_by(FilterValue::Facet(Facet::Completion, "Playing".into()), false);
    ui.refresh_view_for(&lib);
    assert_eq!(ids(&ui), [2, 3]);
    ui.filter_by(FilterValue::Facet(Facet::Year, "2023".into()), true);
    ui.refresh_view_for(&lib);
    assert_eq!(ids(&ui), [2], "Ctrl+click adds to the filter");
    assert_eq!(ui.active_filters().len(), 2);
    ui.filter_by(FilterValue::Source(SrcKey::source(Source::Steam)), false);
    assert_eq!(ui.active_filters().len(), 1, "a plain click replaces the filter");
    ui.unfilter(&FilterValue::Source(SrcKey::source(Source::Steam)));
    assert!(ui.no_extra_filters());

    // Persisted with the explorer state.
    ui.facet_sel[Facet::Completion.index()] = vec!["playing".into()];
    ui.facet_sel[Facet::Year.index()] = vec!["2023".into()];
    let json = serde_json::to_string(&ui.explorer_state()).unwrap();
    let mut back = DesktopUi::new();
    back.apply_explorer_state(&serde_json::from_str(&json).unwrap());
    assert_eq!(back.facet_sel, ui.facet_sel);
}

#[test]
fn multi_selection_hide_and_not_a_game_keep_the_users_place() {
    use crate::db::Db;
    let dir = tempfile::tempdir().unwrap();
    let mut st = AppState::for_test(dir.path());
    let scanned: Vec<ScannedGame> = ["A", "B", "C", "D", "E"]
        .iter()
        .map(|n| ScannedGame::new(Source::Desktop, format!("{n}.desktop"), *n))
        .collect();
    Db::open(&st.paths.db_file()).unwrap().merge_scan(&scanned, &[Source::Desktop]).unwrap();
    st.reload_library();
    let id = |st: &AppState, n: &str| st.library.games.iter().chain(&st.excl.games).find(|g| g.name == n).unwrap().id;
    let ctx = st.ctx().clone();
    let mut ui = DesktopUi::new();
    ui.selected = Some(id(&st, "B"));
    ui.refresh_view_for(&st.library);
    // Multi-selection B, C (focused C): Hide → C's place goes to D.
    ui.push(Action::SelectToggle(id(&st, "C")));
    ui.apply_actions(&ctx, &mut st);
    assert_eq!(ui.multi.len(), 2);
    let ids = ui.multi_ids(&st);
    assert_eq!(ids, [id(&st, "B"), id(&st, "C")], "view order");
    ui.push(Action::HideMany(ids.clone()));
    ui.apply_actions(&ctx, &mut st);
    assert!(ui.multi.is_empty());
    assert_eq!(ui.selected, Some(id(&st, "D")));
    assert!(matches!(&ui.undo, Some(Undo { what: UndoWhat::Hidden(h), .. }) if *h == ids));
    assert!(ids.iter().all(|&i| st.library.get(i).unwrap().hidden));
    // Not a game on the last one: the selection steps back to the one before.
    ui.refresh_view_for(&st.library);
    let e = id(&st, "E");
    ui.push(Action::Select(e));
    ui.push(Action::NotAGame(vec![e]));
    ui.apply_actions(&ctx, &mut st);
    assert_eq!(ui.selected, Some(id(&st, "D")));
    assert!(st.library.get(e).is_none(), "out of the library");
    let Some(Undo { what: UndoWhat::Marked(u), .. }) = &ui.undo else { panic!("undo") };
    st.undo_marks(&u.clone());
    assert!(st.library.get(e).is_some(), "Undo brings it back");
    // A rule that matches nothing yet still says it was added.
    let rule = crate::config::ExcludeRule { pattern: "Zzz*".into(), ..Default::default() };
    ui.rule_added(&ctx, &mut st, rule.clone(), e);
    assert!(matches!(&ui.undo, Some(u) if u.text.starts_with("Rule added")));
    // The same rule again: nothing added, so no Undo that would take the first one away.
    ui.undo = None;
    ui.rule_added(&ctx, &mut st, rule.clone(), e);
    assert!(ui.undo.is_none());
    assert_eq!(st.config.exclude.rules, [rule]);
}

#[test]
fn cloud_games_are_listed_by_service_under_sources() {
    let mut gfn = fake_game(1, "Cyberpunk 2077", Source::Manual);
    gfn.source_id = "cloud:gfn:auto:fs:1-a".into();
    let mut xbox = fake_game(2, "Forza Horizon 5", Source::Manual);
    xbox.source_id = "cloud:xbox:tab:fs:2-b".into();
    let own = fake_game(4, "My Game", Source::Manual);
    let steam = fake_game(5, "Celeste", Source::Steam);
    let lib = Library::new(vec![gfn, xbox, own, steam]);
    let c = Counts::compute(&lib);
    let labels: Vec<(&str, usize)> = c.sources.iter().map(|(k, n)| (k.label(), *n)).collect();
    assert_eq!(labels, [("GeForce NOW", 1), ("Manual", 1), ("Steam", 1), ("Xbox Cloud Gaming", 1)]);

    let mut ui = DesktopUi::new();
    ui.filter.show_hidden = true;
    let pick = |ui: &mut DesktopUi, key: &str| {
        ui.src_sel = vec![SrcKey::from_key(key).unwrap()];
        ui.refresh_view_for(&lib);
        ui.view.iter().map(|&i| lib.games[i].id).collect::<Vec<_>>()
    };
    assert_eq!(pick(&mut ui, "cloud-gfn"), [1]);
    assert_eq!(pick(&mut ui, "manual"), [4], "Manual no longer includes cloud games");
    // Saved and restored like any source.
    let json = serde_json::to_string(&ui.explorer_state()).unwrap();
    let mut back = DesktopUi::new();
    back.apply_explorer_state(&serde_json::from_str(&json).unwrap());
    assert_eq!(back.src_sel, [SrcKey("manual")]);
    assert_eq!(SrcKey::from_key("cloud-xbox").map(SrcKey::label), Some("Xbox Cloud Gaming"));
}
