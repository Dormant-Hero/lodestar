use super::*;
use crate::config::{EmulationConfig, RomFolder};
use crate::emulation::detect::{self, Emulator, InstallKind};
use crate::test_support::{sv, write_file};

/// The fixture machine's registry (the user's 7 Flatpaks), with `/mnt/space1/games` set up as a
/// ROM folder `f1` whose sub-folders are consoles.
fn user_machine() -> (tempfile::TempDir, Registry) {
    let (t, roots, mut ctx) = detect::tests::machine();
    ctx.emulation.rom_folders.insert("f1".into(), RomFolder { path: "/mnt/space1/games".into(), ..Default::default() });
    let emulators = detect::detect_with(&ctx, &roots);
    (t, Registry { emulators, config: ctx.emulation.clone(), home: roots.home.clone() })
}

fn emu_ref(platform: &str, rom: &str, files: &[&str]) -> EmuRef {
    EmuRef {
        platform: platform.into(),
        folder: "f1".into(),
        rom: rom.into(),
        files: files.iter().map(PathBuf::from).collect(),
        ..Default::default()
    }
}

fn argv(spec: Option<LaunchSpec>) -> (String, Vec<String>) {
    match spec {
        // Programs start in their own folder (portable set-ups), system ones and Flatpaks as they are.
        Some(LaunchSpec::Command { program, args, cwd, env })
            if env.is_empty()
                && cwd.as_deref()
                    == std::path::Path::new(&program)
                        .parent()
                        .filter(|_| !program.starts_with("/usr/") && program != "flatpak") =>
        {
            (program, args)
        }
        other => panic!("{other:?}"),
    }
}

/// The command `reg` builds for a ROM with the emulator `choose` picks.
fn command(reg: &Registry, e: &EmuRef, fullscreen: bool) -> (String, Vec<String>) {
    let (emu, prof) =
        choose(reg, &e.platform, &e.rom, None, None, reg.config.rom_folders.get(&e.folder)).expect("an emulator");
    argv(build_spec_opts(reg, &emu, &prof, e, fullscreen))
}

#[test]
fn user_flatpaks_get_exactly_the_access_they_lack() {
    let (_t, reg) = user_machine();
    let g = "/mnt/space1/games";
    let cases: &[(&str, &str, &[&str], &[&str])] = &[
        (
            "sony_playstation",
            "/mnt/space1/games/ps1/Crash Bandicoot (USA).cue",
            &["/mnt/space1/games/ps1/Crash Bandicoot (USA).bin"],
            &[
                "run",
                "--filesystem=/mnt/space1/games",
                "org.duckstation.DuckStation",
                "-batch",
                "-fullscreen",
                "--",
                "/mnt/space1/games/ps1/Crash Bandicoot (USA).cue",
            ],
        ),
        (
            "sony_playstation2",
            "/mnt/space1/games/ps2/Ico (USA)/Ico (USA).cue",
            &["/mnt/space1/games/ps2/Ico (USA)/Ico (USA).bin"],
            // PCSX2 can't open .cue: the .bin is started.
            &[
                "run",
                "--filesystem=/mnt/space1/games",
                "net.pcsx2.PCSX2",
                "-batch",
                "-fullscreen",
                "--",
                "/mnt/space1/games/ps2/Ico (USA)/Ico (USA).bin",
            ],
        ),
        (
            "sony_playstation3",
            "/mnt/space1/games/ps3/Demon's Souls (USA)",
            &[],
            &[
                "run",
                "--filesystem=/mnt/space1/games",
                "net.rpcs3.RPCS3",
                "--no-gui",
                "--fullscreen",
                "/mnt/space1/games/ps3/Demon's Souls (USA)",
            ],
        ),
        (
            "nintendo_switch",
            "/mnt/space1/games/switch/Fire Emblem Three Houses [010055D009F78000][v0].nsp",
            &[],
            &[
                "run",
                "--filesystem=/mnt/space1/games",
                "dev.eden_emu.eden",
                "-f",
                "-g",
                "/mnt/space1/games/switch/Fire Emblem Three Houses [010055D009F78000][v0].nsp",
            ],
        ),
        (
            "nintendo_gamecube",
            "/mnt/space1/games/gc/Fire Emblem - Path of Radiance (USA).rvz",
            &[],
            &[
                "run",
                "org.DolphinEmu.dolphin-emu",
                "-b",
                "-C",
                "Dolphin.Display.Fullscreen=True",
                "-e",
                "/mnt/space1/games/gc/Fire Emblem - Path of Radiance (USA).rvz",
            ],
        ),
        (
            "sony_psp",
            "/mnt/space1/games/psp/Patapon (USA).cso",
            &[],
            &[
                "run",
                "org.ppsspp.PPSSPP",
                "--fullscreen",
                "--pause-menu-exit",
                "/mnt/space1/games/psp/Patapon (USA).cso",
            ],
        ),
        (
            "nintendo_gameboyadvance",
            "/mnt/space1/games/gba/Pokemon - FireRed Version (USA, Europe).gba",
            &[],
            &["run", "io.mgba.mGBA", "-f", "/mnt/space1/games/gba/Pokemon - FireRed Version (USA, Europe).gba"],
        ),
    ];
    for (platform, rom, files, want) in cases {
        let e = emu_ref(platform, rom, files);
        let (program, args) = command(&reg, &e, true);
        assert_eq!(program, "flatpak", "{platform}");
        assert_eq!(args, sv(want), "{platform}");
        assert!(args.iter().filter(|a| a.starts_with("--filesystem")).all(|a| a == &format!("--filesystem={g}")));
    }
}

