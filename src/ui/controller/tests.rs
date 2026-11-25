use super::*;

#[test]
fn closing_and_tray_settings_cycle() {
    use crate::config::WhenClosing;
    let dir = tempfile::tempdir().unwrap();
    let mut st = AppState::for_test(dir.path());
    let mut ui = ControllerUi::new();
    let value = |st: &AppState, label: &str| {
        ControllerUi::launchers_items(st).into_iter().find(|i| i.label == label).and_then(|i| i.value).unwrap()
    };
    // No tray (tests never have one): automatic is "minimise while playing".
    assert_eq!(value(&st, "Closing Lodestar"), "Minimise while playing");
    assert_eq!(value(&st, "System tray icon"), "On");
    ui.menu_act(&mut st, Act::CycleCloseWhilePlaying, 0.0);
    assert_eq!(st.config.when_closing, Some(WhenClosing::Quit));
    ui.menu_act(&mut st, Act::CycleCloseWhilePlaying, 0.0);
    ui.menu_act(&mut st, Act::CycleCloseWhilePlaying, 0.0);
    assert_eq!(st.config.when_closing, Some(WhenClosing::TrayWhilePlaying), "wraps around");
    ui.menu_act(&mut st, Act::ToggleTrayIcon, 0.0);
    assert!(!st.config.tray_icon);
    assert_eq!(value(&st, "System tray icon"), "Off");
    // Saved.
    let saved = crate::config::Config::load(&st.paths.config_file());
    assert_eq!((saved.when_closing, saved.tray_icon), (Some(WhenClosing::TrayWhilePlaying), false));
}

#[test]
fn steam_settings_in_the_launchers_menu() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = AppState::for_test(dir.path());
    let mut ui = ControllerUi::new();
    let value = |st: &AppState, label: &str| {
        ControllerUi::launchers_items(st).into_iter().find(|i| i.label == label).and_then(|i| i.value).unwrap()
    };
    assert_eq!(value(&st, "Minimise Steam while playing"), "On");
    assert_eq!(value(&st, "Close Steam when the game ends"), "Off");
    assert_eq!(value(&st, "Keep Steam for controllers"), "Off");
    ui.menu_act(&mut st, Act::ToggleHideSteam, 0.0);
    ui.menu_act(&mut st, Act::ToggleCloseSteam, 0.0);
    assert_eq!(value(&st, "Minimise Steam while playing"), "Off");
    assert_eq!(value(&st, "Close Steam when the game ends"), "On");
    // Keeping Steam for controllers turns closing it off.
    ui.menu_act(&mut st, Act::ToggleKeepSteam, 0.0);
    assert_eq!(value(&st, "Keep Steam for controllers"), "On");
    assert_eq!(value(&st, "Close Steam when the game ends"), "Off");
    let saved = crate::config::Config::load(&st.paths.config_file());
    assert!(
        !saved.launchers.hide_steam && saved.launchers.keep_steam_for_controller && !saved.launchers.close_steam_after
    );
}

#[test]
fn grid_and_carousel_steps() {
    let g = FocusKind::Grid { columns: 4 };
    assert_eq!(step(g, Nav::Right, 3, 10, 8, false), Some(4));
    assert_eq!(step(g, Nav::Down, 1, 10, 8, false), Some(5));
    assert_eq!(step(g, Nav::Down, 5, 10, 8, false), Some(9)); // partial last row
    assert_eq!(step(g, Nav::Down, 9, 10, 8, false), Some(9));
    assert_eq!(step(g, Nav::Up, 2, 10, 8, false), Some(2));
    assert_eq!(step(g, Nav::PageDown, 0, 10, 8, false), Some(8));
    let c = FocusKind::Carousel { vertical: false };
    assert_eq!(step(c, Nav::Left, 0, 5, 3, true), Some(4));
    assert_eq!(step(c, Nav::Left, 0, 5, 3, false), Some(0));
    assert_eq!(step(c, Nav::Up, 2, 5, 3, false), None);
    assert_eq!(step(FocusKind::List, Nav::Down, 4, 5, 3, false), Some(4));
    assert_eq!(step(FocusKind::List, Nav::Down, 0, 0, 3, false), None);
}

