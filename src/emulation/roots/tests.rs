//! Fixture trees for every portable / AppImage / Flatpak layout: where each install keeps its
//! files, BIOS already there = ready, firmware installed, the wider BIOS search, "Set up
//! everything".

use super::*;
use crate::config::EmulationConfig;
use crate::emulation::bios::{self, BiosState, Mood};
use crate::emulation::detect::{self, ProbeRoots};
use crate::emulation::registry::Registry;
use crate::test_support::{write_exe, write_file};

const FAKE_PS1: &[u8] = b"lodestar fake ps1 bios";
/// An ELF that is an AppImage (type 2) by its content.
const APPIMAGE: &[u8] = b"\x7fELF\x02\x01\x01\0AI\x02\0\0\0\0\0rest";

/// A 4 MiB PS2 BIOS look-alike (RESET + ROMDIR + ROMVER), region byte `r`.
fn fake_ps2(r: u8) -> Vec<u8> {
    let mut d = vec![0u8; 4 * 1024 * 1024];
    let entry = |name: &[u8], size: u32| {
        let mut e = [0u8; 16];
        e[..name.len()].copy_from_slice(name);
        e[12..].copy_from_slice(&size.to_le_bytes());
        e
    };
    d[0x100..0x110].copy_from_slice(&entry(b"RESET", 0x100));
    d[0x110..0x120].copy_from_slice(&entry(b"ROMDIR", 0x40));
    d[0x120..0x130].copy_from_slice(&entry(b"ROMVER", 0x10));
    let mut v = *b"0230AC20080220";
    v[4] = r;
    d[0x140..0x14e].copy_from_slice(&v);
    d
}

/// A PUP whose version.txt entry says `v`.
fn fake_pup(v: &[u8]) -> Vec<u8> {
    let mut pup = vec![0u8; 0x80];
    pup[..8].copy_from_slice(b"SCEUF\0\0\0");
    pup[0x18..0x20].copy_from_slice(&1u64.to_be_bytes());
    pup[0x30..0x38].copy_from_slice(&0x100u64.to_be_bytes());
    pup[0x38..0x40].copy_from_slice(&0x70u64.to_be_bytes());
    pup[0x40..0x48].copy_from_slice(&(v.len() as u64).to_be_bytes());
    pup[0x70..0x70 + v.len()].copy_from_slice(v);
    pup
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
        auto: true,
        blocked: false,
    }
}

fn appimage(p: &Path) -> InstallKind {
    InstallKind::AppImage { path: p.to_path_buf() }
}

fn program(p: &Path) -> InstallKind {
    InstallKind::Binary { path: p.to_path_buf() }
}

fn flatpak(app: &str) -> InstallKind {
    InstallKind::Flatpak { app_id: app.into() }
}

fn reg(home: &Path, emulators: Vec<Emulator>, bios_dirs: Vec<PathBuf>) -> Registry {
    Registry {
        emulators,
        config: EmulationConfig { bios_dirs, ..EmulationConfig::detect_nothing() },
        home: home.to_path_buf(),
    }
}

fn names(id: &str) -> String {
    match id {
        "duckstation" | "duckstation-appimage" => "DuckStation".into(),
        "pcsx2" => "PCSX2".into(),
        "eden" => "Eden".into(),
        "rpcs3" => "RPCS3".into(),
        other => other.into(),
    }
}

fn summary_of(r: &Registry, st: &bios::BiosStatus) -> (bios::Summary, Vec<bios::Step>) {
    let plan = bios::setup_plan(r, st, None);
    let s = bios::summary(st, &plan, &r.home, &names, &|id| r.emulator(id).is_some_and(|e| !e.blocked));
    (s, plan)
}

#[test]
fn portable_duckstation_on_a_drive_is_found_and_its_bios_is_ready() {
    // A real portable layout: /mnt/FastSSD/Gaming/Emulation/Emulators/Duckstation/{Duckstation, settings.ini, bios/}.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let dir = root.join("mnt/FastSSD/Gaming/Emulation/Emulators/Duckstation");
    write_exe(&dir.join("Duckstation"), APPIMAGE);
    write_file(&dir.join("settings.ini"), "[Main]\nSettingsVersion = 3\n");
    write_file(&dir.join("bios/my ps1 dump.bin"), FAKE_PS1);
    std::fs::create_dir_all(dir.join("memcards")).unwrap();
    std::fs::create_dir_all(dir.join("gamesettings")).unwrap();
    let roots = ProbeRoots::under(root, &home);
    let mut ctx = crate::scan::desktop::test_support::ctx(&home);
    ctx.emulation = EmulationConfig { auto_emulators: true, auto_folders: false, ..Default::default() };
    // A config from before its one-time check: the drives' "Emulators" folders are searched.
    let found = detect::detect_auto(&ctx, &detect::searched_roots(&roots, &ctx.emulation));
    let ds: Vec<&Emulator> = found.iter().filter(|e| e.def == "duckstation").collect();
    assert_eq!(ds.len(), 1, "{found:?}");
    let ds = ds[0].clone();
    assert_eq!(ds.kind, appimage(&dir.join("Duckstation")), "an AppImage by its content, without the ending");
    assert!(ds.how.contains("Emulators/Duckstation") && ds.how.ends_with(" · portable"), "{}", ds.how);
    let u = user_dir(&ds, &home).unwrap();
    assert_eq!((u.path.clone(), u.how), (dir.clone(), Where::Portable));
    assert_eq!(bios::bios_dir(&ds, &home), Some(dir.join("bios")));
    // BIOS already in its own folder: ready, nothing to choose or copy.
    let r = reg(&home, vec![ds.clone()], vec![]);
    let st = bios::check(&r, "sony_playstation");
    assert_eq!(st.state, BiosState::Ok);
    assert_eq!(st.targets.len(), 1);
    assert!(st.targets[0].ready && st.targets[0].missing.is_empty(), "{:?}", st.targets);
    let (s, plan) = summary_of(&r, &st);
    assert!(plan.is_empty(), "{plan:?}");
    assert_eq!(s.text, "Ready in DuckStation ✓");
    assert_eq!((s.mood, s.button), (Mood::Ready, None));
    // Its own setting moves the BIOS folder.
    let other = root.join("mnt/FastSSD/bios-elsewhere");
    write_file(&dir.join("settings.ini"), format!("[BIOS]\nSearchDirectory = {}\n", other.display()));
    assert_eq!(bios::bios_dir(&ds, &home), Some(other));
    // portable.txt works the same.
    std::fs::remove_file(dir.join("settings.ini")).unwrap();
    assert_eq!(user_dir(&ds, &home).unwrap().how, Where::Usual);
    write_file(&dir.join("portable.txt"), "");
    assert_eq!(user_dir(&ds, &home).unwrap().path, dir);
}

