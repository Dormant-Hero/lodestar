//! Emulation scanner tests: fake ROMs with real headers in temporary folders (built below),
//! the emulator-settings fixture home in tests/fixtures/scanners/emulation, roots with injected
//! drive folders, and the whole path through `scan::run_all` + `Db::merge_scan`.

use super::identify::{self, probe_file};
use super::roots::{self, Env};
use super::*;
use crate::config::{EmulationConfig, RomFolder};
use crate::test_support::write_file;
use std::path::Path;

// ---------------------------------------------------------------------------------------------
// Fake content

/// A PARAM.SFO with string values.
pub fn sfo(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut keys = vec![];
    let mut data = vec![];
    let mut index = vec![];
    for (k, v) in entries {
        let ko = keys.len() as u16;
        keys.extend_from_slice(k.as_bytes());
        keys.push(0);
        let dof = data.len() as u32;
        let mut val = v.as_bytes().to_vec();
        val.push(0);
        let len = val.len() as u32;
        while val.len() % 4 != 0 {
            val.push(0);
        }
        let max = val.len() as u32;
        data.extend_from_slice(&val);
        index.extend_from_slice(&ko.to_le_bytes());
        index.extend_from_slice(&0x0204u16.to_le_bytes());
        index.extend_from_slice(&len.to_le_bytes());
        index.extend_from_slice(&max.to_le_bytes());
        index.extend_from_slice(&dof.to_le_bytes());
    }
    while keys.len() % 4 != 0 {
        keys.push(0);
    }
    let key_off = 20 + index.len() as u32;
    let data_off = key_off + keys.len() as u32;
    let mut out = b"\0PSF".to_vec();
    out.extend_from_slice(&0x0101u32.to_le_bytes());
    out.extend_from_slice(&key_off.to_le_bytes());
    out.extend_from_slice(&data_off.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    out.extend(index);
    out.extend(keys);
    out.extend(data);
    out
}

fn dir_record(name: &[u8], lba: u32, size: u32, dir: bool) -> Vec<u8> {
    let mut r = vec![0u8; 33];
    r[2..6].copy_from_slice(&lba.to_le_bytes());
    r[6..10].copy_from_slice(&lba.to_be_bytes());
    r[10..14].copy_from_slice(&size.to_le_bytes());
    r[14..18].copy_from_slice(&size.to_be_bytes());
    r[25] = if dir { 2 } else { 0 };
    r[32] = name.len() as u8;
    r.extend_from_slice(name);
    if r.len() % 2 == 1 {
        r.push(0);
    }
    r[0] = r.len() as u8;
    r
}

/// A minimal ISO9660 image (2048-byte sectors) with files at the root or one folder deep.
pub fn iso(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut dirs: Vec<&str> = files.iter().filter_map(|(p, _)| p.split_once('/').map(|(d, _)| d)).collect();
    dirs.dedup();
    let mut next = 19 + dirs.len() as u32;
    let mut file_lba = vec![];
    for (_, data) in files {
        file_lba.push(next);
        next += (data.len() as u32).div_ceil(2048).max(1);
    }
    let mut img = vec![0u8; next as usize * 2048];
    let sector = |img: &mut Vec<u8>, lba: u32, bytes: &[u8]| {
        let o = lba as usize * 2048;
        img[o..o + bytes.len()].copy_from_slice(bytes);
    };
    let mut pvd = vec![0u8; 2048];
    pvd[0] = 1;
    pvd[1..6].copy_from_slice(b"CD001");
    pvd[156..156 + 34].copy_from_slice(&dir_record(&[0], 18, 2048, true));
    sector(&mut img, 16, &pvd);
    sector(&mut img, 17, &[255, b'C', b'D', b'0', b'0', b'1']);
    let dir_bytes = |parent: Option<&str>, own: u32| {
        let mut d = dir_record(&[0], own, 2048, true);
        d.extend(dir_record(&[1], 18, 2048, true));
        for (i, (p, data)) in files.iter().enumerate() {
            let (dir, name) = p.split_once('/').map(|(a, b)| (Some(a), b)).unwrap_or((None, p));
            if dir == parent {
                d.extend(dir_record(format!("{name};1").as_bytes(), file_lba[i], data.len() as u32, false));
            }
        }
        if parent.is_none() {
            for (i, sub) in dirs.iter().enumerate() {
                d.extend(dir_record(sub.as_bytes(), 19 + i as u32, 2048, true));
            }
        }
        d
    };
    let root = dir_bytes(None, 18);
    sector(&mut img, 18, &root);
    for (i, sub) in dirs.iter().enumerate() {
        let d = dir_bytes(Some(sub), 19 + i as u32);
        sector(&mut img, 19 + i as u32, &d);
    }
    for (i, (_, data)) in files.iter().enumerate() {
        sector(&mut img, file_lba[i], data);
    }
    img
}

/// The same image as raw 2352-byte Mode 2 sectors (a .bin next to a .cue).
pub fn raw_bin(cooked: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    for s in cooked.chunks(2048) {
        let mut sec = vec![0u8; 2352];
        sec[1..11].fill(0xFF);
        sec[15] = 2;
        sec[24..24 + s.len()].copy_from_slice(s);
        out.extend(sec);
    }
    out
}

fn ps2_iso(serial_file: &str) -> Vec<u8> {
    iso(&[("SYSTEM.CNF", format!("BOOT2 = cdrom0:\\{serial_file};1\r\nVER = 1.00\r\nVMODE = NTSC\r\n").as_bytes())])
}

fn gba(code: &str, filler: u8) -> Vec<u8> {
    let mut h = vec![filler; 0x200];
    h[0xA0..0xAC].copy_from_slice(b"POKEMON FIRE");
    h[0xAC..0xB0].copy_from_slice(code.as_bytes());
    h[0xB2] = 0x96;
    h
}

fn nds(code: &str) -> Vec<u8> {
    let mut h = vec![0u8; 0x200];
    h[..12].copy_from_slice(b"POKEMON B\0\0\0");
    h[0x0C..0x10].copy_from_slice(code.as_bytes());
    h[0x12] = 2;
    h
}

fn disc_header(id: &str, wii: bool) -> Vec<u8> {
    let mut h = vec![0u8; 0x80];
    h[..6].copy_from_slice(id.as_bytes());
    if wii {
        h[0x18..0x1C].copy_from_slice(&0x5D1C_9EA3u32.to_be_bytes());
    } else {
        h[0x1C..0x20].copy_from_slice(&0xC233_9F3Du32.to_be_bytes());
    }
    h
}

fn rvz(id: &str, wii: bool) -> Vec<u8> {
    let mut b = vec![0u8; 0x100];
    b[..4].copy_from_slice(b"RVZ\x01");
    b[0x48..0x4C].copy_from_slice(&(if wii { 2u32 } else { 1 }).to_be_bytes());
    b[0x58..0xD8].copy_from_slice(&disc_header(id, wii));
    b
}

fn tmd(title_id: u64) -> Vec<u8> {
    let mut b = vec![0u8; 0x200];
    b[..4].copy_from_slice(&0x0001_0004u32.to_be_bytes());
    b[0x18C..0x194].copy_from_slice(&title_id.to_be_bytes());
    b
}

fn nsp(title_id: &str) -> Vec<u8> {
    let names =
        format!("{}0000000000000008.tik\0{}0000000000000008.cert\0", title_id.to_lowercase(), title_id.to_lowercase());
    let mut b = b"PFS0".to_vec();
    b.extend_from_slice(&2u32.to_le_bytes());
    b.extend_from_slice(&(names.len() as u32).to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend(vec![0u8; 0x18 * 2]);
    b.extend_from_slice(names.as_bytes());
    b.extend(vec![0u8; 64]);
    b
}

/// A zip whose central directory says the entry `name` has CRC `crc` (content is never read).
fn zip_with_crc(name: &str, crc: u32) -> Vec<u8> {
    let mut b = vec![];
    // Local header (not read by the scanner, written for realism).
    b.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]);
    b.extend(vec![0u8; 26]);
    b.extend_from_slice(name.as_bytes());
    let cd_off = b.len() as u32;
    let mut cd = vec![0x50, 0x4B, 0x01, 0x02];
    cd.extend(vec![0u8; 12]);
    cd.extend_from_slice(&crc.to_le_bytes());
    cd.extend(vec![0u8; 8]);
    cd.extend_from_slice(&(name.len() as u16).to_le_bytes());
    cd.extend(vec![0u8; 16]);
    cd.extend_from_slice(name.as_bytes());
    let cd_len = cd.len() as u32;
    b.extend(cd);
    b.extend_from_slice(&[0x50, 0x4B, 0x05, 0x06, 0, 0, 0, 0, 1, 0, 1, 0]);
    b.extend_from_slice(&cd_len.to_le_bytes());
    b.extend_from_slice(&cd_off.to_le_bytes());
    b.extend_from_slice(&[0, 0]);
    b
}

fn ctx(home: &Path, data: &Path, cfg: EmulationConfig) -> ScanContext {
    let mut c = crate::scan::desktop::test_support::ctx(home);
    c.data_dir = data.to_path_buf();
    c.emulation = cfg;
    c
}

fn folder(p: &Path) -> RomFolder {
    RomFolder { path: p.to_path_buf(), ..Default::default() }
}

fn cfg_with(folders: &[(&str, RomFolder)]) -> EmulationConfig {
    let mut c = EmulationConfig::detect_nothing();
    for (k, f) in folders {
        c.rom_folders.insert(k.to_string(), f.clone());
    }
    c
}

fn find<'a>(games: &'a [ScannedGame], name: &str) -> &'a ScannedGame {
    games
        .iter()
        .find(|g| g.name == name)
        .unwrap_or_else(|| panic!("no {name:?} in {:#?}", games.iter().map(|g| &g.name).collect::<Vec<_>>()))
}

