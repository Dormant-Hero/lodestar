//! Background lookups that help the Steam scanner name games Steam's own files don't describe
//! (scan/steam/names.rs reads what this writes; scanners never go online):
//!
//!   * names + store types of apps no local source names, from Steam's keyless store API
//!     `IStoreBrowseService/GetItems` (up to 100 apps per request, one request a second).
//!     `ISteamApps/GetAppList/v2` (the old full app list) is gone (404 since 2025), and
//!     `IStoreService/GetAppList` needs a key, so only the apps that are needed are asked for.
//!     Known names are kept; apps the store doesn't know are asked again after a week.
//!   * with the user's own Steam Web API key: every game the account owns
//!     (`IPlayerService/GetOwnedGames`, `include_appinfo=1`, `include_played_free_games=1`),
//!     also games never started on this PC. For the key owner's own account Steam answers even
//!     when the profile is private; for another account its "Game details" must be public.
//!     Refreshed at most once a day, or at once when the key changes.
//!
//! The key goes only into the request URL; error texts (transport errors quote the URL) are
//! passed through `config::redact` before they are logged or shown. Screenshot runs
//! (`art::net::local_only`) never start a job.

use crate::art::net::{self, MAX_JSON_BYTES, Net, Throttle};
use crate::config::redact;
use crate::scan::steam::names::{self, NameCache, OwnedCache, OwnedGame, OwnedList, StoreName};
use std::path::PathBuf;

const API: &str = "https://api.steampowered.com";
/// Apps per `GetItems` request.
pub const BATCH: usize = 100;
/// The owned-games list is asked for again after this long.
pub const OWNED_TTL: i64 = 24 * 3600;
/// Steam64 id of account id 0.
const STEAMID64_BASE: u64 = 76_561_197_960_265_728;

/// One run of the background job.
#[derive(Clone, Default)]
pub struct Job {
    pub cache_dir: PathBuf,
    /// Apps to ask the store about.
    pub names: Vec<u32>,
    /// Accounts (`userdata/<id>`) whose owned games to fetch (needs `key`).
    pub owned: Vec<u32>,
    pub key: Option<String>,
    /// Values to scrub from every text (the key).
    pub secrets: Vec<String>,
    /// Screenshot runs: never go online.
    pub local_only: bool,
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Job")
            .field("names", &self.names.len())
            .field("owned", &self.owned)
            .field("key", &self.key.as_ref().map(|_| "<set>"))
            .finish()
    }
}

/// What a run did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    /// Apps the store named (new names for the library).
    pub names_found: usize,
    /// Apps the store was asked about.
    pub names_asked: usize,
    /// The saved owned-games list changed.
    pub owned_changed: bool,
    /// Owned games in the newest list (per run, all accounts).
    pub owned_count: Option<usize>,
    /// Why something failed, in plain words, without the key.
    pub error: Option<String>,
}

impl Outcome {
    /// The Steam library should be scanned again.
    pub fn library_changed(&self) -> bool {
        self.names_found > 0 || self.owned_changed
    }
}

/// The store's app type number (`EStoreAppType`) as the word appinfo.vdf uses.
pub fn store_kind(t: i64) -> Option<&'static str> {
    Some(match t {
        0 => "game",
        1 => "demo",
        2 => "mod",
        3 | 7 => "video",
        4 => "dlc",
        5 => "guide",
        6 => "application",
        8 => "series",
        9 => "episode",
        10 => "hardware",
        11 => "music",
        12 => "beta",
        13 => "tool",
        14 => "advertising",
        _ => return None,
    })
}

pub fn get_items_url(ids: &[u32]) -> String {
    let input = serde_json::json!({
        "ids": ids.iter().map(|id| serde_json::json!({ "appid": id })).collect::<Vec<_>>(),
        "context": { "language": "english", "country_code": "US", "steam_realm": 1 },
        "data_request": {},
    });
    format!("{API}/IStoreBrowseService/GetItems/v1/?input_json={}", net::encode(&input.to_string()))
}

/// A `GetItems` answer: every asked id gets an entry (apps the store doesn't know: no name).
pub fn parse_get_items(v: &serde_json::Value, asked: &[u32], now: i64) -> Vec<(u32, StoreName)> {
    let items = v.get("response").and_then(|r| r.get("store_items")).and_then(|a| a.as_array());
    let mut found: std::collections::HashMap<u32, StoreName> = Default::default();
    for it in items.into_iter().flatten() {
        let Some(id) = it.get("id").or_else(|| it.get("appid")).and_then(|i| i.as_u64()) else { continue };
        let Ok(id) = u32::try_from(id) else { continue };
        let ok = it.get("success").and_then(|s| s.as_i64()) == Some(1);
        let name = it.get("name").and_then(|n| n.as_str()).map(str::trim).filter(|n| ok && !n.is_empty());
        let kind = it.get("type").and_then(|t| t.as_i64()).and_then(store_kind);
        found.insert(
            id,
            StoreName { name: name.map(str::to_string), kind: name.and(kind).map(str::to_string), checked: now },
        );
    }
    asked
        .iter()
        .map(|&id| (id, found.remove(&id).unwrap_or(StoreName { checked: now, ..Default::default() })))
        .collect()
}

pub fn owned_url(key: &str, account: u32) -> String {
    format!(
        "{API}/IPlayerService/GetOwnedGames/v1/?key={}&steamid={}&include_appinfo=1&include_played_free_games=1&format=json",
        net::encode(key),
        STEAMID64_BASE + account as u64
    )
}

