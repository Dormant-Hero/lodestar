//! What a ROM is: cheap header reads (disc serials, cartridge game codes, title ids), CRC32 of
//! small files or a zip's stored CRC, a cache of both by (path, size, mtime), and the hash
//! database match with the acceptance rule (ROM hacks keep their file names).
//!
//! Every reader takes a few KB at most from the file (the CRC reads files up to 64 MiB); big
//! discs are never hashed.

use super::Role;
use super::names;
use crate::emulation::platforms::Platform;
use crate::emulation::romdb::{self, DatEntry};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

/// Largest file the CRC is computed for.
pub const MAX_CRC: u64 = 64 << 20;

/// What the content of one file says.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Probe {
    /// Disc serial / game code as the database spells it ("SLUS-20946", "BPRE", "GM8E01",
    /// "CTR-P-BNEA", "BLUS30001", Wii U "ALZE01").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crc: Option<u32>,
    /// CRC of the file without its copier / iNES header, or byte-swapped to .z64.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crc_alt: Option<u32>,
    /// Platform id the content proves (N64 magic, GameCube vs Wii disc magic, PS1 vs PS2 boot line).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// Switch / Wii U / 3DS title id (16 hex), PS3 title id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_id: Option<String>,
    /// Update / DLC by content (Wii U TMD, Switch title id, PS3 CATEGORY).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    /// Title the content carries (Wii U meta.xml, PARAM.SFO).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The ROM's file name inside a zip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inner: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Cache

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CacheEntry {
    size: u64,
    mtime: i64,
    probe: Probe,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct CacheFile {
    version: u32,
    files: HashMap<String, CacheEntry>,
}

const CACHE_VERSION: u32 = 1;

/// Probes by (path, size, mtime) in `<data_dir>/emulation/hash-cache.json`.
#[derive(Default)]
pub struct HashCache {
    path: Option<PathBuf>,
    files: HashMap<String, CacheEntry>,
    touched: HashSet<String>,
    dirty: bool,
    pub hits: usize,
    pub probed: usize,
}

impl HashCache {
    /// Load (an unreadable or old cache starts empty). `None`: in memory only.
    pub fn load(path: Option<PathBuf>) -> HashCache {
        let files = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<CacheFile>(&b).ok())
            .filter(|c| c.version == CACHE_VERSION)
            .map(|c| c.files)
            .unwrap_or_default();
        HashCache { path, files, ..Default::default() }
    }

    /// The probe of `path`, from the cache when size and mtime are unchanged.
    pub fn probe(&mut self, path: &Path, size: u64, mtime: i64) -> Probe {
        let key = path.to_string_lossy().into_owned();
        self.touched.insert(key.clone());
        if let Some(e) = self.files.get(&key)
            && e.size == size
            && e.mtime == mtime
        {
            self.hits += 1;
            return e.probe.clone();
        }
        self.probed += 1;
        let probe = probe_file(path, size);
        self.files.insert(key, CacheEntry { size, mtime, probe: probe.clone() });
        self.dirty = true;
        probe
    }

    /// Write the cache: entries seen this scan plus those outside every scanned root (a drive
    /// that is unplugged keeps its entries).
    pub fn save(&mut self, scanned_roots: &[PathBuf]) {
        let Some(path) = self.path.clone() else { return };
        let before = self.files.len();
        let touched = &self.touched;
        self.files.retain(|k, _| touched.contains(k) || !scanned_roots.iter().any(|r| Path::new(k).starts_with(r)));
        if !self.dirty && before == self.files.len() {
            return;
        }
        let file = CacheFile { version: CACHE_VERSION, files: std::mem::take(&mut self.files) };
        if let Ok(json) = serde_json::to_vec(&file) {
            save_cache(&path, &json);
        }
        self.files = file.files;
        self.dirty = false;
    }
}

/// Save a scanner cache file. A cache that can't be written is only slower next time, so the
/// error is logged, never passed on.
pub fn save_cache(path: &Path, bytes: &[u8]) {
    if let Err(e) = crate::fsutil::write_atomic(path, bytes) {
        log::debug!("ROM scan cache {}: {e}", path.display());
    }
}

// ---------------------------------------------------------------------------------------------
// Byte helpers

fn read_at(f: &File, off: u64, len: usize) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; len];
    let mut got = 0;
    while got < len {
        match f.read_at(&mut buf[got..], off + got as u64) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    buf.truncate(got);
    Some(buf)
}

fn u16le(b: &[u8], p: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(p..p + 2)?.try_into().ok()?))
}
fn u32le(b: &[u8], p: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(p..p + 4)?.try_into().ok()?))
}
fn u64le(b: &[u8], p: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(p..p + 8)?.try_into().ok()?))
}
fn u32be(b: &[u8], p: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(p..p + 4)?.try_into().ok()?))
}
fn u64be(b: &[u8], p: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(p..p + 8)?.try_into().ok()?))
}

