//! Steam's web side: the keyless achievement list and global percentages, the player's unlocks
//! with the user's own Steam Web API key, and achievement pictures. Answers are cached under
//! `<cache>/steam/<appid>/` (lists and percentages a week, the player's unlocks half an hour,
//! pictures forever: their names are content hashes).
//!
//! The key goes only into the request URL; every text that could hold it (transport errors quote
//! the URL) is passed through `config::redact` before it is logged or shown.

use crate::art::net::{self, HttpResponse, MAX_JSON_BYTES};
use crate::config::redact;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const API: &str = "https://api.steampowered.com";
const ICON_HOST: &str = "https://shared.steamstatic.com/community_assets/images/apps";
/// Achievement pictures are 64×64 JPEGs of a few KB.
const MAX_ICON_BYTES: u64 = 2 << 20;
/// Pictures downloaded at once (first page open of a game with 100+ achievements).
const ICON_THREADS: usize = 8;
/// Picture downloads stop after this long per page open (the rest come on a later one), so a
/// game with 700 achievements on a slow line never holds up the next game's page for long.
const ICON_BUDGET: Duration = Duration::from_secs(15);
pub const WEEK: i64 = 7 * 24 * 3600;
pub const PLAYER_TTL: i64 = 30 * 60;

/// What a lookup needs: transport, where to cache, and whether going online is allowed.
pub struct Web<'a> {
    pub net: &'a dyn net::Net,
    /// `<cache>/steam/<appid>`.
    pub dir: PathBuf,
    pub appid: u32,
    /// Screenshot runs: only this machine (`art::net::local_only`).
    pub local_only: bool,
    /// Values to scrub from every text (the Steam Web API key).
    pub secrets: Vec<String>,
}

/// Why a web answer is missing.
#[derive(Clone, Debug, PartialEq)]
pub enum Fail {
    /// Offline / network error / server trouble: try again later.
    Transient(String),
    /// The service said no (404, 403, `success: false`).
    Refused(String),
}

