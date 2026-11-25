//! Names and kinds of Steam apps that Steam's app cache doesn't describe (it may be missing, in a
//! layout this version can't read, or simply not know an app). The scanner only ever reads
//! files here; the background job in `steam_names.rs` fills the two caches online.
//!
//! Sources, best first:
//!   * Steam's own desktop shortcuts (`~/.local/share/applications`, `~/Desktop`) with
//!     `steam://rungameid/<appid>`: real names, written by the Steam client for games,
//!   * `<cache>/steam-app-names.json`: names + store type from Steam's keyless store API
//!     (`IStoreBrowseService/GetItems`), asked only for apps no other source names,
//!   * `<cache>/steam-owned-games.json`: the account's owned games (`IPlayerService/
//!     GetOwnedGames`, with the user's own Steam Web API key), also games never started here.
//!
//! Not used: `appcache/stats/UserGameStatsSchema_<appid>.bin` "gamename" is often an internal
//! code name ("Vena Cava" for CODE VEIN), and `userdata/<id>/config/librarycache/<appid>.json`
//! holds achievements, news and descriptions but no names or app types.

use super::super::ScanContext;
use crate::model::Source;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const NAMES_FILE: &str = "steam-app-names.json";
pub const OWNED_FILE: &str = "steam-owned-games.json";
/// An app the store didn't know is asked about again after this long.
pub const RECHECK_SECS: i64 = 7 * 24 * 3600;

/// One store answer: the name and type (`appinfo` words: "game", "dlc", "demo", "music" …).
/// `name: None` = the store doesn't know the app (tools, servers and runtimes have no page).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StoreName {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Unix seconds of the answer.
    #[serde(default)]
    pub checked: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NameCache {
    #[serde(default)]
    pub apps: BTreeMap<u32, StoreName>,
}

/// One owned game from `GetOwnedGames`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OwnedGame {
    pub appid: u32,
    pub name: String,
    /// Minutes played on every PC.
    #[serde(default)]
    pub playtime_mins: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_played: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OwnedList {
    /// Unix seconds.
    pub fetched: i64,
    pub games: Vec<OwnedGame>,
}

/// Owned games per Steam account id (the `userdata/<id>` number).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OwnedCache {
    #[serde(default)]
    pub accounts: BTreeMap<u32, OwnedList>,
}

fn load<T: Default + serde::de::DeserializeOwned>(path: &Path) -> T {
    let Ok(bytes) = crate::scan::util::read_bytes(path) else {
        return T::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        log::warn!("{}: {e}", path.display());
        T::default()
    })
}

pub fn load_names(cache_dir: &Path) -> NameCache {
    load(&cache_dir.join(NAMES_FILE))
}

pub fn load_owned(cache_dir: &Path) -> OwnedCache {
    load(&cache_dir.join(OWNED_FILE))
}

fn save<T: Serialize>(path: &Path, v: &T) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(v).map_err(std::io::Error::other)?;
    crate::fsutil::write_atomic(path, &bytes)
}

pub fn save_names(cache_dir: &Path, c: &NameCache) -> std::io::Result<()> {
    save(&cache_dir.join(NAMES_FILE), c)
}

pub fn save_owned(cache_dir: &Path, c: &OwnedCache) -> std::io::Result<()> {
    save(&cache_dir.join(OWNED_FILE), c)
}

/// What the Steam scanner found missing, for the background job (shared by the clones of one
/// `ScanContext`; read after `run_all`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Needs {
    /// Apps in the account's list that no source names yet (or that the store didn't know a
    /// week ago or more).
    pub names: BTreeSet<u32>,
}

/// The Steam scanner's link to Lodestar's cache folder.
#[derive(Clone, Debug, Default)]
pub struct Shared {
    /// Lodestar's cache folder (None in tests that don't use the caches).
    pub cache_dir: Option<PathBuf>,
    pub needs: Arc<Mutex<Needs>>,
}

impl Shared {
    pub fn new(cache_dir: PathBuf) -> Shared {
        Shared { cache_dir: Some(cache_dir), needs: Default::default() }
    }
}

/// Steam's desktop shortcuts: appid -> the shortcut's name.
pub fn desktop_names(ctx: &ScanContext) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    for dir in [ctx.data_home().join("applications"), ctx.home.join("Desktop")] {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("desktop"))
            .collect();
        files.sort();
        for p in files {
            let Some(e) = crate::scan::desktop::read_entry_file(&p) else { continue };
            let Some(exec) = e.get("Exec") else { continue };
            let argv = crate::scan::desktop::parse_exec(exec);
            let Some((Source::Steam, id)) = crate::scan::desktop::shortcut_target(&argv) else { continue };
            let (Ok(id), Some(name)) = (id.parse::<u32>(), e.string("Name")) else { continue };
            let name = name.trim();
            if !name.is_empty() {
                out.entry(id).or_insert_with(|| name.to_string());
            }
        }
    }
    out
}

