//! Settings → Emulation, Add game → Emulated games… and Edit → Installation: the pure logic,
//! plus drawing each page once with made-up emulators (no registry is published: other tests
//! rely on none being there).

use super::catalog::{self, CatEmu, Catalog, ChipTone, LibFacts, Picked};
use super::lists;
use super::*;
use crate::config::{EmulationConfig, ProfileEntry, RomFolder};
use crate::emulation::bios::BiosState;
use crate::emulation::detect::{InstallKind, Profile};
use crate::emulation::registry::Registry;
use crate::model::{EmuRef, Game, ScannedGame, Source};

fn emu(id: &str, name: &str, user: bool) -> Emulator {
    Emulator {
        id: id.into(),
        def: id.into(),
        name: name.into(),
        kind: InstallKind::Flatpak { app_id: format!("org.test.{name}") },
        how: "Flatpak".into(),
        user,
        cores: vec![],
        args: String::new(),
        auto: false,
        blocked: false,
    }
}

fn prof(id: &str, name: &str, platforms: &[&str]) -> Profile {
    Profile {
        id: id.into(),
        name: name.into(),
        platforms: platforms.iter().map(|s| s.to_string()).collect(),
        extensions: vec!["iso".into()],
        args: "{rom}".into(),
        core: None,
        folder_launch: vec![],
        custom: false,
        ..Default::default()
    }
}

fn fake_catalog() -> Catalog {
    let emus = vec![
        CatEmu {
            emu: emu("pcsx2", "PCSX2", false),
            profiles: vec![prof("default", "Default", &["sony_playstation2"])],
        },
        CatEmu {
            emu: emu("retroarch", "RetroArch", false),
            profiles: vec![
                prof("swanstation", "SwanStation", &["sony_playstation"]),
                prof("beetle", "Beetle PSX", &["sony_playstation"]),
                prof("mgba", "mGBA core", &["nintendo_gameboyadvance"]),
            ],
        },
    ];
    let reg = Registry {
        emulators: emus.iter().map(|c| c.emu.clone()).collect(),
        config: EmulationConfig::default(),
        home: PathBuf::from("/home/me"),
    };
    let auto = [("sony_playstation2".to_string(), ("pcsx2".to_string(), "default".to_string()))].into_iter().collect();
    Catalog::from_parts(reg, emus, auto)
}

fn emu_game(id: i64, platform: &str, folder: &str, root: &str, rel: &str) -> Game {
    Game {
        id,
        source: Source::Emulator,
        source_id: format!("{folder}/{rel}"),
        installed: true,
        emu: Some(EmuRef {
            platform: platform.into(),
            folder: folder.into(),
            rom: Path::new(root).join(rel),
            ..Default::default()
        }),
        ..Game::blank(rel)
    }
}

// ------------------------------------------------------------------------- catalog

#[test]
fn catalog_lists_compatible_emulators_and_labels_profiles_only_when_needed() {
    let c = fake_catalog();
    assert_eq!(c.compatible("sony_playstation").len(), 2);
    assert_eq!(c.emulators_for("sony_playstation").len(), 1);
    assert!(c.compatible("nintendo_switch").is_empty());
    // One profile for the console: just the emulator's name.
    assert_eq!(c.choice_label("pcsx2", "default", "sony_playstation2").as_deref(), Some("PCSX2"));
    // Several: the profile too.
    assert_eq!(c.choice_label("retroarch", "beetle", "sony_playstation").as_deref(), Some("RetroArch — Beetle PSX"));
    assert_eq!(c.choice_label("nope", "x", "sony_playstation"), None);
    let covered = c.covered();
    assert!(covered.contains("nintendo_gameboyadvance") && covered.contains("sony_playstation2"));
    // Two choices for PS1 (RetroArch's two cores), one for PS2.
    assert_eq!(c.choices("sony_playstation").len(), 2);
    assert_eq!(c.choices("sony_playstation2"), vec![("pcsx2".to_string(), "default".to_string())]);
}

/// A program of some kind, at `path` (an AppImage when it ends so).
fn at(id: &str, def: &str, name: &str, path: &str) -> Emulator {
    let kind = if path.ends_with(".AppImage") {
        InstallKind::AppImage { path: path.into() }
    } else {
        InstallKind::Binary { path: path.into() }
    };
    Emulator { def: def.into(), kind, how: format!("AppImage in {path}"), auto: true, ..emu(id, name, false) }
}

