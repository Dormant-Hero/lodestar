use super::*;
use crate::config::{EmulationConfig, RomFolder};
use crate::test_support::write_file;

const FAKE_PS1: &[u8] = b"lodestar fake ps1 bios";

/// A 4 MiB PS2 BIOS look-alike: RESET + ROMDIR + ROMVER entries, ROMVER "0230AC20080220".
fn fake_ps2(region: u8) -> Vec<u8> {
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
    v[4] = region;
    d[0x140..0x14e].copy_from_slice(&v);
    d
}

fn flatpak(id: &str, def: &str, app: &str) -> Emulator {
    Emulator {
        id: id.into(),
        def: def.into(),
        name: def.into(),
        kind: InstallKind::Flatpak { app_id: app.into() },
        how: String::new(),
        user: false,
        cores: vec![],
        args: String::new(),
        auto: false,
        blocked: false,
    }
}

fn reg(home: &Path, emulators: Vec<Emulator>, bios_dirs: Vec<PathBuf>) -> Registry {
    Registry {
        emulators,
        config: EmulationConfig { bios_dirs, ..EmulationConfig::detect_nothing() },
        home: home.to_path_buf(),
    }
}

#[test]
fn romver_and_identify() {
    assert_eq!(ps2_romver(&fake_ps2(b'A')).as_deref(), Some("0230AC20080220"));
    assert_eq!(ps2_romver(&[0u8; 1024]), None);
    let tmp = tempfile::tempdir().unwrap();
    let t = tmp.path();
    write_file(&t.join("picked/any-name.bin"), FAKE_PS1);
    write_file(&t.join("picked/ps2/SCPH-39001.bin"), fake_ps2(b'A'));
    write_file(&t.join("picked/ps2/jp.bin"), fake_ps2(b'J'));
    write_file(&t.join("picked/prod.keys"), "header_key = 0123\n");
    write_file(&t.join("picked/title.keys"), "");
    write_file(&t.join("picked/PS3UPDAT.PUP"), b"SCEUF\0\0\0more");
    write_file(&t.join("picked/notes.txt"), "hello");
    write_file(&t.join("picked/.hidden.bin"), FAKE_PS1);
    // A huge file is never read.
    let big = std::fs::File::create(t.join("big.bin")).unwrap();
    big.set_len(65 * 1024 * 1024).unwrap();
    let mut got: Vec<(String, &str, Option<&str>)> = identify(&[t.join("picked"), t.join("big.bin")])
        .into_iter()
        .map(|(p, b)| (p.file_name().unwrap().to_string_lossy().into_owned(), b.platform, b.region))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            ("PS3UPDAT.PUP".to_string(), "sony_playstation3", None),
            ("SCPH-39001.bin".to_string(), "sony_playstation2", Some("USA")),
            ("any-name.bin".to_string(), "sony_playstation", Some("USA")),
            ("jp.bin".to_string(), "sony_playstation2", Some("Japan")),
            ("prod.keys".to_string(), "nintendo_switch", None),
        ]
    );
    assert_eq!(md5_of(&t.join("big.bin")), None);
    assert_eq!(md5_of(&t.join("picked/any-name.bin")).as_deref(), Some("5f4cdec3d512806f5b867409a9ad8e97"));
}

#[test]
fn needs() {
    assert!(needs_bios("sony_playstation"));
    assert!(needs_bios("sony_playstation2"));
    assert!(needs_bios("nintendo_switch"));
    assert!(!needs_bios("nintendo_wii"));
    assert!(!needs_bios("nintendo_gameboyadvance"));
    let tmp = tempfile::tempdir().unwrap();
    let r = reg(tmp.path(), vec![], vec![]);
    assert_eq!(check(&r, "nintendo_wii").state, BiosState::NotNeeded);
    assert_eq!(check(&r, "nintendo_gameboyadvance").state, BiosState::Optional);
    assert_eq!(check(&r, "sony_playstation").state, BiosState::Missing);
}