/// The library the grouping tests share (consoles layout).
fn library(root: &Path) {
    // GBA: a hack with FireRed's code, a bare code file name, a CRC hit via a zip, a patch kit.
    write_file(&root.join("gba/Pokemon Radical Red v4.1.gba"), gba("BPRE", 1));
    write_file(&root.join("gba/BPRE.gba"), gba("BPRE", 2));
    write_file(
        &root.join("gba/Pokemon - FireRed Version (USA).zip"),
        zip_with_crc("Pokemon - FireRed Version (USA).gba", 0xDD88_761C),
    );
    write_file(&root.join("gba/Pokemon Radical Red v4.1.sav"), b"save");
    write_file(&root.join("gba/readme.txt"), b"hi");
    write_file(&root.join("gba/Unbound Patch/Pokemon Unbound.ups"), b"UPS1");
    write_file(&root.join("gba/Unbound Patch/1636 - Pokemon Fire Red (U)(Squirrels).gba"), gba("BPRE", 3));
    // A hack kept next to its patch: the patched ROM is a game.
    write_file(&root.join("gba/Glazed/Pokemon Glazed.ups"), b"UPS1");
    write_file(&root.join("gba/Glazed/Pokemon Glazed.gba"), gba("BPEE", 5));
    write_file(&root.join("gba/Glazed/Pokemon - Emerald Version (USA, Europe).gba"), gba("BPEE", 6));
    // The same ROM twice (CRC duplicate): one game.
    write_file(&root.join("gba/Pokemon Unbound.gba"), gba("BPRE", 4));
    write_file(&root.join("gba/copies/Pokemon Unbound copy.gba"), gba("BPRE", 4));
    // PS2: a cue + raw bin folder (bad file name, the DB names it), a plain ISO.
    let bin = raw_bin(&ps2_iso("SLUS_211.91"));
    write_file(&root.join("ps2/CrashTagTeamRacing(USA)/CrashTagTeamRacing(USA).bin"), bin);
    write_file(
        &root.join("ps2/CrashTagTeamRacing(USA)/CrashTagTeamRacing(USA).cue"),
        "FILE \"CrashTagTeamRacing(USA).bin\" BINARY\r\n  TRACK 01 MODE2/2352\r\n    INDEX 01 00:00:00\r\n",
    );
    write_file(&root.join("ps2/Crash Bandicoot - The Wrath of Cortex (USA) (v1.01).iso"), ps2_iso("SLUS_202.38"));
    // PS1: BIOS folder, an m3u over two cue/bin discs, a disc set without m3u.
    let mut bios = vec![0u8; 524_288];
    bios[0] = 1;
    write_file(&root.join("ps1/BIOS/scph1001.bin"), &bios);
    write_file(&root.join("ps1/scph5501.bin"), &bios);
    let ps1 = raw_bin(&iso(&[("SYSTEM.CNF", b"BOOT = cdrom:\\SCUS_944.26;1\r\nTCB = 4\r\n")]));
    for d in 1..=2 {
        write_file(&root.join(format!("ps1/Multi (USA) (Disc {d}).bin")), &ps1);
        write_file(
            &root.join(format!("ps1/Multi (USA) (Disc {d}).cue")),
            format!("FILE \"Multi (USA) (Disc {d}).bin\" BINARY\n TRACK 01 MODE2/2352\n"),
        );
    }
    write_file(
        &root.join("ps1/Multi (USA).m3u"),
        "# discs\nMulti (USA) (Disc 1).cue\nMulti (USA) (Disc 2).cue\nMissing (Disc 3).cue\n",
    );
    for d in 1..=3 {
        write_file(&root.join(format!("ps1/Saga (USA) (Disc {d}).chd")), b"MComprHD");
    }
    // GameCube: an RVZ, a stray N64 ROM.
    write_file(&root.join("gc/Legend of Zelda, The - The Wind Waker (USA).rvz"), rvz("GZLE01", false));
    let mut z64 = vec![0u8; 0x1000];
    z64[..4].copy_from_slice(&[0x80, 0x37, 0x12, 0x40]);
    z64[0x3B..0x3F].copy_from_slice(b"CZLE");
    write_file(&root.join("gc/Legend of Zelda, The - Ocarina of Time (USA).z64"), z64);
    // Wii U NUS: base + update + DLC folders.
    let botw = "wiiu/Legend of Zelda, The - Breath of the Wild (USA)";
    write_file(&root.join(botw).join("tmd.0"), tmd(0x0005_0000_101C_9400));
    write_file(&root.join(botw).join("00000000"), b"enc");
    write_file(&root.join(botw).join("00000001.h3"), b"h3");
    write_file(&root.join(format!("{botw} (Update)/tmd.208")), tmd(0x0005_000E_101C_9400));
    write_file(&root.join(format!("{botw} (Update)/00000000")), b"enc");
    write_file(&root.join(format!("{botw} (DLC)/tmd.80")), tmd(0x0005_000C_101C_9400));
    // Switch: base + update + DLC files.
    write_file(&root.join("switch/Pokemon Sword[0100ABF008968000][US][v0].nsp"), nsp("0100ABF008968000"));
    write_file(&root.join("switch/Pokemon Sword Update[0100ABF008968800][US][v65536].nsp"), nsp("0100ABF008968800"));
    write_file(&root.join("switch/Pokemon Sword DLC[0100ABF008969001][US].nsp"), nsp("0100ABF008969001"));
    write_file(&root.join("switch/Pokemon Violet [01008F6008C5E000][v0][US](nsw2u.com).nsp"), b"no pfs0 here");
    write_file(&root.join("switch/Pokemon Shining Pearl [010018E011D92000][v0](nsw2u.xyz).nsp"), b"x");
    // Not consoles / not games.
    write_file(&root.join("pc/Some Game/Game.exe"), b"MZ");
    write_file(&root.join("ps3games/Disc Games Can Be Put Here For Automatic Detection.txt"), b"");
    write_file(&root.join("ps3games/＄locks/x"), b"");
}

// ---------------------------------------------------------------------------------------------

#[test]
fn headers_are_read() {
    let tmp = tempfile::tempdir().unwrap();
    let w = |name: &str, bytes: &[u8]| {
        let p = tmp.path().join(name);
        write_file(&p, bytes);
        probe_file(&p, bytes.len() as u64)
    };
    let p = w("a.iso", &ps2_iso("SLUS_203.12"));
    assert_eq!((p.serial.as_deref(), p.platform.as_deref()), (Some("SLUS-20312"), Some("sony_playstation2")));
    let p = w("a.bin", &raw_bin(&iso(&[("SYSTEM.CNF", b"BOOT=cdrom:\\SLES_123.45;1")])));
    assert_eq!((p.serial.as_deref(), p.platform.as_deref()), (Some("SLES-12345"), Some("sony_playstation")));
    let p = w("psp.iso", &iso(&[("UMD_DATA.BIN", b"ULUS-10041|0000000000000001|0001|G")]));
    assert_eq!((p.serial.as_deref(), p.platform.as_deref()), (Some("ULUS-10041"), Some("sony_psp")));
    let p = w("psp2.iso", &iso(&[("PSP_GAME/PARAM.SFO", &sfo(&[("DISC_ID", "ULES01513"), ("TITLE", "X")]))]));
    assert_eq!(p.serial.as_deref(), Some("ULES-01513"));
    let p = w("ps3.iso", &iso(&[("PS3_GAME/PARAM.SFO", &sfo(&[("TITLE_ID", "BLUS30001")]))]));
    assert_eq!((p.serial.as_deref(), p.platform.as_deref()), (Some("BLUS30001"), Some("sony_playstation3")));
    assert_eq!(w("a.gba", &gba("BPEE", 0)).serial.as_deref(), Some("BPEE"));
    assert_eq!(w("a.nds", &nds("IRBO")).serial.as_deref(), Some("IRBO"));
    assert_eq!(w("gc.rvz", &rvz("GXXE01", false)).platform.as_deref(), Some("nintendo_gamecube"));
    let p = w("wii.rvz", &rvz("RZDE01", true));
    assert_eq!((p.serial.as_deref(), p.platform.as_deref()), (Some("RZDE01"), Some("nintendo_wii")));
    let mut gciso = disc_header("GM8E01", false);
    gciso.resize(0x1000, 0);
    assert_eq!(w("gc.iso", &gciso).serial.as_deref(), Some("GM8E01"));
    let mut wbfs = b"WBFS\0\0\0\x10\x09".to_vec();
    wbfs.resize(0x200, 0);
    wbfs.extend(disc_header("RMCE01", true));
    assert_eq!(w("x.wbfs", &wbfs).serial.as_deref(), Some("RMCE01"));
    // N64 in all three byte orders.
    let mut z = vec![0u8; 0x40];
    z[..4].copy_from_slice(&[0x80, 0x37, 0x12, 0x40]);
    z[0x3B..0x3F].copy_from_slice(b"NSME");
    let v: Vec<u8> = z.chunks(2).flat_map(|c| [c[1], c[0]]).collect();
    let n: Vec<u8> = z.chunks(4).flat_map(|c| [c[3], c[2], c[1], c[0]]).collect();
    for (name, b) in [("a.z64", &z), ("a.v64", &v), ("a.n64", &n)] {
        let p = w(name, b);
        assert_eq!((p.serial.as_deref(), p.platform.as_deref()), (Some("NSME"), Some("nintendo_64")), "{name}");
    }
    // .n64: the CRC of the big-endian bytes too (the database has .z64 rows).
    assert_eq!(w("b.n64", &n).crc_alt, Some(crc32fast::hash(&z)));
    // 3DS cartridge: NCSD → partition 0 NCCH product code.
    let mut cci = vec![0u8; 0x4200];
    cci[0x100..0x104].copy_from_slice(b"NCSD");
    cci[0x108..0x110].copy_from_slice(&0x0004_0000_0017_5E00u64.to_le_bytes());
    cci[0x120..0x124].copy_from_slice(&0x20u32.to_le_bytes());
    cci[0x4100..0x4104].copy_from_slice(b"NCCH");
    cci[0x4150..0x415A].copy_from_slice(b"CTR-P-BNEA");
    let p = w("moon.3ds", &cci);
    assert_eq!((p.serial.as_deref(), p.title_id.as_deref()), (Some("CTR-P-BNEA"), Some("0004000000175E00")));
    // Switch: the ticket name; update / DLC by the id's low bits.
    let p = w("s.nsp", &nsp("0100ABF008968800"));
    assert_eq!((p.title_id.as_deref(), p.role), (Some("0100ABF008968800"), Some(Role::Update)));
    assert_eq!(w("Game [0100ABF008969001].nsp", b"x").role, Some(Role::Dlc));
    // Zip: the stored CRC and the inner name.
    let p = w("g.zip", &zip_with_crc("Game (USA).gba", 0x1234_5678));
    assert_eq!((p.crc, p.inner.as_deref()), (Some(0x1234_5678), Some("Game (USA).gba")));
    // iNES: CRC with and without the 16-byte header.
    let mut nes = b"NES\x1a\x02\x01\0\0\0\0\0\0\0\0\0\0".to_vec();
    nes.extend(vec![7u8; 100]);
    let p = w("g.nes", &nes);
    assert_eq!((p.crc, p.crc_alt), (Some(crc32fast::hash(&nes)), Some(crc32fast::hash(&nes[16..]))));
    // Big discs are never hashed; unknown junk gives nothing.
    assert_eq!(w("junk.iso", &[1u8; 5000]).serial, None);
    // Wii U TMD + meta.xml.
    let t = tmp.path().join("tmd.208");
    write_file(&t, tmd(0x0005_000E_101C_9400));
    assert_eq!(identify::wiiu_tmd(&t), Some(("0005000E101C9400".into(), Some(Role::Update))));
}

