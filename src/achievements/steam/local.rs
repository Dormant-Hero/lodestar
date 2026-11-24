//! The Steam client's own achievement files (read-only): which Steam install and account, the
//! achievement list (`UserGameStatsSchema_<appid>.bin`), the player's unlocks
//! (`UserGameStats_<accountid>_<appid>.bin`) and global rarity from the library page cache
//! (`userdata/<id>/config/librarycache/<appid>.json`).
//!
//! Only these files are opened (plus `config/loginusers.vdf` through `scan::steam::active_user`);
//! never `config/config.vdf`, `ssfn*`, the client's web caches or anything holding a login.

use crate::scan::util::read_bytes;
use crate::scan::vdf::{self, Vdf};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Steam64 id of account id 0.
pub const STEAMID64_BASE: u64 = 76_561_197_960_265_728;

/// Files bigger than this aren't Steam's (a big schema with 30 languages is ~1 MB).
pub(crate) const MAX_FILE: u64 = 16 << 20;

/// Where a game's files are.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Files {
    pub schema: Option<PathBuf>,
    pub user_stats: Option<PathBuf>,
    /// `userdata/<accountid>/config/librarycache/<appid>.json` (may not exist).
    pub library_json: Option<PathBuf>,
    pub account: Option<u32>,
}

impl Files {
    /// Size + modification time of every file, to tell when anything changed.
    pub fn fingerprint(&self) -> Vec<Option<(u64, SystemTime)>> {
        [&self.schema, &self.user_stats, &self.library_json]
            .into_iter()
            .map(|p| {
                let m = std::fs::metadata(p.as_ref()?).ok()?;
                Some((m.len(), m.modified().ok()?))
            })
            .collect()
    }
}

/// Steam installs to look in for a game, most likely first: the one the game is installed in
/// (covers a custom Steam folder), then every install the library scan finds (native, `~/.steam`,
/// Flatpak, Snap).
pub fn candidate_roots(game: &crate::model::Game) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(r) = game.install_dir.as_deref().and_then(install_root) {
        out.push(r);
    }
    let ctx = crate::scan::ScanContext::from_config(&crate::config::Config::default());
    for r in crate::scan::steam::find_roots(&ctx) {
        if !out.contains(&r.path) {
            out.push(r.path);
        }
    }
    out
}

/// `<root>/steamapps/common/<dir>` → `<root>` when that folder is a Steam install.
pub fn install_root(dir: &Path) -> Option<PathBuf> {
    let lib = dir.parent().and_then(Path::parent).and_then(Path::parent)?;
    if !lib.join("appcache/stats").is_dir() {
        return None;
    }
    lib.canonicalize().ok()
}

/// The files for `appid` in the first root that has its achievement list (else the first root
/// with a Steam account, for the web lookups and the page link).
pub fn find(roots: &[PathBuf], appid: u32) -> Files {
    let mut fallback: Option<Files> = None;
    for root in roots {
        let files = files_in(root, appid);
        if files.schema.is_some() {
            return files;
        }
        if fallback.is_none() && files.account.is_some() {
            fallback = Some(files);
        }
    }
    fallback.unwrap_or_default()
}

fn files_in(root: &Path, appid: u32) -> Files {
    let stats = root.join("appcache/stats");
    let schema = file(stats.join(format!("UserGameStatsSchema_{appid}.bin")));
    let active: Option<u32> =
        crate::scan::steam::active_user(root).and_then(|d| d.file_name()?.to_str()?.parse().ok()).filter(|a| *a != 0);
    // The active account's file, else the newest one of any account on this machine.
    let mut account = active;
    let mut user_stats = active.and_then(|a| file(stats.join(format!("UserGameStats_{a}_{appid}.bin"))));
    if user_stats.is_none()
        && let Some((a, p)) = newest_user_stats(&stats, appid)
    {
        account = Some(a);
        user_stats = Some(p);
    }
    let library_json =
        account.map(|a| root.join(format!("userdata/{a}/config/librarycache/{appid}.json"))).and_then(file);
    Files { schema, user_stats, library_json, account }
}

fn file(p: PathBuf) -> Option<PathBuf> {
    std::fs::metadata(&p).ok().filter(|m| m.is_file() && m.len() > 0 && m.len() <= MAX_FILE).map(|_| p)
}

