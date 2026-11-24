//! ProtonDB tiers for Steam games (keyless): parsing of the summaries API and the small on-disk
//! cache that remembers when each appid was last asked, so a game is looked up at most once per
//! `REFRESH_SECS` in the background (`Download metadata…` may ask again after `MANUAL_SECS`).
//!
//! API: `https://www.protondb.com/api/v1/reports/summaries/<appid>.json` →
//! `{"tier":"gold","trendingTier":"platinum","bestReportedTier":"platinum","score":…,"total":…}`;
//! 404 for games without reports.
//!
//! Cache file `<art_dir>/protondb.json`: `{"version":1,"games":{"<appid>":{"tier":"gold","at":<unix>}}}`
//! (`tier` absent = ProtonDB had no report). Written by the metadata worker only.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const API: &str = "https://www.protondb.com/api/v1/reports/summaries";
/// Background refresh: at most once per ~2 weeks per game.
pub const REFRESH_SECS: i64 = 14 * 24 * 3600;
/// User-initiated (`Download metadata…`): at most once a day per game.
pub const MANUAL_SECS: i64 = 24 * 3600;
pub const CACHE_FILE: &str = "protondb.json";

/// Tiers ProtonDB reports, best first.
pub const TIERS: [&str; 7] = ["native", "platinum", "gold", "silver", "bronze", "borked", "pending"];

pub fn url(appid: u32) -> String {
    format!("{API}/{appid}.json")
}

/// None = not a summaries response; Some(None) = valid but without a known tier.
pub fn parse_summary(body: &[u8]) -> Option<Option<String>> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let obj = v.as_object()?;
    let tier = |k: &str| {
        obj.get(k)
            .and_then(|t| t.as_str())
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| TIERS.contains(&t.as_str()))
    };
    // "tier" is what the site shows; the others only when it is missing / not a tier.
    Some(tier("tier").or_else(|| tier("trendingTier")).or_else(|| tier("bestReportedTier")))
}

/// Display name of a stored tier ("gold" → "Gold"); None for unknown values.
pub fn tier_label(tier: &str) -> Option<&'static str> {
    Some(match tier.trim().to_ascii_lowercase().as_str() {
        "native" => "Native",
        "platinum" => "Platinum",
        "gold" => "Gold",
        "silver" => "Silver",
        "bronze" => "Bronze",
        "borked" => "Borked",
        "pending" => "Pending",
        _ => return None,
    })
}

/// ProtonDB's own tier colours (sRGB), for chips.
pub fn tier_rgb(tier: &str) -> Option<[u8; 3]> {
    Some(match tier.trim().to_ascii_lowercase().as_str() {
        "native" => [0x29, 0xb6, 0x6e],
        "platinum" => [0xb4, 0xc7, 0xdc],
        "gold" => [0xcf, 0xb5, 0x3b],
        "silver" => [0xa6, 0xa6, 0xa6],
        "bronze" => [0xcd, 0x7f, 0x32],
        "borked" => [0xd9, 0x3b, 0x3b],
        "pending" => [0x5a, 0x5f, 0x6a],
        _ => return None,
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    /// Unix seconds of the last definitive answer.
    pub at: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    games: HashMap<String, Entry>,
}

/// appid → last answer. Shared between the UI thread (cheap staleness checks before queueing)
/// and the worker (authoritative check, updates, saving).
#[derive(Debug, Default)]
pub struct Cache {
    pub entries: HashMap<u32, Entry>,
    /// Updates since the last save.
    pub dirty: usize,
}

impl Cache {
    pub fn path(art_dir: &Path) -> PathBuf {
        art_dir.join(CACHE_FILE)
    }

    /// Missing / unreadable file = empty cache (everything gets asked again, politely).
    pub fn load(path: &Path) -> Cache {
        let Ok(bytes) = std::fs::read(path) else {
            return Cache::default();
        };
        let Ok(f) = serde_json::from_slice::<CacheFile>(&bytes) else {
            log::warn!("{}: unreadable, starting over", path.display());
            return Cache::default();
        };
        Cache { entries: f.games.into_iter().filter_map(|(k, v)| Some((k.parse().ok()?, v))).collect(), dirty: 0 }
    }

    pub fn to_json(&self) -> Vec<u8> {
        let f = CacheFile { version: 1, games: self.entries.iter().map(|(k, v)| (k.to_string(), v.clone())).collect() };
        serde_json::to_vec(&f).unwrap_or_default()
    }

    /// Asked less than `max_age` seconds ago. A timestamp far in the future (the clock was
    /// wrong when it was written) counts as stale, so it can't block refreshes for good.
    pub fn is_fresh(&self, appid: u32, max_age: i64, now: i64) -> bool {
        self.entries.get(&appid).is_some_and(|e| {
            let age = now - e.at;
            (-24 * 3600..max_age).contains(&age)
        })
    }

    pub fn get(&self, appid: u32) -> Option<&Entry> {
        self.entries.get(&appid)
    }

    pub fn record(&mut self, appid: u32, tier: Option<String>, now: i64) {
        self.entries.insert(appid, Entry { tier, at: now });
        self.dirty += 1;
    }
}
