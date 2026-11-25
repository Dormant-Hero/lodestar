//! Selective reader for Steam's app cache: the single `appcache/appinfo.vdf` (binary, v27/v28/v29)
//! or, since the Steam client beta of November 2026, the 256 shard files
//! `appcache/appinfo/appinfo_00.vdf` … `appinfo_ff.vdf` (v2A). [`read_root`] picks the layout.
//!
//! The file can exceed 100 MB, so only the entries for the requested appids are read and parsed;
//! every other entry is skipped using its size field. Reading stops as soon as all requested
//! appids were found.
//!
//! Layout (little endian):
//!   header: magic u32, universe u32, [v29: string_table_offset i64]
//!   entry:  appid u32 (0 = end), size u32 (bytes after this field), info_state u32,
//!           last_updated u32, pics_token u64, sha1 [20], change_number u32,
//!           [v28+: binary_sha1 [20]], binary KeyValues (v29: keys are string-table indexes)
//!   v29 string table at `string_table_offset`: count u32, then `count` NUL-terminated strings.
//!
//! Shard files (magic 0x0756442A; layout as documented by xPaw/SteamAppInfo, the SteamDB
//! maintainers' reference parser):
//!   header (44 bytes): magic, universe, shard_index, shard_count, app_count, index_size,
//!           string_count, string_table_size, data_size, index_crc, header_crc (all u32)
//!   index (index_size bytes, app_count entries): appid u32, data_offset u32, data_size u32,
//!           summary_size u32, then `summary_size` bytes of summary fields (skipped)
//!   string table (string_table_size bytes): `string_count` NUL-terminated strings, no count
//!   data (data_size bytes): at data_start + data_offset, `data_size` bytes: binary_sha1 [20],
//!           binary KeyValues whose keys index this shard's string table
//!   An app lives in shard `fmix32(appid) & 0xFF` (MurmurHash3 finalizer).

use crate::scan::vdf::{self, Vdf};
use anyhow::{Context, Result, bail};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub const MAGIC_V27: u32 = 0x0756_4427;
pub const MAGIC_V28: u32 = 0x0756_4428;
pub const MAGIC_V29: u32 = 0x0756_4429;
/// Shard files (`appcache/appinfo/appinfo_XX.vdf`).
pub const MAGIC_V2A: u32 = 0x0756_442A;
/// Size of a shard file's header.
const SHARD_HEADER: usize = 44;
/// Shards Steam writes (file names carry the index as two hex digits).
pub const SHARD_COUNT: u32 = 256;

/// Refuse absurd entry sizes (corrupt file) instead of allocating them.
const MAX_ENTRY: u32 = 64 << 20;

/// One `config/launch/<n>` entry of an app.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LaunchEntry {
    pub executable: String,
    pub arguments: String,
    pub workingdir: String,
    /// "default", "option1", "none" …
    pub kind: String,
    pub description: String,
    /// `config/oslist`, e.g. "windows" / "linux" (empty = any).
    pub oslist: String,
    /// `config/betakey`: entry only applies to that beta branch.
    pub betakey: String,
}

/// Fields Lodestar uses from an app's appinfo.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppInfo {
    pub appid: u32,
    /// `common/type`, lowercased ("game", "tool", "config", "application", "dlc", "demo" …).
    pub kind: Option<String>,
    pub name: Option<String>,
    /// `common/oslist` split on commas, lowercased.
    pub oslist: Vec<String>,
    pub developer: Option<String>,
    pub publisher: Option<String>,
    pub release_year: Option<i32>,
    pub genres: Vec<String>,
    pub launch: Vec<LaunchEntry>,
    /// `common/controller_support` ("full" / "partial"), else store categories 28 / 18.
    pub controller: Option<crate::model::ControllerSupport>,
    /// `common/steam_deck_compatibility/category`: 3 verified, 2 playable, 1 unsupported.
    pub deck: Option<crate::model::DeckRating>,
    /// "1" / "1+" / "2+" from the store categories (single-player, multiplayer, co-op …).
    pub players: Option<String>,
}

/// Steam store genre id -> name. Content descriptors (71–74) are mapped but not used as genres.
pub fn genre_name(id: u32) -> Option<&'static str> {
    Some(match id {
        1 => "Action",
        2 => "Strategy",
        3 => "RPG",
        4 => "Casual",
        9 => "Racing",
        18 => "Sports",
        23 => "Indie",
        25 => "Adventure",
        28 => "Simulation",
        29 => "Massively Multiplayer",
        37 => "Free To Play",
        51 => "Animation & Modeling",
        52 => "Audio Production",
        53 => "Design & Illustration",
        54 => "Education",
        55 => "Photo Editing",
        56 => "Software Training",
        57 => "Utilities",
        58 => "Video Production",
        59 => "Web Publishing",
        60 => "Game Development",
        70 => "Early Access",
        71 => "Sexual Content",
        72 => "Nudity",
        73 => "Violent",
        74 => "Gore",
        81 => "Documentary",
        84 => "Tutorial",
        _ => return None,
    })
}

fn is_content_descriptor(id: u32) -> bool {
    (71..=74).contains(&id)
}

fn read_u32(r: &mut impl Read) -> std::io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