#[test]
fn ps1_found_in_a_rom_folder_and_copied_to_duckstation() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let roms = tmp.path().join("games");
    write_file(&roms.join("ps1/BIOS/ps-30a.bin"), FAKE_PS1);
    write_file(&roms.join("ps1/BIOS/scph1001.bin"), b"not a real bios");
    let mut r = reg(&home, vec![flatpak("duckstation", "duckstation", "org.duckstation.DuckStation")], vec![]);
    r.config.rom_folders.insert("f1".into(), RomFolder { path: roms.clone(), ..Default::default() });
    assert!(search_dirs(&r).contains(&roms.join("ps1/BIOS")));
    let st = check(&r, "sony_playstation");
    assert_eq!(st.state, BiosState::Ok);
    let ok: Vec<_> = st.files.iter().filter(|(_, s, _)| *s == BiosState::Ok).collect();
    assert_eq!(ok.len(), 1);
    assert_eq!(ok[0].2.as_deref(), Some(roms.join("ps1/BIOS/ps-30a.bin").as_path()));
    // The bad scph1001.bin is a wrong dump.
    assert!(st.files.iter().any(|(b, s, _)| b.file == "scph1001.bin" && *s == BiosState::Wrong));
    assert!(st.note.contains("USA ✓") && st.note.contains("Japan ✗"), "{}", st.note);
    let dir = home.join(".var/app/org.duckstation.DuckStation/config/duckstation/bios");
    assert_eq!(
        st.targets,
        vec![BiosTarget {
            emulator: "duckstation".into(),
            dir: dir.clone(),
            missing: vec![roms.join("ps1/BIOS/ps-30a.bin")],
            ready: false,
            firmware: None
        }]
    );
    assert_eq!(install(&r, &st, "duckstation", false).unwrap(), 1);
    assert_eq!(std::fs::read(dir.join("scph-test.bin")).unwrap(), FAKE_PS1);
    // Now it's there: nothing left to copy, a second install copies nothing.
    let st2 = check(&r, "sony_playstation");
    assert!(st2.targets[0].missing.is_empty(), "{:?}", st2.targets);
    assert_eq!(install(&r, &st, "duckstation", false).unwrap(), 0);
    // A different file already there is kept unless overwriting is asked for.
    std::fs::write(dir.join("scph-test.bin"), b"user's own file").unwrap();
    assert_eq!(install(&r, &st, "duckstation", false).unwrap(), 0);
    assert_eq!(std::fs::read(dir.join("scph-test.bin")).unwrap(), b"user's own file");
    assert_eq!(install(&r, &st, "duckstation", true).unwrap(), 1);
    assert_eq!(std::fs::read(dir.join("scph-test.bin")).unwrap(), FAKE_PS1);
    assert!(install(&r, &st, "pcsx2", false).is_err());
}

#[test]
fn ps2_any_dump_and_pcsx2_keeps_the_name() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let picked = tmp.path().join("my bios");
    write_file(&picked.join("SCPH-70012.bin"), fake_ps2(b'A'));
    let r = reg(&home, vec![flatpak("pcsx2", "pcsx2", "net.pcsx2.PCSX2")], vec![picked.clone()]);
    let st = check(&r, "sony_playstation2");
    assert_eq!(st.state, BiosState::Ok);
    assert_eq!(st.files.len(), 2, "{:?}", st.files); // the US dump + the known-good v2.30 row (optional now)
    let dir = home.join(".var/app/net.pcsx2.PCSX2/config/PCSX2/bios");
    assert_eq!(install(&r, &st, "pcsx2", false).unwrap(), 1);
    assert!(dir.join("SCPH-70012.bin").is_file());
    // A native PCSX2 with a moved BIOS folder.
    let mut native = flatpak("pcsx2", "pcsx2", "");
    native.kind = InstallKind::Binary { path: "/usr/bin/pcsx2-qt".into() };
    write_file(&home.join(".config/PCSX2/inis/PCSX2.ini"), "[Folders]\nBios = /data/ps2bios\n");
    assert_eq!(bios_dir(&native, &home), Some(PathBuf::from("/data/ps2bios")));
    // Nothing found: one "PS2 BIOS" row, missing.
    let empty = reg(&home.join("x"), vec![], vec![]);
    let st = check(&empty, "sony_playstation2");
    assert_eq!(st.state, BiosState::Missing);
    assert_eq!(st.files.iter().filter(|(b, _, _)| b.file.is_empty()).count(), 1);
}

#[test]
fn switch_keys_and_firmware_presence() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_path_buf();
    let eden = flatpak("eden", "eden", "dev.eden_emu.eden");
    let r = reg(&home, vec![eden], vec![]);
    let st = check(&r, "nintendo_switch");
    assert_eq!(st.state, BiosState::Missing);
    let data = home.join(".var/app/dev.eden_emu.eden/data/eden");
    write_file(&data.join("keys/prod.keys"), "header_key = 00\n");
    let st = check(&r, "nintendo_switch");
    assert_eq!(st.state, BiosState::Missing, "firmware still missing");
    assert!(st.note.contains("Install Firmware"), "{}", st.note);
    write_file(&data.join("nand/system/Contents/registered/x.nca"), b"x");
    let st = check(&r, "nintendo_switch");
    assert_eq!(st.state, BiosState::Ok, "{:?}", st.files);
    // Keys found elsewhere are offered to Eden.
    let picked = tmp.path().join("dump");
    write_file(&picked.join("prod.keys"), "header_key = 11\n");
    let r2 = reg(&home.join("other"), vec![flatpak("eden", "eden", "dev.eden_emu.eden")], vec![picked.clone()]);
    let st = check(&r2, "nintendo_switch");
    assert_eq!(st.targets[0].missing, vec![picked.join("prod.keys")]);
    assert_eq!(install(&r2, &st, "eden", false).unwrap(), 1);
    assert!(home.join("other/.var/app/dev.eden_emu.eden/data/eden/keys/prod.keys").is_file());
}