#[test]
fn appimage_home_and_config_folders_are_the_emulators_folders() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    // PCSX2 AppImage with its own HOME (<AppImage>.home).
    let ai = home.join("Applications/pcsx2-v2.2-linux-appimage-x64-Qt.AppImage");
    write_exe(&ai, APPIMAGE);
    let side = PathBuf::from(format!("{}.home", ai.display()));
    write_file(&side.join(".config/PCSX2/bios/usa.bin"), fake_ps2(b'A'));
    let p = emu("pcsx2", "pcsx2", appimage(&ai));
    let u = user_dir(&p, &home).unwrap();
    assert_eq!((u.path, u.how), (side.join(".config/PCSX2"), Where::AppImageHome));
    let r = reg(&home, vec![p.clone()], vec![]);
    let st = bios::check(&r, "sony_playstation2");
    assert!(st.targets[0].ready, "{:?}", st.targets);
    assert_eq!(summary_of(&r, &st).0.text, "Ready in PCSX2 ✓");
    // DuckStation AppImage with <AppImage>.config (its XDG_CONFIG_HOME).
    let ds = home.join("Apps/DuckStation-x64.AppImage");
    write_exe(&ds, APPIMAGE);
    let cfg = PathBuf::from(format!("{}.config", ds.display()));
    std::fs::create_dir_all(cfg.join("duckstation")).unwrap();
    let d = emu("duckstation", "duckstation", appimage(&ds));
    assert_eq!(bios::bios_dir(&d, &home), Some(cfg.join("duckstation/bios")));
    // RetroArch AppImage: its .config folder holds retroarch.cfg (system_directory relative to it).
    let ra = home.join("Apps/RetroArch-Linux-x86_64.AppImage");
    write_exe(&ra, APPIMAGE);
    let ra_cfg = PathBuf::from(format!("{}.config", ra.display())).join("retroarch/retroarch.cfg");
    write_file(&ra_cfg, "system_directory = \":/sys\"\n");
    let e = emu("retroarch", "retroarch", appimage(&ra));
    assert_eq!(detect::retroarch_cfg(&e.kind, &home), ra_cfg);
    assert_eq!(bios::bios_dir(&e, &home), Some(ra_cfg.parent().unwrap().join("sys")));
    // A plain AppImage without folders of its own: the usual ones.
    let plain = home.join("Apps/Cemu-2.6.AppImage");
    write_exe(&plain, APPIMAGE);
    let c = emu("cemu", "cemu", appimage(&plain));
    assert_eq!(user_dir(&c, &home).unwrap(), UserDir { path: home.join(".local/share/Cemu"), how: Where::Usual });
}

#[test]
fn pcsx2_portable_reads_its_own_ini() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let dir = tmp.path().join("drive/Emulators/PCSX2");
    write_exe(&dir.join("pcsx2-qt"), b"\x7fELF");
    write_file(&dir.join("portable.ini"), "");
    write_file(&dir.join("inis/PCSX2.ini"), "[Folders]\nBios = mybios\n");
    write_file(&dir.join("mybios/scph39001.bin"), fake_ps2(b'A'));
    let p = emu("pcsx2", "pcsx2", program(&dir.join("pcsx2-qt")));
    assert_eq!(user_dir(&p, &home).unwrap(), UserDir { path: dir.clone(), how: Where::Portable });
    assert_eq!(bios::bios_dir(&p, &home), Some(dir.join("mybios")));
    let r = reg(&home, vec![p], vec![]);
    let st = bios::check(&r, "sony_playstation2");
    assert_eq!(st.state, BiosState::Ok);
    assert!(st.targets[0].ready);
    // portable.txt instead of portable.ini.
    std::fs::remove_file(dir.join("portable.ini")).unwrap();
    write_file(&dir.join("portable.txt"), "");
    let p = emu("pcsx2", "pcsx2", program(&dir.join("pcsx2-qt")));
    assert_eq!(user_dir(&p, &home).unwrap().how, Where::Portable);
}