#[test]
fn windowed_drops_the_fullscreen_words() {
    let (_t, reg) = user_machine();
    let duck = emu_ref("sony_playstation", "/mnt/space1/games/ps1/Spyro.chd", &[]);
    assert_eq!(
        command(&reg, &duck, false).1,
        sv(&[
            "run",
            "--filesystem=/mnt/space1/games",
            "org.duckstation.DuckStation",
            "-batch",
            "-nofullscreen",
            "--",
            "/mnt/space1/games/ps1/Spyro.chd"
        ])
    );
    let gc = emu_ref("nintendo_wii", "/mnt/space1/games/wii/Zelda.rvz", &[]);
    assert_eq!(
        command(&reg, &gc, false).1,
        sv(&[
            "run",
            "org.DolphinEmu.dolphin-emu",
            "-b",
            "-C",
            "Dolphin.Display.Fullscreen=False",
            "-e",
            "/mnt/space1/games/wii/Zelda.rvz"
        ])
    );
    let gba = emu_ref("nintendo_gameboy", "/mnt/space1/games/gb/Tetris.gb", &[]);
    assert_eq!(command(&reg, &gba, false).1, sv(&["run", "io.mgba.mGBA", "/mnt/space1/games/gb/Tetris.gb"]));
    let psp = emu_ref("sony_psp", "/mnt/space1/games/psp/x.iso", &[]);
    assert_eq!(
        command(&reg, &psp, false).1,
        sv(&["run", "org.ppsspp.PPSSPP", "--windowed", "--pause-menu-exit", "/mnt/space1/games/psp/x.iso"])
    );
    // The setting decides for build_spec.
    let mut r2 = reg.clone();
    r2.config.fullscreen = false;
    let (emu, prof) = choose(&r2, "nintendo_switch", Path::new("/mnt/x.nsp"), None, None, None).unwrap();
    let e = emu_ref("nintendo_switch", "/mnt/space1/games/switch/x.nsp", &[]);
    assert_eq!(
        argv(build_spec(&r2, &emu, &prof, &e)).1,
        sv(&["run", "--filesystem=/mnt/space1/games", "dev.eden_emu.eden", "-g", "/mnt/space1/games/switch/x.nsp"])
    );
}

#[test]
fn without_a_rom_folder_the_games_own_folder_is_granted() {
    let (_t, mut reg) = user_machine();
    reg.config.rom_folders.clear();
    let e = EmuRef {
        platform: "sony_playstation".into(),
        folder: "auto-12345678".into(),
        rom: "/mnt/disk/psx/FF7/FF7.m3u".into(),
        files: vec!["/mnt/disk/psx/FF7/Disc 1.chd".into(), "/mnt/disk/psx/FF7/Disc 2.chd".into()],
        ..Default::default()
    };
    let (_, args) = command(&reg, &e, true);
    assert_eq!(args[1], "--filesystem=/mnt/disk/psx/FF7");
    assert_eq!(args.last().unwrap(), "/mnt/disk/psx/FF7/FF7.m3u");
    // A root the scanner noted wins.
    note_roots(&[("auto-12345678".into(), "/mnt/disk".into())]);
    assert_eq!(command(&reg, &e, true).1[1], "--filesystem=/mnt/disk");
    // Under the home folder RPCS3 (home:ro) needs nothing.
    let home_game = reg.home.join("ROMs/ps3/Game");
    let e = EmuRef { platform: "sony_playstation3".into(), rom: home_game.clone(), ..Default::default() };
    assert_eq!(
        command(&reg, &e, true).1,
        sv(&["run", "net.rpcs3.RPCS3", "--no-gui", "--fullscreen", home_game.to_str().unwrap()])
    );
}

fn emu(id: &str, def: &str, kind: InstallKind) -> Emulator {
    Emulator {
        id: id.into(),
        def: def.into(),
        name: def.into(),
        kind,
        how: String::new(),
        user: false,
        cores: vec![],
        args: String::new(),
        auto: false,
        blocked: false,
    }
}