/// Read the raw KeyValues of the wanted appids. Returns the parsed root of each entry
/// (an `Obj` normally holding a single `appinfo` object).
pub fn read_entries(path: &Path, wanted: &HashSet<u32>) -> Result<HashMap<u32, Vdf>> {
    let mut out = HashMap::new();
    if wanted.is_empty() {
        return Ok(out);
    }
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let file_len = file.metadata().map(|m| m.len()).unwrap_or(u64::MAX);
    let mut r = BufReader::with_capacity(256 * 1024, file);
    let magic = read_u32(&mut r).context("appinfo header")?;
    let _universe = read_u32(&mut r).context("appinfo header")?;
    let header_skip: usize = match magic {
        MAGIC_V27 => 40,
        MAGIC_V28 | MAGIC_V29 => 60,
        _ => bail!("unsupported appinfo.vdf magic 0x{magic:08x}"),
    };
    let table_offset = if magic == MAGIC_V29 {
        let mut b = [0u8; 8];
        r.read_exact(&mut b).context("appinfo header")?;
        let off = i64::from_le_bytes(b);
        if off <= 0 || off as u64 > file_len {
            bail!("appinfo.vdf string table offset {off} out of range");
        }
        Some(off as u64)
    } else {
        None
    };

    let mut blobs: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut remaining = wanted.len();
    let mut pos: u64 = if table_offset.is_some() { 16 } else { 8 };
    let end = table_offset.unwrap_or(file_len);
    while remaining > 0 && pos + 8 <= end {
        let appid = match read_u32(&mut r) {
            Ok(v) => v,
            Err(_) => break, // truncated file: keep what we have
        };
        if appid == 0 {
            break;
        }
        let Ok(size) = read_u32(&mut r) else { break };
        pos += 8;
        if size > MAX_ENTRY || pos + size as u64 > end {
            // Corrupt (or being rewritten by Steam): keep the entries read so far.
            log::warn!("appinfo.vdf entry {appid} has bad size {size}; stopping there");
            break;
        }
        if wanted.contains(&appid) && !blobs.iter().any(|(a, _)| *a == appid) {
            let mut buf = vec![0u8; size as usize];
            if r.read_exact(&mut buf).is_err() {
                break;
            }
            blobs.push((appid, buf));
            remaining -= 1;
        } else {
            r.seek_relative(size as i64)?;
        }
        pos += size as u64;
    }

    let strings: Option<Vec<String>> = match table_offset {
        Some(off) if !blobs.is_empty() => {
            r.seek(SeekFrom::Start(off))?;
            let count = read_u32(&mut r).context("appinfo string table")? as usize;
            let mut rest = Vec::new();
            (&mut r).take(MAX_ENTRY as u64).read_to_end(&mut rest)?;
            let mut v = Vec::with_capacity(count.min(1 << 20));
            for s in rest.split(|&b| b == 0).take(count) {
                v.push(String::from_utf8_lossy(s).into_owned());
            }
            Some(v)
        }
        _ => None,
    };

    for (appid, blob) in blobs {
        let Some(kv) = blob.get(header_skip..) else {
            continue;
        };
        let parsed = match &strings {
            Some(t) => vdf::parse_binary_with_strings(kv, t),
            None => vdf::parse_binary(kv),
        };
        match parsed {
            Ok(v) => {
                out.insert(appid, v);
            }
            Err(e) => log::debug!("appinfo {appid}: {e:#}"),
        }
    }
    Ok(out)
}

/// Where a Steam root keeps its app cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Layout {
    /// `appcache/appinfo.vdf` (every Steam client up to the November 2026 beta).
    Single(PathBuf),
    /// `appcache/appinfo/` with `appinfo_00.vdf` … `appinfo_ff.vdf`.
    Shards(PathBuf),
    /// Neither: a fresh install, or a layout this reader doesn't know yet.
    Missing,
}

impl Layout {
    /// The app cache of a Steam root. When both exist (a client switched between beta and
    /// stable), the one written last wins.
    pub fn of(root: &Path) -> Layout {
        let single = root.join("appcache/appinfo.vdf");
        let dir = root.join("appcache/appinfo");
        let single_t = single.is_file().then(|| crate::scan::util::mtime(&single).unwrap_or(0));
        // Steam rewrites shards one by one: the newest shard says when the folder was written.
        let shards_t = shard_files(&dir).map(|p| crate::scan::util::mtime(&p).unwrap_or(0)).max();
        match (single_t, shards_t) {
            (Some(a), Some(b)) if b > a => Layout::Shards(dir),
            (Some(_), _) => Layout::Single(single),
            (None, Some(_)) => Layout::Shards(dir),
            (None, None) => Layout::Missing,
        }
    }

    /// For log lines.
    pub fn label(&self) -> &'static str {
        match self {
            Layout::Single(_) => "appinfo.vdf",
            Layout::Shards(_) => "appinfo/ shards",
            Layout::Missing => "none found",
        }
    }
}

/// Read the raw KeyValues of the wanted appids from a Steam root's app cache, whichever layout
/// it has. `Err` when there is none or it can't be read.
pub fn read_entries_root(root: &Path, wanted: &HashSet<u32>) -> Result<HashMap<u32, Vdf>> {
    if wanted.is_empty() {
        return Ok(HashMap::new());
    }
    match Layout::of(root) {
        Layout::Single(p) => read_entries(&p, wanted),
        Layout::Shards(d) => read_shards(&d, wanted),
        Layout::Missing => {
            bail!("no app cache in {} (neither appinfo.vdf nor appinfo/appinfo_*.vdf)", root.join("appcache").display())
        }
    }
}