/// A `GetOwnedGames` answer. `Err` when Steam didn't share the list (an empty `response`).
pub fn parse_owned(v: &serde_json::Value) -> Result<Vec<OwnedGame>, String> {
    let r = v.get("response").ok_or("Steam sent no owned-games list")?;
    let Some(games) = r.get("games").and_then(|g| g.as_array()) else {
        if r.get("game_count").and_then(|c| c.as_u64()) == Some(0) {
            return Ok(Vec::new());
        }
        return Err("Steam didn't share the list of your games. Set your Steam profile's Game details to Public \
                    (Steam → your profile → Edit Profile → Privacy Settings)."
            .into());
    };
    let mut out: Vec<OwnedGame> = games
        .iter()
        .filter_map(|g| {
            let appid = u32::try_from(g.get("appid")?.as_u64()?).ok()?;
            let name = g.get("name").and_then(|n| n.as_str()).unwrap_or("").trim().to_string();
            Some(OwnedGame {
                appid,
                name,
                playtime_mins: g.get("playtime_forever").and_then(|p| p.as_u64()).unwrap_or(0),
                last_played: g.get("rtime_last_played").and_then(|t| t.as_i64()).filter(|&t| t > 0),
            })
        })
        .filter(|g| !g.name.is_empty())
        .collect();
    out.sort_by_key(|g| g.appid);
    out.dedup_by_key(|g| g.appid);
    Ok(out)
}

fn get_json(net: &dyn Net, throttle: &Throttle, url: &str, secrets: &[String]) -> Result<serde_json::Value, String> {
    throttle.wait(url, false, None);
    let r = net.get(url, None, MAX_JSON_BYTES).map_err(|e| redact(&e, secrets))?;
    match r.status {
        200 => serde_json::from_slice(&r.body).map_err(|e| format!("Steam sent an answer Lodestar can't read ({e})")),
        401 | 403 => Err("Steam didn't accept the Web API key".into()),
        429 => Err("Steam asked to slow down; trying again later".into()),
        s => Err(format!("Steam answered HTTP {s}")),
    }
}

/// Run the job: ask what it says, save the caches. Never panics; failures leave the caches as
/// they were (names asked so far are kept).
pub fn run(net: &dyn Net, throttle: &Throttle, job: &Job) -> Outcome {
    let mut out = Outcome::default();
    if job.local_only {
        return out;
    }
    let now = names::now();
    if !job.names.is_empty() {
        let mut cache: NameCache = names::load_names(&job.cache_dir);
        let mut changed = false;
        for chunk in job.names.chunks(BATCH) {
            match get_json(net, throttle, &get_items_url(chunk), &job.secrets) {
                Ok(v) => {
                    for (id, e) in parse_get_items(&v, chunk, now) {
                        out.names_asked += 1;
                        if e.name.is_some() && cache.apps.get(&id).is_none_or(|old| old.name != e.name) {
                            out.names_found += 1;
                        }
                        cache.apps.insert(id, e);
                        changed = true;
                    }
                }
                Err(e) => {
                    log::info!("steam names: store lookup failed: {e}");
                    out.error.get_or_insert(e);
                    break;
                }
            }
        }
        if changed && let Err(e) = names::save_names(&job.cache_dir, &cache) {
            log::warn!("steam names: saving {}: {e}", names::NAMES_FILE);
        }
    }
    if let Some(key) = job.key.as_deref().map(str::trim).filter(|k| !k.is_empty())
        && !job.owned.is_empty()
    {
        let mut cache: OwnedCache = names::load_owned(&job.cache_dir);
        let mut changed = false;
        for &account in &job.owned {
            let res = get_json(net, throttle, &owned_url(key, account), &job.secrets).and_then(|v| parse_owned(&v));
            match res {
                Ok(games) => {
                    *out.owned_count.get_or_insert(0) += games.len();
                    let old = cache.accounts.get(&account).map(|l| &l.games);
                    if old != Some(&games) {
                        out.owned_changed = true;
                    }
                    cache.accounts.insert(account, OwnedList { fetched: now, games });
                    changed = true;
                }
                Err(e) => {
                    let e = redact(&e, &job.secrets);
                    log::info!("steam names: owned games of account {account}: {e}");
                    out.error = Some(e);
                }
            }
        }
        if changed && let Err(e) = names::save_owned(&job.cache_dir, &cache) {
            log::warn!("steam names: saving {}: {e}", names::OWNED_FILE);
        }
    }
    log::info!(
        "steam names: asked the store about {} apps ({} named){}",
        out.names_asked,
        out.names_found,
        match out.owned_count {
            Some(n) => format!(", {n} owned games{}", if out.owned_changed { " (changed)" } else { "" }),
            None => String::new(),
        }
    );
    out
}

/// The accounts whose owned-games list is older than [`OWNED_TTL`] (or missing); `force`: all.
pub fn owned_due(cache_dir: &std::path::Path, accounts: &[u32], force: bool, now: i64) -> Vec<u32> {
    let cache = names::load_owned(cache_dir);
    accounts
        .iter()
        .copied()
        .filter(|a| force || cache.accounts.get(a).is_none_or(|l| !(0..OWNED_TTL).contains(&(now - l.fetched))))
        .collect()
}

/// A Steam Web API key looks complete (32 hex digits): only then is it tried, so typing one
/// doesn't send a request per key press.
pub fn key_looks_complete(key: &str) -> bool {
    let k = key.trim();
    k.len() == 32 && k.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The accounts the Steam scanner lists games of (one per Steam root).
pub fn scanned_accounts(ctx: &crate::scan::ScanContext) -> Vec<u32> {
    let mut out: Vec<u32> = crate::scan::steam::find_roots(ctx)
        .iter()
        .filter_map(|r| crate::scan::steam::main_user(&r.path))
        .filter_map(|u| u.file_name()?.to_str()?.parse().ok())
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests;