fn g(id: i64, name: &str) -> Game {
    Game { id, source: Source::Steam, source_id: id.to_string(), ..Game::blank(name) }
}

#[test]
fn letter_jumps() {
    let games =
        vec![g(1, "Alan Wake"), g(2, "Amnesia"), g(3, "Bastion"), g(4, "Celeste"), g(5, "Cuphead"), g(6, "2064")];
    let list = vec![5, 0, 1, 2, 3, 4]; // "2064" sorts first ('#')
    assert_eq!(jump_letter(&games, &list, 0, true), 1);
    assert_eq!(jump_letter(&games, &list, 1, true), 3);
    assert_eq!(jump_letter(&games, &list, 3, true), 4);
    assert_eq!(jump_letter(&games, &list, 5, true), 5);
    assert_eq!(jump_letter(&games, &list, 5, false), 4);
    assert_eq!(jump_letter(&games, &list, 4, false), 3);
    assert_eq!(jump_letter(&games, &list, 2, false), 1);
    assert_eq!(jump_letter(&games, &list, 1, false), 0);
}

fn lib_of(games: &[(i64, &str)]) -> Library {
    Library::new(games.iter().map(|&(id, n)| g(id, n)).collect())
}

fn open(ui: &mut ControllerUi, lib: &Library, key: &str) {
    ui.refresh_collections(lib);
    let i = ui.colls.iter().position(|c| c.collection.key() == key).expect("collection");
    ui.coll_sel = i;
    ui.coll_key = Some(key.into());
    ui.enter_collection(lib, i, 0.0);
}

fn sel_name(ui: &ControllerUi, lib: &Library) -> Option<String> {
    ui.sel_id().and_then(|id| lib.get(id)).map(|g| g.name.clone())
}

/// Lists, ids and selection always agree with the current library.
fn assert_consistent(ui: &ControllerUi, lib: &Library) {
    assert_eq!(ui.list.len(), ui.list_ids.len());
    for (&i, &id) in ui.list.iter().zip(&ui.list_ids) {
        assert_eq!(lib.games.get(i).map(|g| g.id), Some(id));
    }
    assert!(ui.list.is_empty() || ui.game_sel < ui.list.len());
    assert!(ui.colls.is_empty() || ui.coll_sel < ui.colls.len());
    assert!(ui.cur.is_none_or(|c| c < ui.colls.len()));
    for v in &ui.colls_games {
        assert!(v.iter().all(|&i| i < lib.games.len()));
    }
}