#[test]
fn ps3_firmware_version() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_path_buf();
    let r = reg(&home, vec![flatpak("rpcs3", "rpcs3", "net.rpcs3.RPCS3")], vec![]);
    assert_eq!(check(&r, "sony_playstation3").state, BiosState::Missing);
    write_file(
        &home.join(".var/app/net.rpcs3.RPCS3/config/rpcs3/dev_flash/vsh/etc/version.txt"),
        "release:04.92:build:...\n",
    );
    let st = check(&r, "sony_playstation3");
    assert_eq!(st.state, BiosState::Ok);
    assert!(st.note.contains("04.92"), "{}", st.note);
    // RPCS3 is listed for its firmware only: nothing is ever copied, it is ready.
    assert_eq!(st.targets.len(), 1);
    assert!(st.targets[0].missing.is_empty(), "firmware is never copied");
    assert!(st.targets[0].ready);
    assert_eq!(st.targets[0].firmware.as_deref(), Some("04.92"));
}

#[test]
fn retroarch_wants_canonical_names() {
    let b = table().find(|b| b.file == "dc/dc_boot.bin").unwrap();
    assert_eq!(target_rel("retroarch", b, Path::new("/x/boot.bin")), Some(PathBuf::from("dc/dc_boot.bin")));
    assert_eq!(target_rel("flycast", b, Path::new("/x/boot.bin")), Some(PathBuf::from("dc_boot.bin")));
    let ps2 = table().find(|b| b.platform == "sony_playstation2" && b.file.is_empty()).unwrap();
    assert_eq!(target_rel("retroarch", ps2, Path::new("/x/SCPH.bin")), Some(PathBuf::from("pcsx2/bios/SCPH.bin")));
    let fw = table().find(|b| b.file == "firmware").unwrap();
    assert_eq!(target_rel("eden", fw, Path::new("/x/f")), None);
}

#[test]
fn table_is_sane() {
    for b in table() {
        assert!(super::super::platforms::by_id(b.platform).is_some(), "{}", b.platform);
        for m in b.md5 {
            assert!(m.len() == 32 && m.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{m}");
        }
        assert!(b.group.is_none() || b.required, "{}", b.file);
    }
}

#[test]
fn emulator_folders_are_searched_only_where_bios_files_live() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let dol = home.join(".var/app/org.DolphinEmu.dolphin-emu/data/dolphin-emu");
    write_file(&dol.join("GC/USA/IPL.bin"), b"not a real ipl");
    write_file(&dol.join("Load/Textures/GALE01/tex1.png"), FAKE_PS1);
    let cemu = home.join(".var/app/info.cemu.Cemu/data/Cemu");
    write_file(&cemu.join("keys.txt"), b"abc = def");
    write_file(&cemu.join("mlc01/usr/x/scph-test.bin"), FAKE_PS1);
    let r = reg(
        &home,
        vec![flatpak("dolphin", "dolphin", "org.DolphinEmu.dolphin-emu"), flatpak("cemu", "cemu", "info.cemu.Cemu")],
        vec![],
    );
    let dirs = search_dirs_depth(&r);
    assert!(dirs.contains(&(dol.join("GC"), 2)), "{dirs:?}");
    assert!(dirs.contains(&(cemu.clone(), 0)), "{dirs:?}");
    assert!(!dirs.iter().any(|(d, _)| *d == dol));
    // The GC IPL is still seen (as a wrong dump), Cemu's keys.txt is found at the top.
    let gc = check(&r, "nintendo_gamecube");
    assert!(gc.files.iter().any(|(b, s, _)| b.file == "GC/USA/IPL.bin" && *s == BiosState::Wrong), "{gc:?}");
    let wiiu = check(&r, "nintendo_wiiu");
    assert!(wiiu.files.iter().any(|(b, s, _)| b.file == "keys.txt" && *s == BiosState::Ok), "{wiiu:?}");
    // Nothing from the texture pack / installed games is taken for a PS1 BIOS.
    assert!(check(&r, "sony_playstation").files.iter().all(|(_, s, _)| *s != BiosState::Ok));
}

/// A fake PS2 dump with a given ROMVER ("0230AC20080220").
fn fake_ps2_ver(romver: &[u8; 14]) -> Vec<u8> {
    let mut d = fake_ps2(b'A');
    d[0x140..0x14e].copy_from_slice(romver);
    d
}