fn bin(p: &str) -> InstallKind {
    InstallKind::Binary { path: p.into() }
}

fn retroarch(cores: &[&str]) -> Emulator {
    Emulator {
        cores: cores.iter().map(|c| PathBuf::from(format!("/usr/lib/libretro/{c}_libretro.so"))).collect(),
        ..emu("retroarch", "retroarch", bin("/usr/bin/retroarch"))
    }
}

fn reg_of(emulators: Vec<Emulator>) -> Registry {
    Registry { emulators, config: EmulationConfig::default(), home: "/home/u".into() }
}

fn chosen(reg: &Registry, platform: &str, rom: &str) -> Option<(String, String)> {
    choose(reg, platform, Path::new(rom), None, None, None).map(|(e, p)| (e.id, p.id))
}

#[test]
fn choose_precedence() {
    let mut reg = reg_of(vec![
        retroarch(&["mednafen_psx_hw", "snes9x", "pcsx2", "genesis_plus_gx"]),
        emu("duckstation", "duckstation", bin("/usr/bin/duckstation-qt")),
        emu("snes9x", "snes9x", bin("/usr/bin/snes9x-gtk")),
        emu("pcsx2", "pcsx2", bin("/usr/bin/pcsx2-qt")),
    ]);
    let ch = |r: &Registry, p: &str, rom: &str| chosen(r, p, rom).map(|(a, b)| format!("{a}/{b}"));
    // Standalone first for PlayStation, RetroArch first for the SNES.
    assert_eq!(ch(&reg, "sony_playstation", "/r/a.cue").as_deref(), Some("duckstation/default"));
    assert_eq!(ch(&reg, "nintendo_super_nes", "/r/a.sfc").as_deref(), Some("retroarch/snes9x"));
    // PCSX2 opens a .cue game through its .bin and still beats RetroArch's core that opens the .cue.
    assert_eq!(ch(&reg, "sony_playstation2", "/r/a.cue").as_deref(), Some("pcsx2/default"));
    assert_eq!(ch(&reg, "sega_genesis", "/r/a.md").as_deref(), Some("retroarch/genesis_plus_gx"));
    // Nothing for the platform.
    assert_eq!(ch(&reg, "nintendo_switch", "/r/a.nsp"), None);

    // Settings → Emulation's per-console choice beats the automatic one …
    reg.config.platform_emulator.insert("sony_playstation".into(), "retroarch/mednafen_psx_hw".into());
    assert_eq!(ch(&reg, "sony_playstation", "/r/a.cue").as_deref(), Some("retroarch/mednafen_psx_hw"));
    // … the ROM folder's beats that …
    let folder = RomFolder { path: "/r".into(), emulator: "duckstation".into(), ..Default::default() };
    let f = |ge: Option<&str>, gp: Option<&str>| {
        choose(&reg, "sony_playstation", Path::new("/r/a.cue"), ge, gp, Some(&folder))
            .map(|(e, p)| format!("{}/{}", e.id, p.id))
    };
    assert_eq!(f(None, None).as_deref(), Some("duckstation/default"));
    // … and the game's own choice beats everything (profile given or not).
    assert_eq!(f(Some("retroarch"), Some("mednafen_psx_hw")).as_deref(), Some("retroarch/mednafen_psx_hw"));
    assert_eq!(f(Some("retroarch"), None).as_deref(), Some("retroarch/mednafen_psx_hw"));
    // A chosen emulator that isn't installed falls through to the next choice.
    assert_eq!(f(Some("gone"), None).as_deref(), Some("duckstation/default"));
    // Only "emu" in platform_emulator: its best profile for the platform.
    reg.config.platform_emulator.insert("nintendo_super_nes".into(), "snes9x".into());
    assert_eq!(ch(&reg, "nintendo_super_nes", "/r/a.sfc").as_deref(), Some("snes9x/default"));

    // An emulator the user added comes first.
    let mut mine = emu("mine", "", bin("/opt/mine"));
    mine.user = true;
    let mut reg2 = reg_of(vec![emu("duckstation", "duckstation", bin("/d")), mine]);
    reg2.config.profiles.insert(
        "mine/psx".into(),
        crate::config::ProfileEntry {
            platforms: sv(&["sony_playstation"]),
            extensions: sv(&["cue"]),
            ..Default::default()
        },
    );
    assert_eq!(chosen(&reg2, "sony_playstation", "/r/a.cue"), Some(("mine".into(), "psx".into())));
    // … unless it can't open the file while another can.
    assert_eq!(chosen(&reg2, "sony_playstation", "/r/a.chd"), Some(("duckstation".into(), "default".into())));
}