/// ASCII letters / digits of `b`, NUL padding dropped; None when anything else is there.
fn code(b: &[u8]) -> Option<String> {
    let s: Vec<u8> = b.iter().copied().take_while(|&c| c != 0).collect();
    (!s.is_empty() && s.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-'))
        .then(|| String::from_utf8_lossy(&s).into_owned())
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

// ---------------------------------------------------------------------------------------------
// The probe

const COMPRESSED: &[&str] = &[
    "chd", "cso", "zso", "rvz", "wia", "gcz", "wbfs", "wud", "wux", "wua", "nsp", "xci", "nsz", "xcz", "cue", "m3u",
    "gdi", "ccd", "mds", "toc", "7z", "rar", "ciso", "pbp",
];

/// Read what the file's content says (no cache).
pub fn probe_file(path: &Path, size: u64) -> Probe {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = names::ext(&name);
    let Ok(f) = File::open(path) else { return Probe::default() };
    let mut p = Probe::default();
    let head = read_at(&f, 0, 0x200).unwrap_or_default();
    match ext.as_str() {
        "iso" | "bin" | "img" | "gcm" | "tgc" | "ciso" | "cso" | "mdf" => {
            if let Some((id, plat)) = gc_wii_disc(&f, &head) {
                p.serial = Some(id);
                p.platform = Some(plat.into());
            } else {
                disc_serial(&f, &head, &mut p);
            }
        }
        "rvz" | "wia" => {
            if (head.starts_with(b"RVZ\x01") || head.starts_with(b"WIA\x01"))
                && let Some((id, plat)) = head.get(0x58..0xD8).and_then(gc_wii_header)
            {
                p.serial = Some(id);
                p.platform = Some(plat.into());
            }
        }
        "wbfs" => {
            if head.starts_with(b"WBFS") {
                let shift = *head.get(8).unwrap_or(&9) as u32;
                if (9..=20).contains(&shift)
                    && let Some((id, plat)) = read_at(&f, 1 << shift, 0x80).as_deref().and_then(gc_wii_header)
                {
                    p.serial = Some(id);
                    p.platform = Some(plat.into());
                }
            }
        }
        "gcz" => {
            if let Some((id, plat)) = gcz(&f, &head) {
                p.serial = Some(id);
                p.platform = Some(plat.into());
            }
        }
        "nds" | "dsi" | "ids" => {
            p.serial = head.get(0x0C..0x10).and_then(code).filter(|c| c.len() == 4);
        }
        "gba" | "agb" => {
            if head.get(0xB2) == Some(&0x96) {
                p.serial = head.get(0xAC..0xB0).and_then(code).filter(|c| c.len() == 4);
            }
        }
        "z64" | "n64" | "v64" | "u1" => {
            if let Some(h) = n64_to_z64(&head) {
                p.platform = Some("nintendo_64".into());
                p.serial = h.get(0x3B..0x3F).and_then(code).filter(|c| c.len() == 4);
            }
        }
        "3ds" | "cci" => three_ds(&f, &head, &mut p),
        "cxi" => {
            if head.get(0x100..0x104) == Some(b"NCCH") {
                p.serial = head.get(0x150..0x160).and_then(code);
            }
        }
        "wud" => {
            if head.starts_with(b"WUP-") {
                p.serial = head.get(6..10).and_then(code);
            }
        }
        "wux" => p.serial = wux(&f, &head),
        "nsp" | "nsz" => p.title_id = pfs0_title_id(&f, &head),
        "pbp" => {
            if head.starts_with(b"\0PBP")
                && let Some(off) = u32le(&head, 8)
                && let Some(sfo) = read_at(&f, off as u64, 0x2000)
            {
                let v = sfo_values(&sfo);
                p.serial = v.get("DISC_ID").map(|s| dash_serial(s));
                p.title = v.get("TITLE").cloned();
            }
        }
        "zip" => {
            if let Some((crc, inner)) = zip_crc(&f, size) {
                p.crc = Some(crc);
                p.inner = Some(inner);
            }
        }
        _ => {}
    }
    if p.title_id.is_none()
        && let Some(id) = names::switch_title_id(&name)
    {
        p.title_id = Some(id);
    }
    if matches!(ext.as_str(), "nsp" | "nsz" | "xci" | "xcz")
        && let Some(id) = &p.title_id
        && let Ok(v) = u64::from_str_radix(id, 16)
    {
        p.role = match v & 0xFFF {
            0 => None,
            0x800 => Some(Role::Update),
            _ => Some(Role::Dlc),
        };
    }
    if size <= MAX_CRC && size > 0 && p.crc.is_none() && !COMPRESSED.contains(&ext.as_str()) {
        let (crc, alt) = crc_file(&f, &head, &ext, size);
        p.crc = crc;
        p.crc_alt = alt;
    }
    p
}

/// CRC32 of the whole file, plus the CRC without a copier / iNES header (or byte-swapped to
/// big-endian for .n64).
fn crc_file(f: &File, head: &[u8], ext: &str, size: u64) -> (Option<u32>, Option<u32>) {
    let skip: u64 = if head.starts_with(b"NES\x1A") {
        16 + if head.get(6).is_some_and(|b| b & 4 != 0) { 512 } else { 0 }
    } else if head.starts_with(b"FDS\x1A") {
        16
    } else if head.get(1..10) == Some(b"ATARI7800") {
        128
    } else if head.starts_with(b"LYNX") {
        64
    } else if (matches!(ext, "sfc" | "smc" | "fig" | "swc") && size % 1024 == 512)
        || (ext == "pce" && size % 8192 == 512)
    {
        512
    } else {
        0
    };
    let swap = head.starts_with(&[0x40, 0x12, 0x37, 0x80]);
    let mut full = crc32fast::Hasher::new();
    let mut alt = crc32fast::Hasher::new();
    let mut off = 0u64;
    const CHUNK: usize = 1 << 20;
    while off < size {
        let Some(mut buf) = read_at(f, off, CHUNK) else { return (None, None) };
        if buf.is_empty() {
            break;
        }
        full.update(&buf);
        if skip > 0 && off + buf.len() as u64 > skip {
            let from = skip.saturating_sub(off) as usize;
            alt.update(&buf[from..]);
        } else if swap {
            for w in buf.as_chunks_mut::<4>().0 {
                w.reverse();
            }
            alt.update(&buf);
        }
        off += buf.len() as u64;
    }
    (Some(full.finalize()), (skip > 0 || swap).then(|| alt.finalize()))
}

/// N64 header normalised to big-endian (.z64) order.
fn n64_to_z64(head: &[u8]) -> Option<Vec<u8>> {
    let h = head.get(..0x40)?;
    match h[..4] {
        [0x80, 0x37, 0x12, 0x40] => Some(h.to_vec()),
        [0x37, 0x80, 0x40, 0x12] => Some(h.as_chunks::<2>().0.iter().flat_map(|c| [c[1], c[0]]).collect()),
        [0x40, 0x12, 0x37, 0x80] => Some(h.as_chunks::<4>().0.iter().flat_map(|c| [c[3], c[2], c[1], c[0]]).collect()),
        _ => None,
    }
}

/// GameCube / Wii disc header (0x80 bytes from the disc start): game id and platform.
fn gc_wii_header(h: &[u8]) -> Option<(String, &'static str)> {
    let plat = if u32be(h, 0x18)? == 0x5D1C_9EA3 {
        "nintendo_wii"
    } else if u32be(h, 0x1C)? == 0xC233_9F3D {
        "nintendo_gamecube"
    } else {
        return None;
    };
    let id = code(h.get(..6)?).filter(|c| c.len() == 6)?;
    Some((id, plat))
}

/// Plain ISO / GCM, or Dolphin's CISO.
fn gc_wii_disc(f: &File, head: &[u8]) -> Option<(String, &'static str)> {
    if let Some(r) = gc_wii_header(head) {
        return Some(r);
    }
    // Dolphin / WiiFlow CISO: block size u32 LE at 4 (PSP CSO has the header size 0x18 there).
    if head.starts_with(b"CISO") && u32le(head, 4).is_some_and(|b| b != 0x18) && head.get(8) == Some(&1) {
        return read_at(f, 0x8000, 0x80).as_deref().and_then(gc_wii_header);
    }
    None
}

fn gcz(f: &File, head: &[u8]) -> Option<(String, &'static str)> {
    if u32le(head, 0)? != 0xB10B_C001 {
        return None;
    }
    let block_size = u32le(head, 0x18)? as usize;
    let blocks = u32le(head, 0x1C)? as u64;
    if blocks == 0 || block_size == 0 || block_size > 1 << 24 {
        return None;
    }
    let ptrs = read_at(f, 0x20, 16)?;
    let p0 = u64le(&ptrs, 0)?;
    let data = 0x20 + 12 * blocks;
    let stored = p0 & (1 << 63) != 0;
    let start = p0 & !(1 << 63);
    let end = if blocks > 1 { u64le(&ptrs, 8)? & !(1 << 63) } else { start + block_size as u64 };
    let raw = read_at(f, data + start, (end.saturating_sub(start) as usize).min(block_size + 1024))?;
    let block =
        if stored { raw } else { miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&raw, block_size).ok()? };
    gc_wii_header(&block)
}

fn three_ds(f: &File, head: &[u8], p: &mut Probe) {
    if head.get(0x100..0x104) != Some(b"NCSD") {
        return;
    }
    if let Some(id) = u64le(head, 0x108) {
        p.title_id = Some(format!("{id:016X}"));
    }
    let unit = 0x200u64 << head.get(0x18E).copied().unwrap_or(0).min(8);
    let Some(off) = u32le(head, 0x120).map(|o| o as u64 * unit) else { return };
    if let Some(n) = read_at(f, off + 0x100, 0x60)
        && n.starts_with(b"NCCH")
    {
        p.serial = n.get(0x50..0x60).and_then(code);
    }
}

fn wux(f: &File, head: &[u8]) -> Option<String> {
    if !head.starts_with(&[0x57, 0x55, 0x58, 0x30, 0x2E, 0xD0, 0x99, 0x10]) {
        return None;
    }
    let sector = u32le(head, 8)? as u64;
    let total = u64le(head, 0x10)?;
    if !(0x100..=1 << 24).contains(&sector) {
        return None;
    }
    let count = total.div_ceil(sector);
    let start = (0x20 + 4 * count).div_ceil(sector) * sector;
    let idx0 = u32le(head, 0x20)? as u64;
    let s0 = read_at(f, start + idx0 * sector, 16)?;
    s0.starts_with(b"WUP-").then(|| s0.get(6..10).and_then(code)).flatten()
}

/// Switch NSP (PFS0): the title id from the ticket's file name (rights id = title id + key gen).
fn pfs0_title_id(f: &File, head: &[u8]) -> Option<String> {
    if !head.starts_with(b"PFS0") {
        return None;
    }
    let count = u32le(head, 4)? as u64;
    let strtab = u32le(head, 8)? as usize;
    if count > 4096 || strtab > 1 << 16 {
        return None;
    }
    let names = read_at(f, 0x10 + 0x18 * count, strtab)?;
    names
        .split(|&b| b == 0)
        .filter_map(|n| std::str::from_utf8(n).ok())
        .find(|n| n.to_ascii_lowercase().ends_with(".tik") && n.len() >= 20)
        .map(|n| n[..16].to_ascii_uppercase())
        .filter(|id| id.chars().all(|c| c.is_ascii_hexdigit()))
}

// ---------------------------------------------------------------------------------------------
// ISO9660 (PS1 / PS2 / PSP / PS3 discs)

enum Disc<'a> {
    /// 2048-byte sectors.
    Cooked(&'a File),
    /// 2352-byte raw sectors, user data at this offset.
    Raw(&'a File, u64),
    /// PSP CSO (deflate blocks).
    Cso { f: &'a File, block: u64, align: u32 },
}

impl Disc<'_> {
    fn open<'a>(f: &'a File, head: &[u8]) -> Option<Disc<'a>> {
        if head.starts_with(b"CISO") && u32le(head, 4) == Some(0x18) {
            let block = u32le(head, 0x10)? as u64;
            if block == 0 || block > 1 << 20 || !block.is_multiple_of(2048) {
                return None;
            }
            return Some(Disc::Cso { f, block, align: *head.get(0x15)? as u32 });
        }
        if head.len() >= 16 && head[0] == 0 && head[1..11].iter().all(|&b| b == 0xFF) && head[11] == 0 {
            return Some(Disc::Raw(f, if head[15] == 2 { 24 } else { 16 }));
        }
        Some(Disc::Cooked(f))
    }

    fn sector(&self, lba: u32) -> Option<Vec<u8>> {
        let s = match self {
            Disc::Cooked(f) => read_at(f, lba as u64 * 2048, 2048)?,
            Disc::Raw(f, user) => read_at(f, lba as u64 * 2352 + user, 2048)?,
            Disc::Cso { f, block, align } => {
                let byte = lba as u64 * 2048;
                let b = byte / block;
                let idx = read_at(f, 0x18 + 4 * b, 8)?;
                let (i0, i1) = (u32le(&idx, 0)?, u32le(&idx, 4)?);
                let pos = ((i0 & 0x7FFF_FFFF) as u64) << align;
                let end = ((i1 & 0x7FFF_FFFF) as u64) << align;
                let within = (byte % block) as usize;
                let data = if i0 & 0x8000_0000 != 0 {
                    read_at(f, pos, *block as usize)?
                } else {
                    let raw = read_at(f, pos, (end.saturating_sub(pos) as usize).min(*block as usize + 64))?;
                    miniz_oxide::inflate::decompress_to_vec_with_limit(&raw, *block as usize).ok()?
                };
                data.get(within..within + 2048)?.to_vec()
            }
        };
        (s.len() == 2048).then_some(s)
    }

    fn root(&self) -> Option<(u32, u32)> {
        let pvd = self.sector(16)?;
        if pvd[0] != 1 || &pvd[1..6] != b"CD001" {
            return None;
        }
        Some((u32le(&pvd, 156 + 2)?, u32le(&pvd, 156 + 10)?))
    }

    /// (extent, size) of `path` ("PSP_GAME/PARAM.SFO").
    fn find(&self, path: &str) -> Option<(u32, u32)> {
        let (mut lba, mut size) = self.root()?;
        for part in path.split('/') {
            (lba, size) = self.find_in(lba, size, part)?;
        }
        Some((lba, size))
    }

    fn find_in(&self, lba: u32, size: u32, name: &str) -> Option<(u32, u32)> {
        let sectors = size.div_ceil(2048).min(32);
        for s in 0..sectors {
            let buf = self.sector(lba + s)?;
            let mut pos = 0usize;
            while pos + 34 <= 2048 {
                let len = buf[pos] as usize;
                if len == 0 {
                    break;
                }
                let nl = buf[pos + 32] as usize;
                if let Some(raw) = buf.get(pos + 33..pos + 33 + nl) {
                    let n = String::from_utf8_lossy(raw);
                    let n = n.split(';').next().unwrap_or("");
                    if n.eq_ignore_ascii_case(name) {
                        return Some((u32le(&buf, pos + 2)?, u32le(&buf, pos + 10)?));
                    }
                }
                pos += len;
            }
        }
        None
    }

    fn read(&self, lba: u32, size: u32, max: usize) -> Vec<u8> {
        let mut out = vec![];
        let want = (size as usize).min(max);
        let mut s = 0;
        while out.len() < want {
            let Some(sec) = self.sector(lba + s) else { break };
            out.extend_from_slice(&sec);
            s += 1;
        }
        out.truncate(want);
        out
    }

    fn file(&self, path: &str, max: usize) -> Option<Vec<u8>> {
        let (lba, size) = self.find(path)?;
        Some(self.read(lba, size, max))
    }
}

