//! Selective reader for Steam's `appcache/appinfo.vdf` (binary, v27/v28/v29).
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

use crate::scan::vdf::{self, Vdf};
use anyhow::{Context, Result, bail};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

pub const MAGIC_V27: u32 = 0x0756_4427;
pub const MAGIC_V28: u32 = 0x0756_4428;
pub const MAGIC_V29: u32 = 0x0756_4429;

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

/// Read and extract the wanted apps.
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