#[test]
fn choose_for_reads_game_edits_and_folder() {
    let mut reg = reg_of(vec![retroarch(&["mednafen_psx_hw"]), emu("duckstation", "duckstation", bin("/d"))]);
    reg.config
        .rom_folders
        .insert("f1".into(), RomFolder { path: "/r".into(), emulator: "retroarch".into(), ..Default::default() });
    let e = EmuRef {
        platform: "sony_playstation".into(),
        folder: "f1".into(),
        rom: "/r/a.cue".into(),
        ..Default::default()
    };
    assert_eq!(choose_for(&reg, None, &e).unwrap().0.id, "retroarch");
    let mut g = Game::blank("A");
    g.edits.emulator = Some("duckstation".into());
    g.emu = Some(e.clone());
    assert_eq!(choose_for(&reg, Some(&g), &e).unwrap().0.id, "duckstation");
    let spec = spec_with(&reg, &g, &e, Some(false)).unwrap();
    assert_eq!(argv(Some(spec)), ("/d".into(), sv(&["-batch", "-nofullscreen", "--", "/r/a.cue"])));
}

#[test]
fn retroarch_and_templates() {
    let reg = reg_of(vec![retroarch(&["snes9x"])]);
    let (e, p) = choose(&reg, "nintendo_super_nes", Path::new("/r/a.sfc"), None, None, None).unwrap();
    let r = EmuRef {
        platform: "nintendo_super_nes".into(),
        rom: "/r/Super Metroid (Japan, USA).sfc".into(),
        ..Default::default()
    };
    assert_eq!(
        argv(build_spec_opts(&reg, &e, &p, &r, true)),
        (
            "/usr/bin/retroarch".into(),
            sv(&["-f", "-L", "/usr/lib/libretro/snes9x_libretro.so", "/r/Super Metroid (Japan, USA).sfc"])
        )
    );
    assert_eq!(argv(build_spec_opts(&reg, &e, &p, &r, false)).1[0], "-L");
    // The user's arguments replace the profile's; Playnite tokens work.
    let mut e2 = e.clone();
    e2.args =
        "--verbose -L {core} \"{ImagePath}\" {ImageName} {ImageNameNoExt} {ImageDir} {EmuDir} {fullscreen}".into();
    assert_eq!(
        argv(build_spec_opts(&reg, &e2, &p, &r, true)).1,
        sv(&[
            "--verbose",
            "-L",
            "/usr/lib/libretro/snes9x_libretro.so",
            "/r/Super Metroid (Japan, USA).sfc",
            "Super Metroid (Japan, USA).sfc",
            "Super Metroid (Japan, USA)",
            "/r",
            "/usr/bin"
        ])
    );
    // {core} without a core: no command.
    let mut p2 = p.clone();
    p2.core = None;
    assert!(build_spec_opts(&reg, &e, &p2, &r, true).is_none());
    // MAME starts the set by name from its folder.
    let mame = emu("mame", "mame", bin("/usr/bin/mame"));
    let reg = reg_of(vec![mame]);
    let (e, p) = choose(&reg, "arcade", Path::new("/a/sf2.zip"), None, None, None).unwrap();
    let r = EmuRef { platform: "arcade".into(), rom: "/a/sf2.zip".into(), ..Default::default() };
    assert_eq!(argv(build_spec_opts(&reg, &e, &p, &r, true)).1, sv(&["-rompath", "/a", "sf2"]));
    assert_eq!(argv(build_spec_opts(&reg, &e, &p, &r, false)).1, sv(&["-rompath", "/a", "-window", "sf2"]));
    // A ROM whose name looks like a token is passed as it is (one substitution pass).
    let r = EmuRef { platform: "arcade".into(), rom: "/a/{rom_dir} {x}.zip".into(), ..Default::default() };
    let mut e2 = e.clone();
    e2.args = "{rom} pre{rom_name}post".into();
    assert_eq!(argv(build_spec_opts(&reg, &e2, &p, &r, true)).1, sv(&["/a/{rom_dir} {x}.zip", "pre{rom_dir} {x}post"]));
}

#[test]
fn flatpak_command_for_shadps4_and_open() {
    let tmp = tempfile::tempdir().unwrap();
    let mut reg =
        reg_of(vec![emu("shadps4", "shadps4", InstallKind::Flatpak { app_id: "net.shadps4.shadPS4".into() })]);
    reg.home = tmp.path().to_path_buf();
    let game = tmp.path().join("ROMs/ps4/CUSA00001");
    write_file(&game.join("eboot.bin"), b"x");
    write_file(&game.join("sce_sys/param.sfo"), b"x");
    let r = EmuRef { platform: "sony_playstation4".into(), rom: game.clone(), ..Default::default() };
    let (e, p) = choose(&reg, "sony_playstation4", &game, None, None, None).unwrap();
    let (prog, args) = argv(build_spec_opts(&reg, &e, &p, &r, true));
    assert_eq!(prog, "flatpak");
    let eboot = game.join("eboot.bin");
    assert_eq!(args[..3], sv(&["run", &format!("--filesystem={}", game.display()), "--command=shadps4"]));
    assert_eq!(args[3..], sv(&["net.shadps4.shadPS4", "-g", eboot.to_str().unwrap(), "-f", "true"]));
    // Opening the emulator: its own UI (default command); RetroArch opens its menu.
    assert_eq!(argv(Some(open_spec(&e))), ("flatpak".into(), sv(&["run", "net.shadps4.shadPS4"])));
    assert_eq!(argv(Some(open_spec(&retroarch(&[])))), ("/usr/bin/retroarch".into(), sv(&["--menu"])));
}