/// A tester's emulators: DuckStation and an old copy of it, Eden, a Ryujinx based on Eden's
/// definition, shadPS4 (no PS4 games).
fn copies_catalog(pick: Option<(&str, &str)>) -> Catalog {
    let e = "/mnt/x/Emulation/Emulators";
    let ds = |id: &str, path: &str| CatEmu {
        emu: at(id, "duckstation", "DuckStation", path),
        profiles: vec![prof("default", "Default", &["sony_playstation"])],
    };
    let sw = |id: &str, name: &str, path: &str| CatEmu {
        emu: at(id, "eden", name, path),
        profiles: vec![prof("default", "Default", &["nintendo_switch"])],
    };
    let emus = vec![
        ds("duckstation", &format!("{e}/DuckStation.backup/DuckStation.AppImage")),
        ds("duckstation-appimage", &format!("{e}/DuckStation/DuckStation.AppImage")),
        sw("eden", "Ryujinx", &format!("{e}/Ryujinx/Ryujinx.AppImage")),
        sw("eden-2", "Eden", &format!("{e}/Eden/Eden.AppImage")),
        CatEmu {
            emu: at("shadps4", "shadps4", "shadPS4", &format!("{e}/shadPS4/shadPS4-qt.AppImage")),
            profiles: vec![prof("default", "Default", &["sony_playstation4"])],
        },
    ];
    let mut config = EmulationConfig::default();
    if let Some((pf, v)) = pick {
        config.platform_emulator.insert(pf.into(), v.into());
    }
    let reg =
        Registry { emulators: emus.iter().map(|c| c.emu.clone()).collect(), config, home: PathBuf::from("/home/me") };
    let auto = [("sony_playstation", "duckstation"), ("nintendo_switch", "eden"), ("sony_playstation4", "shadps4")]
        .into_iter()
        .map(|(p, e)| (p.to_string(), (e.to_string(), "default".to_string())))
        .collect();
    Catalog::from_parts(reg, emus, auto)
}

#[test]
fn copies_of_one_emulator_are_one_row_and_names_are_the_ones_given() {
    let c = copies_catalog(None);
    // DuckStation twice = one row; Ryujinx (based on Eden) and Eden = two rows; shadPS4.
    assert_eq!(c.groups.len(), 4, "{:?}", c.groups);
    let ds = &c.groups[0];
    assert_eq!(ds.copies.len(), 1);
    // The one PS1 uses stands for the row, even though its folder says "backup".
    assert_eq!(c.emus[ds.main].emu.id, "duckstation");
    assert!(catalog::looks_old(&c.emus[ds.main].emu));
    // Names: the row's own; a copy says its kind; Ryujinx stays Ryujinx.
    assert_eq!(c.name_of(&c.emus[0].emu), "DuckStation");
    assert_eq!(c.name_of(&c.emus[1].emu), "DuckStation — DuckStation.AppImage");
    assert_eq!(c.emus[0].emu.copy_label(), "DuckStation — DuckStation.backup");
    assert_eq!(c.name_by_id("eden"), "Ryujinx");
    assert_eq!(c.name_by_id("eden-2"), "Eden");
    // A copy that isn't used and doesn't look old wins over one that does.
    let c2 = copies_catalog(Some(("sony_playstation", "duckstation-appimage/default")));
    assert_eq!(c2.emus[c2.groups[0].main].emu.id, "duckstation-appimage");
    // Not used anywhere: the one that doesn't look old.
    let mut emus = c.emus.clone();
    emus.swap(0, 1);
    let g = catalog::group(&emus, &Default::default());
    assert_eq!(emus[g[0].main].emu.id, "duckstation-appimage");
}

#[test]
fn only_consoles_with_games_are_listed_and_choices_need_two_emulators() {
    let c = copies_catalog(None);
    let mut facts = LibFacts::default();
    for pf in ["sony_playstation", "nintendo_switch", "nintendo_gamecube"] {
        facts.per_platform.insert(pf.into(), 3);
    }
    let l = ConsoleLists::new(&facts, Some(&c));
    // shadPS4 runs PS4 but there are no PS4 games: no console row (its emulator row stays).
    assert!(!l.with_games.contains(&"sony_playstation4".to_string()));
    assert_eq!(l.needs, vec!["nintendo_gamecube".to_string()]);
    // Switch: Ryujinx and Eden. PS1: DuckStation and its copy = no choice.
    assert_eq!(l.choice, vec!["nintendo_switch".to_string()]);
    assert_eq!(c.choices("sony_playstation").len(), 1);
    // Picked a copy ("Use this copy"): it stands for the row now, still no choice (the other
    // copy's "Use this copy" goes back).
    let c2 = copies_catalog(Some(("sony_playstation", "duckstation-appimage/default")));
    assert_eq!(c2.choices("sony_playstation"), vec![("duckstation-appimage".to_string(), "default".to_string())]);
    assert_eq!(c2.uses("sony_playstation"), Some(("duckstation-appimage".into(), "default".into())));
    assert_eq!(c.uses("sony_playstation"), Some(("duckstation".into(), "default".into())));
    // The emulator rows: those with games first, then by name.
    let order: Vec<String> =
        lists::group_order(&c, &facts).into_iter().map(|g| c.emus[c.groups[g].main].emu.name.clone()).collect();
    assert_eq!(order, ["DuckStation", "Eden", "Ryujinx", "shadPS4"]);
    // Still looking: no catalog, so nothing "needs" an emulator yet.
    assert!(ConsoleLists::new(&facts, None).needs.is_empty());
}