#[test]
fn bios_in_zips_and_bios_named_folders_of_the_home_folder() {
    use super::super::zipread::tests::write_zip;
    // The user's layout: ~/Documents/bios files/ps2/{x.zip, x/} and ~/Downloads/ps2-bios-*.zip
    // with nested folders and macOS junk.
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let docs = home.join("Documents/bios files/ps2");
    write_file(&docs.join("ps2-bios-all-bios/scph39001.bin"), fake_ps2_ver(b"0160AC20020207"));
    write_file(&docs.join("ps2-bios-all-bios/scph39001.NVM"), b"nvm");
    write_file(&docs.join("ps2-bios-all-bios/scph10000.bin"), fake_ps2_ver(b"0100JC20000117"));
    let usa230 = fake_ps2_ver(b"0230AC20080220");
    let eur = fake_ps2_ver(b"0200EC20040614");
    write_zip(&docs.join("ps2-bios-all-bios.zip"), &[("scph39001.bin", &fake_ps2_ver(b"0160AC20020207"), true)]);
    write_zip(
        &home.join("Downloads/ps2-bios-usa.zip"),
        &[
            ("ps2 bios usa/.DS_Store", b"junk", true),
            ("__MACOSX/ps2 bios usa/._x", b"junk", true),
            ("ps2 bios usa/SCPH-90001_BIOS_V18_USA_230/SCPH-90001_BIOS_V18_USA_230.ROM0", &usa230, true),
        ],
    );
    write_zip(&home.join("Downloads/ps2-bios-europe.zip"), &[("eu/SCPH-70004_BIOS_V12_EUR_200.BIN", &eur, false)]);
    // Not a BIOS folder: never searched.
    write_file(&home.join("Downloads/other/scph-test.bin"), FAKE_PS1);
    let r = reg(&home, vec![flatpak("pcsx2", "pcsx2", "net.pcsx2.PCSX2")], vec![]);
    let dirs = search_dirs(&r);
    assert!(dirs.contains(&home.join("Documents/bios files")), "{dirs:?}");
    assert!(dirs.contains(&home.join("Downloads/ps2-bios-usa.zip")), "{dirs:?}");
    assert!(!dirs.iter().any(|d| d.ends_with("other")));
    let st = check(&r, "sony_playstation2");
    assert_eq!(st.state, BiosState::Ok);
    // One per region: the newest USA dump (in a zip), the Japanese and the European one.
    let ok: Vec<(Option<&str>, PathBuf)> = st
        .files
        .iter()
        .filter(|(_, s, _)| *s == BiosState::Ok)
        .map(|(b, _, p)| (b.region, p.clone().unwrap()))
        .collect();
    assert_eq!(ok.len(), 3, "{ok:?}");
    let usa = ok.iter().find(|(r, _)| *r == Some("USA")).unwrap();
    assert!(usa.1.ends_with("SCPH-90001_BIOS_V18_USA_230.ROM0"), "{usa:?}");
    assert!(usa.1.starts_with(home.join("Downloads/ps2-bios-usa.zip")));
    assert!(st.note.contains("PS2 BIOS files found in"), "{}", st.note);
    assert!(st.note.contains("ps2-bios-usa.zip") && st.note.contains("ps2-bios-all-bios.zip"), "{}", st.note);
    // PCSX2 gets the three (not every copy), extracted from the zips; the .ROM0 is saved as .bin.
    let t = &st.targets[0];
    assert_eq!(t.missing.len(), 3, "{:?}", t.missing);
    assert_eq!(install(&r, &st, "pcsx2", false).unwrap(), 3);
    let dir = home.join(".var/app/net.pcsx2.PCSX2/config/PCSX2/bios");
    assert_eq!(std::fs::read(dir.join("SCPH-90001_BIOS_V18_USA_230.bin")).unwrap(), usa230);
    assert_eq!(std::fs::read(dir.join("SCPH-70004_BIOS_V12_EUR_200.BIN")).unwrap(), eur);
    assert!(dir.join("scph10000.bin").is_file());
    assert!(!dir.join("scph39001.NVM").exists(), "PCSX2 makes its own NVM");
    let names: Vec<String> =
        std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names.len(), 3, "no temporary files left: {names:?}");
    // Copied: nothing left to do.
    assert!(check(&r, "sony_playstation2").targets[0].missing.is_empty());
    assert_eq!(describe(&usa.1, &home), "SCPH-90001_BIOS_V18_USA_230.ROM0 in ~/Downloads/ps2-bios-usa.zip");
}

