//! GeForce NOW's public list of supported games (no key, no login): NVIDIA publishes it as JSON
//! (`[{"id":…,"title":"7 Days to Die","store":"Steam","status":"AVAILABLE",…}]`, ~400 KB).
//! Add game → Cloud game… suggests names from it while typing, so a GeForce NOW game gets the
//! exact title (artwork and metadata are then found by name). Cached for a week in
//! `<cache>/cloud/gfn-games.json`; fetched on a helper thread, never in screenshot runs.

use crate::art::net::Net;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const LIST_URL: &str = "https://static.nvidiagrid.net/supported-public-game-list/locales/gfnpc-en-US.json";
/// How long a cached list is used before it is fetched again.
pub const FRESH: Duration = Duration::from_secs(7 * 24 * 3600);
const LIMIT: u64 = 16 * 1024 * 1024;

pub fn cache_file(cache_dir: &Path) -> PathBuf {
    cache_dir.join("cloud").join("gfn-games.json")
}

/// The available games' titles (sorted, without duplicates).
pub fn parse_titles(json: &[u8]) -> Result<Vec<String>, String> {
    #[derive(serde::Deserialize)]
    struct Item {
        #[serde(default)]
        title: String,
        #[serde(default)]
        status: Option<String>,
    }
    let items: Vec<Item> = serde_json::from_slice(json).map_err(|e| format!("GeForce NOW's game list: {e}"))?;
    let mut v: Vec<String> = items
        .into_iter()
        .filter(|i| i.status.as_deref().is_none_or(|s| s.eq_ignore_ascii_case("AVAILABLE")))
        // "Cyberpunk 2077®" → "Cyberpunk 2077": names as other sources spell them.
        .map(|i| i.title.replace(['®', '™', '©'], "").split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|t| !t.is_empty())
        .collect();
    v.sort_by_key(|t| t.to_lowercase());
    v.dedup();
    Ok(v)
}

/// The titles: from the cache when it is fresh, else fetched from `url` (and cached); a stale
/// cache is used when fetching fails. `offline`: never fetch.
pub fn load(cache_dir: &Path, net: &dyn Net, url: &str, offline: bool) -> Result<Vec<String>, String> {
    let f = cache_file(cache_dir);
    let age =
        std::fs::metadata(&f).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok());
    let cached = || std::fs::read(&f).ok().and_then(|b| parse_titles(&b).ok());
    if offline || age.is_some_and(|a| a < FRESH) {
        if let Some(v) = cached() {
            return Ok(v);
        }
        if offline {
            return Ok(vec![]);
        }
    }
    let fetched = net.get(url, None, LIMIT).and_then(|r| {
        if r.status != 200 {
            return Err(format!("GeForce NOW's game list: HTTP {}", r.status));
        }
        let v = parse_titles(&r.body)?;
        if let Err(e) = crate::fsutil::write_atomic(&f, &r.body) {
            log::debug!("GeForce NOW cache {}: {e}", f.display());
        }
        Ok(v)
    });
    match fetched {
        Ok(v) => Ok(v),
        Err(e) => cached().ok_or(e),
    }
}

fn norm(s: &str) -> String {
    s.chars()
        .filter_map(|c| {
            if c.is_alphanumeric() {
                Some(c.to_lowercase().next().unwrap_or(c))
            } else if c.is_whitespace() {
                Some(' ')
            } else {
                None
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Up to `n` titles for what was typed: names starting with it first, then names with a word
/// starting with it, then names containing it. Nothing for fewer than 2 letters.
pub fn suggest(query: &str, titles: &[String], n: usize) -> Vec<String> {
    let q = norm(query);
    if q.chars().count() < 2 {
        return vec![];
    }
    let mut ranked: Vec<(u8, usize, &String)> = titles
        .iter()
        .filter_map(|t| {
            let nt = norm(t);
            let rank = if nt == q {
                0
            } else if nt.starts_with(&q) {
                1
            } else if nt.split(' ').any(|w| w.starts_with(&q)) || nt.contains(&format!(" {q}")) {
                2
            } else if nt.contains(&q) {
                3
            } else {
                return None;
            };
            Some((rank, nt.len(), t))
        })
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then_with(|| a.2.cmp(b.2)));
    ranked.into_iter().take(n).map(|(_, _, t)| t.clone()).collect()
}

/// Is `name` exactly one of the titles (ignoring case and punctuation)?
pub fn contains(name: &str, titles: &[String]) -> bool {
    let q = norm(name);
    !q.is_empty() && titles.iter().any(|t| norm(t) == q)
}
