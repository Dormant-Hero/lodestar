//! Reply field mapping shared by the scanner and metadata look-ups, and the metadata look-up
//! through enabled metadata add-ons (`search` then `fetch`).
//!
//! `MetadataFetcher`'s worker calls [`lookup`] for a game after the built-in sources, fills the
//! missing `Metadata` fields and downloads `ArtRefs::urls` like its other art.

use super::{AddonKind, AddonManager};
use crate::config::AddonsConfig;
use crate::model::{Art, Metadata};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Art named by a reply: local files that exist, and http(s) URLs (left for a downloader).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArtRefs {
    pub local: Art,
    pub urls: ArtUrls,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArtUrls {
    pub cover: Option<String>,
    pub hero: Option<String>,
    pub logo: Option<String>,
    pub icon: Option<String>,
    pub banner: Option<String>,
}

pub fn str_field(v: &Value, key: &str, max: usize) -> Option<String> {
    let s = v.get(key)?.as_str()?.trim();
    (!s.is_empty()).then(|| super::run::clip(s, max))
}

pub fn u64_field(v: &Value, key: &str) -> Option<u64> {
    let x = v.get(key)?;
    x.as_u64()
        .or_else(|| x.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
        .or_else(|| x.as_str()?.trim().parse().ok())
}

pub fn i64_field(v: &Value, key: &str) -> Option<i64> {
    let x = v.get(key)?;
    x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)).or_else(|| x.as_str()?.trim().parse().ok())
}

/// Metadata fields of a `list` entry or a `fetch` reply.
pub fn meta_from(v: &Value) -> Metadata {
    let genres = v
        .get("genres")
        .and_then(|g| g.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::trim).filter(|s| !s.is_empty()).map(|s| super::run::clip(s, 64)))
                .take(32)
                .collect()
        })
        .unwrap_or_default();
    Metadata {
        description: str_field(v, "description", 20_000),
        developer: str_field(v, "developer", 200),
        publisher: str_field(v, "publisher", 200),
        genres,
        release_year: i64_field(v, "release_year").filter(|y| (1950..=2200).contains(y)).map(|y| y as i32),
        ..Default::default()
    }
}

/// `cover` / `hero` / `logo` / `icon` / `banner` of a reply. Paths may be absolute, `file://` or
/// relative to the add-on folder; only files that exist are kept.
pub fn art_from(v: &Value, addon_dir: &Path) -> ArtRefs {
    let mut out = ArtRefs::default();
    let slots: [(&str, &mut Option<PathBuf>, &mut Option<String>); 5] = [
        ("cover", &mut out.local.cover, &mut out.urls.cover),
        ("hero", &mut out.local.hero, &mut out.urls.hero),
        ("logo", &mut out.local.logo, &mut out.urls.logo),
        ("icon", &mut out.local.icon, &mut out.urls.icon),
        ("banner", &mut out.local.banner, &mut out.urls.banner),
    ];
    for (key, local, url) in slots {
        let Some(s) = v.get(key).and_then(|x| x.as_str()).map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        if s.starts_with("https://") || s.starts_with("http://") {
            *url = Some(s.to_string());
            continue;
        }
        let p = s.strip_prefix("file://").unwrap_or(s);
        let p = if p.starts_with('/') { PathBuf::from(p) } else { addon_dir.join(p) };
        *local = crate::scan::util::file_if_exists(&p);
    }
    out
}

/// A metadata add-on's answer for one game.
#[derive(Clone, Debug, Default)]
pub struct AddonMetadata {
    pub addon: String,
    pub meta: Metadata,
    pub art: ArtRefs,
}

/// Ask the enabled metadata add-ons (in name order) about `name`: `search`, pick the best result,
/// `fetch` it. The first add-on with an answer wins; failures are logged (and kept as the add-on's
/// `last_error`) and the next one is asked. Blocks (each call up to its timeout): worker thread only.
pub fn lookup(mgr: &AddonManager, cfg: &AddonsConfig, name: &str, year: Option<i32>) -> Option<AddonMetadata> {
    for info in mgr.enabled_of_kind(AddonKind::Metadata, cfg) {
        let id = &info.manifest.id;
        let Ok(reply) = mgr.call(id, "search", serde_json::json!({ "name": name, "year": year }), cfg) else {
            continue;
        };
        let results = reply.get("results").and_then(|r| r.as_array()).cloned().unwrap_or_default();
        // Automatic fills take only a result with the game's own name: a wrong game's details
        // would be worse than none (the user can still edit the game by hand).
        let named: Vec<Value> = results.into_iter().filter(|r| same_name(r, name)).collect();
        let Some(best) = best_match(&named, name, year) else {
            continue;
        };
        let Ok(found) = mgr.call(id, "fetch", serde_json::json!({ "id": best, "name": name }), cfg) else {
            continue;
        };
        return Some(AddonMetadata { addon: id.clone(), meta: meta_from(&found), art: art_from(&found, &info.dir) });
    }
    None
}

/// The result's `name` equals `name`, ignoring case and punctuation.
fn same_name(r: &Value, name: &str) -> bool {
    let norm = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    r.get("name").and_then(|n| n.as_str()).is_some_and(|n| norm(n) == norm(name))
}

/// Result id whose name matches best: exact (normalised) name and year, exact name, else the first.
pub fn best_match(results: &[Value], name: &str, year: Option<i32>) -> Option<String> {
    let norm = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let want = norm(name);
    let id_of = |r: &Value| -> Option<String> {
        match r.get("id")? {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    };
    let named = |r: &&Value| r.get("name").and_then(|n| n.as_str()).is_some_and(|n| norm(n) == want);
    let year_ok = |r: &&Value| year.is_some() && i64_field(r, "year") == year.map(i64::from);
    results
        .iter()
        .filter(named)
        .find(year_ok)
        .or_else(|| results.iter().find(named))
        .or_else(|| results.first())
        .and_then(id_of)
}