#[test]
fn removed_emulators_are_named_from_their_ids() {
    assert_eq!(lists::hidden_name("duckstation-appimage-2"), "DuckStation (AppImage, second copy)");
    assert_eq!(lists::hidden_name("duckstation-appimage"), "DuckStation (AppImage)");
    assert_eq!(lists::hidden_name("dolphin-flatpak-3"), "Dolphin (Flatpak, third copy)");
    assert_eq!(lists::hidden_name("dosbox-staging"), "DOSBox Staging");
    assert_eq!(lists::hidden_name("pcsx2"), "PCSX2");
    assert_eq!(lists::hidden_name("my-own"), "my-own");
}

#[test]
fn a_game_without_a_registry_choice_keeps_its_scanned_emulator() {
    let c = fake_catalog();
    let mut g = emu_game(1, "sony_playstation2", "f1", "/roms", "ps2/a.iso");
    g.emu.as_mut().unwrap().emulator = "pcsx2".into();
    g.emu.as_mut().unwrap().profile = "default".into();
    let (e, p) = c.game_emulator(&g).expect("scan-time emulator");
    assert_eq!(e.id, "pcsx2");
    assert_eq!(p.map(|p| p.id).as_deref(), Some("default"));
    // A choice of an emulator that can't run this console (no PS2 profile) is ignored, as at
    // play time (registry::choose), so the page shows what Play really starts.
    g.edits.emulator = Some("retroarch".into());
    assert_eq!(c.game_emulator(&g).unwrap().0.id, "pcsx2");
    // Not emulated: nothing.
    assert!(c.game_emulator(&Game::blank("x")).is_none());
}

#[test]
fn folder_roots_come_from_the_games_paths() {
    assert_eq!(
        catalog::root_of("auto-1234/ps2/Game (USA).chd", Path::new("/mnt/games/ps2/Game (USA).chd")),
        Some(PathBuf::from("/mnt/games"))
    );
    // Folder games: the folder is the ROM.
    assert_eq!(
        catalog::root_of("auto-1/wiiu/Mario [ARDE01]", Path::new("/r/wiiu/Mario [ARDE01]")),
        Some(PathBuf::from("/r"))
    );
    assert_eq!(catalog::root_of("auto-1/ps2/a.iso", Path::new("/elsewhere/b.iso")), None);
    assert_eq!(catalog::root_of("no-slash", Path::new("/r/a")), None);
}

#[test]
fn library_facts_count_visible_emulated_games() {
    let mut hidden = emu_game(3, "sony_playstation2", "f1", "/mine", "c.iso");
    hidden.hidden = true;
    let games = vec![
        emu_game(1, "sony_playstation2", "auto-aa", "/mnt/games", "ps2/a.iso"),
        emu_game(2, "sony_playstation", "auto-aa", "/mnt/games", "ps1/b.cue"),
        hidden,
        Game { id: 4, source: Source::Steam, ..Game::blank("Steam game") },
    ];
    let f = LibFacts::compute(&games, 7);
    assert_eq!(f.generation, 7);
    assert_eq!(f.per_platform.get("sony_playstation2"), Some(&1));
    assert_eq!(f.per_platform.get("sony_playstation"), Some(&1));
    assert_eq!(f.per_folder.get("auto-aa"), Some(&2));
    assert!(!f.per_folder.contains_key("f1"));
    assert_eq!(f.auto_roots.get("auto-aa"), Some(&PathBuf::from("/mnt/games")));
    assert_eq!(f.folder_platforms["auto-aa"].len(), 2);
}

#[test]
fn new_keys_never_collide() {
    let mut cfg = EmulationConfig::default();
    assert_eq!(catalog::next_folder_key(&cfg.rom_folders), "f1");
    cfg.rom_folders.insert("f1".into(), RomFolder::default());
    cfg.rom_folders.insert("f3".into(), RomFolder::default());
    assert_eq!(catalog::next_folder_key(&cfg.rom_folders), "f2");

    let found = vec!["pcsx2".to_string()];
    assert_eq!(catalog::new_emulator_key("pcsx2", &cfg, &found), "pcsx2-2");
    assert_eq!(catalog::new_emulator_key("dolphin", &cfg, &found), "dolphin");
    assert_eq!(catalog::new_emulator_key("", &cfg, &found), "custom");
    cfg.emulators.insert("dolphin".into(), Default::default());
    assert_eq!(catalog::new_emulator_key("dolphin", &cfg, &found), "dolphin-2");

    assert_eq!(catalog::new_profile_key("pcsx2", &cfg), "pcsx2/custom");
    cfg.profiles.insert("pcsx2/custom".into(), ProfileEntry::default());
    assert_eq!(catalog::new_profile_key("pcsx2", &cfg), "pcsx2/custom-2");
}