#[test]
fn switch_keys_in_zips_and_firmware_installed_into_eden() {
    use super::super::zipread::tests::write_zip;
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let sw = home.join("Documents/bios files/switch");
    write_zip(
        &sw.join("ProdKeys.NET-v22.5.0.zip"),
        &[("prod.keys", b"header_key = 00\n", true), ("title.keys", b"0100 = 11\n", true)],
    );
    let ncas: Vec<(String, Vec<u8>)> = (0..24).map(|i| (format!("{i:032x}.nca"), vec![i as u8; 100])).collect();
    for (n, d) in &ncas {
        write_file(&sw.join("Firmware.23.5.0").join(n), d);
    }
    let zip_entries: Vec<(&str, &[u8], bool)> = ncas.iter().map(|(n, d)| (n.as_str(), d.as_slice(), true)).collect();
    write_zip(&sw.join("Firmware.22.0.0.zip"), &zip_entries);
    let r = reg(&home, vec![flatpak("eden", "eden", "dev.eden_emu.eden")], vec![]);
    let st = check(&r, "nintendo_switch");
    // Keys found inside the zip; firmware found, newest version first, not installed yet.
    let keys = st.files.iter().find(|(b, _, _)| b.file == "prod.keys").unwrap();
    assert_eq!(keys.1, BiosState::Ok);
    assert_eq!(keys.2.as_deref(), Some(sw.join("ProdKeys.NET-v22.5.0.zip/prod.keys").as_path()));
    let fw = st.files.iter().find(|(b, _, _)| b.file == "firmware").unwrap();
    assert_eq!(fw.1, BiosState::Missing);
    assert_eq!(st.packages.len(), 1, "{:?}", st.packages);
    let pk = &st.packages[0];
    assert_eq!((pk.kind, pk.version.as_deref(), pk.files), (PackageKind::SwitchFirmware, Some("23.5.0"), 24));
    assert_eq!(pk.path, sw.join("Firmware.23.5.0"));
    let data = home.join(".var/app/dev.eden_emu.eden/data/eden");
    assert_eq!(
        pk.targets,
        vec![PackageTarget {
            emulator: "eden".into(),
            dir: data.join("nand/system/Contents/registered"),
            installed: None
        }]
    );
    assert!(!st.note.contains("Tools → Install Firmware"), "a found package replaces the hint: {}", st.note);
    // Copy keys to Eden: extracted from the zip.
    assert_eq!(st.targets[0].missing.len(), 2);
    assert_eq!(install(&r, &st, "eden", false).unwrap(), 2);
    assert_eq!(std::fs::read(data.join("keys/prod.keys")).unwrap(), b"header_key = 00\n");
    // Install firmware into Eden.
    assert_eq!(install_firmware(&st, "eden", false).unwrap(), 24);
    let reg_dir = data.join("nand/system/Contents/registered");
    assert_eq!(std::fs::read(reg_dir.join(&ncas[3].0)).unwrap(), ncas[3].1);
    assert_eq!(std::fs::read_dir(&reg_dir).unwrap().count(), 24);
    let st2 = check(&r, "nintendo_switch");
    assert_eq!(st2.state, BiosState::Ok, "{:?}", st2.files);
    assert_eq!(st2.packages[0].targets[0].installed.as_deref(), Some("installed"));
    assert!(!st2.packages[0].replaces(&st2.packages[0].targets[0]));
    // Already installed: only with replace (old files go, new ones come).
    assert!(install_firmware(&st2, "eden", false).is_err());
    write_file(&reg_dir.join("old.nca"), b"old");
    let mut zip_only = st2.clone();
    zip_only.packages[0].path = sw.join("Firmware.22.0.0.zip");
    assert_eq!(install_firmware(&zip_only, "eden", true).unwrap(), 24);
    assert!(!reg_dir.join("old.nca").exists());
    // Another firmware installed (other files): offered as a replacement.
    std::fs::remove_file(reg_dir.join(&ncas[0].0)).unwrap();
    write_file(&reg_dir.join("other.nca"), b"x");
    let st3 = check(&r, "nintendo_switch");
    assert_eq!(st3.packages[0].targets[0].installed.as_deref(), Some("other"));
    assert!(st3.packages[0].replaces(&st3.packages[0].targets[0]));
    assert_eq!(std::fs::read(reg_dir.join(&ncas[5].0)).unwrap(), ncas[5].1);
    assert_eq!(version_in_name("ProdKeys.NET-v22.5.0"), Some("22.5.0".into()));
    assert_eq!(version_in_name("Firmware"), None);
}