/// PS1 / PS2 (SYSTEM.CNF), PSP (UMD_DATA.BIN, PARAM.SFO), PS3 (PARAM.SFO) serial of a disc.
fn disc_serial(f: &File, head: &[u8], p: &mut Probe) {
    let Some(disc) = Disc::open(f, head) else { return };
    if disc.root().is_none() {
        return;
    }
    if let Some(cnf) = disc.file("SYSTEM.CNF", 4096) {
        if let Some((serial, ps2)) = system_cnf(&String::from_utf8_lossy(&cnf)) {
            p.serial = Some(serial);
            p.platform = Some(if ps2 { "sony_playstation2" } else { "sony_playstation" }.into());
        }
        return;
    }
    if let Some(umd) = disc.file("UMD_DATA.BIN", 256) {
        let s = String::from_utf8_lossy(&umd);
        if let Some(id) = s.split('|').next().map(str::trim).filter(|s| s.len() >= 9) {
            p.serial = Some(dash_serial(id));
            p.platform = Some("sony_psp".into());
            return;
        }
    }
    if let Some(sfo) = disc.file("PSP_GAME/PARAM.SFO", 0x4000) {
        let v = sfo_values(&sfo);
        p.serial = v.get("DISC_ID").map(|s| dash_serial(s));
        p.title = v.get("TITLE").cloned();
        p.platform = Some("sony_psp".into());
        return;
    }
    if let Some(sfo) = disc.file("PS3_GAME/PARAM.SFO", 0x4000) {
        let v = sfo_values(&sfo);
        p.serial = v.get("TITLE_ID").cloned();
        p.title = v.get("TITLE").cloned();
        p.platform = Some("sony_playstation3".into());
    }
}