impl Web<'_> {
    fn get(&self, url: &str, limit: u64) -> Result<HttpResponse, Fail> {
        if self.local_only && !net::is_local(url) {
            return Err(Fail::Transient("going online is off in this run".into()));
        }
        self.net.get(url, None, limit).map_err(|e| Fail::Transient(redact(&e, &self.secrets)))
    }

    /// GET a JSON document; non-200 is a failure.
    fn json(&self, url: &str) -> Result<serde_json::Value, Fail> {
        let r = self.get(url, MAX_JSON_BYTES)?;
        if r.status != 200 {
            let why = format!("HTTP {}", r.status);
            return Err(if net::is_definitive(r.status) { Fail::Refused(why) } else { Fail::Transient(why) });
        }
        serde_json::from_slice(&r.body).map_err(|e| Fail::Transient(format!("bad answer: {e}")))
    }

    fn cache_path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// A cached answer younger than `ttl` seconds.
    fn cached<T: DeserializeOwned>(&self, name: &str, ttl: i64) -> Option<T> {
        let bytes = std::fs::read(self.cache_path(name)).ok()?;
        let c: Cached<T> = serde_json::from_slice(&bytes).ok()?;
        let age = now() - c.fetched;
        (0..ttl).contains(&age).then_some(c.data)
    }

    fn store<T: Serialize>(&self, name: &str, data: &T) {
        let c = Cached { fetched: now(), data };
        if let Ok(bytes) = serde_json::to_vec(&c)
            && let Err(e) = crate::fsutil::write_atomic(&self.cache_path(name), &bytes)
        {
            log::debug!("achievements: cache {name}: {e}");
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Cached<T> {
    fetched: i64,
    data: T,
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// One achievement from `IPlayerService/GetGameAchievements` (no player data).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WebAch {
    pub id: String,
    pub name: String,
    pub desc: String,
    pub hidden: bool,
    pub icon: Option<String>,
    pub icon_gray: Option<String>,
    pub percent: Option<f32>,
}

/// A number Steam may send as a string ("74.0").
fn num(v: Option<&serde_json::Value>) -> Option<f64> {
    let v = v?;
    v.as_f64().or_else(|| v.as_str()?.trim().parse().ok())
}

fn text(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn percent(v: Option<&serde_json::Value>) -> Option<f32> {
    num(v).filter(|p| (0.0..=100.0).contains(p)).map(|p| p as f32)
}

/// The game's achievement list in `lang` (keyless). Empty = the game has none.
pub fn game_achievements(w: &Web, lang: &str) -> Result<Vec<WebAch>, Fail> {
    let name = format!("schema-{lang}.json");
    if let Some(v) = w.cached(&name, WEEK) {
        return Ok(v);
    }
    let url = format!("{API}/IPlayerService/GetGameAchievements/v1/?appid={}&language={lang}", w.appid);
    let v = w.json(&url)?;
    let list = parse_game_achievements(&v);
    w.store(&name, &list);
    Ok(list)
}

pub fn parse_game_achievements(v: &serde_json::Value) -> Vec<WebAch> {
    let items = v.get("response").and_then(|r| r.get("achievements")).and_then(|a| a.as_array());
    items
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let id = text(a, "internal_name")?;
            Some(WebAch {
                name: text(a, "localized_name").unwrap_or_else(|| id.clone()),
                desc: text(a, "localized_desc").unwrap_or_default(),
                hidden: a.get("hidden").is_some_and(|h| h.as_bool().unwrap_or_else(|| h.as_i64().unwrap_or(0) != 0)),
                icon: text(a, "icon"),
                icon_gray: text(a, "icon_gray"),
                percent: percent(a.get("player_percent_unlocked")),
                id,
            })
        })
        .collect()
}

/// Global unlock percentages (API name → 0..=100), keyless. Empty = Steam has none for the game.
pub fn global_percentages(w: &Web) -> Result<HashMap<String, f32>, Fail> {
    if let Some(v) = w.cached("global.json", WEEK) {
        return Ok(v);
    }
    let url = format!("{API}/ISteamUserStats/GetGlobalAchievementPercentagesForApp/v2/?gameid={}", w.appid);
    let map = match w.json(&url) {
        Ok(v) => parse_global_percentages(&v),
        // Steam answers 403 for games without achievements: remember that too.
        Err(Fail::Refused(_)) => HashMap::new(),
        Err(e) => return Err(e),
    };
    w.store("global.json", &map);
    Ok(map)
}

pub fn parse_global_percentages(v: &serde_json::Value) -> HashMap<String, f32> {
    let items = v.get("achievementpercentages").and_then(|r| r.get("achievements")).and_then(|a| a.as_array());
    items.into_iter().flatten().filter_map(|a| Some((text(a, "name")?, percent(a.get("percent"))?))).collect()
}

/// The player's unlocks (API name → unix seconds; 0 = time unknown) with the user's own key.
/// Needs the profile's "Game details" to be public. `force`: skip the cached answer.
pub fn player_achievements(
    w: &Web,
    key: &str,
    account: u32,
    lang: &str,
    force: bool,
) -> Result<HashMap<String, i64>, Fail> {
    let name = format!("player-{account}.json");
    if !force && let Some(v) = w.cached(&name, PLAYER_TTL) {
        return Ok(v);
    }
    let sid = super::local::STEAMID64_BASE + account as u64;
    let url = format!(
        "{API}/ISteamUserStats/GetPlayerAchievements/v1/?key={}&steamid={sid}&appid={}&l={lang}",
        net::encode(key),
        w.appid
    );
    let map = parse_player_achievements(&w.json(&url)?).map_err(|e| Fail::Refused(redact(&e, &w.secrets)))?;
    w.store(&name, &map);
    Ok(map)
}

pub fn parse_player_achievements(v: &serde_json::Value) -> Result<HashMap<String, i64>, String> {
    let stats = v.get("playerstats").ok_or("no player stats")?;
    if stats.get("success").and_then(|s| s.as_bool()) == Some(false) {
        return Err(text(stats, "error").unwrap_or_else(|| "Steam said no".into()));
    }
    Ok(stats
        .get("achievements")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
        .filter(|a| num(a.get("achieved")).is_some_and(|n| n != 0.0))
        .filter_map(|a| Some((text(a, "apiname")?, num(a.get("unlocktime")).unwrap_or(0.0).max(0.0) as i64)))
        .collect())
}

/// A picture name from a file or Steam's answer, safe as a file name: `<hash>.jpg`.
pub fn icon_file_name(name: &str) -> Option<String> {
    let name = name.trim();
    let ok = !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.');
    if !ok {
        return None;
    }
    let lower = name.to_ascii_lowercase();
    Some(if [".jpg", ".jpeg", ".png"].iter().any(|e| lower.ends_with(e)) {
        name.to_string()
    } else {
        format!("{name}.jpg")
    })
}

/// Local copies of the pictures `names` (Steam's file names, most wanted first): already
/// downloaded ones as they are, the rest fetched now within [`ICON_BUDGET`]. Returns name → path
/// for every picture there is, and whether all could be tried (false: the network failed or time
/// ran out, so the rest come next time).
pub fn icons(w: &Web, names: &[String]) -> (HashMap<String, PathBuf>, bool) {
    icons_within(w, names, ICON_BUDGET)
}

pub(crate) fn icons_within(w: &Web, names: &[String], budget: Duration) -> (HashMap<String, PathBuf>, bool) {
    let mut out = HashMap::new();
    let mut todo: Vec<(String, String, PathBuf)> = Vec::new();
    for n in names {
        let Some(file) = icon_file_name(n) else {
            continue;
        };
        let path = w.dir.join(&file);
        if is_image_file(&path) {
            out.insert(n.clone(), path);
        } else if !todo.iter().any(|(m, _, _)| m == n) {
            todo.push((n.clone(), file, path));
        }
    }
    if todo.is_empty() {
        return (out, true);
    }
    if w.local_only {
        return (out, true);
    }
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let started = Instant::now();
    let got = parking_lot::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..ICON_THREADS.min(todo.len()) {
            s.spawn(|| {
                while !stop.load(Ordering::Relaxed) {
                    if started.elapsed() >= budget {
                        // A game with hundreds of achievements on a slow line: the page (and the
                        // next game's) shouldn't wait; the rest come on a later open.
                        stop.store(true, Ordering::Relaxed);
                        break;
                    }
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some((name, file, path)) = todo.get(i) else {
                        break;
                    };
                    match download_icon(w, file, path) {
                        Ok(true) => got.lock().push((name.clone(), path.clone())),
                        Ok(false) => {}
                        Err(e) => {
                            log::debug!("achievements: picture {file} for {}: {e}", w.appid);
                            stop.store(true, Ordering::Relaxed);
                        }
                    }
                }
            });
        }
    });
    out.extend(got.into_inner());
    (out, !stop.load(Ordering::Relaxed))
}

/// Ok(false): Steam has no such picture. Err: network trouble.
fn download_icon(w: &Web, file: &str, path: &Path) -> Result<bool, String> {
    let url = format!("{ICON_HOST}/{}/{file}", w.appid);
    let r = w.get(&url, MAX_ICON_BYTES).map_err(|f| match f {
        Fail::Transient(e) | Fail::Refused(e) => e,
    })?;
    if r.status != 200 {
        return if net::is_definitive(r.status) { Ok(false) } else { Err(format!("HTTP {}", r.status)) };
    }
    if !looks_like_image(&r.body) {
        return Ok(false);
    }
    crate::fsutil::write_atomic(path, &r.body).map_err(|e| e.to_string())?;
    Ok(true)
}

fn looks_like_image(b: &[u8]) -> bool {
    b.starts_with(&[0xFF, 0xD8, 0xFF]) || b.starts_with(b"\x89PNG") || b.starts_with(b"GIF8")
}

fn is_image_file(p: &Path) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() > 0)
}