#[test]
fn eden_portable_user_folder_has_keys_and_firmware() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let dir = tmp.path().join("drive/Emulators/Eden");
    write_exe(&dir.join("eden"), b"\x7fELF");
    write_file(&dir.join("user/keys/prod.keys"), "header_key = 00\n");
    write_file(&dir.join("user/keys/title.keys"), "0100 = 11\n");
    write_file(&dir.join("user/nand/system/Contents/registered/aaaa.nca"), b"nca");
    // A firmware package elsewhere (a different version): never asked for, Eden has firmware.
    let fw = home.join("Documents/bios files/switch/Firmware.23.5.0");
    for i in 0..24 {
        write_file(&fw.join(format!("{i:02x}.nca")), b"x");
    }
    let e = emu("eden", "eden", program(&dir.join("eden")));
    assert_eq!(user_dir(&e, &home).unwrap(), UserDir { path: dir.join("user"), how: Where::Portable });
    assert_eq!(bios::bios_dir(&e, &home), Some(dir.join("user/keys")));
    assert_eq!(bios::switch_firmware_dir(&e, &home), Some(dir.join("user/nand/system/Contents/registered")));
    let r = reg(&home, vec![e.clone()], vec![]);
    let st = bios::check(&r, "nintendo_switch");
    assert_eq!(st.state, BiosState::Ok, "{:?}", st.files);
    let t = &st.targets[0];
    assert!(t.ready && t.firmware.as_deref() == Some("installed"), "{t:?}");
    let (s, plan) = summary_of(&r, &st);
    assert!(plan.is_empty(), "nothing to install: {plan:?}");
    assert_eq!(s.text, "Keys and firmware are in Eden ✓");
    // Its qt-config.ini moves the NAND.
    let nand = tmp.path().join("big/nand");
    write_file(
        &dir.join("user/config/qt-config.ini"),
        format!("[Data%20Storage]\nnand_directory\\default=false\nnand_directory={}/\n", nand.display()),
    );
    assert_eq!(bios::switch_firmware_dir(&e, &home), Some(nand.join("system/Contents/registered")));
    // Keys there but no firmware anywhere: the sentence says which half is missing.
    std::fs::remove_dir_all(&fw).unwrap();
    let r = reg(&home, vec![e], vec![]);
    let st = bios::check(&r, "nintendo_switch");
    let (s, _) = summary_of(&r, &st);
    assert_eq!(s.text, "Keys are in Eden; no Switch firmware found.");
    assert_eq!(s.mood, Mood::Missing);
}

#[test]
fn rpcs3_with_firmware_installed_never_asks_for_a_pup() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let dir = tmp.path().join("drive/Emulators/RPCS3");
    let ai = dir.join("rpcs3-v0.0.43-20644_linux64.AppImage");
    write_exe(&ai, APPIMAGE);
    write_file(&dir.join("portable/dev_flash/vsh/etc/version.txt"), "release:04.93:build:x\n");
    // The user's BIOS folder exists from the start (the "*bios*" folder search is cached a few seconds).
    std::fs::create_dir_all(home.join("Documents/bios files/ps3")).unwrap();
    let e = emu("rpcs3", "rpcs3", appimage(&ai));
    assert_eq!(user_dir(&e, &home).unwrap(), UserDir { path: dir.join("portable"), how: Where::Portable });
    assert_eq!(bios::ps3_firmware(&e, &home).as_deref(), Some("04.93"));
    // Installed, no PUP anywhere (it was deleted after installing): ready, nothing asked.
    let r = reg(&home, vec![e.clone()], vec![]);
    let st = bios::check(&r, "sony_playstation3");
    assert_eq!(st.state, BiosState::Ok);
    let (s, plan) = summary_of(&r, &st);
    assert!(plan.is_empty());
    assert_eq!(s.text, "Firmware 4.93 installed in RPCS3 ✓");
    assert_eq!(s.button, None);
    // A PUP of the same version found too: still nothing to do.
    write_file(&home.join("Documents/bios files/ps3/PS3UPDAT.PUP"), fake_pup(b"4.93"));
    let st = bios::check(&r, "sony_playstation3");
    assert!(summary_of(&r, &st).1.is_empty());
    // A Flatpak whose vfs.yml moved dev_flash.
    let f = emu("rpcs3", "rpcs3", flatpak("net.rpcs3.RPCS3"));
    let cfg = home.join(".var/app/net.rpcs3.RPCS3/config/rpcs3");
    write_file(
        &cfg.join("vfs.yml"),
        "$(EmulatorDir): \"\"\n/dev_hdd0/: $(EmulatorDir)dev_hdd0/\n/dev_flash/: $(EmulatorDir)flash_moved/\n",
    );
    assert_eq!(rpcs3_dev_flash(&f, &home), Some(cfg.join("flash_moved")));
    // Not installed there: the PUP found is the one step.
    let r = reg(&home, vec![f], vec![]);
    let st = bios::check(&r, "sony_playstation3");
    let (s, plan) = summary_of(&r, &st);
    assert!(matches!(plan.as_slice(), [bios::Step::Ps3Firmware { .. }]), "{plan:?}");
    assert_eq!(s.text, "PS3 firmware found in ~/Documents/bios files/ps3 — not in RPCS3 yet.");
    assert_eq!(s.button.as_deref(), Some("Install into RPCS3"));
    // Nothing anywhere: say how to get it, one button to choose it.
    std::fs::remove_file(home.join("Documents/bios files/ps3/PS3UPDAT.PUP")).unwrap();
    let st = bios::check(&r, "sony_playstation3");
    let (s, _) = summary_of(&r, &st);
    assert_eq!(s.text, "No PS3 firmware found.");
    assert_eq!(s.button.as_deref(), Some("Choose firmware file…"));
    assert!(s.help.unwrap().contains("PS3UPDAT.PUP"));
}

