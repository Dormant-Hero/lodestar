//! Which RetroAchievements console a Lodestar platform is, and the ROM hash RetroAchievements
//! knows a game by — computed like rcheevos (`rc_hash_rom.c`) does for cartridge consoles:
//!
//! * most consoles: MD5 of the whole file (at most the first 64 MiB);
//! * NES / FDS: without the 16-byte header after `NES\x1A` / `FDS\x1A`;
//! * SNES: without a 512-byte copier header (size % 8192 == 512);
//! * PC Engine / SuperGrafx: without 512 bytes when `size & 512`;
//! * Lynx: without the 64-byte `LYNX\0` header; 7800: without the 128-byte `ATARI7800` header;
//! * N64: byte order turned into big-endian `.z64` first (`.v64` swapped in pairs, `.n64` in fours);
//! * Nintendo DS / DSi: header (0x160) + ARM9 code + ARM7 code + 0xA00 bytes of icon / title;
//! * arcade: MD5 of the file name without its extension (some FBNeo folders as a prefix); a
//!   Geolith `.neo` cartridge: its ROM data after the 4096-byte header.
//!
//! A zipped cartridge ROM is hashed from the ROM inside it (what RetroArch does). `.m3u` lists
//! hash their first entry. Disc consoles aren't hashed (yet): they are matched by name.
//! Results are cached per file (path + size + modification time) in `<cache>/ra/rom-hashes.json`.

use crate::emulation::zipread;
use md5::{Digest, Md5};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// rcheevos' `MAX_BUFFER_SIZE`: whole-file hashes stop here.
const MAX: u64 = 64 * 1024 * 1024;

/// How a console's ROMs are hashed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    Whole,
    Nes,
    Snes,
    Pce,
    Lynx,
    A7800,
    N64,
    Nds,
    Arcade,
    /// Not hashed by Lodestar (discs, encrypted images): matched by name only.
    NameOnly,
}

/// A Lodestar platform as RetroAchievements sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Console {
    /// RetroAchievements console ids, the main one first.
    pub ids: &'static [u32],
    pub rule: Rule,
}

/// The RetroAchievements console of a Lodestar platform id (`emulation::platforms`); None:
/// RetroAchievements has no achievements there (or can't tell games apart).
pub fn console(platform: &str) -> Option<Console> {
    use Rule::*;
    let c = |ids: &'static [u32], rule| Some(Console { ids, rule });
    match platform {
        "sega_genesis" => c(&[1], Whole),
        "nintendo_64" => c(&[2], N64),
        "nintendo_super_nes" => c(&[3], Snes),
        "nintendo_gameboy" => c(&[4], Whole),
        "nintendo_gameboyadvance" => c(&[5], Whole),
        "nintendo_gameboycolor" => c(&[6], Whole),
        "nintendo_nes" => c(&[7], Nes),
        // Famicom Disk System games moved to a console of their own; older ones sit with the NES.
        "nintendo_famicom_disk" => c(&[81, 7], Nes),
        "nec_turbografx_16" | "nec_supergrafx" => c(&[8], Pce),
        "sega_cd" => c(&[9], NameOnly),
        "sega_32x" => c(&[10], Whole),
        "sega_mastersystem" => c(&[11], Whole),
        "sony_playstation" => c(&[12], NameOnly),
        "atari_lynx" => c(&[13], Lynx),
        "snk_neogeopocket" | "snk_neogeopocket_color" => c(&[14], Whole),
        "sega_gamegear" => c(&[15], Whole),
        "nintendo_gamecube" => c(&[16], NameOnly),
        "atari_jaguar" => c(&[17], Whole),
        "nintendo_ds" => c(&[18], Nds),
        "nintendo_wii" => c(&[19], NameOnly),
        "sony_playstation2" => c(&[21], NameOnly),
        "magnavox_odyssey_2" => c(&[23], Whole),
        "atari_2600" => c(&[25], Whole),
        "pc_dos" => c(&[26], NameOnly),
        // Neo Geo carts run through FBNeo / MAME: RetroAchievements lists them under Arcade.
        "arcade" | "snk_neogeo_aes" => c(&[27], Arcade),
        "nintendo_virtualboy" => c(&[28], Whole),
        "microsoft_msx" | "microsoft_msx2" => c(&[29], Whole),
        "commodore_64" => c(&[30], Whole),
        "sega_sg1000" => c(&[33], Whole),
        "amstrad_cpc" => c(&[37], Whole),
        "apple_2" => c(&[38], Whole),
        "sega_saturn" => c(&[39], NameOnly),
        "sega_dreamcast" => c(&[40], NameOnly),
        "sony_psp" => c(&[41], NameOnly),
        "3do" => c(&[43], NameOnly),
        "coleco_vision" => c(&[44], Whole),
        "mattel_intellivision" => c(&[45], Whole),
        "vectrex" => c(&[46], Whole),
        "nec_pcfx" => c(&[49], NameOnly),
        "atari_5200" => c(&[50], Whole),
        "atari_7800" => c(&[51], A7800),
        "bandai_wonderswan" | "bandai_wonderswan_color" => c(&[53], Whole),
        "snk_neogeo_cd" => c(&[56], NameOnly),
        "sinclair_zxspectrum" => c(&[59], Whole),
        "nintendo_3ds" => c(&[62], NameOnly),
        "watara_supervision" => c(&[63], Whole),
        "tic_80" => c(&[65], Whole),
        "nec_turbografx_cd" => c(&[76], NameOnly),
        "nintendo_dsi" => c(&[78], Nds),
        "ti_83" => c(&[79], Whole),
        "uzebox" => c(&[80], Whole),
        "sony_playstation3" => c(&[82], NameOnly),
        _ => None,
    }
}