#[test]
fn small_helpers() {
    assert_eq!(catalog::program_kind(Path::new("/a/pcsx2-v2.AppImage")), "appimage");
    assert_eq!(catalog::program_kind(Path::new("/usr/bin/dolphin-emu")), "binary");
    assert_eq!(catalog::parse_extensions("ISO, .chd;cue  iso"), vec!["iso", "chd", "cue"]);
    assert_eq!(catalog::split_choice("retroarch/beetle"), ("retroarch", "beetle"));
    assert_eq!(catalog::split_choice("pcsx2"), ("pcsx2", ""));
    assert_eq!(catalog::choice_value("a", "b"), "a/b");
    assert_eq!(catalog::flathub_url("net.pcsx2.PCSX2"), "https://flathub.org/apps/net.pcsx2.PCSX2");
    assert_eq!(catalog::flathub_url("https://x.org/y"), "https://x.org/y");
    assert_eq!(catalog::join_limited(&["a".into(), "b".into(), "c".into()], 2), "a, b +1");
    assert_eq!(Picked::File("/opt/pcsx2-v2.2.0-linux-appimage-x64-Qt.AppImage".into()).name_guess(), "PCSX2");
    assert_eq!(Picked::File("/opt/my_emu-1.0.AppImage".into()).name_guess(), "my");
    assert_eq!(Picked::Flatpak { id: "x.y".into(), name: "Xemu".into() }.name_guess(), "Xemu");
    assert_eq!(
        catalog::picked_kind(&Picked::File("/a/b.AppImage".into())),
        InstallKind::AppImage { path: "/a/b.AppImage".into() }
    );
    assert_eq!(emulator_add::slug("  My Emu 2! "), "my-emu-2");
    assert_eq!(emulator_add::slug("!!"), "custom");
}

#[test]
fn bios_chip_words_follow_the_console() {
    assert_eq!(catalog::bios_chip("sony_playstation2", BiosState::Ok), Some(("BIOS OK".into(), ChipTone::Good)));
    assert_eq!(catalog::bios_chip("nintendo_switch", BiosState::Missing), Some(("Keys missing".into(), ChipTone::Bad)));
    assert_eq!(
        catalog::bios_chip("sony_playstation3", BiosState::Missing),
        Some(("Firmware missing".into(), ChipTone::Bad))
    );
    assert_eq!(catalog::bios_chip("sony_playstation2", BiosState::Wrong).unwrap().1, ChipTone::Warn);
    assert_eq!(catalog::bios_chip("x", BiosState::NotNeeded), None);
    assert!(catalog::wants_check("nintendo_switch") && catalog::wants_check("sony_playstation3"));
}

#[test]
fn entries_for_found_emulators_keep_their_program() {
    let cfg = EmulationConfig::default();
    let e = emu("pcsx2", "PCSX2", false);
    let x = catalog::entry_for(&e, &cfg);
    assert_eq!((x.kind.as_str(), x.app_id.as_str(), x.follows.as_str()), ("flatpak", "org.test.PCSX2", "pcsx2"));
    let mut x2 = x.clone();
    catalog::set_program(&mut x2, &InstallKind::Binary { path: "/usr/bin/pcsx2".into() });
    assert_eq!((x2.kind.as_str(), x2.app_id.as_str()), ("binary", ""));
    assert_eq!(x2.path, PathBuf::from("/usr/bin/pcsx2"));
}

// ------------------------------------------------------------------------- config edits

#[test]
fn removing_an_emulator_hides_found_ones_and_deletes_yours() {
    let mut cfg = EmulationConfig::default();
    cfg.profiles.insert("pcsx2/custom".into(), ProfileEntry::default());
    cfg.profiles.insert("pcsx2x/custom".into(), ProfileEntry::default());
    cfg.platform_emulator.insert("sony_playstation2".into(), "pcsx2/default".into());
    cfg.platform_emulator.insert("sony_playstation".into(), "duck/default".into());
    lists::remove_emulator(&mut cfg, &emu("pcsx2", "PCSX2", false));
    assert_eq!(cfg.hidden_emulators, vec!["pcsx2".to_string()]);
    assert!(!cfg.profiles.contains_key("pcsx2/custom") && cfg.profiles.contains_key("pcsx2x/custom"));
    assert!(!cfg.platform_emulator.contains_key("sony_playstation2"));
    assert!(cfg.platform_emulator.contains_key("sony_playstation"));
    // Twice: listed once.
    lists::remove_emulator(&mut cfg, &emu("pcsx2", "PCSX2", false));
    assert_eq!(cfg.hidden_emulators.len(), 1);

    cfg.emulators.insert("mine".into(), Default::default());
    lists::remove_emulator(&mut cfg, &emu("mine", "Mine", true));
    assert!(!cfg.emulators.contains_key("mine"));
    assert!(!cfg.hidden_emulators.contains(&"mine".to_string()));

    // A found emulator the user changed (override, same key): Remove drops the change AND hides
    // the found one, so it doesn't come straight back.
    cfg.emulators
        .insert("dolphin".into(), crate::config::EmulatorEntry { args: "-b {rom}".into(), ..Default::default() });
    let overridden = Emulator { auto: true, ..emu("dolphin", "Dolphin", true) };
    lists::remove_emulator(&mut cfg, &overridden);
    assert!(!cfg.emulators.contains_key("dolphin"));
    assert!(cfg.hidden_emulators.contains(&"dolphin".to_string()));
}