#[test]
fn folder_games_and_cue_tracks() {
    let tmp = tempfile::tempdir().unwrap();
    let reg = reg_of(vec![
        emu("cemu", "cemu", InstallKind::AppImage { path: "/apps/Cemu.AppImage".into() }),
        emu("rpcs3", "rpcs3", bin("/usr/bin/rpcs3")),
        emu("pcsx2", "pcsx2", bin("/usr/bin/pcsx2-qt")),
    ]);
    // Wii U NUS folder: title.tmd inside it.
    let wiiu = tmp.path().join("wiiu/Mario Kart 8 (USA)");
    write_file(&wiiu.join("title.tmd"), b"x");
    write_file(&wiiu.join("00000000.app"), b"x");
    let r = EmuRef { platform: "nintendo_wiiu".into(), rom: wiiu.clone(), ..Default::default() };
    let (e, p) = choose(&reg, "nintendo_wiiu", &wiiu, None, None, None).unwrap();
    assert_eq!(e.id, "cemu");
    assert_eq!(argv(build_spec(&reg, &e, &p, &r)).1, sv(&["-f", "-g", wiiu.join("title.tmd").to_str().unwrap()]));
    // Loadiine folder: code/*.rpx.
    let ld = tmp.path().join("wiiu/Game");
    write_file(&ld.join("code/game.rpx"), b"x");
    write_file(&ld.join("meta/meta.xml"), b"x");
    let r = EmuRef { platform: "nintendo_wiiu".into(), rom: ld.clone(), ..Default::default() };
    assert_eq!(argv(build_spec(&reg, &e, &p, &r)).1.last().unwrap(), ld.join("code/game.rpx").to_str().unwrap());
    // PS3 disc folder: PS3_GAME/USRDIR/EBOOT.BIN (case-insensitive).
    let ps3 = tmp.path().join("ps3/Demon's Souls");
    write_file(&ps3.join("PS3_GAME/USRDIR/eboot.bin"), b"x");
    let r = EmuRef { platform: "sony_playstation3".into(), rom: ps3.clone(), ..Default::default() };
    let (e, p) = choose(&reg, "sony_playstation3", &ps3, None, None, None).unwrap();
    assert_eq!(
        argv(build_spec(&reg, &e, &p, &r)).1.last().unwrap(),
        ps3.join("PS3_GAME/USRDIR/eboot.bin").to_str().unwrap()
    );
    // A cue sheet without its files listed: PCSX2 gets the track the sheet names.
    let cue = tmp.path().join("ps2/Ico.cue");
    write_file(&cue, "FILE \"Ico (Track 1).bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n");
    let r = EmuRef { platform: "sony_playstation2".into(), rom: cue.clone(), ..Default::default() };
    let (e, p) = choose(&reg, "sony_playstation2", &cue, None, None, None).unwrap();
    assert_eq!(e.id, "pcsx2");
    assert_eq!(
        argv(build_spec(&reg, &e, &p, &r)).1.last().unwrap(),
        tmp.path().join("ps2/Ico (Track 1).bin").to_str().unwrap()
    );
}

#[test]
fn reasons_in_plain_words() {
    let mut g = Game::blank("Ico");
    assert_eq!(missing_reason_with(None, &g), None, "not emulated");
    g.emu = Some(EmuRef { platform: "sony_playstation2".into(), rom: "/r/ico.iso".into(), ..Default::default() });
    assert_eq!(
        missing_reason_with(None, &g).unwrap(),
        "No emulator for PlayStation 2 yet — set one up in Settings → Emulation."
    );
    let empty = reg_of(vec![]);
    assert_eq!(
        missing_reason_with(Some(&empty), &g).unwrap(),
        "No emulator for PlayStation 2 was found on this computer. Install PCSX2 (on Flathub: net.pcsx2.PCSX2), or set one up in Settings → Emulation."
    );
    let snes =
        Game { emu: Some(EmuRef { platform: "nintendo_super_nes".into(), ..Default::default() }), ..Game::blank("x") };
    assert!(
        missing_reason_with(Some(&empty), &snes).unwrap().contains("RetroArch (on Flathub: org.libretro.RetroArch)")
    );
    let ready = reg_of(vec![emu("pcsx2", "pcsx2", bin("/p"))]);
    assert_eq!(missing_reason_with(Some(&ready), &g), None);
}