fn hex(d: impl AsRef<[u8]>) -> String {
    d.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn md5(b: &[u8]) -> String {
    hex(Md5::digest(&b[..b.len().min(MAX as usize)]))
}

/// The hash of a ROM already in memory (a file's first 64 MiB, or a zip entry). `name` is the
/// file's path (arcade hashes its name). None: not a ROM of this kind (N64 with an unknown first
/// byte, a DS header that makes no sense) or a rule that doesn't hash.
pub fn hash_bytes(rule: Rule, data: &[u8], name: &Path) -> Option<String> {
    let skip = |n: usize| Some(md5(&data[n.min(data.len())..]));
    match rule {
        Rule::Whole => Some(md5(data)),
        Rule::Nes if data.len() > 16 && (data.starts_with(b"NES\x1a") || data.starts_with(b"FDS\x1a")) => skip(16),
        Rule::Nes => Some(md5(data)),
        Rule::Snes if data.len() % 0x2000 == 512 => skip(512),
        Rule::Snes => Some(md5(data)),
        Rule::Pce if data.len() & 512 != 0 => skip(512),
        Rule::Pce => Some(md5(data)),
        Rule::Lynx if data.len() > 64 && data.starts_with(b"LYNX\0") => skip(64),
        Rule::Lynx => Some(md5(data)),
        Rule::A7800 if data.len() > 128 && data.get(1..10) == Some(b"ATARI7800") => skip(128),
        Rule::A7800 => Some(md5(data)),
        Rule::N64 => n64(data),
        Rule::Nds => nds(&mut std::io::Cursor::new(data)),
        Rule::Arcade => Some(arcade(name)),
        Rule::NameOnly => None,
    }
}

/// N64: to big-endian (z64) by the first byte, then MD5.
fn n64(data: &[u8]) -> Option<String> {
    let data = &data[..data.len().min(MAX as usize)];
    let mut v = data.to_vec();
    match data.first()? {
        0x80 | 0xE8 | 0x22 => {}
        // v64: 16-bit words swapped (rcheevos swaps whole 32-bit words only).
        0x37 => v.as_chunks_mut::<4>().0.iter_mut().for_each(|c| {
            c.swap(0, 1);
            c.swap(2, 3);
        }),
        // n64: little-endian 32-bit words.
        0x40 => v.as_chunks_mut::<4>().0.iter_mut().for_each(|c| c.reverse()),
        _ => return None,
    }
    Some(hex(Md5::digest(&v)))
}

fn le32(b: &[u8], i: usize) -> u64 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as u64
}

/// Read up to `n` bytes at `at` (zero-filled past the end).
fn read_at(f: &mut (impl Read + Seek), at: u64, n: usize) -> Option<Vec<u8>> {
    f.seek(SeekFrom::Start(at)).ok()?;
    let mut buf = vec![0u8; n];
    let mut got = 0;
    while got < n {
        match f.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(k) => got += k,
            Err(_) => return None,
        }
    }
    Some(buf)
}