#[test]
fn every_portable_marker_and_the_usual_folders() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    // (definition, program, marker, marker is a folder, portable folder relative to the program's)
    let cases: &[(&str, &str, &str, bool, &str)] = &[
        ("duckstation", "duckstation-qt", "portable.txt", false, ""),
        ("pcsx2", "pcsx2-qt", "portable.ini", false, ""),
        ("dolphin", "dolphin-emu", "portable.txt", false, "User"),
        ("cemu", "Cemu", "portable", true, "portable"),
        ("ryujinx", "Ryujinx", "portable", true, "portable"),
        ("rpcs3", "rpcs3", "portable", true, "portable"),
        ("eden", "eden", "user", true, "user"),
        ("citron", "citron", "user", true, "user"),
        ("sudachi", "sudachi", "user", true, "user"),
        ("azahar", "azahar", "user", true, "user"),
        ("citra", "citra-qt", "user", true, "user"),
        ("shadps4", "shadps4", "user", true, "user"),
        ("melonds", "melonDS", "melonDS.toml", false, ""),
        ("mgba", "mgba-qt", "portable.ini", false, ""),
        ("xemu", "xemu", "xemu.toml", false, ""),
        ("retroarch", "retroarch", "retroarch.cfg", false, ""),
    ];
    for (def, prog, marker, is_dir, rel) in cases {
        let dir = tmp.path().join("portable").join(def);
        write_exe(&dir.join(prog), b"\x7fELF");
        let e = emu(def, def, program(&dir.join(prog)));
        assert_ne!(user_dir(&e, &home).map(|u| u.how), Some(Where::Portable), "{def} without its marker");
        if *is_dir {
            std::fs::create_dir_all(dir.join(marker)).unwrap();
        } else {
            write_file(&dir.join(marker), "");
        }
        let u = user_dir(&e, &home).unwrap_or_else(|| panic!("{def}"));
        let want = if rel.is_empty() { dir.clone() } else { dir.join(rel) };
        assert_eq!((u.path, u.how), (want, Where::Portable), "{def}");
    }
    // The usual folders (native) and the Flatpak ones.
    let usual: &[(&str, &str)] = &[
        ("pcsx2", ".config/PCSX2"),
        ("dolphin", ".local/share/dolphin-emu"),
        ("cemu", ".local/share/Cemu"),
        ("ryujinx", ".config/Ryujinx"),
        ("rpcs3", ".config/rpcs3"),
        ("eden", ".local/share/eden"),
        ("duckstation", ".local/share/duckstation"),
        ("mednafen", ".mednafen"),
        ("flycast", ".local/share/flycast"),
    ];
    for (def, rel) in usual {
        let e = emu(def, def, program(Path::new("/usr/bin/x")));
        assert_eq!(user_dir(&e, &home), Some(UserDir { path: home.join(rel), how: Where::Usual }), "{def}");
    }
    let fl: &[(&str, &str, &str)] = &[
        ("duckstation", "org.duckstation.DuckStation", "config/duckstation"),
        ("pcsx2", "net.pcsx2.PCSX2", "config/PCSX2"),
        ("dolphin", "org.DolphinEmu.dolphin-emu", "data/dolphin-emu"),
        ("eden", "dev.eden_emu.eden", "data/eden"),
        ("rpcs3", "net.rpcs3.RPCS3", "config/rpcs3"),
        ("ryujinx", "io.github.ryubing.Ryujinx", "config/Ryujinx"),
        ("cemu", "info.cemu.Cemu", "data/Cemu"),
    ];
    for (def, app, rel) in fl {
        let e = emu(def, def, flatpak(app));
        let want = home.join(".var/app").join(app).join(rel);
        assert_eq!(user_dir(&e, &home), Some(UserDir { path: want, how: Where::Flatpak }), "{def}");
    }
    // Where BIOS / keys / firmware go in portable copies.
    let ry = tmp.path().join("portable/ryujinx");
    let e = emu("ryujinx", "ryujinx", program(&ry.join("Ryujinx")));
    assert_eq!(bios::bios_dir(&e, &home), Some(ry.join("portable/system")));
    assert_eq!(bios::switch_firmware_dir(&e, &home), Some(ry.join("portable/bis/system/Contents/registered")));
    let ce = tmp.path().join("portable/cemu");
    assert_eq!(bios::bios_dir(&emu("cemu", "cemu", program(&ce.join("Cemu"))), &home), Some(ce.join("portable")));
    let dol = tmp.path().join("portable/dolphin");
    assert_eq!(
        bios::bios_dir(&emu("dolphin", "dolphin", program(&dol.join("dolphin-emu"))), &home),
        Some(dol.join("User"))
    );
    // RetroArch: retroarch.cfg next to the program.
    let ra = tmp.path().join("portable/retroarch");
    let e = emu("retroarch", "retroarch", program(&ra.join("retroarch")));
    assert_eq!(bios::bios_dir(&e, &home), Some(ra.join("system")));
}

#[test]
fn files_the_emulators_settings_name_count_as_theirs() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let dir = tmp.path().join("melonds");
    write_exe(&dir.join("melonDS"), b"\x7fELF");
    let b9 = tmp.path().join("elsewhere/bios9.bin");
    write_file(&b9, b"arm9");
    write_file(&dir.join("melonDS.toml"), format!("[DS]\nBIOS9Path = \"{}\"\nBIOS7Path = \"\"\n", b9.display()));
    let e = emu("melonds", "melonds", program(&dir.join("melonDS")));
    assert_eq!(config_named_files(&e, &home), vec![b9]);
    let x = tmp.path().join("xemu");
    write_exe(&x.join("xemu"), b"\x7fELF");
    write_file(&x.join("mcpx.bin"), b"boot");
    write_file(&x.join("xemu.toml"), "[sys.files]\nbootrom_path = 'mcpx.bin'\nhdd_path = 'missing.qcow2'\n");
    let e = emu("xemu", "xemu", program(&x.join("xemu")));
    assert_eq!(config_named_files(&e, &home), vec![x.join("mcpx.bin")]);
}