#[test]
fn acceptance_rule_keeps_hack_names() {
    let gba = platforms::by_id("nintendo_gameboyadvance").unwrap();
    let probe = |serial: &str| identify::Probe { serial: Some(serial.into()), ..Default::default() };
    // A hack with FireRed's code keeps its name.
    let id = identify::identify(
        gba,
        "Pokemon Radical Red v4.1",
        &["Pokemon Radical Red v4.1.gba".into()],
        &probe("BPRE"),
        false,
    );
    assert!(id.entry.is_none(), "{id:?}");
    assert_eq!(id.how, "serial (name kept)");
    // A file named after its code takes the database name.
    let id = identify::identify(gba, "BPRE", &["BPRE.gba".into()], &probe("BPRE"), false);
    assert_eq!(id.entry.as_ref().map(|e| e.name.as_str()), Some("Pokemon - FireRed Version (USA)"));
    // Same title: accepted.
    let id = identify::identify(gba, "Pokemon - FireRed Version", &[], &probe("BPRE"), false);
    assert!(id.entry.is_some());
    // CRC: accepted whatever the file is called; the row whose CRC matches wins.
    let p = identify::Probe { serial: Some("BPRE".into()), crc: Some(0x84EE_4776), ..Default::default() };
    let id = identify::identify(gba, "my rom", &[], &p, false);
    assert_eq!(id.entry.as_ref().map(|e| e.name.as_str()), Some("Pokemon - FireRed Version (USA, Europe) (Rev 1)"));
    assert_eq!(id.how, "crc");
    // Discs: a serial hit is trusted.
    let ps2 = platforms::by_id("sony_playstation2").unwrap();
    let id = identify::identify(ps2, "CrashTagTeamRacing(USA)", &[], &probe("SLUS-21191"), true);
    assert_eq!(id.entry.as_ref().map(|e| e.name.as_str()), Some("Crash Tag Team Racing (USA)"));
    // A dump-set name with the same title keeps its own tags as the art name.
    let id =
        identify::identify(ps2, "Crash Bandicoot - The Wrath of Cortex (USA) (v1.01)", &[], &probe("SLUS-20238"), true);
    assert_eq!(id.dat_name.as_deref(), Some("Crash Bandicoot - The Wrath of Cortex (USA) (v1.01)"));
    assert_eq!(id.region.as_deref(), Some("USA"));
    // GameCube: GameTDB name for display, the Redump row (right region) for art, year.
    let gc = platforms::by_id("nintendo_gamecube").unwrap();
    let id = identify::identify(gc, "zelda", &[], &probe("GZLE01"), true);
    assert_eq!(id.entry.as_ref().map(|e| e.name.as_str()), Some("The Legend of Zelda: The Wind Waker (USA)"));
    assert_eq!(id.dat_name.as_deref(), Some("Legend of Zelda, The - The Wind Waker (USA)"));
    assert_eq!(id.year, Some(2003));
}

#[test]
fn library_is_grouped_and_named() {
    let tmp = tempfile::tempdir().unwrap();
    let roms = tmp.path().join("roms");
    library(&roms);
    let c = ctx(tmp.path(), &tmp.path().join("data"), cfg_with(&[("f1", folder(&roms))]));
    let games = scan_env(&c, &Env::default()).unwrap();
    let names: Vec<&str> = games.iter().map(|g| g.name.as_str()).collect();
    // GBA: hack keeps its name, the code-named file is FireRed, the zip CRC hit too (same name
    // on one console → regions), the patch kit is not a game, the CRC duplicate is one game.
    let hack = find(&games, "Pokemon Radical Red v4.1");
    let e = hack.emu.as_ref().unwrap();
    assert_eq!(
        (e.serial.as_deref(), e.dat_name.as_deref(), e.platform.as_str()),
        (Some("BPRE"), None, "nintendo_gameboyadvance")
    );
    assert_eq!(hack.platform.as_deref(), Some("Nintendo Game Boy Advance"));
    assert_eq!(hack.source_id, "f1/gba/Pokemon Radical Red v4.1.gba");
    assert_eq!(hack.install_dir.as_deref(), Some(roms.join("gba").as_path()));
    assert_eq!(hack.size_bytes, Some(0x200));
    assert!(hack.launch.is_none(), "no emulator was detected");
    let fire: Vec<&&str> = names.iter().filter(|n| n.starts_with("Pokémon FireRed Version")).collect();
    assert_eq!(fire.len(), 2, "{names:?}");
    assert!(names.contains(&"Pokémon FireRed Version (USA)"), "{names:?}");
    let zip = games.iter().find(|g| g.source_id.ends_with(".zip")).unwrap();
    assert_eq!(zip.emu.as_ref().unwrap().crc.as_deref(), Some("dd88761c"));
    assert!(!names.iter().any(|n| n.contains("Fire Red")), "patch kit skipped: {names:?}");
    assert!(names.contains(&"Pokemon Glazed"), "patched ROM next to its patch: {names:?}");
    assert!(!names.iter().any(|n| n.contains("Emerald")), "original ROM in a patch kit: {names:?}");
    let unbound = find(&games, "Pokemon Unbound");
    assert_eq!(unbound.emu.as_ref().unwrap().files, vec![roms.join("gba/copies/Pokemon Unbound copy.gba")]);
    assert_eq!(games.iter().filter(|g| g.name.contains("Unbound")).count(), 1);
    // PS2: cue+bin folder is one game named by the DB, installed in its folder; plain ISO.
    let ctr = find(&games, "Crash Tag Team Racing");
    let e = ctr.emu.as_ref().unwrap();
    let dir = roms.join("ps2/CrashTagTeamRacing(USA)");
    assert_eq!(e.rom, dir.join("CrashTagTeamRacing(USA).cue"));
    assert_eq!(e.files, vec![dir.join("CrashTagTeamRacing(USA).bin")]);
    assert_eq!(e.serial.as_deref(), Some("SLUS-21191"));
    assert_eq!(ctr.install_dir.as_deref(), Some(dir.as_path()));
    let cortex = find(&games, "Crash Bandicoot: The Wrath of Cortex");
    assert_eq!(cortex.emu.as_ref().unwrap().region.as_deref(), Some("USA"));
    // PS1: BIOS skipped, m3u hides its cues and bins, the chd set is one game.
    assert!(!games.iter().any(|g| g.source_id.contains("scph")), "{names:?}");
    let multi = games.iter().find(|g| g.source_id.ends_with("Multi (USA).m3u")).unwrap();
    assert_eq!(multi.name, "CTR: Crash Team Racing", "serial → database name");
    assert_eq!(multi.emu.as_ref().unwrap().files.len(), 4);
    assert_eq!(multi.emu.as_ref().unwrap().serial.as_deref(), Some("SCUS-94426"));
    assert_eq!(games.iter().filter(|g| g.source_id.contains("Multi")).count(), 1);
    let saga = find(&games, "Saga");
    assert!(saga.emu.as_ref().unwrap().rom.ends_with("Saga (USA) (Disc 1).chd"));
    assert_eq!(saga.emu.as_ref().unwrap().files.len(), 2);
    // GameCube RVZ → DB name and year; the stray .z64 is never a GameCube game.
    let ww = find(&games, "The Legend of Zelda: The Wind Waker");
    assert_eq!(ww.meta.release_year, Some(2003));
    assert_eq!(ww.emu.as_ref().unwrap().dat_name.as_deref(), Some("Legend of Zelda, The - The Wind Waker (USA)"));
    let oot = games.iter().find(|g| g.source_id.ends_with(".z64"));
    match platforms::by_id("nintendo_64") {
        Some(n64) => assert_eq!(oot.unwrap().emu.as_ref().unwrap().platform, n64.id),
        None => assert!(oot.is_none(), "no N64 in the platform table: skipped"),
    }
    // Wii U: one game; update and DLC hidden and attached; a folder game has no size.
    let botw = find(&games, "The Legend of Zelda: Breath of the Wild");
    let e = botw.emu.as_ref().unwrap();
    let base = roms.join("wiiu/Legend of Zelda, The - Breath of the Wild (USA)");
    assert_eq!(e.rom, base);
    assert_eq!(e.serial.as_deref(), Some("00050000101C9400"));
    assert_eq!(e.files.len(), 2);
    assert_eq!(botw.size_bytes, None);
    assert_eq!(botw.install_dir.as_deref(), Some(base.as_path()));
    assert_eq!(games.iter().filter(|g| g.platform.as_deref() == Some("Nintendo Wii U")).count(), 1);
    // Switch: clean names, update + DLC attached.
    let sword = find(&games, "Pokémon Sword");
    assert_eq!(sword.emu.as_ref().unwrap().files.len(), 2);
    assert_eq!(sword.emu.as_ref().unwrap().region.as_deref(), Some("USA"));
    find(&games, "Pokémon Violet");
    find(&games, "Pokémon Shining Pearl");
    assert_eq!(games.iter().filter(|g| g.platform.as_deref() == Some("Nintendo Switch")).count(), 3);
    // Nothing from pc/, ps3games/ or non-ROM files.
    for g in &games {
        assert!(
            !g.source_id.contains("/pc/") && !g.source_id.contains("ps3games") && !g.source_id.ends_with(".sav"),
            "{}",
            g.source_id
        );
        assert!(g.installed && g.source == Source::Emulator);
    }
}