#[test]
fn adding_a_custom_emulator_adds_its_profile() {
    let mut cfg = EmulationConfig::default();
    let f = AddEmulator {
        follows: String::new(),
        program: Some(Picked::File("/opt/xemu.AppImage".into())),
        name: "Xemu".into(),
        args: String::new(),
        platforms: vec!["sony_playstation2".into()],
        ..Default::default()
    };
    let key = emulator_add::add_emulator(&mut cfg, &f, &[]);
    assert_eq!(key, "xemu");
    let x = &cfg.emulators["xemu"];
    assert_eq!((x.kind.as_str(), x.follows.as_str(), x.name.as_str()), ("appimage", "", "Xemu"));
    let p = &cfg.profiles["xemu/default"];
    assert_eq!(p.emulator, "xemu");
    assert_eq!(p.args, "{rom}");
    assert_eq!(p.platforms, vec!["sony_playstation2".to_string()]);
    assert!(p.extensions.contains(&"iso".to_string()), "the console's file types: {:?}", p.extensions);

    // Following a built-in definition: no profile of its own; a second one gets a new key.
    let f2 = AddEmulator {
        follows: "pcsx2".into(),
        program: Some(Picked::Flatpak { id: "net.pcsx2.PCSX2".into(), name: "PCSX2".into() }),
        name: "PCSX2".into(),
        ..Default::default()
    };
    assert_eq!(emulator_add::add_emulator(&mut cfg, &f2, &["pcsx2".into()]), "pcsx2-2");
    assert_eq!(cfg.emulators["pcsx2-2"].app_id, "net.pcsx2.PCSX2");
    assert!(!cfg.profiles.keys().any(|k| k.starts_with("pcsx2")));
}