/// `BOOT2 = cdrom0:\SLUS_203.12;1` → ("SLUS-20312", PS2); `BOOT = cdrom:\SCUS_944.26;1` → PS1.
pub fn system_cnf(text: &str) -> Option<(String, bool)> {
    for line in text.lines() {
        let l = line.trim_matches(|c: char| c.is_whitespace() || c == '\0');
        let upper = l.to_ascii_uppercase();
        let ps2 = upper.starts_with("BOOT2");
        if !ps2 && !upper.starts_with("BOOT") {
            continue;
        }
        let Some((_, v)) = l.split_once('=') else { continue };
        let file = v.trim().rsplit(['\\', ':', '/']).next().unwrap_or("");
        let file = file.split(';').next().unwrap_or("").trim();
        if let Some(s) = ps_serial(file) {
            return Some((s, ps2));
        }
    }
    None
}

/// `SLUS_203.12` / `SLUS-20312` / `SLUS20312` → `SLUS-20312`.
pub fn ps_serial(file: &str) -> Option<String> {
    let letters: String = file.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let digits: String = file[letters.len()..].chars().filter(|c| c.is_ascii_digit()).collect();
    let rest_ok = file[letters.len()..].chars().all(|c| c.is_ascii_digit() || matches!(c, '_' | '-' | '.'));
    (letters.len() == 4 && digits.len() == 5 && rest_ok).then(|| format!("{}-{digits}", letters.to_ascii_uppercase()))
}