/// `UserGameStats_<accountid>_<appid>.bin` changed last, with its account id.
fn newest_user_stats(stats: &Path, appid: u32) -> Option<(u32, PathBuf)> {
    let suffix = format!("_{appid}.bin");
    std::fs::read_dir(stats)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let account: u32 = name.strip_prefix("UserGameStats_")?.strip_suffix(&suffix)?.parse().ok()?;
            let p = file(e.path())?;
            let t = std::fs::metadata(&p).ok()?.modified().ok()?;
            Some((t, account, p))
        })
        .max_by_key(|(t, _, _)| *t)
        .map(|(_, a, p)| (a, p))
}

/// The Steam language name for the desktop's language (`$LC_ALL` / `$LC_MESSAGES` / `$LANG`).
pub fn language() -> &'static str {
    let var = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty() && v != "C" && v != "POSIX" && !v.starts_with("C."))
        .unwrap_or_default();
    steam_language(&var)
}

/// "de_DE.UTF-8" → "german" (Steam's API language names); unknown → "english".
pub fn steam_language(locale: &str) -> &'static str {
    let tag = locale.split(['.', '@']).next().unwrap_or("").to_ascii_lowercase().replace('-', "_");
    let (lang, region) = tag.split_once('_').unwrap_or((&tag, ""));
    match (lang, region) {
        ("zh", "tw" | "hk" | "mo") => "tchinese",
        ("zh", _) => "schinese",
        ("pt", "br") => "brazilian",
        ("pt", _) => "portuguese",
        ("es", "es" | "") => "spanish",
        ("es", _) => "latam",
        ("de", _) => "german",
        ("fr", _) => "french",
        ("it", _) => "italian",
        ("ru", _) => "russian",
        ("pl", _) => "polish",
        ("ja", _) => "japanese",
        ("ko", _) => "koreana",
        ("nl", _) => "dutch",
        ("sv", _) => "swedish",
        ("da", _) => "danish",
        ("fi", _) => "finnish",
        ("nb" | "nn" | "no", _) => "norwegian",
        ("cs", _) => "czech",
        ("hu", _) => "hungarian",
        ("ro", _) => "romanian",
        ("tr", _) => "turkish",
        ("uk", _) => "ukrainian",
        ("el", _) => "greek",
        ("bg", _) => "bulgarian",
        ("th", _) => "thai",
        ("vi", _) => "vietnamese",
        ("ar", _) => "arabic",
        ("id", _) => "indonesian",
        _ => "english",
    }
}

/// One achievement as the schema describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Def {
    pub stat: String,
    pub bit: u32,
    /// API name.
    pub id: String,
    pub name: String,
    pub desc: String,
    pub hidden: bool,
    /// Picture file names (`<sha1>.jpg`).
    pub icon: Option<String>,
    pub icon_gray: Option<String>,
}

fn read_vdf(p: &Path) -> Option<Vdf> {
    let bytes = read_bytes(p).ok()?;
    match vdf::parse_binary(&bytes) {
        Ok(v) => Some(v),
        Err(e) => {
            log::debug!("achievements: {}: {e}", p.display());
            None
        }
    }
}

/// Every achievement in a schema file, in the developer's order. None: unreadable.
pub fn read_schema(p: &Path, lang: &str) -> Option<Vec<Def>> {
    parse_schema(&read_vdf(p)?, lang)
}