/// Names that are add-ons, tools or media rather than games, for apps whose type no source
/// says. Careful on purpose: a stray tool is better than a missing game.
pub fn non_game_name(name: &str) -> bool {
    if super::TOOL_NAME_PREFIXES.iter().any(|p| name.starts_with(p)) {
        return true;
    }
    let lower = name.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |w: &str| words.contains(&w);
    let pair = |a: &str, b: &str| words.windows(2).any(|p| p[0] == a && p[1] == b);
    has("soundtrack")
        || has("ost")
        || has("soundtracks")
        || has("artbook")
        || has("sdk")
        || has("dlc")
        || has("demo")
        || pair("dedicated", "server")
        || pair("season", "pass")
        || pair("art", "book")
        || pair("mod", "tools")
        || pair("original", "score")
        || words.last() == Some(&"server")
}

/// Where a game's name came from (for the scan's log line).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameSource {
    AppInfo,
    Desktop,
    Store,
    Owned,
}

/// A name from the fallback sources, with the type when one says it.
#[derive(Clone, Debug, PartialEq)]
pub struct Named {
    pub name: String,
    /// "game" / "dlc" …; None = unknown (filter by name).
    pub kind: Option<String>,
    pub source: NameSource,
}

/// What the fallback sources know about one app.
#[derive(Clone, Debug, PartialEq)]
pub enum Lookup {
    Named(Named),
    /// The store said it has no such app (within the last week).
    NotOnStore,
    /// Nobody knows it yet: ask the store.
    Unknown,
}

/// The fallback sources for one scan of a root, read on first use.
pub struct Fallback<'a> {
    ctx: &'a ScanContext,
    owned: HashMap<u32, &'a OwnedGame>,
    desktop: Option<HashMap<u32, String>>,
    store: Option<NameCache>,
    now: i64,
}

impl<'a> Fallback<'a> {
    pub fn new(ctx: &'a ScanContext, owned: &'a [OwnedGame], now: i64) -> Fallback<'a> {
        Fallback { ctx, owned: owned.iter().map(|g| (g.appid, g)).collect(), desktop: None, store: None, now }
    }

    pub fn lookup(&mut self, appid: u32) -> Lookup {
        let ctx = self.ctx;
        let desktop = self.desktop.get_or_insert_with(|| desktop_names(ctx)).get(&appid).cloned();
        let store = self
            .store
            .get_or_insert_with(|| ctx.steam_names.cache_dir.as_deref().map(load_names).unwrap_or_default())
            .apps
            .get(&appid)
            .cloned();
        let owned = self.owned.get(&appid).map(|g| g.name.trim().to_string()).filter(|n| !n.is_empty());
        let kind = store.as_ref().and_then(|s| s.kind.clone()).or_else(|| owned.as_ref().map(|_| "game".to_string()));
        let store_name = store.as_ref().and_then(|s| s.name.clone()).filter(|n| !n.trim().is_empty());
        let named = |name: String, source| Lookup::Named(Named { name, kind: kind.clone(), source });
        if let Some(n) = desktop {
            return named(n, NameSource::Desktop);
        }
        if let Some(n) = store_name {
            return named(n.trim().to_string(), NameSource::Store);
        }
        if let Some(n) = owned {
            return named(n, NameSource::Owned);
        }
        match store {
            Some(s) if (0..RECHECK_SECS).contains(&(self.now - s.checked)) => Lookup::NotOnStore,
            _ => Lookup::Unknown,
        }
    }
}

/// The owned games the background job saved for this account.
pub fn owned_games(ctx: &ScanContext, account: Option<u32>) -> Vec<OwnedGame> {
    let (Some(dir), Some(account)) = (ctx.steam_names.cache_dir.as_deref(), account) else {
        return Vec::new();
    };
    load_owned(dir).accounts.remove(&account).map(|l| l.games).unwrap_or_default()
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_that_are_not_games() {
        for n in [
            "Proton 9.0",
            "Steam Linux Runtime 3.0 (sniper)",
            "Hollow Knight - Official Soundtrack",
            "DOOM Eternal OST",
            "Elden Ring Digital Artbook & Original Soundtrack",
            "Team Fortress 2 Dedicated Server",
            "ARK Survival Evolved Dedicated Server",
            "Source SDK",
            "Portal 2 - Demo",
            "Cyberpunk 2077 Season Pass",
            "The Witcher 3 Art Book",
            "Some Game DLC",
            "Half-Life 2: Deathmatch Server",
        ] {
            assert!(non_game_name(n), "{n}");
        }
        for n in [
            "Baldur's Gate 3",
            "The Jackbox Party Pack 3",
            "Demon's Souls",
            "Demolish & Build",
            "Lost Ark",
            "Server Wars: The Game",
            "Hades",
            "Postal 2",
        ] {
            assert!(!non_game_name(n), "{n}");
        }
    }
}