#[test]
fn wiiu_folders_cemu_cannot_open_explain_instead_of_starting() {
    let tmp = tempfile::tempdir().unwrap();
    let cemu = emu("cemu", "cemu", bin("/usr/bin/cemu"));
    let reg = Registry { emulators: vec![cemu], config: EmulationConfig::detect_nothing(), home: tmp.path().into() };
    // The user's BotW: tmd.0 + bare content files, no title.tmd / title.tik / .app names.
    let bare = tmp.path().join("wiiu/BotW (USA)");
    write_file(&bare.join("tmd.0"), b"tmd");
    write_file(&bare.join("00000000"), b"c");
    write_file(&bare.join("00000003.h3"), b"h");
    let mut g = Game::blank("BotW");
    g.emu = Some(EmuRef { platform: "nintendo_wiiu".into(), rom: bare.clone(), ..Default::default() });
    let why = unplayable(&g).unwrap();
    assert!(why.contains("no title.tmd, no title.tik and none of the .app content files"), "{why}");
    assert!(why.contains("tmd.0") && why.contains(".wua"), "{why}");
    assert_eq!(spec_with(&reg, &g, g.emu.as_ref().unwrap(), None), None, "never starts a folder Cemu can't open");
    assert_eq!(missing_reason_with(Some(&reg), &g), Some(why));
    // A proper NUS folder and a decrypted copy start.
    let nus = tmp.path().join("wiiu/Good (USA)");
    for f in ["title.tmd", "title.tik", "00000000.app", "00000001.app"] {
        write_file(&nus.join(f), b"x");
    }
    let mut ok = Game::blank("Good");
    ok.emu = Some(EmuRef { platform: "nintendo_wiiu".into(), rom: nus.clone(), ..Default::default() });
    assert_eq!(unplayable(&ok), None);
    let spec = spec_with(&reg, &ok, ok.emu.as_ref().unwrap(), None).unwrap();
    let LaunchSpec::Command { args, .. } = spec else { panic!() };
    assert_eq!(args.last().unwrap(), nus.join("title.tmd").to_str().unwrap());
    let dec = tmp.path().join("wiiu/Dec");
    for f in ["code/a.rpx", "content/x", "meta/meta.xml"] {
        write_file(&dec.join(f), b"x");
    }
    assert_eq!(wiiu_problem_uncached(&dec), None);
    // A missing title.tik alone is named.
    let half = tmp.path().join("wiiu/Half");
    for f in ["title.tmd", "00000000.app"] {
        write_file(&half.join(f), b"x");
    }
    assert!(wiiu_problem_uncached(&half).unwrap().contains("it has no title.tik."));
}

#[test]
fn disk_facts_are_looked_up_again_off_the_ui_thread_on_refresh() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("ps3/Game");
    std::fs::create_dir_all(&dir).unwrap();
    assert!(is_dir(&dir));
    std::fs::remove_dir(&dir).unwrap();
    assert!(is_dir(&dir), "cached until the next refresh");
    rewarm_facts();
    assert!(!is_dir(&dir), "refresh re-reads every cached path");
}

#[test]
fn a_picked_emulator_not_allowed_to_run_is_never_swapped_for_another() {
    let mut ai =
        emu("duckstation-appimage", "duckstation", InstallKind::AppImage { path: "/apps/DuckStation.AppImage".into() });
    ai.name = "DuckStation".into();
    ai.blocked = true;
    let mut fp =
        emu("duckstation", "duckstation", InstallKind::Flatpak { app_id: "org.duckstation.DuckStation".into() });
    fp.name = "DuckStation".into();
    let mut reg = reg_of(vec![fp, ai]);
    let mut g = Game::blank("Crash");
    g.emu =
        Some(EmuRef { platform: "sony_playstation".into(), rom: "/roms/psx/Crash.chd".into(), ..Default::default() });
    let e = g.emu.clone().unwrap();
    // Nothing picked: the Flatpak.
    assert_eq!(choose_for(&reg, Some(&g), &e).map(|(e, _)| e.id).as_deref(), Some("duckstation"));
    assert!(blocked_choice(&reg, &g).is_none());
    // The console set to the AppImage that can't run yet: nothing (Play asks to allow it).
    reg.config.platform_emulator.insert("sony_playstation".into(), "duckstation-appimage/default".into());
    assert!(choose_for(&reg, Some(&g), &e).is_none());
    assert_eq!(blocked_choice(&reg, &g).map(|b| b.id).as_deref(), Some("duckstation-appimage"));
    let why = missing_reason_with(Some(&reg), &g).unwrap();
    assert!(why.contains("DuckStation (AppImage) isn't allowed to run yet"), "{why}");
    // The game's own pick (the Flatpak) wins over the console's.
    g.edits.emulator = Some("duckstation".into());
    assert_eq!(choose_for(&reg, Some(&g), &e).map(|(e, _)| e.id).as_deref(), Some("duckstation"));
    assert!(blocked_choice(&reg, &g).is_none());
}