/// [`read_entries_root`], extracted.
pub fn read_root(root: &Path, wanted: &HashSet<u32>) -> Result<HashMap<u32, AppInfo>> {
    Ok(read_entries_root(root, wanted)?.into_iter().map(|(id, v)| (id, extract(id, &v))).collect())
}

/// MurmurHash3's 32-bit finalizer; its low byte is an app's shard.
pub fn fmix32(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    h = h.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 16;
    h
}

/// The shard an app is kept in, out of `count` shards.
pub fn shard_of(appid: u32, count: u32) -> u32 {
    let h = fmix32(appid);
    if count.is_power_of_two() { h & (count - 1) } else { h % count.max(1) }
}

/// `appinfo_XX.vdf` in `dir`.
pub fn shard_path(dir: &Path, index: u32) -> PathBuf {
    dir.join(format!("appinfo_{index:02x}.vdf"))
}

/// The `appinfo_*.vdf` files of a shard folder.
fn shard_files(dir: &Path) -> impl Iterator<Item = PathBuf> {
    std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| {
        p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("appinfo_") && n.ends_with(".vdf"))
    })
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Shard header fields this reader uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShardHeader {
    pub shard_index: u32,
    pub shard_count: u32,
    pub app_count: u32,
    pub index_size: u32,
    pub string_count: u32,
    pub string_table_size: u32,
    pub data_size: u32,
}

pub fn shard_header(b: &[u8]) -> Result<ShardHeader> {
    if b.len() < SHARD_HEADER {
        bail!("appinfo shard too short ({} bytes)", b.len());
    }
    let f = |i: usize| u32_at(b, i * 4).unwrap_or(0);
    if f(0) != MAGIC_V2A {
        bail!("unsupported appinfo shard magic 0x{:08x}", f(0));
    }
    Ok(ShardHeader {
        shard_index: f(2),
        shard_count: f(3),
        app_count: f(4),
        index_size: f(5),
        string_count: f(6),
        string_table_size: f(7),
        data_size: f(8),
    })
}

/// The index of one shard file: (appid, data offset, data size) per app.
pub fn shard_index(b: &[u8]) -> Result<Vec<(u32, usize, usize)>> {
    let h = shard_header(b)?;
    let strings_start = SHARD_HEADER + h.index_size as usize;
    if strings_start + h.string_table_size as usize > b.len() {
        bail!("appinfo shard index / string table run past the end of the file");
    }
    let mut out = Vec::with_capacity((h.app_count as usize).min(1 << 16));
    let mut pos = SHARD_HEADER;
    for _ in 0..h.app_count {
        if pos + 16 > strings_start {
            break;
        }
        let (Some(appid), Some(off), Some(size), Some(summary)) =
            (u32_at(b, pos), u32_at(b, pos + 4), u32_at(b, pos + 8), u32_at(b, pos + 12))
        else {
            break;
        };
        out.push((appid, off as usize, size as usize));
        pos = pos.saturating_add(16 + summary as usize);
    }
    Ok(out)
}

/// The wanted apps of one shard file (already in memory).
pub fn parse_shard(b: &[u8], wanted: &HashSet<u32>) -> Result<HashMap<u32, Vdf>> {
    let h = shard_header(b)?;
    let strings_start = SHARD_HEADER + h.index_size as usize;
    let data_start = strings_start + h.string_table_size as usize;
    let hits: Vec<(u32, usize, usize)> = shard_index(b)?.into_iter().filter(|(id, _, _)| wanted.contains(id)).collect();
    let mut out = HashMap::new();
    if hits.is_empty() {
        return Ok(out);
    }
    let strings: Vec<String> = b[strings_start..data_start]
        .split(|&c| c == 0)
        .take(h.string_count as usize)
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    for (appid, off, size) in hits {
        let start = data_start.saturating_add(off);
        let Some(entry) = start.checked_add(size).and_then(|end| b.get(start..end)) else {
            log::debug!("appinfo shard: entry {appid} runs past the end of the file");
            continue;
        };
        // binary_sha1 [20], then the KeyValues.
        let Some(kv) = entry.get(20..) else { continue };
        match vdf::parse_binary_with_strings(kv, &strings) {
            Ok(v) => {
                out.insert(appid, v);
            }
            Err(e) => log::debug!("appinfo {appid}: {e:#}"),
        }
    }
    Ok(out)
}

/// Does the index of this shard file list any wanted app? (Reads the header and index only.)
fn shard_lists(p: &Path, wanted: &HashSet<u32>) -> Result<bool> {
    let mut f = File::open(p).with_context(|| format!("open {}", p.display()))?;
    let mut head = vec![0u8; SHARD_HEADER];
    f.read_exact(&mut head).context("appinfo shard header")?;
    let h = shard_header(&head)?;
    if h.index_size > MAX_ENTRY {
        bail!("appinfo shard index size {} out of range", h.index_size);
    }
    let mut index = Vec::with_capacity(h.index_size as usize);
    f.take(h.index_size as u64).read_to_end(&mut index)?;
    let mut pos = 0usize;
    for _ in 0..h.app_count {
        let (Some(appid), Some(summary)) = (u32_at(&index, pos), u32_at(&index, pos + 12)) else { break };
        if wanted.contains(&appid) {
            return Ok(true);
        }
        pos = pos.saturating_add(16 + summary as usize);
    }
    Ok(false)
}