#[test]
fn bios_folders_on_drives_and_in_emulator_folders_are_found_and_set_up() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    set_test_drives(vec![root.join("mnt")]);
    // A drive's Emulation/bios folder, and a PS2 dump only inside an emulator collection.
    write_file(&root.join("mnt/FastSSD/Gaming/Emulation/bios/ps1 usa.bin"), FAKE_PS1);
    let deep = root.join("mnt/Y/a/b/c/Emulators/Other/bios");
    write_file(&deep.join("ps2.bin"), fake_ps2(b'E'));
    let ds = emu("duckstation", "duckstation", flatpak("org.duckstation.DuckStation"));
    let mut portable = emu(
        "duckstation-appimage",
        "duckstation",
        appimage(&root.join("mnt/FastSSD/Emulators/DS/DuckStation.AppImage")),
    );
    write_exe(&root.join("mnt/FastSSD/Emulators/DS/DuckStation.AppImage"), APPIMAGE);
    write_file(&root.join("mnt/FastSSD/Emulators/DS/portable.txt"), "");
    let p2 = emu("pcsx2", "pcsx2", flatpak("net.pcsx2.PCSX2"));
    let r = reg(&home, vec![ds.clone(), portable.clone(), p2], vec![]);
    let st = bios::check(&r, "sony_playstation");
    assert_eq!(st.state, BiosState::Ok, "{:?}", st.files);
    let src = root.join("mnt/FastSSD/Gaming/Emulation/bios/ps1 usa.bin");
    assert!(st.files.iter().any(|(_, s, p)| *s == BiosState::Ok && p.as_deref() == Some(src.as_path())));
    let (s, plan) = summary_of(&r, &st);
    assert_eq!(plan.len(), 2, "{plan:?}");
    assert_eq!(s.button.as_deref(), Some("Set up everything"));
    assert_eq!(s.mood, Mood::Todo);
    assert!(
        s.text.starts_with("PlayStation BIOS found in ") && s.text.ends_with("— not in DuckStation yet."),
        "{}",
        s.text
    );
    let line = bios::step_line(&plan[1], &names, &home);
    assert!(line.starts_with("Copy scph-test.bin to DuckStation ("), "{line}");
    // Do it: both have it, both ready, nothing left.
    assert_eq!(bios::run_setup(&st, &plan).unwrap(), 2);
    let st = bios::check(&r, "sony_playstation");
    assert!(st.targets.iter().all(|t| t.ready), "{:?}", st.targets);
    let (s, plan) = summary_of(&r, &st);
    assert!(plan.is_empty());
    assert_eq!(s.text, "Ready in DuckStation and DuckStation ✓");
    // One not allowed to run yet: left out of the plan.
    let r2 = {
        portable.blocked = true;
        let fresh = root.join("home2");
        std::fs::create_dir_all(&fresh).unwrap();
        reg(&fresh, vec![emu("duckstation", "duckstation", flatpak("org.duckstation.DuckStation")), portable], vec![])
    };
    let st2 = bios::check(&r2, "sony_playstation");
    let (s2, plan2) = summary_of(&r2, &st2);
    assert_eq!(plan2.len(), 1, "{plan2:?}");
    assert_eq!(s2.button.as_deref(), Some("Copy to DuckStation"));
    // The PS2 dump inside a collection that isn't one of the user's emulator folders (an old
    // copy) is never searched; once that folder is an emulator folder, it is.
    let st = bios::check(&r, "sony_playstation2");
    assert!(st.files.iter().all(|(_, _, p)| p.as_ref().is_none_or(|p| !p.starts_with(&deep))), "{:?}", st.files);
    let mut r = r;
    r.config.emulator_dirs = vec![root.join("mnt/Y/a/b/c/Emulators")];
    let st = bios::check(&r, "sony_playstation2");
    assert!(
        st.files.iter().any(|(_, s, p)| *s == BiosState::Ok && p.as_ref().is_some_and(|p| p.starts_with(&deep))),
        "{:?}",
        st.files
    );
    set_test_drives(vec![]);
}

#[test]
fn set_up_everything_never_replaces_a_different_file() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    write_file(&home.join("Documents/bios files/ps1/scph5501.bin"), FAKE_PS1);
    let flat_bios = home.join(".var/app/org.duckstation.DuckStation/config/duckstation/bios");
    // A different file with the name the copy would get.
    write_file(&flat_bios.join("scph-test.bin"), b"something else");
    let r = reg(&home, vec![emu("duckstation", "duckstation", flatpak("org.duckstation.DuckStation"))], vec![]);
    let st = bios::check(&r, "sony_playstation");
    assert_eq!(st.targets[0].missing.len(), 1);
    assert!(bios::setup_plan(&r, &st, None).is_empty(), "the copy it has is kept");
    assert_eq!(std::fs::read(flat_bios.join("scph-test.bin")).unwrap(), b"something else");
}

#[test]
fn nothing_found_says_how_to_get_it() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let r = reg(&home, vec![emu("duckstation", "duckstation", flatpak("org.duckstation.DuckStation"))], vec![]);
    let st = bios::check(&r, "sony_playstation");
    let (s, plan) = summary_of(&r, &st);
    assert!(plan.is_empty());
    assert_eq!(s.text, "No PlayStation BIOS found.");
    assert_eq!(s.button.as_deref(), Some("Choose BIOS files…"));
    assert!(s.help.unwrap().contains("never downloads"));
    assert_eq!(bios::what_of("sony_playstation2"), "PlayStation 2 BIOS");
    assert_eq!(bios::join_and(&["a".into(), "b".into(), "c".into()]), "a, b and c");
}