// ---- 0.10: emulator actions, switches, extra arguments ---------------------------------------

fn ps2_game(rom: &str) -> Game {
    let mut g = Game::blank("Biohazard Outbreak");
    g.emu = Some(EmuRef {
        platform: "sony_playstation2".into(),
        folder: "f9".into(),
        rom: rom.into(),
        ..Default::default()
    });
    g
}

fn tail(spec: Result<LaunchSpec, String>) -> Vec<String> {
    let (_, args) = argv(Some(spec.expect("a command")));
    // After the Flatpak's own words.
    match args.iter().position(|a| a == "net.pcsx2.PCSX2") {
        Some(i) => args[i + 1..].to_vec(),
        None => args,
    }
}

/// A Play action of type Program with the program left empty and arguments
/// `-nofullscreen -- '<the iso>'` → PCSX2 with its usual words, windowed, the ROM once.
#[test]
fn program_action_without_a_program_runs_the_emulator() {
    let (_t, reg) = user_machine();
    let iso = "/mnt/Storage/Games/PS2/Biohazard Outbreak.iso";
    let g = ps2_game(iso);
    let a = crate::model::GameAction {
        name: "Play".into(),
        play: true,
        args: format!("-nofullscreen -- '{iso}'"),
        ..Default::default()
    };
    assert_eq!(tail(action_spec(&reg, &g, &a, None)), sv(&["-batch", "-nofullscreen", "--", iso]));
    // The same as an emulator action with only the extra word.
    let e = crate::model::GameAction {
        kind: crate::model::ActionKind::Emulator,
        args: "-nofullscreen".into(),
        ..Default::default()
    };
    assert_eq!(tail(action_spec(&reg, &g, &e, None)), sv(&["-batch", "-nofullscreen", "--", iso]));
    // {Rom} instead of the path, and no arguments at all.
    let r = crate::model::GameAction { args: "-nofullscreen -- {Rom}".into(), ..Default::default() };
    assert_eq!(tail(action_spec(&reg, &g, &r, None)), sv(&["-batch", "-nofullscreen", "--", iso]));
    let plain = crate::model::GameAction::default();
    assert_eq!(tail(action_spec(&reg, &g, &plain, None)), sv(&["-batch", "-fullscreen", "--", iso]));
}

#[test]
fn switches_at_every_level() {
    let (_t, mut reg) = user_machine();
    let iso = "/mnt/space1/games/ps2/Ico.iso";
    let mut g = ps2_game(iso);
    g.emu.as_mut().unwrap().folder = "f1".into();
    let play =
        |reg: &Registry, g: &Game| tail(spec_with(reg, g, g.emu.as_ref().unwrap(), None).ok_or_else(String::new));
    assert_eq!(play(&reg, &g), sv(&["-batch", "-fullscreen", "--", iso]));
    // The emulator: fast boot on, close-when-done off.
    let mut emu_set = crate::emulation::options::EmuSettings::default();
    emu_set.options.insert("fastboot".into(), true);
    emu_set.options.insert("batch".into(), false);
    reg.config.emulator_settings.insert("pcsx2".into(), emu_set);
    assert_eq!(play(&reg, &g), sv(&["-fullscreen", "-fastboot", "--", iso]));
    // The console: windowed, plus an extra argument.
    let mut con = crate::emulation::options::EmuSettings::default();
    con.options.insert("fullscreen".into(), false);
    con.extra_args = "-turbo".into();
    reg.config.console_settings.insert("sony_playstation2".into(), con);
    assert_eq!(play(&reg, &g), sv(&["-nofullscreen", "-fastboot", "-turbo", "--", iso]));
    // The game: slow boot (Off adds the other side) and fullscreen typed back in.
    g.edits.emulator_options.insert("fastboot".into(), false);
    g.edits.emulator_args = "-fullscreen".into();
    assert_eq!(play(&reg, &g), sv(&["-fullscreen", "-slowboot", "-turbo", "--", iso]));
    // Play in a window wins over everything.
    let w = spec_with(&reg, &g, g.emu.as_ref().unwrap(), Some(false)).unwrap();
    assert_eq!(tail(Ok(w)), sv(&["-nofullscreen", "-slowboot", "-turbo", "--", iso]));
    // An emulator action on top: Big Picture on, its own extra word last before the ROM.
    let a = crate::model::GameAction {
        kind: crate::model::ActionKind::Emulator,
        args: "-state 1".into(),
        options: [("bigpicture".to_string(), true)].into(),
        ..Default::default()
    };
    assert_eq!(
        tail(action_spec(&reg, &g, &a, None)),
        sv(&["-fullscreen", "-slowboot", "-bigpicture", "-turbo", "-state", "1", "--", iso])
    );
}