/// The wanted apps from a shard folder. Every shard's header and index is looked at (they are
/// small; a whole folder is a few MB), in the order of the documented sharding rule so the
/// search usually stops after a few files; a shard is read whole only when it lists a wanted app.
/// Steam rewrites shards one by one, so nothing here is cached between scans.
pub fn read_shards(dir: &Path, wanted: &HashSet<u32>) -> Result<HashMap<u32, Vdf>> {
    let mut files: Vec<PathBuf> = shard_files(dir).collect();
    files.sort();
    let predicted: HashSet<PathBuf> = wanted.iter().map(|&id| shard_path(dir, shard_of(id, SHARD_COUNT))).collect();
    // Stable: the predicted shards first, the rest after them.
    files.sort_by_key(|p| !predicted.contains(p));
    let mut out = HashMap::new();
    let mut read_ok = 0usize;
    let mut first_err: Option<anyhow::Error> = None;
    for p in files {
        if out.len() == wanted.len() {
            break;
        }
        let rest: HashSet<u32> = wanted.iter().copied().filter(|id| !out.contains_key(id)).collect();
        let res = match shard_lists(&p, &rest) {
            Ok(false) => Ok(HashMap::new()),
            Ok(true) => crate::scan::util::read_bytes(&p)
                .with_context(|| format!("read {}", p.display()))
                .and_then(|b| parse_shard(&b, &rest)),
            Err(e) => Err(e),
        };
        match res {
            Ok(m) => {
                read_ok += 1;
                out.extend(m);
            }
            Err(e) => {
                log::debug!("{}: {e:#}", p.display());
                first_err.get_or_insert(e);
            }
        }
    }
    match first_err {
        Some(e) if read_ok == 0 => Err(e),
        _ => Ok(out),
    }
}

/// Read and extract the wanted apps of one appinfo.vdf.
#[cfg(test)]
pub fn read(path: &Path, wanted: &HashSet<u32>) -> Result<HashMap<u32, AppInfo>> {
    Ok(read_entries(path, wanted)?.into_iter().map(|(id, v)| (id, extract(id, &v))).collect())
}