/// Regression: a rescan that removed games while a list was open indexed the new, shorter
/// `library.games` with the old list's indices and panicked.
#[test]
fn rescan_shrinking_library_under_open_list() {
    let mut lib = lib_of(&[(1, "Alpha"), (2, "Bravo"), (3, "Charlie")]);
    let mut ui = ControllerUi::new();
    open(&mut ui, &lib, "all");
    ui.game_sel = 2;
    assert_eq!(sel_name(&ui, &lib).as_deref(), Some("Charlie"));
    lib.replace(vec![g(3, "Charlie")]);
    ui.refresh_list(&lib);
    assert_consistent(&ui, &lib);
    assert_eq!(sel_name(&ui, &lib).as_deref(), Some("Charlie"));
    assert_eq!(ui.screen, Screen::Games);

    // The selected game itself disappears: the selection clamps to a game that exists.
    let mut lib2 = lib_of(&[(1, "Alpha"), (2, "Bravo"), (3, "Charlie"), (4, "Delta")]);
    let mut ui = ControllerUi::new();
    open(&mut ui, &lib2, "all");
    ui.game_sel = 3;
    lib2.replace(vec![g(1, "Alpha")]);
    ui.refresh_list(&lib2);
    assert_consistent(&ui, &lib2);
    assert_eq!(sel_name(&ui, &lib2).as_deref(), Some("Alpha"));

    // A search result list and the details page survive it too.
    let mut lib3 = lib_of(&[(1, "Alpha"), (2, "Alphabet"), (3, "Alpine")]);
    let mut ui = ControllerUi::new();
    open(&mut ui, &lib3, "all");
    ui.search = "alp".into();
    ui.refresh_list(&lib3);
    ui.game_sel = ui.list.len() - 1;
    let want = ui.sel_id();
    ui.screen = Screen::Details;
    lib3.replace(vec![g(3, "Alpine"), g(2, "Alphabet")]);
    ui.refresh_list(&lib3);
    assert_consistent(&ui, &lib3);
    if want.is_some_and(|id| lib3.get(id).is_some()) {
        assert_eq!(ui.sel_id(), want);
    }

    // Everything goes: back to the (empty) collections screen, nothing to index.
    lib3.replace(vec![]);
    ui.refresh_list(&lib3);
    assert_consistent(&ui, &lib3);
    assert!(ui.colls.is_empty() && ui.list.is_empty());
    assert_eq!(ui.screen, Screen::Collections);
    assert!(jump_letter(&lib3.games, &ui.list, ui.game_sel, true) == 0);
}

/// A rescan that adds a game sorting first and reorders `library.games` keeps the same
/// game (and collection) selected.
#[test]
fn selection_survives_rescan_adding_and_reordering() {
    let mut lib = lib_of(&[(1, "Bravo"), (2, "Charlie"), (3, "Delta")]);
    let mut ui = ControllerUi::new();
    open(&mut ui, &lib, "all");
    ui.game_sel = 1;
    assert_eq!(sel_name(&ui, &lib).as_deref(), Some("Charlie"));
    lib.replace(vec![g(3, "Delta"), g(9, "Alpha"), g(1, "Bravo"), g(2, "Charlie")]);
    ui.refresh_list(&lib);
    assert_consistent(&ui, &lib);
    assert_eq!(sel_name(&ui, &lib).as_deref(), Some("Charlie"));
    assert_eq!(ui.cur_key().as_deref(), Some("all"));

    // A new collection appearing before the open one keeps the open one by key.
    let mut fav = g(5, "Echo");
    fav.favorite = true;
    let mut games = lib.games.clone();
    games.push(fav);
    lib.replace(games);
    ui.refresh_list(&lib);
    assert_consistent(&ui, &lib);
    assert_eq!(ui.cur_key().as_deref(), Some("all"));
    assert_eq!(ui.colls[ui.coll_sel].collection.key(), "all");
    assert_eq!(sel_name(&ui, &lib).as_deref(), Some("Charlie"));
}

/// Hidden games can be found again: the Hidden Games collection lists them (and only
/// them) while "Show hidden games" is on, and disappears when it is turned off.
#[test]
fn hidden_games_collection() {
    let mut games = vec![g(1, "Alpha"), g(2, "Bravo"), g(3, "Charlie")];
    games[1].hidden = true;
    let mut lib = Library::new(games);
    let mut ui = ControllerUi::new();
    ui.refresh_collections(&lib);
    let hkey = hidden_collection().key();
    assert!(ui.colls.iter().all(|c| c.collection.key() != hkey));
    ui.show_hidden = true;
    open(&mut ui, &lib, &hkey);
    assert!(ui.cur_is_hidden());
    assert_eq!(ui.colls[ui.cur.unwrap()].name, "Hidden Games");
    assert_eq!(sel_name(&ui, &lib).as_deref(), Some("Bravo"));
    assert_eq!(ui.list.len(), 1);
    // Unhide: the list empties, then the collection goes and the UI falls back.
    lib.update(2, |g| g.hidden = false);
    ui.refresh_list(&lib);
    assert_consistent(&ui, &lib);
    assert!(ui.cur.is_none());
    assert_eq!(ui.screen, Screen::Collections);
    // Off again: no Hidden collection even with hidden games.
    lib.update(1, |g| g.hidden = true);
    ui.show_hidden = false;
    ui.refresh_list(&lib);
    assert!(ui.colls.iter().all(|c| c.collection.key() != hkey));
}