#[test]
fn rom_folders_list_yours_found_and_switched_off() {
    let mut cfg = EmulationConfig::default();
    cfg.rom_folders.insert("f1".into(), RomFolder { path: "/home/me/ROMs".into(), ..Default::default() });
    cfg.ignored_folders.push("/media/old".into());
    let facts = LibFacts {
        auto_roots: [
            ("auto-1".to_string(), PathBuf::from("/mnt/games")),
            ("auto-2".to_string(), PathBuf::from("/home/me/ROMs")),
            ("auto-3".to_string(), PathBuf::from("/media/old")),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    let rows = folders::folder_rows_with(&cfg, &facts, &[]);
    let keys: Vec<(&str, bool, bool)> = rows.iter().map(|r| (r.key.as_str(), r.user, r.enabled)).collect();
    assert_eq!(keys, vec![("f1", true, true), ("auto-1", false, true), ("ignored:0", false, false)]);
    // Folders the scanner remembered show before any of their games are in the library.
    let noted = vec![
        ("auto-9".to_string(), PathBuf::from("/run/media/me/usb/roms")),
        ("auto-1".to_string(), PathBuf::from("/mnt/games")),
    ];
    let rows = folders::folder_rows_with(&cfg, &facts, &noted);
    let keys: Vec<&str> = rows.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(keys, vec!["f1", "auto-1", "auto-9", "ignored:0"]);
    cfg.auto_folders = false;
    assert_eq!(folders::folder_rows_with(&cfg, &facts, &noted).len(), 2);
    // Taken over by the user (adopt_folder ignores the found one): only their row shows.
    folders::adopt_folder(&mut cfg, Path::new("/mnt/games"));
    let rows = folders::folder_rows_with(&cfg, &facts, &noted);
    assert_eq!(rows.iter().filter(|r| r.path == Path::new("/mnt/games")).count(), 1);
}

#[test]
fn folder_console_words() {
    use std::collections::BTreeSet;
    assert_eq!(folders::folder_console_text("sony_playstation2", None), "PlayStation 2");
    assert_eq!(folders::folder_console_text("", None), "Consoles from the folder names");
    let one: BTreeSet<String> = ["sony_playstation2".to_string()].into_iter().collect();
    assert_eq!(folders::folder_console_text("", Some(&one)), "PS2");
    let two: BTreeSet<String> = ["sony_playstation2".to_string(), "sony_playstation".to_string()].into_iter().collect();
    assert!(folders::folder_console_text("", Some(&two)).starts_with("Sub-folders are consoles: PS"));
}

#[test]
fn adding_a_folder_twice_updates_it_and_adopting_ignores_the_found_one() {
    let mut cfg = EmulationConfig::default();
    cfg.ignored_folders.push("/r".into());
    let k = folders::add_folder(&mut cfg, Path::new("/r"), "");
    assert_eq!(k, "f1");
    assert!(cfg.ignored_folders.is_empty(), "adding it by hand scans it again");
    cfg.rom_folders.get_mut("f1").unwrap().enabled = false;
    assert_eq!(folders::add_folder(&mut cfg, Path::new("/r"), "sony_playstation2"), "f1");
    assert!(cfg.rom_folders["f1"].enabled);
    assert_eq!(cfg.rom_folders["f1"].platform, "sony_playstation2");
    assert_eq!(cfg.rom_folders.len(), 1);

    let k2 = folders::adopt_folder(&mut cfg, Path::new("/mnt/games"));
    assert_eq!(k2, "f2");
    assert_eq!(cfg.ignored_folders, vec![PathBuf::from("/mnt/games")]);

    let k3 = save_rom_folder(&mut cfg, Path::new("/r"), "", vec!["ps2/x.iso".into()]);
    assert_eq!(k3, "f1");
    assert_eq!(cfg.rom_folders["f1"].exclude, vec!["ps2/x.iso".to_string()]);
}

#[test]
fn identify_reports_in_plain_words() {
    use crate::emulation::bios::BiosFile;
    static PS2: BiosFile = BiosFile {
        platform: "sony_playstation2",
        file: "scph39001.bin",
        md5: &[],
        required: true,
        group: None,
        region: None,
        note: "",
    };
    let (m, err, dirs) = identify_message("sony_playstation2", &[], 2);
    assert!(err && m.contains("None of these"));
    assert!(dirs.is_empty());
    let found = vec![(PathBuf::from("/b/my bios.bin"), &PS2)];
    let (m, err, dirs) = identify_message("sony_playstation2", &found, 3);
    assert!(!err);
    assert!(m.contains("my bios.bin = scph39001.bin (PS2)"), "{m}");
    assert!(m.contains("2 not recognised"), "{m}");
    assert_eq!(dirs, vec![PathBuf::from("/b")]);
    let (m, _, _) = identify_message("sony_playstation", &found, 1);
    assert!(m.contains("None of them is for"), "{m}");
}

// ------------------------------------------------------------------------- import

fn scanned(root: &str, rel: &str, platform: &str, files: &[&str]) -> ScannedGame {
    let mut s = ScannedGame::new(Source::Emulator, format!("x/{rel}"), rel.to_string());
    s.emu = Some(EmuRef {
        platform: platform.into(),
        rom: Path::new(root).join(rel),
        files: files.iter().map(|f| Path::new(root).join(f)).collect(),
        ..Default::default()
    });
    s
}

#[test]
fn import_rows_untick_known_and_left_out_games_and_unticked_become_exclusions() {
    use super::super::emulation_import::{import_exclusions, rows_for};
    let games = vec![
        scanned("/r", "ps2/b.iso", "sony_playstation2", &[]),
        scanned("/r", "ps1/a [Disc 1].cue", "sony_playstation", &["ps1/a [Disc 1].bin"]),
        scanned("/r", "ps2/a.iso", "sony_playstation2", &[]),
        scanned("/r", "ps2/Demo (Demo).iso", "sony_playstation2", &[]),
        scanned("/r", "ps2/c.iso", "sony_playstation2", &[]),
    ];
    let library = vec![
        emu_game(1, "sony_playstation2", "auto-1", "/r", "ps2/b.iso"),
        // Found through an enclosing folder: a duplicate if this folder scanned it too.
        emu_game(2, "sony_playstation2", "auto-2", "/", "r/ps2/c.iso"),
    ];
    let mut rows = rows_for(games, &library, Path::new("/r"), &["*(Demo)*".to_string()]);
    let names: Vec<(&str, bool, bool)> = rows.iter().map(|r| (r.game.name.as_str(), r.checked, r.in_library)).collect();
    assert_eq!(
        names,
        vec![
            ("ps1/a [Disc 1].cue", true, false),
            ("ps2/a.iso", true, false),
            ("ps2/b.iso", false, true),
            ("ps2/c.iso", false, true),
            ("ps2/Demo (Demo).iso", false, false),
        ]
    );
    assert!(rows[2].same_root && !rows[3].same_root);
    rows[0].checked = false;
    let ex = import_exclusions(&rows, Path::new("/r"));
    // b.iso is in the library through this very folder: excluding it would drop it.
    assert_eq!(ex, vec!["ps1/a [Disc 1].cue", "ps1/a [Disc 1].bin", "ps2/c.iso", "ps2/Demo (Demo).iso"]);
}

#[test]
fn import_keeps_the_folders_own_patterns_unless_they_hide_a_ticked_game() {
    use super::super::emulation_import::{merged_exclusions, rows_for};
    let games = vec![
        scanned("/r", "ps2/a.iso", "sony_playstation2", &[]),
        scanned("/r", "ps2/x (Beta).iso", "sony_playstation2", &[]),
        scanned("/r", "ps2/y (Demo).iso", "sony_playstation2", &[]),
        scanned("/r", "ps2/z.iso", "sony_playstation2", &[]),
    ];
    let existing = vec!["*(Beta)*".to_string(), "*(Demo)*".to_string()];
    let mut rows = rows_for(games, &[], Path::new("/r"), &existing);
    // Ticks the demo back on, unticks z.
    for r in &mut rows {
        let n = r.game.name.as_str();
        r.checked = n != "ps2/z.iso" && n != "ps2/x (Beta).iso";
    }
    let ex = merged_exclusions(&rows, Path::new("/r"), &existing);
    assert_eq!(ex, vec!["*(Beta)*", "ps2/z.iso"]);
}

// ------------------------------------------------------------------------- drawing

/// Three frames (pickers / background results settle) drawing `f` in the root Ui.
fn frame(st: &mut AppState, mut f: impl FnMut(&mut Ui, &mut AppState)) {
    let ctx = st.ctx().clone();
    for _ in 0..3 {
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| f(ui, st));
    }
}

#[test]
fn every_page_draws_with_made_up_emulators() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = AppState::for_test(dir.path());
    st.library.replace(vec![
        emu_game(1, "sony_playstation2", "auto-aa", "/mnt/games", "ps2/a.iso"),
        emu_game(2, "nintendo_wiiu", "f1", "/home/me/ROMs", "wiiu/Game [X]"),
    ]);
    st.config
        .emulation
        .rom_folders
        .insert("f1".into(), RomFolder { path: "/home/me/ROMs".into(), ..Default::default() });
    st.config.emulation.bios_dirs.push("/home/me/bios".into());
    st.config.emulation.hidden_emulators.push("cemu".into());
    let pal = Pal::from_palette(&st.desktop_theme.file.palette);
    let mut d = SettingsDlg::new(super::super::settings::Tab::Emulation);
    d.emu = EmuPage::debug("demo+add+folder+bios:sony_playstation2");
    d.emu.emu_panel = Some(("pcsx2".into(), EmuPanel::Profiles));
    d.emu.exclusions_open = Some("f1".into());
    d.emu.confirm = Some(Confirm { platform: "sony_playstation2".into(), emulator: "pcsx2".into(), job: Job::Copy });
    let before = st.config.emulation.clone();
    frame(&mut st, |ui, st| {
        emulation(ui, st, &mut d, &pal);
    });
    assert_eq!(st.config.emulation, before, "drawing alone changes nothing");
    assert_eq!(d.emu.facts.per_platform.len(), 2);

    // Args panel and profile editor.
    st.config
        .emulation
        .profiles
        .insert("pcsx2/custom".into(), ProfileEntry { name: "Mine".into(), ..Default::default() });
    d.emu.profile_edit = Some("pcsx2/custom".into());
    frame(&mut st, |ui, st| {
        emulation(ui, st, &mut d, &pal);
    });
    d.emu.emu_panel = Some(("pcsx2".into(), EmuPanel::Args));
    frame(&mut st, |ui, st| {
        emulation(ui, st, &mut d, &pal);
    });

    // The import dialog with a preview, and with nothing found.
    let fam = egui::FontFamily::Proportional;
    let mut dlg = Some(super::super::emulation_import::RomImportDlg::open(&mut st, Some("/mnt/games".into())));
    dlg.as_mut().unwrap().demo_from_library(&st);
    frame(&mut st, |ui, st| {
        let ctx = ui.ctx().clone();
        let _ = super::super::emulation_import::show(&ctx, st, &mut dlg, &pal, &fam);
    });
    assert!(dlg.is_some(), "stays open until Import / Cancel");

    // Edit → Installation's section.
    let mut e = super::super::edit_emulation::EmuEdit::from_game(st.library.get(1).unwrap());
    assert!(e.emulated);
    frame(&mut st, |ui, st| {
        super::super::edit_emulation::section(ui, st, &mut e, Some(1), &pal);
    });
    assert_eq!(e.emulator, None);
}