fn non_empty(s: Option<&str>) -> Option<String> {
    let s = s?.trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn year_from_unix(ts: i64) -> Option<i32> {
    if ts <= 0 {
        return None;
    }
    let (y, _, _) = crate::model::civil_from_days(ts.div_euclid(86_400));
    i32::try_from(y).ok()
}

/// Pull the fields Lodestar uses out of an entry's KeyValues.
pub fn extract(appid: u32, root: &Vdf) -> AppInfo {
    let app = root.get("appinfo").unwrap_or(root);
    let mut info = AppInfo { appid, ..Default::default() };
    if let Some(common) = app.get("common") {
        info.kind = non_empty(common.get_str("type")).map(|t| t.to_ascii_lowercase());
        info.name = non_empty(common.get_str("name"));
        {
            use crate::model::{ControllerSupport, DeckRating};
            let cat = |id: u32| common.get("category").and_then(|c| c.get(&format!("category_{id}"))).is_some();
            info.controller = match common.get_str("controller_support").map(str::to_ascii_lowercase).as_deref() {
                Some("full") => Some(ControllerSupport::Full),
                Some("partial") => Some(ControllerSupport::Partial),
                _ if cat(28) => Some(ControllerSupport::Full),
                _ if cat(18) => Some(ControllerSupport::Partial),
                _ => None,
            };
            let cats: Vec<u64> = common
                .get("category")
                .map(|c| c.entries().iter().filter_map(|(k, _)| k.strip_prefix("category_")?.parse().ok()).collect())
                .unwrap_or_default();
            info.players = crate::model::players_from_steam_categories(&cats);
            info.deck = match common.get("steam_deck_compatibility").and_then(|d| d.get_str("category")).map(str::trim)
            {
                Some("3") => Some(DeckRating::Verified),
                Some("2") => Some(DeckRating::Playable),
                Some("1") => Some(DeckRating::Unsupported),
                _ => None,
            };
        }
        info.oslist = common
            .get_str("oslist")
            .unwrap_or("")
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        let mut devs = Vec::new();
        let mut pubs = Vec::new();
        if let Some(assoc) = common.get("associations") {
            for (_, a) in assoc.entries() {
                let Some(name) = non_empty(a.get_str("name")) else {
                    continue;
                };
                match a.get_str("type") {
                    Some(t) if t.eq_ignore_ascii_case("developer") => devs.push(name),
                    Some(t) if t.eq_ignore_ascii_case("publisher") => pubs.push(name),
                    _ => {}
                }
            }
        }
        dedup_keep_order(&mut devs);
        dedup_keep_order(&mut pubs);
        info.developer = (!devs.is_empty()).then(|| devs.join(", "));
        info.publisher = (!pubs.is_empty()).then(|| pubs.join(", "));
        let date = |k: &str| common.get(k).and_then(Vdf::int).and_then(year_from_unix);
        info.release_year = date("original_release_date").or_else(|| date("steam_release_date"));
        if let Some(g) = common.get("genres") {
            for (_, v) in g.entries() {
                if let Some(id) = v.uint().and_then(|i| u32::try_from(i).ok())
                    && !is_content_descriptor(id)
                    && let Some(n) = genre_name(id)
                    && !info.genres.iter().any(|x| x == n)
                {
                    info.genres.push(n.to_string());
                }
            }
        }
    }
    if let Some(ext) = app.get("extended") {
        if info.developer.is_none() {
            info.developer = non_empty(ext.get_str("developer"));
        }
        if info.publisher.is_none() {
            info.publisher = non_empty(ext.get_str("publisher"));
        }
    }
    if let Some(launch) = app.path(&["config", "launch"]) {
        for (_, e) in launch.entries() {
            let cfg = e.get("config");
            let cs = |k: &str| cfg.and_then(|c| c.get_str(k)).unwrap_or("").to_string();
            info.launch.push(LaunchEntry {
                executable: e.get_str("executable").unwrap_or("").to_string(),
                arguments: e.get_str("arguments").unwrap_or("").to_string(),
                workingdir: e.get_str("workingdir").unwrap_or("").to_string(),
                kind: e.get_str("type").unwrap_or("").to_ascii_lowercase(),
                description: e.get_str("description").unwrap_or("").to_string(),
                oslist: cs("oslist").to_ascii_lowercase(),
                betakey: cs("betakey"),
            });
        }
    }
    info
}

fn dedup_keep_order(v: &mut Vec<String>) {
    let mut seen = HashSet::new();
    v.retain(|s| seen.insert(s.to_ascii_lowercase()));
}

/// Writer for synthetic appinfo files (tests and fixtures).
#[cfg(test)]
pub mod testgen {
    use super::*;

    /// Minimal binary-KV node tree used to generate entries.
    pub enum N {
        S(&'static str, String),
        I(&'static str, i32),
        O(&'static str, Vec<N>),
    }

    fn key_index(strings: &mut Vec<String>, k: &str) -> u32 {
        if let Some(i) = strings.iter().position(|s| s == k) {
            return i as u32;
        }
        strings.push(k.to_string());
        (strings.len() - 1) as u32
    }

    fn key(out: &mut Vec<u8>, k: &str, strings: &mut Option<&mut Vec<String>>) {
        match strings {
            Some(t) => out.extend_from_slice(&key_index(t, k).to_le_bytes()),
            None => {
                out.extend_from_slice(k.as_bytes());
                out.push(0);
            }
        }
    }

    fn emit(out: &mut Vec<u8>, n: &N, strings: &mut Option<&mut Vec<String>>) {
        match n {
            N::S(k, v) => {
                out.push(1);
                key(out, k, strings);
                out.extend_from_slice(v.as_bytes());
                out.push(0);
            }
            N::I(k, v) => {
                out.push(2);
                key(out, k, strings);
                out.extend_from_slice(&v.to_le_bytes());
            }
            N::O(k, kids) => {
                out.push(0);
                key(out, k, strings);
                for c in kids {
                    emit(out, c, strings);
                }
                out.push(8);
            }
        }
    }

    /// Build a v29 (or v28 when `v29 == false`) appinfo.vdf from (appid, nodes-under-"appinfo").
    pub fn build(apps: Vec<(u32, Vec<N>)>, v29: bool) -> Vec<u8> {
        let mut strings: Vec<String> = Vec::new();
        let mut body = Vec::new();
        for (appid, nodes) in apps {
            let mut kv = Vec::new();
            let root = N::O("appinfo", nodes);
            {
                let mut t = if v29 { Some(&mut strings) } else { None };
                emit(&mut kv, &root, &mut t);
            }
            kv.push(8);
            let mut entry = Vec::new();
            entry.extend_from_slice(&1u32.to_le_bytes()); // info_state
            entry.extend_from_slice(&0u32.to_le_bytes()); // last_updated
            entry.extend_from_slice(&0u64.to_le_bytes()); // token
            entry.extend_from_slice(&[0u8; 20]); // sha1
            entry.extend_from_slice(&7u32.to_le_bytes()); // change number
            entry.extend_from_slice(&[0u8; 20]); // binary sha1
            entry.extend_from_slice(&kv);
            body.extend_from_slice(&appid.to_le_bytes());
            body.extend_from_slice(&(entry.len() as u32).to_le_bytes());
            body.extend_from_slice(&entry);
        }
        body.extend_from_slice(&0u32.to_le_bytes());
        let mut out = Vec::new();
        out.extend_from_slice(&(if v29 { MAGIC_V29 } else { MAGIC_V28 }).to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        if v29 {
            let off = 16 + body.len() as i64;
            out.extend_from_slice(&off.to_le_bytes());
            out.extend_from_slice(&body);
            out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
            for s in &strings {
                out.extend_from_slice(s.as_bytes());
                out.push(0);
            }
        } else {
            out.extend_from_slice(&body);
        }
        out
    }

    /// Build one v2A shard file (`appcache/appinfo/appinfo_XX.vdf`) holding `apps`.
    pub fn build_shard(index: u32, count: u32, apps: Vec<(u32, Vec<N>)>) -> Vec<u8> {
        let mut strings: Vec<String> = Vec::new();
        let mut idx = Vec::new();
        let mut data = Vec::new();
        let n = apps.len() as u32;
        for (appid, nodes) in apps {
            let mut kv = Vec::new();
            emit(&mut kv, &N::O("appinfo", nodes), &mut Some(&mut strings));
            kv.push(8);
            let off = data.len() as u32;
            data.extend_from_slice(&[0u8; 20]); // binary sha1
            data.extend_from_slice(&kv);
            let mut summary = Vec::new();
            summary.extend_from_slice(&7u32.to_le_bytes()); // change number
            summary.extend_from_slice(&0u32.to_le_bytes()); // last updated
            summary.extend_from_slice(&0u64.to_le_bytes()); // token
            summary.push(0); // prerelease
            summary.extend_from_slice(&[0u8; 20]); // sha1
            summary.extend_from_slice(&1u32.to_le_bytes()); // type flag (game)
            summary.extend_from_slice(&0u32.to_le_bytes()); // parent
            summary.push(0); // exfgls
            summary.extend_from_slice(&1u32.to_le_bytes()); // additional content: one id
            summary.extend_from_slice(&(appid + 1).to_le_bytes());
            for _ in 0..3 {
                summary.extend_from_slice(&0i32.to_le_bytes()); // release states
            }
            summary.push(0);
            summary.extend_from_slice(b"US\0");
            idx.extend_from_slice(&appid.to_le_bytes());
            idx.extend_from_slice(&off.to_le_bytes());
            idx.extend_from_slice(&((data.len() as u32) - off).to_le_bytes());
            idx.extend_from_slice(&(summary.len() as u32).to_le_bytes());
            idx.extend_from_slice(&summary);
        }
        let mut table = Vec::new();
        for t in &strings {
            table.extend_from_slice(t.as_bytes());
            table.push(0);
        }
        let mut out = Vec::new();
        for v in [
            MAGIC_V2A,
            1,
            index,
            count,
            n,
            idx.len() as u32,
            strings.len() as u32,
            table.len() as u32,
            data.len() as u32,
            0, // index crc (not checked)
            0, // header crc (not checked)
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&idx);
        out.extend_from_slice(&table);
        out.extend_from_slice(&data);
        out
    }

    /// Write `apps` as a full shard folder (every one of the 256 files, most of them empty).
    pub fn write_shards(dir: &Path, apps: Vec<(u32, Vec<N>)>) {
        std::fs::create_dir_all(dir).unwrap();
        let mut per: Vec<Vec<(u32, Vec<N>)>> = (0..SHARD_COUNT).map(|_| Vec::new()).collect();
        for (id, nodes) in apps {
            per[shard_of(id, SHARD_COUNT) as usize].push((id, nodes));
        }
        for (i, list) in per.into_iter().enumerate() {
            std::fs::write(shard_path(dir, i as u32), build_shard(i as u32, SHARD_COUNT, list)).unwrap();
        }
    }

    pub fn s(k: &'static str, v: &str) -> N {
        N::S(k, v.to_string())
    }

    /// `common` block for a typical app.
    pub fn common(name: &str, kind: &str, oslist: &str) -> N {
        N::O(
            "common",
            vec![
                s("name", name),
                s("type", kind),
                s("oslist", oslist),
                N::I("original_release_date", 1_431_475_200), // 2015-05-13
                N::O("genres", vec![s("0", "3"), s("1", "1"), s("2", "73")]),
                N::O(
                    "associations",
                    vec![
                        N::O("0", vec![s("type", "developer"), s("name", "Dev Studio")]),
                        N::O("1", vec![s("type", "publisher"), s("name", "Pub Co")]),
                        N::O("2", vec![s("type", "developer"), s("name", "Dev Studio")]),
                    ],
                ),
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::testgen::*;
    use super::*;

    fn write(dir: &Path, bytes: &[u8]) -> std::path::PathBuf {
        let p = dir.join("appinfo.vdf");
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn reads_only_wanted_v29_and_v28() {
        for v29 in [true, false] {
            let apps = vec![
                (10, vec![common("Ten", "Game", "windows,linux")]),
                (228980, vec![common("Steamworks Common Redistributables", "Config", "")]),
                (
                    30,
                    vec![
                        common("Thirty", "game", "windows"),
                        N::O(
                            "config",
                            vec![N::O(
                                "launch",
                                vec![N::O(
                                    "0",
                                    vec![
                                        s("executable", "bin\\game.exe"),
                                        s("arguments", "-dx11"),
                                        s("type", "default"),
                                        N::O("config", vec![s("oslist", "windows")]),
                                    ],
                                )],
                            )],
                        ),
                        N::O("extended", vec![s("developer", "Ignored"), s("publisher", "Ignored")]),
                    ],
                ),
            ];
            let dir = tempfile::tempdir().unwrap();
            let p = write(dir.path(), &build(apps, v29));
            let wanted: HashSet<u32> = [30, 228980, 999].into();
            let m = read(&p, &wanted).unwrap();
            assert_eq!(m.len(), 2, "v29={v29}");
            let a = &m[&30];
            assert_eq!(a.name.as_deref(), Some("Thirty"));
            assert_eq!(a.kind.as_deref(), Some("game"));
            assert_eq!(a.oslist, vec!["windows"]);
            assert_eq!(a.developer.as_deref(), Some("Dev Studio"));
            assert_eq!(a.publisher.as_deref(), Some("Pub Co"));
            assert_eq!(a.release_year, Some(2015));
            assert_eq!(a.genres, vec!["RPG", "Action"]);
            assert_eq!(a.launch.len(), 1);
            assert_eq!(a.launch[0].executable, "bin\\game.exe");
            assert_eq!(a.launch[0].oslist, "windows");
            assert_eq!(m[&228980].kind.as_deref(), Some("config"));
            assert!(!m.contains_key(&10));
        }
    }

    #[test]
    fn empty_wanted_does_not_open_file() {
        let m = read(Path::new("/nonexistent/appinfo.vdf"), &HashSet::new()).unwrap();
        assert!(m.is_empty());
    }

    #[test]
    fn bad_files_are_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let wanted: HashSet<u32> = [10].into();
        // wrong magic
        let p = write(dir.path(), &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        assert!(read(&p, &wanted).is_err());
        // too short
        let p = write(dir.path(), &[0x29]);
        assert!(read(&p, &wanted).is_err());
        // a corrupt entry later in the file keeps the good ones before it
        let mut two = build(
            vec![(10, vec![common("Ten", "Game", "linux")]), (20, vec![common("Twenty", "Game", "linux")])],
            false,
        );
        let second = two.windows(4).rposition(|w| w == 20u32.to_le_bytes()).unwrap();
        two[second + 4..second + 8].copy_from_slice(&u32::MAX.to_le_bytes());
        let p = write(dir.path(), &two);
        let m = read(&p, &[10, 20].into()).unwrap();
        assert_eq!(m.get(&10).and_then(|i| i.name.as_deref()), Some("Ten"));
        assert!(!m.contains_key(&20));
        // truncations of a valid file never panic
        let full = build(vec![(10, vec![common("Ten", "Game", "linux")])], true);
        for n in 0..full.len() {
            let p = write(dir.path(), &full[..n]);
            let _ = read(&p, &wanted);
        }
    }
}

#[cfg(test)]
mod shard_tests {
    use super::testgen::*;
    use super::*;

    fn apps() -> Vec<(u32, Vec<N>)> {
        vec![
            (10, vec![common("Ten", "Game", "windows,linux")]),
            (228980, vec![common("Steamworks Common Redistributables", "Config", "")]),
            (1_086_940, vec![common("Baldur's Gate 3", "game", "windows")]),
            (30, vec![common("Thirty", "game", "windows")]),
        ]
    }

    /// The shard rule from xPaw/SteamAppInfo (MurmurHash3 fmix32, low byte).
    #[test]
    fn shard_rule() {
        assert_eq!(fmix32(0), 0);
        assert_eq!(fmix32(1), 0x514E_28B7);
        assert_eq!(shard_of(1, 256), 0xB7);
        assert!((0..5000).all(|id| shard_of(id, 256) < 256));
        assert_eq!(shard_path(Path::new("/x"), 10), Path::new("/x/appinfo_0a.vdf"));
    }

    #[test]
    fn reads_wanted_apps_from_shards() {
        let dir = tempfile::tempdir().unwrap();
        let shards = dir.path().join("appcache/appinfo");
        write_shards(&shards, apps());
        assert_eq!(Layout::of(dir.path()), Layout::Shards(shards.clone()));
        let wanted: HashSet<u32> = [30, 1_086_940, 228980, 999].into();
        let m = read_root(dir.path(), &wanted).unwrap();
        assert_eq!(m.len(), 3);
        let a = &m[&30];
        assert_eq!(a.name.as_deref(), Some("Thirty"));
        assert_eq!(a.kind.as_deref(), Some("game"));
        assert_eq!(a.developer.as_deref(), Some("Dev Studio"));
        assert_eq!(a.release_year, Some(2015));
        assert_eq!(a.genres, vec!["RPG", "Action"]);
        assert_eq!(m[&1_086_940].name.as_deref(), Some("Baldur's Gate 3"));
        assert_eq!(m[&228980].kind.as_deref(), Some("config"));
        // An app in a shard the rule doesn't name (a changed rule) is still found.
        let other = (shard_of(30, 256) + 1) % 256;
        std::fs::write(
            shard_path(&shards, other),
            build_shard(other, 256, vec![(77, vec![common("Misplaced", "game", "")])]),
        )
        .unwrap();
        let m = read_root(dir.path(), &[30, 77].into()).unwrap();
        assert_eq!(m[&77].name.as_deref(), Some("Misplaced"));
        assert!(m.contains_key(&30));
    }

    /// Everything in one shard (a sharding rule other than the documented one).
    #[test]
    fn unknown_sharding_rule_falls_back_to_every_shard() {
        let dir = tempfile::tempdir().unwrap();
        let shards = dir.path().join("appcache/appinfo");
        std::fs::create_dir_all(&shards).unwrap();
        for i in 0..256 {
            let list = if i == 0x42 { apps() } else { vec![] };
            std::fs::write(shard_path(&shards, i), build_shard(i, 256, list)).unwrap();
        }
        let m = read_root(dir.path(), &[10, 30].into()).unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m[&10].name.as_deref(), Some("Ten"));
    }

    #[test]
    fn layouts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert_eq!(Layout::of(root), Layout::Missing);
        assert!(read_root(root, &[10].into()).is_err());
        assert!(read_root(root, &HashSet::new()).unwrap().is_empty());
        std::fs::create_dir_all(root.join("appcache/appinfo")).unwrap();
        assert_eq!(Layout::of(root), Layout::Missing, "an empty shard folder");
        std::fs::write(root.join("appcache/appinfo.vdf"), build(apps(), true)).unwrap();
        assert_eq!(Layout::of(root), Layout::Single(root.join("appcache/appinfo.vdf")));
        assert_eq!(read_root(root, &[30].into()).unwrap()[&30].name.as_deref(), Some("Thirty"));
    }

    #[test]
    fn bad_shards_are_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let shards = dir.path().join("appcache/appinfo");
        std::fs::create_dir_all(&shards).unwrap();
        let wanted: HashSet<u32> = [30].into();
        let i = shard_of(30, 256);
        std::fs::write(shard_path(&shards, i), b"garbage").unwrap();
        assert!(read_shards(&shards, &wanted).is_err());
        let full = build_shard(i, 256, apps());
        for n in 0..full.len() {
            std::fs::write(shard_path(&shards, i), &full[..n]).unwrap();
            let _ = read_shards(&shards, &wanted);
        }
        // Absurd sizes in the header / index.
        let mut bad = full.clone();
        bad[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_shard(&bad, &wanted).is_err());
        let mut bad = full;
        let at = SHARD_HEADER + 8;
        bad[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        let _ = parse_shard(&bad, &[10, 30, 228980, 1_086_940].into());
    }
}

/// Real shard files: `LODESTAR_APPINFO_SHARDS=<…/appcache/appinfo> cargo test --release
/// real_appinfo_shards -- --ignored --nocapture` reads every app of every shard.
#[cfg(test)]
mod real_shards {
    use super::*;

    #[test]
    #[ignore]
    fn real_appinfo_shards() {
        let Some(dir) = std::env::var_os("LODESTAR_APPINFO_SHARDS").map(PathBuf::from) else { return };
        let (mut apps, mut files) = (0usize, 0usize);
        let mut all: HashMap<u32, AppInfo> = HashMap::new();
        for p in shard_files(&dir) {
            let b = std::fs::read(&p).unwrap();
            let h = shard_header(&b).unwrap();
            let idx = shard_index(&b).unwrap();
            let n = p.file_name().unwrap().to_str().unwrap();
            assert_eq!(format!("appinfo_{:02x}.vdf", h.shard_index), n);
            assert_eq!(idx.len(), h.app_count as usize, "{n}");
            assert_eq!(SHARD_HEADER + (h.index_size + h.string_table_size + h.data_size) as usize, b.len(), "{n}");
            let ids: HashSet<u32> = idx.iter().map(|e| e.0).collect();
            for id in &ids {
                assert_eq!(shard_of(*id, h.shard_count), h.shard_index, "app {id} in {n}");
            }
            let m = parse_shard(&b, &ids).unwrap();
            assert_eq!(m.len(), ids.len(), "every app of {n} parses");
            apps += m.len();
            files += 1;
            all.extend(m.into_iter().map(|(id, v)| (id, extract(id, &v))));
        }
        let mut kinds: HashMap<String, usize> = HashMap::new();
        for i in all.values() {
            *kinds.entry(i.kind.clone().unwrap_or_default()).or_default() += 1;
        }
        eprintln!("{files} shards, {apps} apps; types: {kinds:?}");
        let mut ids: Vec<&u32> = all.keys().collect();
        ids.sort();
        for id in
            ids.iter().filter(|id| [220, 570, 730, 1_086_940, 1_245_620, 1_091_500, 228_980, 1_493_710].contains(**id))
        {
            let i = &all[id];
            eprintln!(
                "{id}: {:?} {:?} dev {:?} pub {:?} {:?} {:?} {:?}",
                i.name, i.kind, i.developer, i.publisher, i.release_year, i.genres, i.oslist
            );
        }
        // The same answers through the folder reader (only the wanted apps).
        let wanted: HashSet<u32> = all.keys().copied().take(50).collect();
        assert_eq!(read_shards(&dir, &wanted).unwrap().len(), wanted.len());
    }
}

#[cfg(test)]
mod bench {
    use super::testgen::*;
    use super::*;

    /// `cargo test --release appinfo_large -- --ignored --nocapture`: ~100 MB file, 60 wanted ids.
    #[test]
    #[ignore]
    fn appinfo_large() {
        let pad = "x".repeat(2000);
        let apps: Vec<(u32, Vec<N>)> =
            (1..=50_000u32).map(|id| (id * 10, vec![common("App", "Game", "windows"), s("pad", &pad)])).collect();
        let bytes = build(apps, true);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("appinfo.vdf");
        std::fs::write(&p, &bytes).unwrap();
        let wanted: HashSet<u32> = (0..60u32).map(|i| 10 * (1 + i * 830)).chain([7]).collect();
        let t = std::time::Instant::now();
        let m = read(&p, &wanted).unwrap();
        eprintln!("{} MB, found {} of {} in {:?}", bytes.len() >> 20, m.len(), wanted.len(), t.elapsed());
        assert_eq!(m.len(), 60); // 7 is absent: whole file scanned
    }
}