#[test]
fn replace_arguments_and_the_insertion_point() {
    let (_t, reg) = user_machine();
    let iso = "/mnt/space1/games/ps2/Ico.iso";
    let mut g = ps2_game(iso);
    g.emu.as_mut().unwrap().folder = "f1".into();
    // The whole list: {Rom} filled in …
    let a = crate::model::GameAction {
        kind: crate::model::ActionKind::Emulator,
        replace_args: true,
        args: "-nogui {Rom}".into(),
        ..Default::default()
    };
    assert_eq!(tail(action_spec(&reg, &g, &a, None)), sv(&["-nogui", iso]));
    // … or added at the end when they don't name it.
    let b = crate::model::GameAction { args: "-nogui".into(), ..a.clone() };
    assert_eq!(tail(action_spec(&reg, &g, &b, None)), sv(&["-nogui", iso]));
    // Dolphin: extra arguments go before `-e <rom>` ({args} in its template).
    let rvz = "/mnt/space1/games/gc/Fire Emblem.rvz";
    let mut gc = Game::blank("FE");
    gc.emu = Some(emu_ref("nintendo_gamecube", rvz, &[]));
    gc.edits.emulator_args = "-d".into();
    let (_, args) = argv(spec_with(&reg, &gc, gc.emu.as_ref().unwrap(), None));
    let i = args.iter().position(|a| a == "-d").unwrap();
    assert_eq!(&args[i..], &sv(&["-d", "-e", rvz])[..]);
    // A missing emulator chosen by the action says so.
    let gone = crate::model::GameAction {
        kind: crate::model::ActionKind::Emulator,
        emulator: Some("nope".into()),
        ..Default::default()
    };
    assert!(action_spec(&reg, &g, &gone, None).unwrap_err().contains("isn't installed"));
    // A typo in typed arguments is an error, not a silent fallback.
    g.edits.emulator_args = "-x 'oops".into();
    assert!(
        action_spec(
            &reg,
            &g,
            &crate::model::GameAction { kind: crate::model::ActionKind::Emulator, ..Default::default() },
            None
        )
        .is_err()
    );
}

#[test]
fn templates_put_extra_arguments_before_value_flags() {
    // Every template whose game file follows a flag that takes it (`-e`, `-g`, `-conf` …) marks
    // where extra arguments go, so they never split the flag from its value.
    for d in super::super::defs::all() {
        for p in d.profiles {
            let w = crate::launch::split_command(p.args).unwrap();
            let Some(i) = w.iter().position(|x| super::super::options::is_rom_word(x)) else { continue };
            let before = if i > 0 { w[i - 1].trim_start_matches("{fs}").trim_start_matches("{win}") } else { "" };
            let takes_value = ["-e", "-g", "-L", "-dvd_path", "-rompath", "-conf", "-p", "--system"].contains(&before)
                || before.starts_with("--system");
            if takes_value && d.id != "retroarch" && d.id != "ares" {
                assert!(w.iter().any(|x| x == "{args}"), "{}/{}: {}", d.id, p.id, p.args);
            }
        }
    }
}

#[test]
fn an_old_copy_kept_aside_is_never_the_automatic_choice() {
    let ds = |id: &str, path: &str| Emulator {
        id: id.into(),
        def: "duckstation".into(),
        name: "DuckStation".into(),
        kind: InstallKind::AppImage { path: path.into() },
        how: String::new(),
        user: false,
        cores: vec![],
        args: String::new(),
        auto: true,
        blocked: false,
    };
    // Found first (its folder sorts first), but a backup.
    let reg = Registry {
        emulators: vec![
            ds("duckstation", "/e/Duckstation.backup/DuckStation.AppImage"),
            ds("duckstation-2", "/e/DuckStation/DuckStation.AppImage"),
        ],
        config: EmulationConfig::detect_nothing(),
        home: "/home/u".into(),
    };
    let (e, _) = choose(&reg, "sony_playstation", Path::new("/r/crash.chd"), None, None, None).expect("an emulator");
    assert_eq!(e.id, "duckstation-2");
    // Picked by the user: theirs, old or not.
    let (e, _) = choose(&reg, "sony_playstation", Path::new("/r/crash.chd"), Some("duckstation"), None, None).unwrap();
    assert_eq!(e.id, "duckstation");
    assert!(reg.emulators[0].looks_old() && !reg.emulators[1].looks_old());
}