/// Nintendo DS: header + ARM9 + ARM7 + icon / title block.
fn nds(f: &mut (impl Read + Seek)) -> Option<String> {
    let mut off = 0u64;
    let mut h = read_at(f, 0, 512)?;
    if h[0] == 0x2E && h[1] == 0 && h[2] == 0 && h[3] == 0xEA && h[0xB0..0xB4] == [0x44, 0x46, 0x96, 0] {
        // SuperCard header.
        off = 512;
        h = read_at(f, off, 512)?;
    }
    let (arm9_at, arm9_len) = (le32(&h, 0x20), le32(&h, 0x2C));
    let (arm7_at, arm7_len) = (le32(&h, 0x30), le32(&h, 0x3C));
    let icon_at = le32(&h, 0x68);
    if arm9_len + arm7_len > 16 * 1024 * 1024 {
        return None;
    }
    let mut m = Md5::new();
    m.update(&h[..0x160]);
    m.update(read_at(f, arm9_at + off, arm9_len as usize)?);
    m.update(read_at(f, arm7_at + off, arm7_len as usize)?);
    m.update(read_at(f, icon_at + off, 0xA00)?);
    Some(hex(m.finalize()))
}

/// A Geolith `.neo` Neo Geo cartridge: the ROM data after its 4096-byte header (whose text
/// fields differ between conversion tools), at most 64 MiB (rcheevos `rc_hash_neogeo_cart`).
fn neo_cart(p: &Path) -> Option<String> {
    let mut f = std::fs::File::open(p).ok()?;
    let mut head = [0u8; 4];
    f.read_exact(&mut head).ok()?;
    if &head != b"NEO\x01" || f.metadata().ok()?.len() <= 4096 {
        return None;
    }
    f.seek(SeekFrom::Start(4096)).ok()?;
    let mut data = vec![];
    f.take(MAX).read_to_end(&mut data).ok()?;
    Some(hex(Md5::digest(&data)))
}

/// FBNeo folder names that become part of an arcade hash (`nes/smb.zip` → "nes_smb").
const ARCADE_FOLDERS: &[&str] = &[
    "nes",
    "fds",
    "sms",
    "msx",
    "ngp",
    "pce",
    "chf",
    "sgx",
    "tg16",
    "msx1",
    "neocd",
    "coleco",
    "sg1000",
    "genesis",
    "gamegear",
    "megadriv",
    "pcengine",
    "channelf",
    "spectrum",
    "megadrive",
    "supergrafx",
    "zxspectrum",
    "mastersystem",
    "colecovision",
];

fn arcade(path: &Path) -> String {
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = match file.rfind('.') {
        Some(i) if i > 0 => &file[..i],
        _ => file.as_str(),
    };
    let folder = path.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().to_lowercase());
    let name = match folder {
        // rcheevos builds the prefixed name in a 128-byte buffer: longer ones go without it.
        Some(f) if f.len() < 16 && ARCADE_FOLDERS.contains(&f.as_str()) && f.len() + stem.len() + 1 < 128 => {
            format!("{f}_{stem}")
        }
        _ => stem.to_string(),
    };
    hex(Md5::digest(name.as_bytes()))
}

/// The ROM in a zip that a frontend loads: the first entry with one of the console's ROM
/// extensions, else the only entry.
fn zip_rom(zip: &Path, platform: &str) -> Option<zipread::Entry> {
    let entries = zipread::list(zip)?;
    let exts = crate::emulation::platforms::by_id(platform).map(|p| p.extensions).unwrap_or(&[]);
    let rom_ext = |e: &zipread::Entry| {
        let n = e.base_name().to_ascii_lowercase();
        n.rsplit_once('.').is_some_and(|(_, x)| x != "zip" && x != "7z" && exts.contains(&x))
    };
    entries.iter().find(|e| rom_ext(e)).cloned().or_else(|| (entries.len() == 1).then(|| entries[0].clone()))
}

/// The first file an `.m3u` names.
fn m3u_first(p: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(p).ok()?;
    let line = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#'))?;
    let f = PathBuf::from(line);
    Some(if f.is_absolute() { f } else { p.parent()?.join(f) })
}