#[test]
fn cache_makes_rescans_cheap_and_state_is_remembered() {
    let tmp = tempfile::tempdir().unwrap();
    let roms = tmp.path().join("roms");
    library(&roms);
    let data = tmp.path().join("data");
    let c = ctx(tmp.path(), &data, cfg_with(&[("f1", folder(&roms))]));
    let first = scan_env(&c, &Env::default()).unwrap();
    assert!(data.join("emulation/hash-cache.json").is_file());
    let mem = roots::RootsFile::load(Some(&data.join("emulation/roots.json")));
    assert_eq!(mem.roots.len(), 1);
    assert_eq!((mem.roots[0].games, mem.roots[0].auto), (first.len(), false));
    // Second scan: every probe from the cache, same result.
    let root = roots::configured("f1", &folder(&roms));
    let mut cache = identify::HashCache::load(Some(data.join("emulation/hash-cache.json")));
    let (again, _) = scan_roots(&c, &scan_registry(&c), &[root], &mut cache);
    assert_eq!(cache.probed, 0);
    assert!(cache.hits > 10);
    let key = |v: &[ScannedGame]| {
        let mut k: Vec<(String, String)> = v.iter().map(|g| (g.source_id.clone(), g.name.clone())).collect();
        k.sort();
        k
    };
    assert_eq!(key(&first), key(&again));
    // A changed file is read again.
    write_file(&roms.join("gba/BPRE.gba"), gba("BPEE", 9));
    let mut cache = identify::HashCache::load(Some(data.join("emulation/hash-cache.json")));
    let root = roots::configured("f1", &folder(&roms));
    scan_roots(&c, &scan_registry(&c), &[root], &mut cache);
    assert_eq!(cache.probed, 1);
}

#[test]
fn missing_configured_folder_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let gone = tmp.path().join("usb/roms");
    let c = ctx(tmp.path(), &tmp.path().join("data"), cfg_with(&[("f1", folder(&gone))]));
    let err = scan_env(&c, &Env::default()).unwrap_err().to_string();
    assert_eq!(err, format!("ROM folder {} isn't available — is the drive connected?", gone.display()));
    // A disabled one is not scanned and not an error.
    let mut f = folder(&gone);
    f.enabled = false;
    let c = ctx(tmp.path(), &tmp.path().join("data"), cfg_with(&[("f1", f)]));
    assert!(scan_env(&c, &Env::default()).unwrap().is_empty());
    // An empty mount point that had games is missing too.
    let mnt = tmp.path().join("mnt/disk");
    write_file(&mnt.join("gba/a.gba"), gba("AAAA", 0));
    let c = ctx(tmp.path(), &tmp.path().join("data"), cfg_with(&[("f1", folder(&mnt))]));
    assert_eq!(scan_env(&c, &Env::default()).unwrap().len(), 1);
    std::fs::remove_dir_all(mnt.join("gba")).unwrap();
    assert!(scan_env(&c, &Env::default()).is_err());
    // preview_folder says the same.
    assert!(preview_folder(&c, &folder(&gone)).unwrap_err().to_string().contains("isn't available"));
}

#[test]
fn drive_roots_are_found_and_remembered() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let mnt = tmp.path().join("mnt");
    let media = tmp.path().join("run/media/me");
    // /mnt/space1/games has console folders (depth 2); /mnt/space1/other doesn't.
    write_file(&mnt.join("space1/games/gba/Game (USA).gba"), gba("AAAA", 0));
    write_file(&mnt.join("space1/games/ps2/x.iso"), ps2_iso("SLUS_211.91"));
    write_file(&mnt.join("space1/other/gba/y.gba"), gba("BBBB", 0));
    // Depth 4 is too deep.
    write_file(&mnt.join("a/b/c/d/gba/deep.gba"), gba("CCCC", 0));
    write_file(&mnt.join("a/b/c/d/ps2/deep.iso"), b"x");
    // A network mount is never walked.
    write_file(&mnt.join("nas/roms/gba/net.gba"), gba("DDDD", 0));
    write_file(&mnt.join("nas/roms/ps2/net.iso"), b"x");
    let mounts = tmp.path().join("mounts");
    write_file(&mounts, format!("server:/x {} nfs4 rw 0 0\n/dev/sda1 / ext4 rw 0 0\n", mnt.join("nas").display()));
    // A removable stick.
    write_file(&media.join("STICK/gba/stick.gba"), gba("EEEE", 0));
    write_file(&media.join("STICK/switch/s [0100000000010000][US].nsp"), b"x");
    // ~/ROMs (one console folder is enough in a well-known place).
    write_file(&home.join("ROMs/gba/home.gba"), gba("FFFF", 0));
    let env = Env {
        system: vec![mnt.clone(), media.clone()],
        removable: vec![tmp.path().join("run/media")],
        mounts: Some(mounts),
    };
    let data = tmp.path().join("data");
    let mut cfg = EmulationConfig::detect_nothing();
    cfg.auto_folders = true;
    let c = ctx(&home, &data, cfg.clone());
    let found = roots::collect(&c, &env);
    let paths: Vec<&Path> = found.iter().map(|r| r.path.as_path()).collect();
    let canon = |p: PathBuf| std::fs::canonicalize(p).unwrap();
    assert!(paths.contains(&canon(mnt.join("space1/games")).as_path()), "{paths:?}");
    assert!(paths.contains(&canon(media.join("STICK")).as_path()), "{paths:?}");
    assert!(paths.contains(&canon(home.join("ROMs")).as_path()), "{paths:?}");
    assert_eq!(found.len(), 3, "{paths:?}");
    assert!(found.iter().all(|r| r.id.starts_with("auto-") && r.id.len() == 13 && r.folder.is_none()));
    let games = scan_env(&c, &env).unwrap();
    assert_eq!(games.len(), 5, "{:?}", games.iter().map(|g| &g.source_id).collect::<Vec<_>>());
    let g = games.iter().find(|g| g.source_id.ends_with("gba/Game (USA).gba")).unwrap();
    assert_eq!(g.source_id, format!("{}/gba/Game (USA).gba", roots::auto_id(&canon(mnt.join("space1/games")))));
    // Ignored folders are skipped.
    let mut ign = cfg.clone();
    ign.ignored_folders = vec![canon(home.join("ROMs"))];
    assert_eq!(roots::collect(&ctx(&home, &data, ign), &env).len(), 2);
    // The stick unplugged: no error (its games just go "not installed").
    std::fs::remove_dir_all(media.join("STICK")).unwrap();
    assert_eq!(scan_env(&c, &env).unwrap().len(), 3);
    // /mnt drive gone: an error for MAX_MISSES scans, then forgotten.
    std::fs::remove_dir_all(mnt.join("space1/games")).unwrap();
    for _ in 0..roots::MAX_MISSES {
        let e = scan_env(&c, &env).unwrap_err().to_string();
        assert!(e.contains("space1/games") && e.contains("isn't available"), "{e}");
    }
    assert_eq!(scan_env(&c, &env).unwrap().len(), 1);
    assert_eq!(scan_env(&c, &env).unwrap().len(), 1);
    // Back again: found again.
    write_file(&mnt.join("space1/games/gba/Game (USA).gba"), gba("AAAA", 0));
    write_file(&mnt.join("space1/games/ps2/x.iso"), b"x");
    assert_eq!(scan_env(&c, &env).unwrap().len(), 3);
    // Switching automatic folders off forgets them (no error even if they vanish).
    std::fs::remove_dir_all(mnt.join("space1/games")).unwrap();
    let mut off = cfg.clone();
    off.auto_folders = false;
    assert!(scan_env(&ctx(&home, &data, off), &env).unwrap().is_empty());
}