#[test]
fn emulator_collections_in_the_home_folder_are_found() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    write_exe(&home.join("Emulation/Emulators/RPCS3/rpcs3"), b"\x7fELF");
    write_exe(&home.join("Emulation/Emulators/PCSX2/pcsx2-v2.2.AppImage"), APPIMAGE);
    let roots = ProbeRoots::under(root, &home);
    let mut ctx = crate::scan::desktop::test_support::ctx(&home);
    ctx.emulation = EmulationConfig { auto_emulators: true, auto_folders: false, ..Default::default() };
    let found = detect::detect_auto(&ctx, &roots);
    let rp = found.iter().find(|e| e.def == "rpcs3").expect("rpcs3");
    assert_eq!(rp.kind, program(&home.join("Emulation/Emulators/RPCS3/rpcs3")));
    let p2 = found.iter().find(|e| e.def == "pcsx2").expect("pcsx2");
    assert!(matches!(p2.kind, InstallKind::AppImage { .. }));
    assert_eq!(home_emulator_trees(&home), vec![home.join("Emulation/Emulators")]);
}

#[test]
fn each_drive_is_searched_on_its_own_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let mnt = tmp.path().join("mnt");
    // A big first drive (more folders than the budget) and a second one with a BIOS folder.
    for i in 0..20 {
        std::fs::create_dir_all(mnt.join(format!("A/folder{i:02}/sub"))).unwrap();
    }
    std::fs::create_dir_all(mnt.join("B/Gaming/Emulation/bios")).unwrap();
    std::fs::create_dir_all(mnt.join("windows/Users/bios")).unwrap();
    let bios = |n: &str| n.contains("bios");
    assert_eq!(find_on_drives(std::slice::from_ref(&mnt), 6, 5, &bios), vec![mnt.join("B/Gaming/Emulation/bios")]);
    // Levels count from the mount folder: B/Gaming/Emulation/bios is 4 down.
    assert!(find_on_drives(std::slice::from_ref(&mnt), 3, 100, &bios).is_empty());
    assert_eq!(find_on_drives(std::slice::from_ref(&mnt), 4, 100, &bios).len(), 1);
}

/// A tester's drive: an "Emulators" folder with current emulators and old copies of them.
fn drive_with_old_copies(root: &Path) -> PathBuf {
    let e = root.join("mnt/space9/Gaming/Emulation/Emulators");
    for f in [
        "DuckStation/DuckStation.AppImage",
        "DuckStation.backup/DuckStation.AppImage",
        "RPCS3/rpcs3-v0.0.38.AppImage",
        "RPCS3_old/rpcs3-v0.0.30.AppImage",
        "Ryujinx/Ryujinx.AppImage",
    ] {
        write_exe(&e.join(f), APPIMAGE);
    }
    e
}

#[test]
fn drive_emulator_folders_are_suggested_not_searched() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let tree = drive_with_old_copies(root);
    // An empty "Emulators" folder on another drive: nothing to suggest there.
    std::fs::create_dir_all(root.join("mnt/usb/Emulators/nothing")).unwrap();
    let roots = ProbeRoots::under(root, &home);
    let mut ctx = crate::scan::desktop::test_support::ctx(&home);
    let checked = EmulationConfig {
        auto_emulators: true,
        auto_folders: false,
        emulator_dirs_checked: true,
        ..Default::default()
    };
    ctx.emulation = checked.clone();
    let on_drive = |v: &[Emulator]| v.iter().filter(|e| e.program_path().is_some_and(|p| p.starts_with(&tree))).count();
    // Checked config without emulator folders: the drive's emulators aren't used …
    assert_eq!(on_drive(&detect::detect_with(&ctx, &roots)), 0);
    // … but offered, once per folder (the empty one isn't).
    assert_eq!(detect::suggested_dirs(&ctx.emulation, &roots), vec![tree.clone()]);
    // Dismissed: not offered again. Emulators found automatically switched off: nothing offered.
    let mut c = checked.clone();
    c.dismissed_emulator_dirs.push(tree.clone());
    assert!(detect::suggested_dirs(&c, &roots).is_empty());
    let mut c = checked.clone();
    c.auto_emulators = false;
    assert!(detect::suggested_dirs(&c, &roots).is_empty());
    // Added as an emulator folder: searched with its sub-folders; no suggestion any more.
    ctx.emulation.emulator_dirs.push(tree.clone());
    let found = detect::detect_with(&ctx, &roots);
    assert_eq!(on_drive(&found), 5, "{found:?}");
    assert!(detect::suggested_dirs(&ctx.emulation, &roots).is_empty());
    assert!(found.iter().any(|e| e.def == "ryujinx" && e.name == "Ryujinx"));
    // Any folder can be one, not only those named "Emulators".
    let other = root.join("games/emus");
    write_exe(&other.join("PCSX2/pcsx2-v2.2.0.AppImage"), APPIMAGE);
    ctx.emulation.emulator_dirs = vec![other.clone()];
    let found = detect::detect_with(&ctx, &roots);
    assert!(
        found.iter().any(|e| e.def == "pcsx2" && e.program_path().is_some_and(|p| p.starts_with(&other))),
        "{found:?}"
    );
    assert_eq!(on_drive(&found), 0);
}