#[test]
fn play_in_a_window_without_an_emulator_starts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = AppState::for_test(dir.path());
    st.library.replace(vec![emu_game(1, "sony_playstation2", "f1", "/r", "ps2/a.iso")]);
    st.launch_windowed(1);
    st.launch_windowed(99);
    assert_eq!(st.library.get(1).unwrap().play_count, 0);
    assert_eq!(st.running_game(), None);
}

#[test]
fn emulator_rows_say_where_without_repeating_the_kind() {
    assert_eq!(lists::how_after_kind("Flatpak", "Flatpak"), "");
    assert_eq!(lists::how_after_kind("Flatpak (installed for you only)", "Flatpak"), "installed for you only");
    assert_eq!(lists::how_after_kind("AppImage in ~/Applications", "AppImage"), "In ~/Applications");
    assert_eq!(lists::how_after_kind("Added by you", "Added by you"), "");
    assert_eq!(
        lists::how_after_kind("Program in PATH (/usr/bin/x)", "installed program"),
        "Program in PATH (/usr/bin/x)"
    );
    assert_eq!(lists::how_after_kind("Changed by you", "AppImage"), "Changed by you");
    // The whole line: where it is in few words, and whether it was added by hand.
    let home = Path::new("/home/me");
    assert_eq!(
        lists::folder_tail(Path::new("/mnt/space1/Gaming/Emulation/Emulators/DuckStation"), home),
        "…/Emulators/DuckStation"
    );
    assert_eq!(lists::folder_tail(Path::new("/home/me/Applications"), home), "~/Applications");
    assert_eq!(lists::folder_tail(Path::new("/opt/pcsx2"), home), "/opt/pcsx2");
    let ds = |how: &str, user: bool, auto: bool| Emulator {
        kind: InstallKind::AppImage { path: "/mnt/x/Gaming/Emulation/Emulators/Ryujinx/Ryujinx.AppImage".into() },
        how: how.into(),
        user,
        auto,
        ..emu("eden", "Ryujinx", false)
    };
    let line = |e: &Emulator| lists::where_line(e, &e.how, e.kind_label(), home);
    assert_eq!(line(&ds("Added by you", true, false)), "In …/Emulators/Ryujinx · added by you");
    assert_eq!(line(&ds("Changed by you", true, true)), "In …/Emulators/Ryujinx · changed by you");
    assert_eq!(
        line(&ds("AppImage in /mnt/x/Gaming/Emulation/Emulators/Ryujinx · portable", false, true)),
        "In …/Emulators/Ryujinx"
    );
    let fp = Emulator {
        kind: InstallKind::Flatpak { app_id: "org.x".into() },
        how: "Added by you".into(),
        user: true,
        ..emu("x", "X", false)
    };
    assert_eq!(line(&fp), "Added by you");
    let path = Emulator {
        kind: InstallKind::Binary { path: "/usr/bin/pcsx2".into() },
        how: "Program in PATH (/usr/bin/pcsx2)".into(),
        ..emu("p", "PCSX2", false)
    };
    assert_eq!(line(&path), "Program in PATH (/usr/bin/pcsx2)");
}