#[test]
fn button_prompts_setting_cycles_and_wins_over_detection() {
    use crate::config::ButtonPrompts;
    use crate::input::ButtonStyle;
    let dir = tempfile::tempdir().unwrap();
    let mut st = AppState::for_test(dir.path());
    let mut ui = ControllerUi::new();
    let value = |st: &AppState, ui: &ControllerUi| {
        let _ = ui;
        ControllerUi::settings_items(st, 1)
            .into_iter()
            .find(|i| i.label == "Button prompts")
            .and_then(|i| i.value)
            .unwrap()
    };
    assert_eq!(st.config.controller.button_prompts, ButtonPrompts::Auto);
    // Automatic names what it picked (whatever pad this machine has, if any).
    assert!(value(&st, &ui).starts_with("Automatic ("), "{}", value(&st, &ui));
    for (want, style) in [
        (ButtonPrompts::Xbox, ButtonStyle::Xbox),
        (ButtonPrompts::PlayStation, ButtonStyle::PlayStation),
        (ButtonPrompts::Nintendo, ButtonStyle::Nintendo),
    ] {
        ui.menu_act(&mut st, Act::CycleButtonPrompts, 0.0);
        assert_eq!(st.config.controller.button_prompts, want);
        assert_eq!(st.prompts().style, style, "manual choice wins");
        assert_eq!(value(&st, &ui), want.label());
    }
    let saved = crate::config::Config::load(&st.paths.config_file());
    assert_eq!(saved.controller.button_prompts, ButtonPrompts::Nintendo);
    // Swap moves confirm to the east button: Nintendo calls that one A.
    ui.menu_act(&mut st, Act::ToggleSwapAb, 0.0);
    assert_eq!(st.prompts().name(Button::A), "A");
    ui.menu_act(&mut st, Act::CycleButtonPrompts, 0.0);
    assert_eq!(st.config.controller.button_prompts, ButtonPrompts::Auto);
}

fn emulated(id: i64, name: &str, platform: &str) -> Game {
    Game {
        source: Source::Emulator,
        emu: Some(EmuRef { platform: platform.into(), emulator: "pcsx2".into(), ..Default::default() }),
        ..g(id, name)
    }
}

/// Consoles get their own collections after the sources, named without the maker.
#[test]
fn console_collections() {
    let lib = Library::new(vec![
        g(1, "Celeste"),
        emulated(2, "Tekken 5", "sony_playstation2"),
        emulated(3, "Gran Turismo 4", "sony_playstation2"),
        emulated(4, "Metroid Prime", "nintendo_gamecube"),
        emulated(5, "Mario Kart 8", "nintendo_wiiu"),
    ]);
    let mut ui = ControllerUi::new();
    ui.refresh_collections(&lib);
    let keys: Vec<String> = ui.colls.iter().map(|c| c.collection.key()).collect();
    let pos = |k: &str| keys.iter().position(|x| x == k).unwrap_or_else(|| panic!("{k} in {keys:?}"));
    // One PC game: no Steam collection of its own, but PC Games (there are emulated games too).
    assert!(!keys.iter().any(|k| k == "steam"), "{keys:?}");
    assert!(pos("pc") < pos("emulator") && pos("emulator") < pos("platform:nintendo_gamecube"));
    assert!(pos("platform:nintendo_gamecube") < pos("platform:nintendo_wiiu"));
    assert!(pos("platform:nintendo_wiiu") < pos("platform:sony_playstation2"));
    let item = |k: &str| &ui.colls[pos(k)];
    assert_eq!(item("platform:nintendo_gamecube").name, "GameCube");
    assert_eq!(item("platform:nintendo_wiiu").name, "Wii U");
    assert_eq!(
        (item("platform:sony_playstation2").name.as_str(), item("platform:sony_playstation2").count),
        ("PlayStation 2", 2)
    );
    // Short names stay whole when the maker is part of the name.
    assert_eq!(console_label("nintendo_switch"), "Switch");
    assert_eq!(console_label("unknown_console"), "unknown_console");
}