#[test]
fn ps3_firmware_file_offered_to_rpcs3() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    // A PUP with a version.txt entry (id 0x100) saying 4.93.
    let mut pup = vec![0u8; 0x80];
    pup[..8].copy_from_slice(b"SCEUF\0\0\0");
    pup[0x18..0x20].copy_from_slice(&1u64.to_be_bytes());
    pup[0x30..0x38].copy_from_slice(&0x100u64.to_be_bytes());
    pup[0x38..0x40].copy_from_slice(&0x70u64.to_be_bytes());
    pup[0x40..0x48].copy_from_slice(&5u64.to_be_bytes());
    pup[0x70..0x75].copy_from_slice(b"4.93\n");
    let file = home.join("Documents/bios files/ps3/PS3UPDAT.PUP");
    write_file(&file, &pup);
    assert_eq!(pup_version(&file).as_deref(), Some("4.93"));
    let rp = flatpak("rpcs3", "rpcs3", "net.rpcs3.RPCS3");
    let r = reg(&home, vec![rp.clone()], vec![]);
    let st = check(&r, "sony_playstation3");
    assert_eq!(st.state, BiosState::Missing);
    assert_eq!(st.packages.len(), 1);
    assert_eq!(st.packages[0].version.as_deref(), Some("4.93"));
    assert_eq!(st.packages[0].targets[0].installed, None);
    // Said once (the package's target line), not again in the note.
    assert!(!st.note.contains("not installed"), "{}", st.note);
    assert!(!st.packages[0].replaces(&st.packages[0].targets[0]));
    // The install command (never run in tests): Flatpak allowed to read the PUP's folder.
    let spec = ps3_install_spec(&rp, &home, &file);
    let crate::model::LaunchSpec::Command { program, args, .. } = spec else { panic!() };
    assert_eq!(program, "flatpak");
    assert_eq!(
        args,
        vec![
            "run".to_string(),
            format!("--filesystem={}:ro", file.parent().unwrap().display()),
            "net.rpcs3.RPCS3".into(),
            "--installfw".into(),
            file.display().to_string(),
        ]
    );
    let mut native = rp.clone();
    native.kind = InstallKind::AppImage { path: "/apps/rpcs3.AppImage".into() };
    let crate::model::LaunchSpec::Command { program, args, .. } = ps3_install_spec(&native, &home, &file) else {
        panic!()
    };
    assert_eq!((program.as_str(), args.len()), ("/apps/rpcs3.AppImage", 2));
    assert_eq!(args[0], "--installfw");
    // Installed: OK, the package says so.
    write_file(&home.join(".var/app/net.rpcs3.RPCS3/config/rpcs3/dev_flash/vsh/etc/version.txt"), "release:04.93:x\n");
    let st = check(&r, "sony_playstation3");
    assert_eq!(st.state, BiosState::Ok);
    assert_eq!(st.packages[0].targets[0].installed.as_deref(), Some("04.93"));
    assert!(!st.packages[0].replaces(&st.packages[0].targets[0]), "same version");
    // An older one installed: offered as a replacement.
    write_file(&home.join(".var/app/net.rpcs3.RPCS3/config/rpcs3/dev_flash/vsh/etc/version.txt"), "release:04.90:x\n");
    let st = check(&r, "sony_playstation3");
    assert!(st.packages[0].replaces(&st.packages[0].targets[0]));
    assert!(version_less("04.92", "4.93") && version_less("4.9", "4.10") && !version_less("4.93", "04.93"));
}

#[test]
fn removed_bios_folders_are_never_searched_unless_a_closer_folder_is_chosen() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let backups = home.join("Documents/PS2 Resources/bios set");
    write_file(&backups.join("scph1001.bin"), FAKE_PS1);
    let mut r = reg(&home, vec![], vec![]);
    assert!(search_dirs(&r).contains(&backups), "found by itself while not removed");

    r.config.ignored_bios_dirs = vec![backups.clone()];
    assert!(!search_dirs(&r).iter().any(|d| d.starts_with(&backups)), "removed: not searched");
    assert!(removed_by_user(&r, &backups.join("scph1001.bin")), "nor any file inside it");

    // Removing a whole folder also covers what is inside it; a folder chosen inside it wins.
    r.config.ignored_bios_dirs = vec![home.join("Documents/PS2 Resources")];
    assert!(removed_by_user(&r, &backups.join("scph1001.bin")));
    r.config.bios_dirs = vec![backups.clone()];
    assert!(!removed_by_user(&r, &backups.join("scph1001.bin")), "the closer, chosen folder decides");
    assert!(search_dirs(&r).contains(&backups));
}

// ---------------------------------------------------------------------------------------------
// Where the search looks: the emulator's own folder first, no old copies or backups on drives
// ---------------------------------------------------------------------------------------------

fn program_emu(id: &str, def: &str, program: &Path) -> Emulator {
    Emulator { kind: InstallKind::Binary { path: program.to_path_buf() }, ..flatpak(id, def, "") }
}

/// A tester's layout: the emulators live on one drive (its "Emulators" folder is the user's
/// emulator folder, both portable with their BIOS inside), a second drive holds old emulator
/// copies and a backup folder with BIOS files (newer PS2 dumps, the same PS1 one).
struct TwoDrives {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    old_drive: PathBuf,
    ds_bios: PathBuf,
    pcsx2_bios: PathBuf,
    reg: Registry,
}

fn two_drives() -> TwoDrives {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mnt = root.join("mnt");
    super::super::roots::set_test_drives(vec![mnt.clone()]);
    let emus_dir = mnt.join("FastSSD/Gaming/Emulation/Emulators");
    let ds = emus_dir.join("Duckstation");
    write_file(&ds.join("duckstation-qt"), b"program");
    write_file(&ds.join("portable.txt"), "");
    write_file(&ds.join("bios/scph-test.bin"), FAKE_PS1);
    let p2 = emus_dir.join("PCSX2");
    write_file(&p2.join("pcsx2-qt"), b"program");
    write_file(&p2.join("portable.ini"), "");
    write_file(&p2.join("bios/SCPH-39001.bin"), fake_ps2_ver(b"0160AC20020207"));
    // The old drive: a backup folder and old portable copies, with newer dumps (they won
    // "newest first" and sorted before the right drive by name).
    let old = mnt.join("4TB");
    let unused = old.join("Gaming/Backups - Modding - Resources/Playstation/PS2 Resources/unused BIOS");
    write_file(&unused.join("SCPH-90001.bin"), fake_ps2_ver(b"0230AC20080220"));
    write_file(&unused.join("ps1/scph-test.bin"), FAKE_PS1);
    let old_emus = old.join("Gaming/Emulation/Emulators");
    write_file(&old_emus.join("Duckstation/bios/scph-test.bin"), FAKE_PS1);
    write_file(&old_emus.join("PCSX2/bios/SCPH-77001.bin"), fake_ps2_ver(b"0220AC20060905"));
    let mut reg = reg(
        &home,
        vec![
            program_emu("duckstation", "duckstation", &ds.join("duckstation-qt")),
            program_emu("pcsx2", "pcsx2", &p2.join("pcsx2-qt")),
        ],
        vec![],
    );
    reg.config.emulator_dirs = vec![emus_dir];
    TwoDrives { _tmp: tmp, home, old_drive: old, ds_bios: ds.join("bios"), pcsx2_bios: p2.join("bios"), reg }
}