#[test]
fn emulator_settings_and_esde_give_folders() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    crate::scan::desktop::test_support::copy_tree(
        &crate::scan::desktop::test_support::fixture("emulation/home"),
        &home,
    );
    for f in [
        ".config/dolphin-emu/Dolphin.ini",
        ".var/app/net.pcsx2.PCSX2/config/PCSX2/inis/PCSX2.ini",
        ".var/app/org.duckstation.DuckStation/config/duckstation/settings.ini",
        ".config/eden/qt-config.ini",
        ".var/app/net.rpcs3.RPCS3/config/rpcs3/vfs.yml",
        ".config/Cemu/settings.xml",
    ] {
        let p = home.join(f);
        let t = std::fs::read_to_string(&p).unwrap().replace("/HOME", &home.to_string_lossy());
        std::fs::write(&p, t).unwrap();
    }
    write_file(&home.join("ESDE-ROMs/ps2/a.iso"), ps2_iso("SLUS_211.91"));
    write_file(&home.join("Games/Dolphin/Zelda (USA).rvz"), rvz("GZLE01", false));
    write_file(&home.join("Games/PS2 ISOs/b.iso"), ps2_iso("SLUS_202.38"));
    write_file(&home.join("Games/psx stuff/c.bin"), raw_bin(&iso(&[("SYSTEM.CNF", b"BOOT = cdrom:\\SCUS_944.26;1")])));
    write_file(&home.join("Games/psx stuff/c.cue"), "FILE \"c.bin\" BINARY\n TRACK 01 MODE2/2352\n");
    write_file(&home.join("Games/Switch dumps/Game [0100000000010000][US].nsp"), b"x");
    write_file(&home.join("Games/WiiU/Game (USA)/tmd.0"), tmd(0x0005_0000_1010_0000));
    write_file(&home.join("ES-DE/downloaded_media/ps2/covers/a.png"), b"\x89PNG\r\n\x1a\nxx");
    let dirs = roots::emulator_dirs(&ctx(&home, Path::new(""), EmulationConfig::detect_nothing()));
    let has = |rel: &str, plat: &str| dirs.iter().any(|(p, pl, _)| *p == home.join(rel) && *pl == plat);
    assert!(has("Games/Dolphin", "nintendo_gamecube"), "{dirs:?}");
    assert!(has("Games/PS2 ISOs", "sony_playstation2"), "{dirs:?}");
    assert!(has("Games/psx stuff", "sony_playstation"), "{dirs:?}");
    assert!(has("Games/Switch dumps", "nintendo_switch"), "{dirs:?}");
    assert!(has("Games/WiiU", "nintendo_wiiu"), "{dirs:?}");
    assert!(has("Games/ps3games", "sony_playstation3"), "{dirs:?}");
    assert!(!dirs.iter().any(|(p, _, _)| p.ends_with("SDMC") || p.ends_with("UserNAND")));
    let mut cfg = EmulationConfig::detect_nothing();
    cfg.auto_folders = true;
    let c = ctx(&home, &tmp.path().join("data"), cfg);
    let games = scan_env(&c, &Env::default()).unwrap();
    let plat = |sid_end: &str| {
        games.iter().find(|g| g.source_id.ends_with(sid_end)).map(|g| g.emu.as_ref().unwrap().platform.clone())
    };
    assert_eq!(plat("a.iso").as_deref(), Some("sony_playstation2"));
    assert_eq!(plat("Zelda (USA).rvz").as_deref(), Some("nintendo_gamecube"));
    assert_eq!(plat("b.iso").as_deref(), Some("sony_playstation2"));
    assert_eq!(plat("c.cue").as_deref(), Some("sony_playstation"));
    assert_eq!(plat("Game [0100000000010000][US].nsp").as_deref(), Some("nintendo_switch"));
    assert_eq!(plat("Game (USA)").as_deref(), Some("nintendo_wiiu"));
    // The ES-DE ps2 folder is found once (PCSX2 lists it too).
    assert_eq!(games.iter().filter(|g| g.source_id.ends_with("a.iso")).count(), 1);
    // ES-DE downloaded media as box art.
    let a = games.iter().find(|g| g.source_id.ends_with("a.iso")).unwrap();
    assert_eq!(a.art.cover.as_deref(), Some(home.join("ES-DE/downloaded_media/ps2/covers/a.png").as_path()));
}

#[test]
fn preview_and_folder_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let ps2 = tmp.path().join("Disc images");
    write_file(&ps2.join("a.iso"), ps2_iso("SLUS_211.91"));
    write_file(&ps2.join("hidden/b.iso"), ps2_iso("SLUS_202.38"));
    write_file(&ps2.join("a.png"), b"\x89PNG\r\n\x1a\nxx");
    let c = ctx(tmp.path(), &tmp.path().join("data"), EmulationConfig::detect_nothing());
    // The folder's name says nothing: as "sub-folders are consoles" nothing is found …
    assert!(preview_folder(&c, &folder(&ps2)).unwrap().is_empty());
    // … with its console chosen, both games; excludes are honoured.
    let mut f = folder(&ps2);
    f.platform = "sony_playstation2".into();
    let games = preview_folder(&c, &f).unwrap();
    assert_eq!(games.len(), 2);
    let a = find(&games, "Crash Tag Team Racing");
    assert!(a.source_id.starts_with("auto-") && a.source_id.ends_with("/a.iso"), "{}", a.source_id);
    assert_eq!(a.art.cover.as_deref(), Some(ps2.join("a.png").as_path()));
    f.exclude = vec!["hidden".into()];
    assert_eq!(preview_folder(&c, &f).unwrap().len(), 1);
    // A configured folder's own id is used; an explicit console isn't moved by content.
    let mut cfg = EmulationConfig::detect_nothing();
    cfg.rom_folders.insert("f7".into(), f.clone());
    let c = ctx(tmp.path(), &tmp.path().join("data"), cfg);
    let games = preview_folder(&c, &f).unwrap();
    assert_eq!(games[0].source_id, "f7/a.iso");
    // Preview writes nothing.
    assert!(!tmp.path().join("data/emulation").exists());
    // Wii U extracted folder: meta.xml names it.
    let wiiu = tmp.path().join("wiiu");
    write_file(&wiiu.join("BOTW/code/U-King.rpx"), b"x");
    write_file(&wiiu.join("BOTW/content/x"), b"x");
    write_file(
        &wiiu.join("BOTW/meta/meta.xml"),
        "<?xml version=\"1.0\"?><menu><title_id type=\"hexBinary\" length=\"8\">00050000101C9400</title_id>\
         <product_code type=\"string\" length=\"32\">WUP-P-ALZE</product_code><company_code type=\"string\" length=\"8\">0001</company_code>\
         <longname_en type=\"string\" length=\"512\">The Legend of Zelda\nBreath of the Wild</longname_en></menu>",
    );
    let games = preview_folder(&c, &folder(&wiiu)).unwrap();
    assert_eq!(games.len(), 1);
    assert_eq!(games[0].name, "The Legend of Zelda: Breath of the Wild");
    assert_eq!(games[0].emu.as_ref().unwrap().serial.as_deref(), Some("ALZE01"));
}

#[test]
fn same_names_keep_their_region() {
    let tmp = tempfile::tempdir().unwrap();
    let roms = tmp.path().join("gba");
    write_file(&roms.join("Some Homebrew (USA).gba"), gba("ZZZ1", 1));
    write_file(&roms.join("Some Homebrew (Europe).gba"), gba("ZZZ2", 2));
    let c = ctx(tmp.path(), &tmp.path().join("data"), EmulationConfig::detect_nothing());
    let mut names: Vec<String> = preview_folder(&c, &folder(&roms)).unwrap().into_iter().map(|g| g.name).collect();
    names.sort();
    assert_eq!(names, ["Some Homebrew (Europe)", "Some Homebrew (USA)"]);
}

#[test]
fn whole_scan_merges_into_the_library() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let roms = tmp.path().join("roms");
    library(&roms);
    let c = ctx(&home, &tmp.path().join("data"), cfg_with(&[("f1", folder(&roms))]));
    let report = crate::scan::run_all(&c);
    assert!(report.ok_sources.contains(&Source::Emulator), "{:?}", report.per_source);
    let emu: Vec<&ScannedGame> = report.games.iter().filter(|g| g.source == Source::Emulator).collect();
    assert!(emu.len() >= 12, "{}", emu.len());
    let mut db = crate::db::Db::open(&tmp.path().join("library.db")).unwrap();
    let st = db.merge_scan(&report.games, &report.ok_sources).unwrap();
    let _ = st;
    let all = db.load_all().unwrap();
    let botw = all.iter().find(|g| g.name == "The Legend of Zelda: Breath of the Wild").unwrap();
    assert_eq!(botw.emu.as_ref().unwrap().platform, "nintendo_wiiu");
    assert_eq!(botw.platform.as_deref(), Some("Nintendo Wii U"));
    assert!(botw.installed);
    // A rescan adds nothing; a missing drive keeps the games as they are.
    let again = crate::scan::run_all(&c);
    let n = all.len();
    db.merge_scan(&again.games, &again.ok_sources).unwrap();
    assert_eq!(db.load_all().unwrap().len(), n);
    let moved = tmp.path().join("roms-away");
    std::fs::rename(&roms, &moved).unwrap();
    let gone = crate::scan::run_all(&c);
    assert!(!gone.ok_sources.contains(&Source::Emulator));
    let err = gone.per_source.iter().find(|(s, _)| *s == Source::Emulator).unwrap().1.error.clone().unwrap();
    assert!(err.contains("isn't available"), "{err}");
    db.merge_scan(&gone.games, &gone.ok_sources).unwrap();
    assert!(db.load_all().unwrap().iter().filter(|g| g.source == Source::Emulator).all(|g| g.installed));
}

/// What a scan of the user's real ROM folder would import (read-only; prints a report).
/// `cargo test --release real_rom_folder -- --ignored --nocapture`
#[test]
#[ignore]
fn real_rom_folder() {
    let path = std::env::var("LODESTAR_ROMS").unwrap_or_else(|_| "/mnt/space1/games".into());
    let path = PathBuf::from(path);
    if !path.is_dir() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let home = dirs::home_dir().unwrap();
    let c = ctx(&home, &tmp.path().join("data"), cfg_with(&[("f1", folder(&path))]));
    let mut cache = identify::HashCache::load(Some(tmp.path().join("cache.json")));
    let reg = scan_registry(&c);
    let t = std::time::Instant::now();
    let (games, _) = scan_roots(&c, &reg, &[roots::configured("f1", &folder(&path))], &mut cache);
    let cold = t.elapsed();
    cache.save(std::slice::from_ref(&path));
    let mut cache2 = identify::HashCache::load(Some(tmp.path().join("cache.json")));
    let t = std::time::Instant::now();
    let (warm_games, _) = scan_roots(&c, &reg, &[roots::configured("f1", &folder(&path))], &mut cache2);
    let warm = t.elapsed();
    let mut games = games;
    games.sort_by_key(|g| (g.platform.clone(), g.name.clone()));
    for g in &games {
        let e = g.emu.as_ref().unwrap();
        println!(
            "{:<26} {:<55} serial={:<18} crc={:<8} region={:<12} year={:<4} files={} rom={}",
            g.platform.as_deref().unwrap_or(""),
            g.name,
            e.serial.as_deref().unwrap_or("-"),
            e.crc.as_deref().unwrap_or("-"),
            e.region.as_deref().unwrap_or("-"),
            g.meta.release_year.map(|y| y.to_string()).unwrap_or("-".into()),
            e.files.len(),
            g.source_id,
        );
        if let Some(d) = &e.dat_name {
            println!("{:<26}   dat: {d}", "");
        }
        for f in &e.files {
            println!("{:<26}   + {}", "", f.strip_prefix(&path).unwrap_or(f).display());
        }
    }
    let mut auto = c.clone();
    auto.emulation.auto_folders = true;
    auto.emulation.rom_folders.clear();
    for r in roots::collect(&auto, &Env::real(&auto)) {
        println!("auto root: {} [{}] {:?} ({})", r.path.display(), r.id, r.layout.map_id(), r.how);
    }
    println!(
        "{} games; cold {cold:?} ({} files read), warm {warm:?} ({} from cache, {} read); same result: {}",
        games.len(),
        cache.probed,
        cache2.hits,
        cache2.probed,
        warm_games.len() == games.len()
    );
}