#[test]
fn emulated_games_quick_menu() {
    use crate::emulation::detect::{Emulator, InstallKind};
    use crate::emulation::registry::Registry;
    let pcsx2 = Emulator {
        id: "pcsx2".into(),
        def: "pcsx2".into(),
        name: "PCSX2".into(),
        kind: InstallKind::Flatpak { app_id: "net.pcsx2.PCSX2".into() },
        how: "Flatpak".into(),
        user: false,
        cores: vec![],
        args: String::new(),
        auto: false,
        blocked: false,
    };
    let reg = Registry {
        emulators: vec![pcsx2],
        config: crate::config::EmulationConfig::detect_nothing(),
        home: "/nonexistent".into(),
    };
    let game = emulated(2, "Tekken 5", "sony_playstation2");
    let labels = |reg: Option<&Registry>, g: &Game| -> Vec<(String, Option<Act>)> {
        super::emu::quick_items(reg, g).into_iter().map(|i| (i.label, i.act)).collect()
    };
    assert_eq!(
        labels(Some(&reg), &game),
        vec![
            ("Play in a window".to_string(), Some(Act::PlayWindowed(2))),
            ("PCSX2 settings…".to_string(), Some(Act::OpenEmulator(2))),
        ]
    );
    // A console no found emulator runs (scanned with one that isn't there), no registry yet, or
    // not emulated: nothing.
    let mut other = emulated(3, "Mario Kart 8", "nintendo_wiiu");
    other.emu.as_mut().unwrap().emulator = "cemu".into();
    assert!(labels(Some(&reg), &other).is_empty());
    assert!(labels(None, &game).is_empty());
    assert!(labels(Some(&reg), &g(1, "Celeste")).is_empty());
    assert_eq!(super::emu::source_label(&game), "Sony PlayStation 2");
    assert_eq!(super::emu::source_label(&g(1, "Celeste")), "Steam");
    // Without a registry the actions say what is missing instead of starting anything.
    let dir = tempfile::tempdir().unwrap();
    let mut st = AppState::for_test(dir.path());
    st.library.replace(vec![game]);
    let mut ui = ControllerUi::new();
    ui.menu_act(&mut st, Act::OpenEmulator(2), 0.0);
    ui.menu_act(&mut st, Act::PlayWindowed(2), 0.0);
}

#[test]
fn clock_corner_follows_the_view_on_screen() {
    use crate::theme::ViewKind;
    let dir = tempfile::tempdir().unwrap();
    let mut st = AppState::for_test(dir.path());
    st.set_controller_theme("Fullscreen");
    st.config.controller.show_clock = true;
    st.config.controller.show_battery = true;
    let clock = |st: &AppState, v| settings::status_text(st, None, v);
    // Its home / grid pages draw a clock: the corner leaves the time to them.
    assert_eq!(clock(&st, Some(ViewKind::Home)), None);
    assert_eq!(clock(&st, Some(ViewKind::Games)), None);
    // The built-in details page (no theme view) and the empty library show the corner's clock.
    assert!(clock(&st, None).is_some_and(|t| t.contains(':')));
    // Classic draws its clock on the collections screen only; its game list was laid out
    // without one (the corner holds "1 / 30"), so the corner stays empty there too.
    st.set_controller_theme("Classic");
    assert_eq!(clock(&st, Some(ViewKind::Collections)), None);
    assert_eq!(clock(&st, Some(ViewKind::Games)), None);
    // The battery: every built-in theme shows it next to its clock (so the corner never
    // doubles it or covers the theme's top-right corner); the built-in details page gets it in
    // the corner.
    for t in ["Lodestar", "Shelf", "Classic", "Fullscreen"] {
        st.set_controller_theme(t);
        assert_eq!(settings::status_text(&st, Some((80, false)), Some(ViewKind::Collections)), None, "{t}");
    }
    let with_battery = settings::status_text(&st, Some((80, false)), None).unwrap();
    assert!(with_battery.contains("Battery 80 %"), "{with_battery}");
    st.config.controller.show_clock = false;
    st.config.controller.show_battery = false;
    assert_eq!(settings::status_text(&st, Some((80, false)), None), None);
}