#[test]
fn configs_from_before_keep_their_drive_emulators() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let tree = drive_with_old_copies(root);
    std::fs::create_dir_all(root.join("mnt/usb/Emulators/nothing")).unwrap();
    let roots = ProbeRoots::under(root, &home);
    let mut ctx = crate::scan::desktop::test_support::ctx(&home);
    // Before the check: everything on the drives is searched, as before …
    ctx.emulation = EmulationConfig { auto_emulators: true, auto_folders: false, ..Default::default() };
    let before = detect::detect_with(&ctx, &roots);
    assert!(before.iter().any(|e| e.program_path().is_some_and(|p| p.starts_with(&tree))));
    // … and the check keeps the folder those emulators came from (not the empty one).
    assert_eq!(detect::dirs_to_migrate(&ctx.emulation, &roots), vec![tree.clone()]);
    // After it: the same emulators, with the same ids (choices and settings keep pointing at them).
    let mut after = ctx.clone();
    after.emulation.emulator_dirs = detect::dirs_to_migrate(&ctx.emulation, &roots);
    after.emulation.emulator_dirs_checked = true;
    detect::invalidate();
    assert_eq!(detect::detect_with(&after, &roots), before);
    // A tester's setup: found automatically switched off, emulators added by hand from that folder.
    let mut hand = EmulationConfig { auto_emulators: false, ..Default::default() };
    hand.emulators.insert(
        "duckstation".into(),
        crate::config::EmulatorEntry {
            name: "DuckStation".into(),
            follows: "duckstation".into(),
            kind: "appimage".into(),
            path: tree.join("DuckStation/DuckStation.AppImage"),
            ..Default::default()
        },
    );
    assert_eq!(detect::dirs_to_migrate(&hand, &roots), vec![tree.clone()]);
    // Nothing from there: nothing to keep.
    hand.emulators.clear();
    assert!(detect::dirs_to_migrate(&hand, &roots).is_empty());
    // Already an emulator folder: nothing to add.
    let mut listed = ctx.emulation.clone();
    listed.emulator_dirs.push(tree);
    assert!(detect::dirs_to_migrate(&listed, &roots).is_empty());
}

/// Settings folders and the path rules the emulators' own source code has:
/// `$XDG_CONFIG_HOME`, PCSX2's `portable.txt` contents and AppImage rule, PPSSPP without a
/// portable folder on Linux, Dolphin's and Flycast's settings apart from their data.
#[test]
fn settings_folders_follow_each_emulators_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let usual = |def: &str| emu(def, def, program(Path::new("/usr/bin/x")));
    let cfg = |e: &Emulator| config_dir(e, &home).map(|u| u.path);
    // No $XDG_CONFIG_HOME (tests never see the real one): the usual folders.
    set_test_xdg_config(None);
    assert_eq!(cfg(&usual("duckstation")), Some(home.join(".local/share/duckstation")));
    assert_eq!(cfg(&usual("pcsx2")), Some(home.join(".config/PCSX2/inis")));
    assert_eq!(cfg(&usual("ppsspp")), Some(home.join(".config/ppsspp/PSP/SYSTEM")));
    assert_eq!(cfg(&usual("dolphin")), Some(home.join(".config/dolphin-emu")));
    assert_eq!(user_dir(&usual("dolphin"), &home).unwrap().path, home.join(".local/share/dolphin-emu"), "data");
    assert_eq!(cfg(&usual("flycast")), Some(home.join(".config/flycast")));
    assert_eq!(user_dir(&usual("flycast"), &home).unwrap().path, home.join(".local/share/flycast"), "BIOS");
    assert_eq!(cfg(&usual("mgba")), None);
    // Flatpaks.
    let fl = |def: &str, app: &str| config_dir(&emu(def, def, flatpak(app)), &home).unwrap().path;
    let app = |a: &str| home.join(".var/app").join(a);
    assert_eq!(fl("dolphin", "org.DolphinEmu.dolphin-emu"), app("org.DolphinEmu.dolphin-emu/config/dolphin-emu"));
    assert_eq!(fl("flycast", "org.flycast.Flycast"), app("org.flycast.Flycast/config/flycast"));
    assert_eq!(fl("ppsspp", "org.ppsspp.PPSSPP"), app("org.ppsspp.PPSSPP/config/ppsspp/PSP/SYSTEM"));
    assert_eq!(fl("pcsx2", "net.pcsx2.PCSX2"), app("net.pcsx2.PCSX2/config/PCSX2/inis"));
    assert_eq!(fl("duckstation", "org.duckstation.DuckStation"), app("org.duckstation.DuckStation/config/duckstation"));
    // Dolphin's single user folders: ~/.dolphin-emu, portable User/.
    std::fs::create_dir_all(home.join(".dolphin-emu")).unwrap();
    assert_eq!(cfg(&usual("dolphin")), Some(home.join(".dolphin-emu/Config")));
    let dol = tmp.path().join("portable/dolphin");
    write_exe(&dol.join("dolphin-emu"), b"\x7fELF");
    write_file(&dol.join("portable.txt"), "");
    assert_eq!(cfg(&emu("d", "dolphin", program(&dol.join("dolphin-emu")))), Some(dol.join("User/Config")));
    // $XDG_CONFIG_HOME set (inside the home): honoured, whether or not the folder exists yet.
    let xdg = home.join("cfg");
    set_test_xdg_config(Some(xdg.clone()));
    assert_eq!(cfg(&usual("duckstation")), Some(xdg.join("duckstation")));
    assert_eq!(cfg(&usual("pcsx2")), Some(xdg.join("PCSX2/inis")));
    assert_eq!(cfg(&usual("ppsspp")), Some(xdg.join("ppsspp/PSP/SYSTEM")));
    assert_eq!(cfg(&usual("flycast")), Some(xdg.join("flycast")));
    assert_eq!(cfg(&usual("dolphin")), Some(home.join(".dolphin-emu/Config")), "~/.dolphin-emu still first");
    assert_eq!(bios::bios_dir(&usual("duckstation"), &home), Some(xdg.join("duckstation/bios")));
    // …but not for a Flatpak (it has its own).
    assert_eq!(fl("flycast", "org.flycast.Flycast"), app("org.flycast.Flycast/config/flycast"));
    // RetroArch: $XDG_CONFIG_HOME/retroarch, then ~/.retroarch.cfg when that's all there is.
    let ra = usual("retroarch");
    assert_eq!(detect::retroarch_cfg(&ra.kind, &home), xdg.join("retroarch/retroarch.cfg"));
    write_file(&home.join(".retroarch.cfg"), "");
    assert_eq!(detect::retroarch_cfg(&ra.kind, &home), home.join(".retroarch.cfg"));
    write_file(&xdg.join("retroarch/retroarch.cfg"), "");
    assert_eq!(detect::retroarch_cfg(&ra.kind, &home), xdg.join("retroarch/retroarch.cfg"));
    assert_eq!(cfg(&ra), Some(xdg.join("retroarch")));
    // An $XDG_CONFIG_HOME outside the home folder is never used.
    set_test_xdg_config(Some(tmp.path().join("elsewhere")));
    assert_eq!(cfg(&usual("flycast")), Some(home.join(".config/flycast")));
    set_test_xdg_config(None);
}