#[test]
fn the_automatic_choice_is_named_once() {
    assert_eq!(Catalog::automatic_label("Eden"), "Eden (automatic)");
    assert_eq!(Catalog::automatic_label("RetroArch — Beetle PSX"), "RetroArch — Beetle PSX (automatic)");
    // Narrow pages: shorter picker text.
    assert_eq!(
        Catalog::short_label("DuckStation — installed program (automatic)"),
        "DuckStation — installed (automatic)"
    );
    assert_eq!(Catalog::short_label("PCSX2"), "PCSX2");
}

#[test]
fn locate_adds_the_program_under_its_own_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = AppState::for_test(dir.path());
    let mut p = EmuPage::default();
    // Ryujinx located on the Switch row: Ryujinx, not the suggested Eden.
    let picked = Picked::File("/mnt/x/Emulators/Ryujinx/Ryujinx-1.3.2-x64.AppImage".into());
    assert!(apply_program(&mut st, &mut p, None, &PickFor::Locate("nintendo_switch".into()), picked));
    let x = &st.config.emulation.emulators["ryujinx"];
    assert_eq!((x.name.as_str(), x.follows.as_str(), x.kind.as_str()), ("Ryujinx", "ryujinx", "appimage"));
    // Something Lodestar doesn't know: the Add emulator form, named after the file.
    let picked = Picked::File("/opt/MySwitchEmu.AppImage".into());
    assert!(!apply_program(&mut st, &mut p, None, &PickFor::Locate("nintendo_switch".into()), picked));
    let f = p.add_emu.clone().expect("the form");
    assert_eq!((f.name.as_str(), f.follows.as_str()), ("MySwitchEmu", "eden"));
    // The form: a program picked after "Based on" keeps its own name, a typed one stays.
    p.add_emu = Some(AddEmulator { follows: "eden".into(), name: "Eden".into(), ..Default::default() });
    apply_program(&mut st, &mut p, None, &PickFor::AddEmulator, Picked::File("/opt/Ryujinx.AppImage".into()));
    assert_eq!(p.add_emu.as_ref().unwrap().name, "Ryujinx");
    p.add_emu = Some(AddEmulator { name: "Mine".into(), name_typed: true, ..Default::default() });
    apply_program(&mut st, &mut p, None, &PickFor::AddEmulator, Picked::File("/opt/pcsx2-v2.2.0.AppImage".into()));
    let f = p.add_emu.as_ref().unwrap();
    assert_eq!((f.name.as_str(), f.follows.as_str()), ("Mine", "pcsx2"), "a known program: based on it");
}

/// Narrow Settings windows: a console row whose name can't sit beside its controls wraps (the
/// name on its own line) instead of cutting the name to "Play…".
#[test]
fn rows_wrap_when_the_name_does_not_fit() {
    // Wide page: picker + Options + BIOS chip (≈380) beside "PlayStation 2" (≈100).
    assert!(!row_wraps(760.0, 380.0, 100.0));
    // A 640 px window's page (≈438): wraps.
    assert!(row_wraps(438.0, 380.0, 100.0));
    // Just fits / just doesn't.
    assert!(!row_wraps(524.0, 380.0, 100.0));
    assert!(row_wraps(523.0, 380.0, 100.0));
}