/// Hash a ROM file (or `<zip>/<entry>`) for `platform`, without the cache.
pub fn hash_file(platform: &str, rom: &Path) -> Option<String> {
    let rule = console(platform)?.rule;
    if rule == Rule::NameOnly {
        return None;
    }
    if rule == Rule::Arcade {
        if rom.extension().is_some_and(|e| e.eq_ignore_ascii_case("neo")) {
            return neo_cart(rom);
        }
        return Some(arcade(rom));
    }
    let lower = rom.to_string_lossy().to_ascii_lowercase();
    if lower.ends_with(".m3u") {
        return hash_file(platform, &m3u_first(rom)?);
    }
    // A zip: the ROM inside (also a `<zip>/<entry>` path).
    let zipped = if let Some((zip, name)) = zipread::split(rom) {
        Some((zip.clone(), zipread::list(&zip)?.into_iter().find(|e| e.name == name)?))
    } else if lower.ends_with(".zip") {
        Some((rom.to_path_buf(), zip_rom(rom, platform)?))
    } else {
        None
    };
    if let Some((zip, entry)) = zipped {
        // A DS ROM is hashed from parts of it, so it may be bigger than the 64 MiB rule.
        let limit = if rule == Rule::Nds { 4 * MAX } else { MAX + 512 };
        let data = zipread::read(&zip, &entry, limit)?;
        return hash_bytes(rule, &data, Path::new(entry.base_name()));
    }
    if lower.ends_with(".7z") {
        return None;
    }
    let mut f = std::fs::File::open(rom).ok()?;
    if rule == Rule::Nds {
        return nds(&mut f);
    }
    let mut data = vec![];
    f.take(MAX).read_to_end(&mut data).ok()?;
    hash_bytes(rule, &data, rom)
}

/// The cached hash of a ROM: `<dir>/rom-hashes.json`, keyed by platform + path, valid while the
/// file's size and modification time stay the same.
pub fn hash_cached(dir: &Path, platform: &str, rom: &Path) -> Option<String> {
    let mut cache = HashCache::load(dir);
    let h = cache.get(platform, rom);
    cache.save();
    h
}

/// `rom-hashes.json`: (size, modification time, hash) per "platform|path".
type HashMap3 = HashMap<String, (u64, u64, Option<String>)>;

/// One save of `rom-hashes.json` at a time (the game page and the library pass both hash).
static SAVING: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

/// The ROM hash cache in memory: load once, hash many files, save once (the library pass hashes
/// a whole library; reading and writing the file per ROM would be slow).
pub struct HashCache {
    file: PathBuf,
    map: HashMap3,
    /// Entries added since loading (merged into the file on save).
    new: HashMap3,
}

impl HashCache {
    pub fn load(dir: &Path) -> HashCache {
        let file = dir.join("rom-hashes.json");
        let map = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        HashCache { file, map, new: HashMap::new() }
    }

    fn key_and_stamp(platform: &str, rom: &Path) -> Option<(String, (u64, u64))> {
        let real = zipread::real_path(rom);
        let meta = std::fs::metadata(&real).ok()?;
        let mtime =
            meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs());
        Some((format!("{platform}|{}", rom.display()), (meta.len(), mtime.unwrap_or(0))))
    }

    /// The hash when it is cached and the file hasn't changed (Some(None): known to have none);
    /// None: not cached (or the file is gone).
    pub fn cached(&self, platform: &str, rom: &Path) -> Option<Option<String>> {
        let (key, stamp) = Self::key_and_stamp(platform, rom)?;
        let (len, t, h) = self.new.get(&key).or_else(|| self.map.get(&key))?;
        ((*len, *t) == stamp).then(|| h.clone())
    }

    /// The hash, computed (and remembered) when it isn't cached.
    pub fn get(&mut self, platform: &str, rom: &Path) -> Option<String> {
        if let Some(h) = self.cached(platform, rom) {
            return h;
        }
        let (key, stamp) = Self::key_and_stamp(platform, rom)?;
        let h = hash_file(platform, rom);
        self.new.insert(key, (stamp.0, stamp.1, h.clone()));
        h
    }

    /// Hashes computed since the last save.
    pub fn unsaved(&self) -> usize {
        self.new.len()
    }

    /// Write the new hashes into the file (merged with what another thread saved meanwhile);
    /// files that are gone are forgotten, so the cache never outgrows the library.
    pub fn save(&mut self) {
        if self.new.is_empty() {
            return;
        }
        let _one = SAVING.lock();
        let mut on_disk: HashMap3 =
            std::fs::read(&self.file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        on_disk.extend(self.new.drain());
        on_disk.retain(|k, _| k.split_once('|').is_some_and(|(_, p)| zipread::real_path(Path::new(p)).exists()));
        if let Ok(b) = serde_json::to_vec(&on_disk) {
            if let Some(d) = self.file.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = crate::fsutil::write_atomic(&self.file, &b);
        }
        self.map = on_disk;
    }
}