#[test]
fn pcsx2_portable_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    // portable.txt naming a sub-folder.
    let dir = tmp.path().join("Emulators/PCSX2");
    write_exe(&dir.join("pcsx2-qt"), b"\x7fELF");
    write_file(&dir.join("portable.txt"), "  data \n");
    let p = emu("p", "pcsx2", program(&dir.join("pcsx2-qt")));
    assert_eq!(user_dir(&p, &home).unwrap(), UserDir { path: dir.join("data"), how: Where::Portable });
    assert_eq!(config_dir(&p, &home).unwrap().path, dir.join("data/inis"));
    // An AppImage ignores a portable.txt beside it (its program folder is the mount point)…
    let ai = tmp.path().join("Apps/pcsx2.AppImage");
    write_exe(&ai, APPIMAGE);
    write_file(&ai.with_file_name("portable.txt"), "");
    let a = emu("a", "pcsx2", appimage(&ai));
    assert_eq!(user_dir(&a, &home).unwrap(), UserDir { path: home.join(".config/PCSX2"), how: Where::Usual });
    // …and with -portable keeps everything in PCSX2/ beside it.
    let a = Emulator { args: "-portable -fullscreen".into(), ..a };
    assert_eq!(user_dir(&a, &home).unwrap(), UserDir { path: ai.with_file_name("PCSX2"), how: Where::Portable });
    // PPSSPP: a memstick folder next to the program means nothing on Linux.
    let pp = tmp.path().join("Emulators/PPSSPP");
    write_exe(&pp.join("PPSSPPSDL"), b"\x7fELF");
    std::fs::create_dir_all(pp.join("memstick")).unwrap();
    let e = emu("pp", "ppsspp", program(&pp.join("PPSSPPSDL")));
    assert_eq!(user_dir(&e, &home).unwrap(), UserDir { path: home.join(".config/ppsspp"), how: Where::Usual });
}

/// `$XDG_CONFIG_HOME` set only where Lodestar runs (a container's shell) while the emulator runs
/// without it: the folder the emulator has been using (it exists, the XDG one doesn't) still wins,
/// so BIOS files, settings and sign-ins are looked up where they are. Seen on the real machine:
/// DuckStation's files in `~/.local/share/duckstation`, `XDG_CONFIG_HOME=~/.config`.
#[test]
fn an_xdg_config_home_the_emulator_never_used_does_not_hide_its_existing_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let usual = |def: &str| emu(def, def, program(Path::new("/usr/bin/x")));
    let xdg = home.join("xdg");
    set_test_xdg_config(Some(xdg.clone()));
    for (def, used) in [
        ("duckstation", ".local/share/duckstation"),
        ("pcsx2", ".config/PCSX2"),
        ("ppsspp", ".config/ppsspp"),
        ("flycast", ".config/flycast"),
        ("dolphin", ".config/dolphin-emu"),
    ] {
        std::fs::create_dir_all(home.join(used)).unwrap();
        let e = usual(def);
        let got = config_dir(&e, &home).unwrap().path;
        assert!(got.starts_with(home.join(used)), "{def}: {}", got.display());
    }
    assert_eq!(bios::bios_dir(&usual("duckstation"), &home), Some(home.join(".local/share/duckstation/bios")));
    let ra = usual("retroarch");
    write_file(&home.join(".config/retroarch/retroarch.cfg"), "");
    assert_eq!(detect::retroarch_cfg(&ra.kind, &home), home.join(".config/retroarch/retroarch.cfg"));
    // Once the emulator has a folder under $XDG_CONFIG_HOME, that one is used.
    std::fs::create_dir_all(xdg.join("duckstation")).unwrap();
    assert_eq!(config_dir(&usual("duckstation"), &home).unwrap().path, xdg.join("duckstation"));
    write_file(&xdg.join("retroarch/retroarch.cfg"), "");
    assert_eq!(detect::retroarch_cfg(&ra.kind, &home), xdg.join("retroarch/retroarch.cfg"));
    set_test_xdg_config(None);
}