/// `ULUS10041` → `ULUS-10041` (already dashed values unchanged).
pub fn dash_serial(s: &str) -> String {
    let s = s.trim();
    if s.len() == 9 && s[..4].chars().all(|c| c.is_ascii_alphabetic()) && s[4..].chars().all(|c| c.is_ascii_digit()) {
        format!("{}-{}", &s[..4], &s[4..])
    } else {
        s.to_string()
    }
}

/// PARAM.SFO (PSP / PS3) key → value.
pub fn sfo_values(b: &[u8]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if !b.starts_with(b"\0PSF") {
        return out;
    }
    let (Some(keys), Some(data), Some(count)) = (u32le(b, 8), u32le(b, 12), u32le(b, 16)) else { return out };
    for i in 0..count.min(256) as usize {
        let e = 20 + 16 * i;
        let (Some(ko), Some(fmt), Some(len), Some(dof)) =
            (u16le(b, e), u16le(b, e + 2), u32le(b, e + 4), u32le(b, e + 12))
        else {
            break;
        };
        let Some(kb) = b.get(keys as usize + ko as usize..) else { continue };
        let key = cstr(kb);
        let start = data as usize + dof as usize;
        let Some(v) = b.get(start..start + len as usize) else { continue };
        let val = if fmt == 0x0404 { u32le(v, 0).map(|n| n.to_string()).unwrap_or_default() } else { cstr(v) };
        out.insert(key, val);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Folder games (Wii U, PS3)

/// Wii U NUS `title.tmd` / `tmd.<n>`: title id and its role (base / update / DLC).
pub fn wiiu_tmd(path: &Path) -> Option<(String, Option<Role>)> {
    let f = File::open(path).ok()?;
    let b = read_at(&f, 0x18C, 8)?;
    let id = u64be(&b, 0)?;
    let role = match (id >> 32) as u32 {
        0x0005_000E => Some(Role::Update),
        0x0005_000C => Some(Role::Dlc),
        _ => None,
    };
    Some((format!("{id:016X}"), role))
}

/// Wii U `meta/meta.xml`: database key ("ALZE01"), title id, English name.
pub fn wiiu_meta(path: &Path) -> Probe {
    let mut p = Probe::default();
    let Ok(text) = crate::scan::util::read_text(path) else { return p };
    let Ok(doc) = roxmltree::Document::parse(&text) else { return p };
    let get =
        |n: &str| doc.descendants().find(|e| e.has_tag_name(n)).and_then(|e| e.text()).map(|t| t.trim().to_string());
    let product = get("product_code").unwrap_or_default();
    let company = get("company_code").unwrap_or_default();
    if let Some(c4) = product.get(6..10).filter(|c| c.chars().all(|c| c.is_ascii_alphanumeric())) {
        p.serial = Some(match company.get(2..4) {
            Some(cc) if cc.chars().all(|c| c.is_ascii_alphanumeric()) => format!("{c4}{cc}"),
            _ => c4.to_string(),
        });
    }
    p.title_id = get("title_id").map(|t| t.to_ascii_uppercase());
    p.title = get("longname_en").map(|t| t.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|t| !t.is_empty());
    if let Some(id) = p.title_id.as_deref().and_then(|t| u64::from_str_radix(t, 16).ok()) {
        p.role = match (id >> 32) as u32 {
            0x0005_000E => Some(Role::Update),
            0x0005_000C => Some(Role::Dlc),
            _ => None,
        };
    }
    p
}

/// PS3 / PSP `PARAM.SFO` file.
pub fn sfo_file(path: &Path) -> HashMap<String, String> {
    File::open(path).ok().and_then(|f| read_at(&f, 0, 0x4000)).map(|b| sfo_values(&b)).unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// ZIP: the stored CRC from the central directory (no decompression)

const NOT_ROMS: &[&str] =
    &["txt", "nfo", "diz", "md", "pdf", "html", "jpg", "jpeg", "png", "gif", "bmp", "xml", "dat", "url", "ini"];

fn zip_crc(f: &File, size: u64) -> Option<(u32, String)> {
    let tail_len = size.min(22 + 65535) as usize;
    let tail = read_at(f, size - tail_len as u64, tail_len)?;
    let eocd = (0..=tail.len().checked_sub(22)?).rev().find(|&i| tail[i..i + 4] == [0x50, 0x4B, 0x05, 0x06])?;
    let mut cd_size = u32le(&tail, eocd + 12)? as u64;
    let mut cd_off = u32le(&tail, eocd + 16)? as u64;
    if (cd_size == 0xFFFF_FFFF || cd_off == 0xFFFF_FFFF)
        && eocd >= 20
        && tail[eocd - 20..eocd - 16] == [0x50, 0x4B, 0x06, 0x07]
    {
        let e64 = u64le(&tail, eocd - 20 + 8)?;
        let rec = read_at(f, e64, 56)?;
        if rec.get(..4)? != [0x50, 0x4B, 0x06, 0x06] {
            return None;
        }
        cd_size = u64le(&rec, 40)?;
        cd_off = u64le(&rec, 48)?;
    }
    if cd_size > 4 << 20 {
        return None;
    }
    let cd = read_at(f, cd_off, cd_size as usize)?;
    let mut pos = 0;
    while pos + 46 <= cd.len() && cd[pos..pos + 4] == [0x50, 0x4B, 0x01, 0x02] {
        let crc = u32le(&cd, pos + 16)?;
        let nl = u16le(&cd, pos + 28)? as usize;
        let el = u16le(&cd, pos + 30)? as usize;
        let cl = u16le(&cd, pos + 32)? as usize;
        let name = String::from_utf8_lossy(cd.get(pos + 46..pos + 46 + nl)?).into_owned();
        pos += 46 + nl + el + cl;
        let base = name.rsplit('/').next().unwrap_or("").to_string();
        if name.ends_with('/') || base.is_empty() || NOT_ROMS.contains(&names::ext(&base).as_str()) {
            continue;
        }
        return Some((crc, base));
    }
    None
}

// ---------------------------------------------------------------------------------------------
// Database match

/// The outcome of matching a game against the hash database.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ident {
    /// Accepted entry (renames the game).
    pub entry: Option<DatEntry>,
    /// No-Intro / Redump name (libretro art key).
    pub dat_name: Option<String>,
    pub year: Option<i32>,
    pub region: Option<String>,
    /// How it matched ("crc", "rom name", "serial", "serial (name kept)").
    pub how: &'static str,
}

fn serial_region(serial: &str) -> Option<&'static str> {
    let s = serial.trim();
    let c = if s.len() == 4 || s.len() == 6 {
        s.chars().nth(3)?
    } else if s.starts_with("CTR-") {
        s.chars().last()?
    } else {
        return None;
    };
    Some(match c {
        'E' => "USA",
        'P' => "Europe",
        'J' => "Japan",
        'K' => "Korea",
        'D' => "Germany",
        'F' => "France",
        'I' => "Italy",
        'S' => "Spain",
        'U' => "Australia",
        _ => return None,
    })
}

/// Best row: CRC match, same region tags as the file, the serial's region letter, release
/// status and region preference, shortest name.
fn best(rows: Vec<DatEntry>, crcs: &[u32], file_region: Option<&str>, serial: Option<&str>) -> Option<DatEntry> {
    let crcs: Vec<String> = crcs.iter().map(|c| format!("{c:08x}")).collect();
    let file_set = file_region.map(names::region_set);
    let letter = serial.and_then(serial_region);
    rows.into_iter().min_by_key(|r| {
        let row_region = names::region(&r.name).or_else(|| r.region.clone());
        let crc_hit = r.crc.as_ref().is_some_and(|c| crcs.contains(c));
        let same_set = match (&file_set, &row_region) {
            (Some(a), Some(b)) => *a == names::region_set(b),
            _ => false,
        };
        let letter_hit = match (letter, &row_region) {
            (Some(l), Some(r)) => r.contains(l),
            _ => false,
        };
        (!crc_hit, !same_set, !letter_hit, names::status_penalty(&r.name) + names::region_rank(&r.name), r.name.len())
    })
}

/// Match a game against `platform`'s databases.
/// `stem`: the file / folder name it would otherwise be named after; `rom_names`: file names to
/// try as database ROM names; `disc`: a disc / folder game (a serial hit is trusted).
pub fn identify(platform: &Platform, stem: &str, rom_names: &[String], probe: &Probe, disc: bool) -> Ident {
    let mut out = Ident::default();
    let dbs = platform.databases;
    if dbs.is_empty() {
        return out;
    }
    let crcs: Vec<u32> = probe.crc.into_iter().chain(probe.crc_alt).collect();
    let file_region = names::region(stem);
    let serial = probe.serial.as_deref();
    let mut pick: Option<(DatEntry, &'static str)> = None;
    let crc_rows: Vec<DatEntry> =
        dbs.iter().flat_map(|db| crcs.iter().flat_map(move |&c| romdb::by_crc(db, c))).collect();
    if !crc_rows.is_empty() {
        pick = best(crc_rows, &crcs, file_region.as_deref(), serial).map(|e| (e, "crc"));
    }
    if pick.is_none() {
        let rn: Vec<DatEntry> =
            dbs.iter().flat_map(|db| rom_names.iter().flat_map(move |n| romdb::by_rom_name(db, n))).collect();
        if !rn.is_empty() {
            pick = best(rn, &crcs, file_region.as_deref(), serial).map(|e| (e, "rom name"));
        }
    }
    let mut serial_rows = vec![];
    if let Some(s) = serial {
        let keys: Vec<String> = if s.len() == 6 && s.chars().all(|c| c.is_ascii_alphanumeric()) {
            vec![s.to_string(), s[..4].to_string()]
        } else {
            vec![s.to_string()]
        };
        for k in &keys {
            serial_rows = dbs.iter().flat_map(|db| romdb::by_serial(db, k)).collect();
            if !serial_rows.is_empty() {
                break;
            }
        }
    }
    if pick.is_none() && !serial_rows.is_empty() {
        let b = best(serial_rows.clone(), &crcs, file_region.as_deref(), serial);
        if let Some(b) = b {
            let same = names::key(stem) == names::key(&b.name);
            if disc || same || names::uninformative(stem) {
                pick = Some((b, "serial"));
            } else {
                out.how = "serial (name kept)";
            }
        }
    }
    let Some((entry, how)) = pick else { return out };
    out.how = how;
    let same_title = names::key(stem) == names::key(&entry.name);
    // A dump-set file name with the same title is more specific than the row (its own region
    // and revision tags): keep its tags.
    let file_is_dump = file_region.is_some() && same_title;
    out.region = if file_is_dump {
        file_region.clone()
    } else {
        names::region(&entry.name).or(entry.region.clone()).or(file_region)
    };
    out.year = entry.year.or_else(|| {
        let k = names::key(&entry.name);
        serial_rows.iter().filter(|r| names::key(&r.name) == k).find_map(|r| r.year)
    });
    out.dat_name = if file_is_dump {
        Some(stem.to_string())
    } else if entry.name.contains(':') {
        // GameTDB-style rows (GameCube / Wii): the Redump row of the same disc has the
        // No-Intro-style name libretro uses.
        let redump = serial.filter(|s| s.len() == 6).and_then(|s| {
            let rows: Vec<DatEntry> = dbs
                .iter()
                .flat_map(|db| romdb::by_serial(db, &format!("DL-DOL-{}", &s[..4])))
                .filter(|r| !r.name.contains(':'))
                .collect();
            let region = out.region.clone().or_else(|| names::region(&entry.name));
            best(rows, &crcs, region.as_deref(), Some(s))
        });
        Some(redump.map(|r| r.name).unwrap_or_else(|| entry.name.clone()))
    } else {
        Some(entry.name.clone())
    };
    out.entry = Some(entry);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_parsing() {
        assert_eq!(system_cnf("BOOT2 = cdrom0:\\SLUS_203.12;1\r\nVER = 1.00\r\n"), Some(("SLUS-20312".into(), true)));
        assert_eq!(system_cnf("BOOT=cdrom:\\SCUS_944.26;1\n"), Some(("SCUS-94426".into(), false)));
        assert_eq!(system_cnf("BOOT\t=\tcdrom:\\PSX.EXE;1"), None);
        assert_eq!(ps_serial("SLES_503.30"), Some("SLES-50330".into()));
        assert_eq!(dash_serial("ULUS10041"), "ULUS-10041");
        assert_eq!(dash_serial("ULUS-10041"), "ULUS-10041");
        assert_eq!(serial_region("GXXE01"), Some("USA"));
        assert_eq!(serial_region("CTR-P-BNEA"), None, "A = every region");
        assert_eq!(serial_region("CTR-P-BNEE"), Some("USA"));
    }

    #[test]
    fn sfo_reads_keys() {
        let sfo = super::super::tests::sfo(&[("TITLE_ID", "BLUS30001"), ("CATEGORY", "DG"), ("TITLE", "Game")]);
        let v = sfo_values(&sfo);
        assert_eq!(v.get("TITLE_ID").map(String::as_str), Some("BLUS30001"));
        assert_eq!(v.get("CATEGORY").map(String::as_str), Some("DG"));
        assert_eq!(v.get("TITLE").map(String::as_str), Some("Game"));
    }
}