fn summary_text(r: &Registry, st: &BiosStatus) -> (String, Vec<Step>) {
    let plan = setup_plan(r, st, None);
    let name = |id: &str| if id == "pcsx2" { "PCSX2".to_string() } else { "DuckStation".to_string() };
    (summary(st, &plan, &r.home, &name, &|_| true).text, plan)
}

#[test]
fn emulators_are_ready_from_their_own_folders_and_old_drives_are_never_searched() {
    let t = two_drives();
    let r = &t.reg;
    let dirs = search_dirs_with_origin(r);
    assert!(!dirs.iter().any(|d| d.path.starts_with(&t.old_drive)), "nothing from the old drive: {dirs:?}");
    assert!(dirs.iter().any(|d| d.path == t.ds_bios && d.origin == Origin::Emulator("duckstation".into())));
    assert!(dirs.iter().any(|d| d.path == t.pcsx2_bios && d.origin == Origin::Emulator("pcsx2".into())));

    // PS2: PCSX2's own (older) dump is the one shown; it is ready, nothing to copy.
    let st = check(r, "sony_playstation2");
    assert_eq!(st.state, BiosState::Ok);
    let shown: Vec<&PathBuf> = st.files.iter().filter_map(|(_, _, p)| p.as_ref()).collect();
    assert_eq!(shown, vec![&t.pcsx2_bios.join("SCPH-39001.bin")], "{:?}", st.files);
    let p2 = st.targets.iter().find(|x| x.emulator == "pcsx2").unwrap();
    assert!(p2.ready && p2.missing.is_empty(), "{p2:?}");
    let (text, plan) = summary_text(r, &st);
    assert!(plan.is_empty(), "{plan:?}");
    assert_eq!(text, "Ready in PCSX2 ✓");

    // PS1: DuckStation's own file, ready, no "not copied".
    let st = check(r, "sony_playstation");
    let ok: Vec<&PathBuf> =
        st.files.iter().filter(|(_, s, _)| *s == BiosState::Ok).filter_map(|(_, _, p)| p.as_ref()).collect();
    assert_eq!(ok, vec![&t.ds_bios.join("scph-test.bin")], "{:?}", st.files);
    let ds = st.targets.iter().find(|x| x.emulator == "duckstation").unwrap();
    assert!(ds.ready && ds.missing.is_empty(), "{ds:?}");
    assert_eq!(summary_text(r, &st).0, "Ready in DuckStation ✓");
    assert!(st.files.iter().all(|(_, _, p)| p.as_ref().is_none_or(|p| !p.starts_with(&t.old_drive))));
    super::super::roots::set_test_drives(vec![]);
}

#[test]
fn an_emulators_own_region_is_never_offered_again_from_elsewhere() {
    // A newer USA dump in ~/Downloads while PCSX2 has an older USA one of its own: PCSX2 keeps
    // its own; a Japanese dump only in ~/Downloads is still offered ("Copy to PCSX2").
    let t = two_drives();
    let dl = t.home.join("Downloads/ps2 bios");
    write_file(&dl.join("SCPH-90001.bin"), fake_ps2_ver(b"0230AC20080220"));
    write_file(&dl.join("SCPH-10000.bin"), fake_ps2_ver(b"0100JC20000117"));
    let st = check(&t.reg, "sony_playstation2");
    let usa = st.files.iter().find(|(b, s, _)| b.region == Some("USA") && *s == BiosState::Ok).unwrap();
    assert_eq!(usa.2.as_deref(), Some(t.pcsx2_bios.join("SCPH-39001.bin").as_path()));
    let p2 = st.targets.iter().find(|x| x.emulator == "pcsx2").unwrap();
    assert!(p2.ready);
    assert_eq!(p2.missing, vec![dl.join("SCPH-10000.bin")]);
    super::super::roots::set_test_drives(vec![]);
}

#[test]
fn a_bios_only_in_downloads_is_offered_to_the_emulator() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let dl = home.join("Downloads/psx bios");
    write_file(&dl.join("scph-test.bin"), FAKE_PS1);
    let r = reg(&home, vec![flatpak("duckstation", "duckstation", "org.duckstation.DuckStation")], vec![]);
    let st = check(&r, "sony_playstation");
    assert_eq!(st.state, BiosState::Ok);
    assert!(!st.targets[0].ready);
    assert_eq!(st.targets[0].missing, vec![dl.join("scph-test.bin")]);
    let (text, plan) = summary_text(&r, &st);
    assert_eq!(text, "PlayStation BIOS found in ~/Downloads/psx bios — not in DuckStation yet.");
    assert_eq!(plan.len(), 1);
}

