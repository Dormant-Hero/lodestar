use super::*;
use crate::config::{EmulationConfig, ProfileEntry};
use crate::emulation::detect::{Emulator, InstallKind};
use crate::emulation::registry::{self, Registry};
use crate::model::{ActionKind, GameAction, LaunchSpec, TrackingMode};
use crate::test_support::write_file;

fn rpcs3() -> Emulator {
    Emulator {
        id: "rpcs3".into(),
        def: "rpcs3".into(),
        name: "RPCS3".into(),
        kind: InstallKind::Binary { path: "/apps/rpcs3/rpcs3".into() },
        how: String::new(),
        user: false,
        cores: vec![],
        args: String::new(),
        auto: true,
        blocked: false,
    }
}

/// RPCS3 with its Default profile and a tester's "SaveMGO": a forked RPCS3 of its own.
fn registry() -> Registry {
    let mut config = EmulationConfig::detect_nothing();
    config.fullscreen = false;
    config.profiles.insert(
        "rpcs3/savemgo".into(),
        ProfileEntry {
            name: "SaveMGO".into(),
            emulator: "rpcs3".into(),
            platforms: vec!["sony_playstation3".into()],
            args: "--no-gui {rom}".into(),
            program: "/apps/rpcs3-savemgo/rpcs3".into(),
            tracking: TrackingMode::Folder,
            ..Default::default()
        },
    );
    Registry { emulators: vec![rpcs3()], config, home: "/home/u".into() }
}

/// Metal Gear Solid 4 as a PS3 folder game, with Metal Gear Online inside it.
fn mgs4(dir: &Path) -> Game {
    let game = dir.join("MGS4");
    write_file(&game.join("PS3_GAME/USRDIR/EBOOT.BIN"), b"x");
    write_file(&game.join("o/MGO2.SELF"), b"x");
    let mut g = Game::blank("Metal Gear Solid 4");
    g.install_dir = Some(game.clone());
    g.emu = Some(EmuRef { platform: "sony_playstation3".into(), rom: game, ..Default::default() });
    g
}

fn with_mgo(mut g: Game) -> Game {
    g.edits.roms = vec![
        GameRom { name: "Metal Gear Solid 4".into(), path: SCANNED.into() },
        GameRom { name: "Metal Gear Online".into(), path: "{InstallDir}/o/MGO2.SELF".into() },
    ];
    g
}

fn argv(spec: LaunchSpec) -> (String, Vec<String>, Option<PathBuf>) {
    match spec {
        LaunchSpec::Command { program, args, cwd, .. } => (program, args, cwd),
        other => panic!("{other:?}"),
    }
}

fn emu_action(name: &str) -> GameAction {
    GameAction { name: name.into(), kind: ActionKind::Emulator, play: true, ..Default::default() }
}