#[test]
fn controller_theme_list_has_no_desktop_only_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let st = AppState::for_test(dir.path());
    let names: Vec<String> = ControllerUi::theme_items(&st).into_iter().map(|i| i.label).collect();
    for n in ["Lodestar", "Shelf", "Classic", "Fullscreen"] {
        assert!(names.iter().any(|x| x == n), "{n} missing from {names:?}");
    }
    assert!(!names.iter().any(|x| x == "Lodestar Ambient"), "identical to Lodestar in controller mode: {names:?}");
}

#[test]
fn hints_name_what_a_does_on_the_built_in_details_page() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = AppState::for_test(dir.path());
    st.set_controller_theme("Lodestar");
    st.library.replace(vec![Game { installed: true, ..g(1, "Alpha") }]);
    let mut ui = ControllerUi::new();
    ui.refresh_collections(&st.library);
    ui.screen = Screen::Details;
    ui.details = Some(details::DetailsState::new(1));
    let a =
        |ui: &ControllerUi, st: &AppState| ui.hints(st).into_iter().find(|h| h.button == Button::A).map(|h| h.label);
    assert_eq!(a(&ui, &st).as_deref(), Some("Play"));
    ui.details.as_mut().unwrap().focus = 2;
    assert_eq!(a(&ui, &st).as_deref(), Some("Artwork"), "A presses the focused button");
    ui.details.as_mut().unwrap().focus = 3;
    assert_eq!(a(&ui, &st).as_deref(), Some("Options"));
}

/// The Lodestar theme with filter tabs over its game list the way imported Playnite themes draw
/// them (each tab a text that a `{collection.key} == …` rule highlights), plus `extra` elements.
fn tabs_theme(st: &AppState, extra: &str) -> Arc<Theme> {
    let mut t = (*st.themes.load("Lodestar")).clone();
    t.key = "tabs".into();
    let mut tab = String::new();
    for (key, label) in [("all", "All Games"), ("recent", "Continue Playing"), ("favorites", "Favourites")] {
        tab += &format!(
            "[[children]]\ntype = \"text\"\ntext = \"{label}\"\nhidden_if = \"{{collection.key}} == {key}\"\n\
             [[children]]\ntype = \"text\"\ntext = \"{label}\"\nvisible_if = \"{{collection.key}} == {key}\"\n"
        );
    }
    tab += "[[children]]\ntype = \"text\"\ntext = \"Installed\"\nvisible_if = \"{collection.key} == installed\"\n";
    let stack = format!("type = \"stack\"\ndirection = \"horizontal\"\npos = [0.3, 0.05]\n{tab}");
    for src in [stack.as_str(), extra].into_iter().filter(|s| !s.is_empty()) {
        let el: crate::theme::model::Element = toml::from_str(src).unwrap_or_else(|e| panic!("{e}\n{src}"));
        t.file.games.elements.push(el);
    }
    Arc::new(t)
}