#[test]
fn m3u_hides_the_discs_it_lists_in_sub_folders() {
    let tmp = tempfile::tempdir().unwrap();
    let roms = tmp.path().join("ps1");
    let ps1 = raw_bin(&iso(&[("SYSTEM.CNF", b"BOOT = cdrom:\\SCUS_944.26;1\r\n")]));
    for d in 1..=2 {
        let stem = format!("Multi (USA) (Disc {d})");
        write_file(&roms.join(format!("Multi (USA)/{stem}.bin")), &ps1);
        write_file(
            &roms.join(format!("Multi (USA)/{stem}.cue")),
            format!("FILE \"{stem}.bin\" BINARY\n TRACK 01 MODE2/2352\n"),
        );
    }
    write_file(
        &roms.join("Multi (USA).m3u"),
        "Multi (USA)/Multi (USA) (Disc 1).cue\nMulti (USA)/Multi (USA) (Disc 2).cue\n",
    );
    let c = ctx(tmp.path(), &tmp.path().join("data"), EmulationConfig::detect_nothing());
    let games = preview_folder(&c, &folder(&roms)).unwrap();
    assert_eq!(games.len(), 1, "{:#?}", games.iter().map(|g| &g.source_id).collect::<Vec<_>>());
    let e = games[0].emu.as_ref().unwrap();
    assert!(e.rom.ends_with("Multi (USA).m3u"));
    assert_eq!(e.files.len(), 4, "both cues and both bins: {:?}", e.files);
}

#[test]
fn every_drive_root_gets_its_own_search_budget() {
    let tmp = tempfile::tempdir().unwrap();
    // A big drive (a Windows partition, say) under the first system root.
    let mnt = tmp.path().join("mnt");
    for i in 0..30 {
        for j in 0..20 {
            std::fs::create_dir_all(mnt.join(format!("windows/d{i:02}/e{j:02}"))).unwrap();
        }
    }
    let media = tmp.path().join("run/media/user");
    std::fs::create_dir_all(media.join("stick/roms/ps2")).unwrap();
    std::fs::create_dir_all(media.join("stick/roms/gba")).unwrap();
    let env = Env { system: vec![mnt, media.clone()], removable: vec![], mounts: None };
    let mut cfg = EmulationConfig::detect_nothing();
    cfg.auto_folders = true;
    let c = ctx(tmp.path(), &tmp.path().join("data"), cfg);
    let found: Vec<PathBuf> = roots::collect(&c, &env).into_iter().map(|r| r.path).collect();
    assert!(found.contains(&media.join("stick/roms")), "{found:?}");
}

#[test]
fn rpcs3_game_list_parents_are_games_folders_only_when_they_look_like_one() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_path_buf();
    let h = home.to_string_lossy();
    write_file(
        &home.join(".config/rpcs3/games.yml"),
        format!(
            "BLUS30001: {h}/Downloads/One Game/\nBLES00002: \"{h}/PS3 Library/A/\"\nBLES00003: {h}/PS3 Library/B/\nNPUB30004: {h}/stuff/ps3/C/\n"
        ),
    );
    let dirs = roots::emulator_dirs(&ctx(&home, Path::new(""), EmulationConfig::detect_nothing()));
    let got: Vec<&PathBuf> = dirs.iter().map(|(p, _, _)| p).collect();
    assert!(got.contains(&&home.join("PS3 Library")), "{got:?}");
    assert!(!got.contains(&&home.join("Downloads")), "{got:?}");
    if crate::emulation::platforms::by_folder_name("ps3").is_some() {
        assert!(got.contains(&&home.join("stuff/ps3")), "{got:?}");
    }
}

#[test]
fn ppsspp_ryujinx_and_emudeck_settings_give_folders() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_path_buf();
    let h = home.to_string_lossy().into_owned();
    write_file(
        &home.join(".var/app/org.ppsspp.PPSSPP/config/ppsspp/PSP/SYSTEM/ppsspp.ini"),
        format!("[General]\nCurrentDirectory = {h}\n[PinnedPaths]\nPath0 = {h}/PSP Games\n"),
    );
    write_file(&home.join(".config/Ryujinx/Config.json"), format!("{{\"game_dirs\": [\"{h}/Switch\"], \"other\": 1}}"));
    let dirs = roots::emulator_dirs(&ctx(&home, Path::new(""), EmulationConfig::detect_nothing()));
    let has = |p: PathBuf, plat: &str| dirs.iter().any(|(q, pl, _)| *q == p && *pl == plat);
    assert!(has(home.join("PSP Games"), "sony_psp"), "{dirs:?}");
    assert!(!dirs.iter().any(|(q, _, _)| *q == home), "the last folder browsed is not a games folder: {dirs:?}");
    assert!(has(home.join("Switch"), "nintendo_switch"), "{dirs:?}");
    // EmuDeck: romsPath from its settings (with $emulationPath).
    write_file(
        &home.join(".config/EmuDeck/settings.sh"),
        "emulationPath=\"$HOME/Deck Emu\"\nromsPath=\"$emulationPath/roms\"\n",
    );
    write_file(&home.join("Deck Emu/roms/gba/Game (USA).gba"), gba("ZZZE", 1));
    let mut cfg = EmulationConfig::detect_nothing();
    cfg.auto_folders = true;
    let found = roots::collect(&ctx(&home, Path::new(""), cfg), &Env::default());
    assert!(found.iter().any(|r| r.path == home.join("Deck Emu/roms") && r.how == "EmuDeck's ROM folder"), "{found:?}");
}

#[test]
fn wiiu_folder_without_title_files_is_listed_without_a_launch() {
    // BotW in the user's shape: tmd.0 + content files, update and DLC next to it.
    let tmp = tempfile::tempdir().unwrap();
    let roms = tmp.path().join("roms");
    library(&roms);
    let mut cfg = cfg_with(&[("f1", folder(&roms))]);
    cfg.emulators.insert(
        "cemu".into(),
        crate::config::EmulatorEntry {
            follows: "cemu".into(),
            kind: "binary".into(),
            path: "/usr/bin/true".into(),
            ..Default::default()
        },
    );
    let c = ctx(tmp.path(), &tmp.path().join("data"), cfg);
    let games = scan_env(&c, &Env::default()).unwrap();
    let botw = find(&games, "The Legend of Zelda: Breath of the Wild");
    assert!(botw.launch.is_none(), "Cemu can't open it: no command, Play explains");
    assert_eq!(botw.emu.as_ref().unwrap().emulator, "cemu");
    // Update and DLC stay hidden (attached).
    assert_eq!(games.iter().filter(|g| g.platform.as_deref() == Some("Nintendo Wii U")).count(), 1);
}

#[test]
fn a_configured_folder_wins_over_the_same_folder_found_automatically() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let roms = home.join("ROMs");
    write_file(&roms.join("ps2/a.iso"), ps2_iso("SLUS_211.91"));
    write_file(&roms.join("gba/b.gba"), gba("BPRE", 1));
    // Configured through a symlink to the same folder.
    let link = tmp.path().join("link-to-roms");
    std::os::unix::fs::symlink(&roms, &link).unwrap();
    let mut cfg = cfg_with(&[("f1", folder(&link))]);
    cfg.auto_folders = true;
    let c = ctx(&home, &tmp.path().join("data"), cfg);
    let found = roots::collect(&c, &Env::default());
    let ids: Vec<&str> = found.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["f1"], "{found:?}");
    let games = scan_env(&c, &Env::default()).unwrap();
    assert!(
        games.iter().all(|g| g.source_id.starts_with("f1/")),
        "{:?}",
        games.iter().map(|g| &g.source_id).collect::<Vec<_>>()
    );
}

#[test]
fn folders_found_automatically_are_remembered_before_they_have_games() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let data = tmp.path().join("data");
    std::fs::create_dir_all(home.join("ROMs/ps2")).unwrap();
    std::fs::create_dir_all(home.join("ROMs/gba")).unwrap();
    let mut cfg = EmulationConfig::detect_nothing();
    cfg.auto_folders = true;
    let c = ctx(&home, &data, cfg);
    assert!(scan_env(&c, &Env::default()).unwrap().is_empty());
    let mem = remembered_roots(&data);
    assert_eq!(mem.len(), 1, "{mem:?}");
    assert!(mem[0].0.starts_with("auto-"));
    assert_eq!(mem[0].1, std::fs::canonicalize(home.join("ROMs")).unwrap());
    // Still there and empty next time: no "drive not connected" error.
    assert!(scan_env(&c, &Env::default()).is_ok());
}

#[test]
fn exclusions_match_like_the_import_dialog() {
    use super::walk::excluded_by;
    let p = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert!(excluded_by(&p(&["*(Beta)*"]), "ps2/Game (Beta).iso"));
    assert!(excluded_by(&p(&["*(beta)*"]), "ps2/Game (Beta).iso"), "case doesn't matter");
    assert!(excluded_by(&p(&["ps2/Game (Beta).iso"]), "ps2/Game (Beta).iso"));
    assert!(excluded_by(&p(&["ps2/Unfinished/*"]), "ps2/Unfinished/x.iso"));
    assert!(!excluded_by(&p(&["ps2/Unfinished/*"]), "gba/Unfinished/x.gba"));
    assert!(excluded_by(&p(&["Unfinished"]), "ps2/Unfinished"), "a bare name hides that folder anywhere");
    assert!(!excluded_by(&p(&["Game?.iso"]), "ps2/GameX.iso"), "? is a plain character, like exclude::glob_match");
    assert!(!excluded_by(&p(&["*", "**", ""]), "ps2/a.iso"), "never everything");
}