#[test]
fn a_flatpaks_usual_folder_is_its_own_and_wins_over_newer_dumps_elsewhere() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let own = home.join(".var/app/net.pcsx2.PCSX2/config/PCSX2/bios");
    write_file(&own.join("usa.bin"), fake_ps2_ver(b"0160AC20020207"));
    let docs = home.join("Documents/bios files");
    write_file(&docs.join("SCPH-90001.bin"), fake_ps2_ver(b"0230AC20080220"));
    let r = reg(&home, vec![flatpak("pcsx2", "pcsx2", "net.pcsx2.PCSX2")], vec![]);
    let dirs = search_dirs_with_origin(&r);
    assert_eq!(dirs[0], SearchDir { path: own.clone(), depth: 3, origin: Origin::Emulator("pcsx2".into()) });
    let st = check(&r, "sony_playstation2");
    let shown: Vec<&PathBuf> = st.files.iter().filter_map(|(_, _, p)| p.as_ref()).collect();
    assert_eq!(shown, vec![&own.join("usa.bin")]);
    assert!(st.targets[0].ready && st.targets[0].missing.is_empty(), "{:?}", st.targets);
    assert_eq!(summary_text(&r, &st).0, "Ready in PCSX2 ✓");
}

#[test]
fn folders_named_like_backups_are_skipped_unless_chosen() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let old = home.join("Documents/Backups/ps1 bios");
    write_file(&old.join("scph-test.bin"), FAKE_PS1);
    let inside = home.join("Documents/bios files/old/scph-test.bin");
    write_file(&inside, FAKE_PS1);
    let mut r = reg(&home, vec![], vec![]);
    assert!(!search_dirs(&r).iter().any(|d| d.starts_with(&old)), "under a backup folder");
    let st = check(&r, "sony_playstation");
    assert_eq!(st.state, BiosState::Missing, "nor an \"old\" folder inside a BIOS folder: {:?}", st.files);
    // Chosen by the user: searched, also inside.
    r.config.bios_dirs = vec![home.join("Documents/bios files")];
    let st = check(&r, "sony_playstation");
    assert!(st.files.iter().any(|(_, s, p)| *s == BiosState::Ok && p.as_deref() == Some(inside.as_path())));
}

#[test]
fn removed_folders_stay_removed_but_an_emulators_own_folder_is_always_read() {
    let t = two_drives();
    let mut r = t.reg.clone();
    let docs = t.home.join("Documents/ps2 bios");
    write_file(&docs.join("jp.bin"), fake_ps2_ver(b"0100JC20000117"));
    assert!(check(&r, "sony_playstation2").targets[0].missing.contains(&docs.join("jp.bin")));
    r.config.ignored_bios_dirs = vec![docs.clone(), t.pcsx2_bios.clone()];
    assert!(!search_dirs(&r).contains(&docs));
    let st = check(&r, "sony_playstation2");
    assert!(st.targets.iter().all(|x| x.missing.is_empty()), "{:?}", st.targets);
    let p2 = st.targets.iter().find(|x| x.emulator == "pcsx2").unwrap();
    assert!(p2.ready, "its own folder is where it reads: {p2:?}");
    super::super::roots::set_test_drives(vec![]);
}

#[test]
fn a_games_drives_emulation_bios_folder_is_found_last() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let mnt = tmp.path().join("mnt");
    super::super::roots::set_test_drives(vec![mnt.clone()]);
    let drive_bios = mnt.join("Games/Gaming/Emulation/bios");
    write_file(&drive_bios.join("a.bin"), FAKE_PS1);
    write_file(&mnt.join("Games/Old stuff/Emulation/bios/b.bin"), FAKE_PS1);
    write_file(&mnt.join("Games/Some/Where/Deep/bios/c.bin"), FAKE_PS1);
    let docs = home.join("Documents/bios");
    write_file(&docs.join("d.bin"), FAKE_PS1);
    let r = reg(&home, vec![flatpak("duckstation", "duckstation", "org.duckstation.DuckStation")], vec![]);
    let dirs = search_dirs_with_origin(&r);
    let on_drive: Vec<&SearchDir> = dirs.iter().filter(|d| d.path.starts_with(&mnt)).collect();
    assert_eq!(on_drive.len(), 1, "{dirs:?}");
    assert_eq!((&on_drive[0].path, &on_drive[0].origin), (&drive_bios, &Origin::Drive));
    // The home folder's copy is offered before the drive's.
    let st = check(&r, "sony_playstation");
    assert_eq!(st.targets[0].missing, vec![docs.join("d.bin")]);
    super::super::roots::set_test_drives(vec![]);
}