#[test]
fn an_existing_game_has_its_scanned_rom_and_nothing_to_store() {
    let mut g = Game::blank("Pokémon Blue");
    g.install_dir = Some("/roms/gb".into());
    let base = EmuRef {
        platform: "nintendo_gameboy".into(),
        rom: "/roms/gb/Pokemon Blue (USA).gb".into(),
        ..Default::default()
    };
    g.emu = Some(base.clone());
    let l = list(&g);
    assert_eq!(l.len(), 1);
    assert_eq!((l[0].name.as_str(), l[0].key.as_str()), ("Pokémon Blue", SCANNED));
    assert_eq!(rom_ref(&g, &base, None).unwrap(), base);
    assert!(to_store(&effective(&g)).is_empty(), "existing libraries keep empty edits");
    assert!(entries(None, &g).is_empty(), "one way to play: no chooser");
    // Old edits (before ROM lists) still load, and a game with no ROM list saves none.
    let old: crate::model::UserEdits =
        serde_json::from_str(r#"{"emulator":"mgba","actions":[{"name":"Play","play":true}]}"#).unwrap();
    assert!(old.roms.is_empty() && old.play_default.is_none() && old.actions[0].rom.is_none());
    let json = serde_json::to_string(&old).unwrap();
    assert!(!json.contains("roms") && !json.contains("choose_profile") && !json.contains("play_default"), "{json}");
    // Not emulated: no ROMs at all.
    assert!(list(&Game::blank("x")).is_empty());
}

#[test]
fn paths_inside_the_game_folder_are_stored_relative() {
    let dir = Path::new("/roms/ps3/MGS4");
    assert_eq!(stored(&dir.join("o/MGO2.SELF"), Some(dir)), "{InstallDir}/o/MGO2.SELF");
    assert_eq!(expand("{InstallDir}/o/MGO2.SELF", Some(dir)), dir.join("o/MGO2.SELF"));
    assert_eq!(stored(Path::new("/other/x.gb"), Some(dir)), "/other/x.gb");
    assert_eq!(expand("/other/x.gb", Some(dir)), PathBuf::from("/other/x.gb"));
    // The game moved (another drive): {InstallDir} follows it.
    assert_eq!(
        expand("{InstallDir}/o/MGO2.SELF", Some(Path::new("/mnt/b/MGS4"))),
        PathBuf::from("/mnt/b/MGS4/o/MGO2.SELF")
    );
    assert_eq!(file_label(Path::new("/roms/gb/Pokemon Azul (Spain).gb")), "Pokémon Azul");
    assert_eq!(file_label(Path::new("/roms/ps3/MGS4/PS3_GAME/USRDIR/EBOOT.BIN")), "MGS4");
}

#[test]
fn several_roms_without_play_actions_give_one_choice_each() {
    let t = tempfile::tempdir().unwrap();
    let g = with_mgo(mgs4(t.path()));
    let e = entries(None, &g);
    let labels: Vec<&str> = e.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(labels, ["Metal Gear Solid 4", "Metal Gear Online"]);
    // Picking the second: it goes first in (a copy of) the game, so Play builds its command.
    let mut picked = g.clone();
    apply(&mut picked, &e[1].choice);
    let reg = registry();
    let base = g.emu.clone().unwrap();
    let (prog, args, _) = argv(registry::spec_with(&reg, &picked, &base, None).unwrap());
    assert_eq!(prog, "/apps/rpcs3/rpcs3");
    assert_eq!(args.last().unwrap(), &t.path().join("MGS4/o/MGO2.SELF").display().to_string());
    // Unchanged game: the default ROM (the folder's EBOOT.BIN).
    let (_, args, _) = argv(registry::spec_with(&reg, &g, &base, None).unwrap());
    assert!(args.last().unwrap().ends_with("PS3_GAME/USRDIR/EBOOT.BIN"), "{args:?}");
}

#[test]
fn a_deleted_extra_rom_is_no_longer_offered() {
    let t = tempfile::tempdir().unwrap();
    let g = with_mgo(mgs4(t.path()));
    assert_eq!(entries(None, &g).len(), 2);
    // The second copy's file is deleted: Play has one way again (no list), the stored ROM
    // list keeps it (an unplugged drive brings it back).
    std::fs::remove_file(t.path().join("MGS4/o/MGO2.SELF")).unwrap();
    assert!(entries(None, &g).is_empty(), "nothing to choose between");
    assert_eq!(playable(&g).len(), 1);
    assert_eq!(list(&g).len(), 2, "still stored");
}

#[test]
fn choose_profile_times_choose_rom_and_pinned_roms() {
    let t = tempfile::tempdir().unwrap();
    let reg = registry();
    // Playnite's way: one action, "Choose on startup" for profile and ROM → 4 choices.
    let mut g = with_mgo(mgs4(t.path()));
    g.edits.actions = vec![GameAction { choose_profile: true, choose_rom: true, ..emu_action("Play") }];
    let labels: Vec<String> = entries(Some(&reg), &g).into_iter().map(|e| e.label).collect();
    assert_eq!(
        labels,
        [
            "Play: Default · Metal Gear Solid 4",
            "Play: Default · Metal Gear Online",
            "Play: SaveMGO · Metal Gear Solid 4",
            "Play: SaveMGO · Metal Gear Online"
        ]
    );
    // A tester's setup: each action pins its ROM and profile → just the two.
    g.edits.actions = vec![
        GameAction {
            emulator: Some("rpcs3".into()),
            profile: Some("default".into()),
            ..emu_action("Metal Gear Solid 4")
        },
        GameAction {
            emulator: Some("rpcs3".into()),
            profile: Some("savemgo".into()),
            rom: Some("{InstallDir}/o/MGO2.SELF".into()),
            ..emu_action("Metal Gear Online")
        },
    ];
    let e = entries(Some(&reg), &g);
    assert_eq!(e.iter().map(|e| e.label.as_str()).collect::<Vec<_>>(), ["Metal Gear Solid 4", "Metal Gear Online"]);
    // The second runs the fork from its own folder, with MGO2.SELF.
    let mut picked = g.clone();
    apply(&mut picked, &e[1].choice);
    let a = picked.play_action().unwrap().clone();
    assert_eq!(a.name, "Metal Gear Online");
    assert!(picked.edits.actions.iter().filter(|a| a.play).count() == 1);
    let (prog, args, cwd) = argv(registry::action_spec(&reg, &picked, &a, None).unwrap());
    assert_eq!(prog, "/apps/rpcs3-savemgo/rpcs3");
    assert_eq!(args, ["--no-gui".to_string(), t.path().join("MGS4/o/MGO2.SELF").display().to_string()]);
    assert_eq!(cwd, Some(PathBuf::from("/apps/rpcs3-savemgo")));
    // Its profile says how to follow it: any process in the fork's folder.
    assert_eq!(
        registry::profile_tracking(&reg, &picked, Some(&a)),
        Some((TrackingMode::Folder, Some(PathBuf::from("/apps/rpcs3-savemgo"))))
    );
    // The first: the stock RPCS3 with the game folder's EBOOT.BIN, followed the usual way.
    let first = g.edits.actions[0].clone();
    let (prog, args, cwd) = argv(registry::action_spec(&reg, &g, &first, None).unwrap());
    assert_eq!(
        (prog.as_str(), cwd.as_deref()),
        ("/apps/rpcs3/rpcs3", Some(std::path::Path::new("/apps/rpcs3"))),
        "starts in its own folder"
    );
    assert!(args.last().unwrap().ends_with("EBOOT.BIN"));
    assert_eq!(registry::profile_tracking(&reg, &g, Some(&first)), None);
    // A removed ROM: says so instead of starting another one.
    let mut gone = g.clone();
    gone.edits.roms.truncate(1);
    let err = registry::action_spec(&reg, &gone, &g.edits.actions[1], None).unwrap_err();
    assert!(err.contains("ROM list"), "{err}");
}

#[test]
fn several_play_actions_and_the_remembered_one() {
    let mut g = Game::blank("Portal");
    g.edits.actions = vec![
        GameAction { name: "Play".into(), path: "/games/portal".into(), play: true, ..Default::default() },
        GameAction {
            name: "Play (Vulkan)".into(),
            path: "/games/portal".into(),
            args: "-vulkan".into(),
            play: true,
            ..Default::default()
        },
        GameAction { name: "Settings".into(), path: "/games/cfg".into(), ..Default::default() },
    ];
    let e = entries(None, &g);
    assert_eq!(e.iter().map(|e| e.label.as_str()).collect::<Vec<_>>(), ["Play", "Play (Vulkan)"]);
    assert_eq!(remembered(&g, &e), None);
    g.edits.play_default = Some(e[1].choice.clone());
    assert_eq!(remembered(&g, &e), Some(e[1].choice.clone()));
    let mut picked = g.clone();
    apply(&mut picked, &e[1].choice);
    assert_eq!(picked.play_action().unwrap().name, "Play (Vulkan)");
    assert!(!picked.edits.actions[2].play, "other actions stay as they were");
    // Renamed since: the remembered line no longer applies (Play asks again).
    g.edits.actions[1].name = "Vulkan".into();
    assert_eq!(remembered(&g, &entries(None, &g)), None);
    // One Play action: fast, no chooser.
    g.edits.actions[1].play = false;
    assert!(entries(None, &g).is_empty());
}

#[test]
fn reordering_makes_another_rom_the_default() {
    let t = tempfile::tempdir().unwrap();
    let mut g = with_mgo(mgs4(t.path()));
    g.edits.roms.swap(0, 1);
    let base = g.emu.clone().unwrap();
    assert!(default_ref(&g, &base).rom.ends_with("o/MGO2.SELF"));
    assert_eq!(list(&g)[1].key, SCANNED, "the scanned ROM keeps following rescans");
    // A rescan moved the game: the scanned ROM follows, the added one too ({InstallDir}).
    let moved = t.path().join("moved/MGS4");
    g.install_dir = Some(moved.clone());
    g.emu.as_mut().unwrap().rom = moved.clone();
    let l = list(&g);
    assert_eq!(l[0].path, moved.join("o/MGO2.SELF"));
    assert_eq!(l[1].path, moved);
}