/// The layout on the user's machine (0.7.0 listed every PS2 game twice): a drive folder with
/// console sub-folders, and PCSX2's own game-folder setting pointing at its `ps2` sub-folder
/// through the Flatpak document portal (`/run/user/1000/doc/<id>/ps2`: same files, other inodes).
/// Builds `tmp/mnt/space1/games/{ps2,gba}` and a portal stand-in holding copies of the PS2 files
/// (marked with the portal's attribute when the file system allows it).
fn portal_layout(tmp: &Path, mark: bool) -> (PathBuf, PathBuf, PathBuf) {
    let home = tmp.join("home");
    let games = tmp.join("mnt/space1/games");
    let portal = tmp.join("run/user/1000/doc/abR1xpfzCH0V/ps2");
    for (n, serial) in [("Homebrew Racer (USA).iso", "SLUS_990.01"), ("Homebrew Kart (USA).iso", "SLUS_990.02")] {
        write_file(&games.join("ps2").join(n), ps2_iso(serial));
        write_file(&portal.join(n), ps2_iso(serial));
    }
    write_file(&games.join("gba/Pocket Thing (USA).gba"), gba("ZZQ1", 3));
    write_file(
        &home.join(".var/app/net.pcsx2.PCSX2/config/PCSX2/inis/PCSX2.ini"),
        format!("[GameList]\nRecursivePaths = {}\n", portal.display()),
    );
    if mark {
        crate::emulation::paths::fake_portal(&portal, &std::fs::canonicalize(games.join("ps2")).unwrap());
    }
    (home, games, portal)
}

#[test]
fn a_folder_seen_through_the_document_portal_is_scanned_once() {
    for mark in [true, false] {
        let tmp = tempfile::tempdir().unwrap();
        let (home, games, _portal) = portal_layout(tmp.path(), mark);
        let mnt = tmp.path().join("mnt");
        let mut cfg = EmulationConfig::detect_nothing();
        cfg.auto_folders = true;
        let c = ctx(&home, &tmp.path().join("data"), cfg);
        let env = Env { system: vec![mnt], ..Default::default() };
        let found = roots::collect(&c, &env);
        let outer = roots::auto_id(&std::fs::canonicalize(&games).unwrap());
        let games_found = scan_env(&c, &env).unwrap();
        let ids: Vec<&str> = games_found.iter().map(|g| g.source_id.as_str()).collect();
        assert_eq!(games_found.len(), 3, "mark={mark} {ids:?} roots {found:?}");
        assert!(games_found.iter().all(|g| g.source_id.starts_with(&format!("{outer}/"))), "{ids:?}");
        let mut names: Vec<&str> = games_found.iter().map(|g| g.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["Homebrew Kart", "Homebrew Racer", "Pocket Thing"], "no region added: {ids:?}");
        let roms: HashSet<&Path> = games_found.iter().map(|g| g.emu.as_ref().unwrap().rom.as_path()).collect();
        assert_eq!(roms.len(), 3);
    }
}

#[test]
fn a_rescan_removes_the_second_copies_an_older_scan_listed() {
    let tmp = tempfile::tempdir().unwrap();
    let (home, games, portal) = portal_layout(tmp.path(), true);
    let mnt = tmp.path().join("mnt");
    let mut cfg = EmulationConfig::detect_nothing();
    cfg.auto_folders = true;
    let c = ctx(&home, &tmp.path().join("data"), cfg);
    let env = Env { system: vec![mnt], ..Default::default() };
    let now = scan_env(&c, &env).unwrap();
    // What 0.7.0 stored: every PS2 game twice (the portal folder as its own root), both named
    // with their region.
    let old_id = roots::auto_id(&portal);
    let mut old: Vec<ScannedGame> = vec![];
    for g in &now {
        let mut g = g.clone();
        if g.platform.as_deref() == Some("Sony PlayStation 2") {
            g.name = format!("{} (USA)", g.name);
            let mut d = g.clone();
            let file = g.emu.as_ref().unwrap().rom.file_name().unwrap().to_owned();
            d.source_id = format!("{old_id}/{}", file.to_string_lossy());
            let e = d.emu.as_mut().unwrap();
            e.folder = old_id.clone();
            e.rom = portal.join(&file);
            old.push(d);
        }
        old.push(g);
    }
    let mut db = crate::db::Db::open(&tmp.path().join("library.db")).unwrap();
    db.merge_scan(&old, &[Source::Emulator]).unwrap();
    let all = db.load_all().unwrap();
    assert_eq!(all.len(), 5);
    // Online art fetched for every row; the user played and starred one of the extra copies.
    for g in &all {
        db.set_fetched(g.id, &Art::default(), &Default::default()).unwrap();
    }
    let dup = all.iter().find(|g| g.source_id == format!("{old_id}/Homebrew Racer (USA).iso")).unwrap();
    db.set_favorite(dup.id, true).unwrap();
    db.add_playtime(dup.id, 600).unwrap();
    db.set_tags(dup.id, &["couch".to_string()]).unwrap();
    // The fixed scan: the extra rows go, names are clean, the user's data is on the row that stays.
    db.merge_scan(&now, &[Source::Emulator]).unwrap();
    let all = db.load_all().unwrap();
    let ids: Vec<&str> = all.iter().map(|g| g.source_id.as_str()).collect();
    assert_eq!(all.len(), 3, "{ids:?}");
    assert!(!ids.iter().any(|i| i.starts_with(&old_id)), "{ids:?}");
    let outer = roots::auto_id(&std::fs::canonicalize(&games).unwrap());
    let racer = all.iter().find(|g| g.source_id == format!("{outer}/ps2/Homebrew Racer (USA).iso")).unwrap();
    assert_eq!(racer.name, "Homebrew Racer");
    assert!(racer.favorite);
    assert_eq!(racer.playtime_secs, 600);
    assert_eq!(racer.tags, ["couch"]);
    assert!(racer.installed);
    assert!(all.iter().all(|g| !g.name.ends_with("(USA)")), "{:?}", all.iter().map(|g| &g.name).collect::<Vec<_>>());
    // And the roots file forgets the portal folder without "is the drive connected?".
    assert!(scan_env(&c, &env).is_ok());
}

#[test]
fn a_folder_added_inside_a_found_one_keeps_its_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let (home, games, _) = portal_layout(tmp.path(), false);
    std::fs::remove_file(home.join(".var/app/net.pcsx2.PCSX2/config/PCSX2/inis/PCSX2.ini")).unwrap();
    let mut inner = folder(&games.join("ps2"));
    inner.emulator = "pcsx2".into();
    let mut cfg = cfg_with(&[("mine", inner)]);
    cfg.auto_folders = true;
    let c = ctx(&home, &tmp.path().join("data"), cfg);
    let env = Env { system: vec![tmp.path().join("mnt")], ..Default::default() };
    let found = roots::collect(&c, &env);
    assert_eq!(found.len(), 2, "{found:?}");
    let games_found = scan_env(&c, &env).unwrap();
    let ids: Vec<&str> = games_found.iter().map(|g| g.source_id.as_str()).collect();
    assert_eq!(games_found.len(), 3, "{ids:?}");
    assert_eq!(ids.iter().filter(|i| i.starts_with("mine/")).count(), 2, "{ids:?}");
}

#[test]
fn rom_folders_anywhere_in_home_are_found() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    // ~/Documents/Retro/{PS2 Games, GBA ROMs}: found.
    write_file(&home.join("Documents/Retro/PS2 Games/Racer (USA).iso"), ps2_iso("SLUS_990.01"));
    write_file(&home.join("Documents/Retro/GBA ROMs/Pocket (USA).gba"), gba("ZZQ2", 1));
    // A code project with emulator cores named like consoles: never.
    write_file(&home.join("Documents/programs/emu/Cargo.toml"), b"[package]");
    write_file(&home.join("Documents/programs/emu/gba/core.rs"), b"fn main() {}");
    write_file(&home.join("Documents/programs/emu/nes/core.rs"), b"fn main() {}");
    // Console-named folders without any games: never.
    std::fs::create_dir_all(home.join("Notes/ps2")).unwrap();
    std::fs::create_dir_all(home.join("Notes/gba")).unwrap();
    write_file(&home.join("Notes/gba/todo.txt"), b"x");
    // Inside a Wine prefix / Steam library / hidden folder: never.
    write_file(&home.join("Games/Heroic/Prefixes/x/ps2/a.iso"), ps2_iso("SLUS_990.03"));
    write_file(&home.join("Games/Heroic/Prefixes/x/gba/a.gba"), gba("ZZQ3", 1));
    write_file(&home.join(".local/share/x/ps2/a.iso"), ps2_iso("SLUS_990.04"));
    write_file(&home.join(".local/share/x/gba/a.gba"), gba("ZZQ4", 1));
    // Too deep (4 levels): never.
    write_file(&home.join("a/b/c/d/ps2/a.iso"), ps2_iso("SLUS_990.05"));
    write_file(&home.join("a/b/c/d/gba/a.gba"), gba("ZZQ5", 1));
    let found = roots::home_roots(&home, &[]);
    let paths: Vec<&Path> = found.iter().map(|(p, _)| p.as_path()).collect();
    assert_eq!(paths, [home.join("Documents/Retro").as_path()], "{found:?}");
    assert_eq!(found[0].1, "Found in ~/Documents/Retro");
    let mut cfg = EmulationConfig::detect_nothing();
    cfg.auto_folders = true;
    let c = ctx(&home, &tmp.path().join("data"), cfg);
    let mut names: Vec<String> = scan_env(&c, &Env::default()).unwrap().into_iter().map(|g| g.name).collect();
    names.sort();
    assert_eq!(names, ["Pocket", "Racer"]);
}