/// `"<appid>" { stats { <statid> { bits { <bit> { name, display { name, desc, hidden, icon,
/// icon_gray } } } } } }`. Any stat with `bits` holds achievements (whatever its `type` says, which
/// older files write as a number); bits retired by the developer aren't in the schema.
pub fn parse_schema(v: &Vdf, lang: &str) -> Option<Vec<Def>> {
    // The single top-level object (named after the appid; not matched, some tools write others).
    let app = &v.entries().first()?.1;
    let stats = app.get("stats")?;
    let mut out = Vec::new();
    for (stat, s) in stats.entries() {
        let Some(bits) = s.get("bits") else {
            continue;
        };
        for (bit, d) in bits.entries() {
            let Some(bit) = bit.trim().parse::<u32>().ok().filter(|b| *b < 32) else {
                continue;
            };
            let Some(id) = d.get_str("name").map(str::trim).filter(|s| !s.is_empty()) else {
                continue;
            };
            let display = d.get("display");
            let text = |key: &str| display.and_then(|x| x.get(key)).and_then(|n| pick(n, lang));
            let icon = |key: &str| {
                display.and_then(|x| x.get_str(key)).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
            };
            out.push(Def {
                stat: stat.clone(),
                bit,
                id: id.to_string(),
                name: text("name").unwrap_or_else(|| id.to_string()),
                desc: text("desc").unwrap_or_default(),
                hidden: display.and_then(|x| x.get("hidden")).and_then(Vdf::int).is_some_and(|h| h != 0),
                icon: icon("icon"),
                icon_gray: icon("icon_gray"),
            });
        }
    }
    Some(out)
}

/// A localised string: the language asked for, else English, else the first language (never the
/// `token` key, which is the developer's string id).
fn pick(node: &Vdf, lang: &str) -> Option<String> {
    let ok = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    if let Some(s) = node.str() {
        return ok(s);
    }
    node.get_str(lang).and_then(ok).or_else(|| node.get_str("english").and_then(ok)).or_else(|| {
        node.entries().iter().filter(|(k, _)| !k.eq_ignore_ascii_case("token")).find_map(|(_, v)| ok(v.str()?))
    })
}

/// The player's unlocks: (stat id, bit) → unlock time (unix seconds; 0 = time unknown).
pub type Unlocks = HashMap<(String, u32), i64>;

/// None: unreadable.
pub fn read_user_stats(p: &Path) -> Option<Unlocks> {
    parse_user_stats(&read_vdf(p)?)
}

/// `cache { <statid> { data <bitfield>, AchievementTimes { <bit> <unix> } } }`. The bitfield is a
/// signed int32 in the file (bit 31 makes it negative); every set bit counts, also ones the schema
/// no longer has (the caller only looks up the schema's bits).
pub fn parse_user_stats(v: &Vdf) -> Option<Unlocks> {
    let cache = v.get("cache")?;
    let mut out = Unlocks::new();
    for (stat, s) in cache.entries() {
        let Some(data) = s.get("data") else {
            continue;
        };
        let Some(field) = data.int().or_else(|| data.uint().map(|u| u as i64)) else {
            continue;
        };
        let field = field as u32;
        let times = s.get("AchievementTimes");
        for bit in 0..32u32 {
            if field >> bit & 1 == 1 {
                let t = times.and_then(|t| t.get(&bit.to_string())).and_then(Vdf::int).map(|t| t as u32 as i64);
                out.insert((stat.clone(), bit), t.unwrap_or(0));
            }
        }
    }
    Some(out)
}

/// Global unlock percentages (API name → 0..=100) from the library page cache. The file is a
/// JSON array of `[section, {version, data}]`; some copies have stray bytes after it, so only the
/// first JSON value is read.
pub fn read_library_rarity(p: &Path) -> HashMap<String, f32> {
    read_bytes(p).map(|b| parse_library_rarity(&b)).unwrap_or_default()
}

pub fn parse_library_rarity(bytes: &[u8]) -> HashMap<String, f32> {
    let mut out = HashMap::new();
    let Some(Ok(v)) = serde_json::Deserializer::from_slice(bytes).into_iter::<serde_json::Value>().next() else {
        return out;
    };
    let Some(sections) = v.as_array() else {
        return out;
    };
    let Some(data) = sections
        .iter()
        .filter_map(|s| s.as_array())
        .find(|s| s.first().and_then(|n| n.as_str()) == Some("achievements"))
        .and_then(|s| s.get(1)?.get("data"))
    else {
        return out;
    };
    for list in ["vecHighlight", "vecUnachieved", "vecAchievedHidden"] {
        for a in data.get(list).and_then(|l| l.as_array()).into_iter().flatten() {
            if let (Some(id), Some(p)) =
                (a.get("strID").and_then(|s| s.as_str()), a.get("flAchieved").and_then(|f| f.as_f64()))
                && (0.0..=100.0).contains(&p)
            {
                out.insert(id.to_string(), p as f32);
            }
        }
    }
    out
}