/// Alpha: installed, favourite, played; Bravo: played; Charlie: neither; Delta: installed.
fn tabs_setup(extra: &str) -> (tempfile::TempDir, AppState, ControllerUi) {
    let dir = tempfile::tempdir().unwrap();
    let mut st = AppState::for_test(dir.path());
    st.library.replace(vec![
        Game { installed: true, favorite: true, last_played: Some(1_700_000_200), ..g(1, "Alpha") },
        Game { last_played: Some(1_700_000_100), ..g(2, "Bravo") },
        g(3, "Charlie"),
        Game { installed: true, ..g(4, "Delta") },
    ]);
    st.controller_theme = tabs_theme(&st, extra);
    let mut ui = ControllerUi::new();
    open(&mut ui, &st.library, "all");
    (dir, st, ui)
}

fn press(ui: &mut ControllerUi, st: &mut AppState, nav: Nav) -> String {
    let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0));
    ui.handle_nav(st, nav, screen, 1.0, (false, false));
    ui.refresh_list(&st.library);
    ui.cur_key().unwrap_or_default()
}

#[test]
fn shoulder_buttons_walk_a_themes_filter_tabs_in_order_and_wrap() {
    let (_d, mut st, mut ui) = tabs_setup("");
    assert_eq!(ui.screen, Screen::Games);
    let mut seen = vec![];
    for _ in 0..4 {
        seen.push(press(&mut ui, &mut st, Nav::PageDown));
    }
    assert_eq!(seen, ["recent", "favorites", "installed", "all"], "RB: the tabs in the theme's order, round");
    assert_eq!(press(&mut ui, &mut st, Nav::PageUp), "installed", "LB from the first tab: the last");
    assert_eq!(press(&mut ui, &mut st, Nav::PageUp), "favorites");
    assert_eq!(ui.screen, Screen::Games);
    assert_consistent(&ui, &st.library);
}

#[test]
fn shoulder_buttons_skip_tabs_without_games_and_leave_other_collections_for_the_first_tab() {
    let (_d, mut st, mut ui) = tabs_setup("");
    // No favourites: that tab has no collection behind it.
    let mut games = st.library.games.clone();
    games.iter_mut().for_each(|g| g.favorite = false);
    st.library.replace(games);
    ui.refresh_list(&st.library);
    assert_eq!(press(&mut ui, &mut st, Nav::PageDown), "recent");
    assert_eq!(press(&mut ui, &mut st, Nav::PageDown), "installed");
    // A collection with no tab of its own (it shows as an extra tab after the theme's):
    // RB goes round to the first tab, LB back to the last.
    open(&mut ui, &st.library, "steam");
    assert_eq!(press(&mut ui, &mut st, Nav::PageDown), "all");
    open(&mut ui, &st.library, "steam");
    assert_eq!(press(&mut ui, &mut st, Nav::PageUp), "installed");
}

#[test]
fn shoulder_buttons_page_the_list_on_themes_without_tabs() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = AppState::for_test(dir.path());
    st.set_controller_theme("Lodestar");
    st.library.replace((1..=40).map(|i| Game { installed: true, ..g(i, &format!("Game {i:02}")) }).collect());
    let mut ui = ControllerUi::new();
    open(&mut ui, &st.library, "all");
    assert_eq!(press(&mut ui, &mut st, Nav::PageDown), "all", "no tabs: the collection stays");
    assert!(ui.game_sel > 0, "RB pages down the list");
    assert!(!ui.hints(&st).iter().any(|h| matches!(h.button, Button::LB | Button::RB)), "and no tab hint");
}

#[test]
fn the_hint_bar_names_the_tab_buttons_unless_the_theme_draws_them() {
    let lb =
        |ui: &ControllerUi, st: &AppState| ui.hints(st).into_iter().find(|h| h.button == Button::LB).map(|h| h.label);
    let (_d, st, ui) = tabs_setup("");
    assert_eq!(lb(&ui, &st).as_deref(), Some("Tabs"));
    // The theme's own LB / RB pictures beside its tabs say it already.
    let prompts = "type = \"helpbar\"\npos = [0.25, 0.05]\nbuttons = [\"lb\"]\nglyphs = true\n";
    let (_d, st, ui) = tabs_setup(prompts);
    assert_eq!(lb(&ui, &st), None);
}