/// A home folder that is itself a git checkout (dotfiles) or holds a Makefile is still searched
/// (the marker only stops the search in sub-folders).
#[test]
fn a_home_with_a_dotfiles_git_is_still_searched() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(home.join(".git")).unwrap();
    write_file(&home.join("Makefile"), b"all:");
    for i in 0..20 {
        std::fs::create_dir_all(home.join(format!("folder{i:02}"))).unwrap();
    }
    write_file(&home.join("Retro/ps2/Racer (USA).iso"), ps2_iso("SLUS_990.01"));
    write_file(&home.join("Retro/gba/Pocket (USA).gba"), gba("ZZQ2", 1));
    let found = roots::home_roots(&home, &[]);
    let paths: Vec<&Path> = found.iter().map(|(p, _)| p.as_path()).collect();
    assert_eq!(paths, [home.join("Retro").as_path()], "{found:?}");
}

/// What the home-folder search finds on this machine (read-only; prints).
/// `cargo test --release real_home_rom_search -- --ignored --nocapture`
#[test]
#[ignore]
fn real_home_rom_search() {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
    let t0 = std::time::Instant::now();
    let found = roots::home_roots(&home, &[]);
    println!("home ROM folder search of {}: {} found in {:?}", home.display(), found.len(), t0.elapsed());
    for (p, how) in &found {
        println!("  {} ({how})", p.display());
    }
}

#[test]
fn ps4_folders_are_one_game_named_from_param_sfo() {
    let tmp = tempfile::tempdir().unwrap();
    let ps4 = tmp.path().join("roms/ps4");
    let game = ps4.join("CUSA00900");
    write_file(&game.join("eboot.bin"), b"\x7fELF");
    write_file(
        &game.join("sce_sys/param.sfo"),
        sfo(&[("TITLE", "Bloodborne"), ("TITLE_ID", "CUSA00900"), ("CATEGORY", "gd")]),
    );
    write_file(&game.join("sce_sys/icon0.png"), b"png");
    write_file(&game.join("sce_sys/pic1.png"), b"png");
    // A system module and other launch files inside it: never games of their own.
    write_file(&game.join("sce_module/libc.prx"), b"x");
    write_file(&game.join("sce_module/libSceFios2.prx"), b"x");
    write_file(&game.join("data/boot.bin"), b"x");
    // Its update next to it.
    let upd = ps4.join("CUSA00900-UPDATE");
    write_file(&upd.join("eboot.bin"), b"\x7fELF");
    write_file(
        &upd.join("sce_sys/param.sfo"),
        sfo(&[("TITLE", "Bloodborne"), ("TITLE_ID", "CUSA00900"), ("CATEGORY", "gp")]),
    );
    // A folder named by hand, holding a game without sce_sys: named after the folder, not "eboot".
    write_file(&ps4.join("Gravity Rush 2/eboot.bin"), b"\x7fELF");
    let c = ctx(tmp.path(), &tmp.path().join("data"), EmulationConfig::detect_nothing());
    let games = preview_folder(&c, &folder(&ps4)).unwrap();
    let names: Vec<&str> = games.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names.len(), 2, "{names:?}");
    let bb = find(&games, "Bloodborne");
    let e = bb.emu.as_ref().unwrap();
    assert_eq!(e.platform, "sony_playstation4");
    assert_eq!(e.rom, game);
    assert_eq!(e.serial.as_deref(), Some("CUSA00900"));
    assert_eq!(e.files, vec![upd.clone()], "the update is listed with its game, not as a game");
    assert_eq!(bb.art.icon.as_deref(), Some(game.join("sce_sys/icon0.png").as_path()));
    assert_eq!(bb.art.hero.as_deref(), Some(game.join("sce_sys/pic1.png").as_path()));
    assert!(names.contains(&"Gravity Rush 2"), "{names:?}");
    assert!(!names.iter().any(|n| n.eq_ignore_ascii_case("eboot") || n.eq_ignore_ascii_case("libc")), "{names:?}");
}

#[test]
fn ps4_games_with_their_update_merged_in_are_games() {
    // A dump with the update merged into the game folder, so sce_sys/param.sfo says
    // CATEGORY "gp" (patch), next to changeinfo/ and origin-deltainfo.dat. No separate base game.
    let tmp = tempfile::tempdir().unwrap();
    let ps4 = tmp.path().join("Games/PS4");
    let game = ps4.join("DRIVECLUB");
    write_file(&game.join("eboot.bin"), b"\x7fELF");
    write_file(&game.join("game000.dat"), b"data");
    write_file(&game.join("libfmod.prx"), b"x");
    write_file(&game.join("sce_module/libc.prx"), b"x");
    write_file(
        &game.join("sce_sys/param.sfo"),
        sfo(&[("TITLE", "DRIVECLUB"), ("TITLE_ID", "CUSA00003"), ("CATEGORY", "gp")]),
    );
    write_file(&game.join("sce_sys/origin-deltainfo.dat"), b"x");
    write_file(&game.join("sce_sys/changeinfo/changeinfo.xml"), b"<x/>");
    write_file(&game.join("sce_sys/icon0.png"), b"png");
    // Lone patch folders without their game stay hidden, whatever way the name says "update".
    for (i, dir) in
        ["CUSA09999-patch", "CUSA03173 Update", "CUSA03174-UPDATE-1.09", "Other Game [Patch]"].iter().enumerate()
    {
        let patch = ps4.join(dir);
        write_file(&patch.join("eboot.bin"), b"\x7fELF");
        let id = format!("CUSA0999{i}");
        write_file(
            &patch.join("sce_sys/param.sfo"),
            sfo(&[("TITLE", "Other"), ("TITLE_ID", id.as_str()), ("CATEGORY", "gp")]),
        );
    }
    let c = ctx(tmp.path(), &tmp.path().join("data"), EmulationConfig::detect_nothing());
    let games = preview_folder(&c, &folder(&ps4)).unwrap();
    let names: Vec<&str> = games.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["DRIVECLUB"], "the merged game is listed, the lone patch isn't");
    let e = find(&games, "DRIVECLUB").emu.as_ref().unwrap();
    assert_eq!((e.platform.as_str(), e.rom.as_path()), ("sony_playstation4", game.as_path()));
}

#[test]
fn prx_files_are_not_psp_games() {
    let tmp = tempfile::tempdir().unwrap();
    let roms = tmp.path().join("roms");
    // A PS4 game copied into a PSP folder by mistake, and a stray PSP plugin.
    write_file(&roms.join("psp/Stuff/sce_module/libc.prx"), b"x");
    write_file(&roms.join("psp/plugin.prx"), b"x");
    write_file(&roms.join("misc/thing.prx"), b"x");
    let c = ctx(tmp.path(), &tmp.path().join("data"), EmulationConfig::detect_nothing());
    let games = preview_folder(&c, &folder(&roms)).unwrap();
    assert!(games.is_empty(), "{:#?}", games.iter().map(|g| &g.name).collect::<Vec<_>>());
}

#[test]
fn generic_launch_file_names_take_the_folder_name() {
    assert!(super::walk::generic_stem("EBOOT"));
    assert!(super::walk::generic_stem("default"));
    assert!(!super::walk::generic_stem("Bloodborne"));
    let tmp = tempfile::tempdir().unwrap();
    let xbox = tmp.path().join("roms/xbox");
    write_file(&xbox.join("Halo 2/game.iso"), b"x");
    let c = ctx(tmp.path(), &tmp.path().join("data"), EmulationConfig::detect_nothing());
    let games = preview_folder(&c, &folder(&tmp.path().join("roms"))).unwrap();
    assert_eq!(games.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), vec!["Halo 2"]);
}

#[test]
fn disc_sets_group_across_free_form_tags_but_not_across_releases() {
    let tmp = tempfile::tempdir().unwrap();
    let roms = tmp.path().join("ps1");
    for f in [
        // A tester's case: each disc names its mode.
        "Gran Turismo 2 (Gran Turismo Mode) (Disc 1).chd",
        "Gran Turismo 2 (Arcade Mode) (Disc 2).chd",
        // Two releases: one set per region.
        "Final Fantasy VII (USA) (Disc 1).chd",
        "Final Fantasy VII (USA) (Disc 2).chd",
        "Final Fantasy VII (Europe) (Disc 1).chd",
        "Final Fantasy VII (Europe) (Disc 2).chd",
        // The same disc number twice: only discs with all the same tags group.
        "Saga (Mode A) (Disc 1).chd",
        "Saga (Mode B) (Disc 1).chd",
        "Saga (Mode A) (Disc 2).chd",
    ] {
        write_file(&roms.join(f), b"MComprHD");
    }
    let c = ctx(tmp.path(), &tmp.path().join("data"), EmulationConfig::detect_nothing());
    let games = preview_folder(&c, &folder(&roms)).unwrap();
    let names: Vec<(String, usize)> = {
        let mut v: Vec<(String, usize)> =
            games.iter().map(|g| (g.name.clone(), g.emu.as_ref().map_or(0, |e| e.files.len()))).collect();
        v.sort();
        v
    };
    let gt: Vec<&ScannedGame> = games.iter().filter(|g| g.name == "Gran Turismo 2").collect();
    assert_eq!(gt.len(), 1, "{names:?}");
    let e = gt[0].emu.as_ref().unwrap();
    assert!(e.rom.to_string_lossy().contains("(Disc 1)"), "disc 1 starts it");
    assert_eq!(e.files.len(), 1, "disc 2 is part of it");
    let ff: Vec<&ScannedGame> = games.iter().filter(|g| g.name.starts_with("Final Fantasy VII")).collect();
    assert_eq!(ff.len(), 2, "one per region: {names:?}");
    assert!(ff.iter().all(|g| g.emu.as_ref().unwrap().files.len() == 1));
    let saga: Vec<&ScannedGame> = games.iter().filter(|g| g.name == "Saga").collect();
    assert_eq!(saga.len(), 2, "{names:?}");
    assert_eq!(saga.iter().map(|g| g.emu.as_ref().unwrap().files.len()).sum::<usize>(), 1, "Mode A discs 1 + 2 only");
}
