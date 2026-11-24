//! Theme catalogue for Add-ons → Browse / Updates: a community theme list that can be read
//! without an account, cached on disk, and installing / updating themes from it through
//! Lodestar's importer.
//!
//! Source: **Playnite** (`pnite`): Playnite's add-on database (desktop + fullscreen themes).
//! Each points at an installer manifest with released `.pthm` packages; the newest one goes
//! through the Playnite importer. Updates: the manifest's newest version vs the installed one.
//!
//! Installed themes carry `install.toml` (where they came from, which version / commit).
//! Nothing from a theme is ever executed.

pub mod install;
pub mod installed;
pub mod net;
pub mod pnite;
#[cfg(test)]
mod tests;
pub mod yaml;

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

pub const PLAYNITE_DB: &str = "https://github.com/JosefNemec/PlayniteAddonDatabase/archive/refs/heads/master.zip";
/// A cached list is refreshed by itself after this long (the Refresh button always works).
pub const MAX_AGE_SECS: i64 = 24 * 3600;

/// Which list an entry comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Index {
    Playnite,
}

impl Index {
    pub fn label(self) -> &'static str {
        match self {
            Index::Playnite => "Playnite",
        }
    }
}

/// What a theme is used for in Lodestar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Desktop,
    Controller,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shot {
    pub thumb: String,
    pub full: String,
    #[serde(default)]
    pub caption: String,
}

/// One theme of a list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub index: Index,
    /// The list's id for the theme (Playnite: the AddonId).
    pub id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub kind: Kind,
    pub shots: Vec<Shot>,
    /// Web page (repository / source).
    pub page: String,
    /// Playnite: the installer manifest URL.
    pub source: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub new_entry: bool,
}

/// Where the lists are read from. `None` = don't go online (cached copy only).
#[derive(Clone, Debug, PartialEq)]
pub struct Endpoints {
    pub playnite: Option<String>,
}

impl Endpoints {
    pub fn online() -> Endpoints {
        Endpoints { playnite: Some(PLAYNITE_DB.into()) }
    }

    /// The real list; `LODESTAR_THEME_INDEX_PLAYNITE` replaces it (a URL or `file://` path:
    /// tests, offline screenshot runs). A screenshot run without it never goes online.
    pub fn for_run(screenshot_run: bool) -> Endpoints {
        let p = std::env::var("LODESTAR_THEME_INDEX_PLAYNITE").ok().filter(|v| !v.trim().is_empty());
        if p.is_some() {
            return Endpoints { playnite: p };
        }
        if screenshot_run {
            return Endpoints { playnite: None };
        }
        Endpoints::online()
    }

    fn of(&self, i: Index) -> Option<&str> {
        match i {
            Index::Playnite => self.playnite.as_deref(),
        }
    }
}

/// The lists as shown in Browse.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    /// Unix seconds of the last successful download per list.
    #[serde(default)]
    pub fetched_playnite: i64,
    /// Lists that couldn't be read this time (plain words), not stored.
    #[serde(skip)]
    pub problems: Vec<String>,
}

impl Catalog {
    pub fn fetched(&self, i: Index) -> i64 {
        match i {
            Index::Playnite => self.fetched_playnite,
        }
    }

    /// The oldest list's age; i64::MAX when one was never downloaded.
    pub fn age(&self, now: i64) -> i64 {
        let oldest = self.fetched_playnite;
        if oldest == 0 { i64::MAX } else { now - oldest }
    }

    pub fn find(&self, index: Index, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.index == index && e.id == id)
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// `<cache>/theme-catalog/`.
pub fn cache_dir(cache_root: &Path) -> PathBuf {
    cache_root.join("theme-catalog")
}

fn cache_file(dir: &Path) -> PathBuf {
    dir.join("catalog.json")
}

/// The list saved by the last fetch (None: never fetched, or unreadable).
pub fn load_cached(dir: &Path) -> Option<Catalog> {
    let bytes = std::fs::read(cache_file(dir)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn fetch_one(i: Index, url: &str, dir: &Path) -> Result<Vec<Entry>, String> {
    match i {
        Index::Playnite => {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            let zip = dir.join("playnite-db.zip");
            let bytes = net::get(url, net::MAX_INDEX_BYTES)?;
            std::fs::write(&zip, &bytes).map_err(|e| e.to_string())?;
            let r = pnite::parse_db_zip(&zip).map_err(|e| format!("{e:#}"));
            let _ = std::fs::remove_file(&zip);
            r
        }
    }
}

/// Download the lists (whatever `ep` allows), falling back per list to the cached copy, and
/// save the result. Blocking: run on a helper thread.
pub fn fetch(ep: &Endpoints, dir: &Path) -> Catalog {
    let mut cat = load_cached(dir).unwrap_or_default();
    let mut any = false;
    for i in [Index::Playnite] {
        let Some(url) = ep.of(i) else {
            if cat.fetched(i) == 0 {
                cat.problems.push(format!("The {} theme list isn't downloaded yet.", i.label()));
            }
            continue;
        };
        match fetch_one(i, url, dir) {
            Ok(list) => {
                cat.entries.retain(|e| e.index != i);
                cat.entries.extend(list);
                match i {
                    Index::Playnite => cat.fetched_playnite = now(),
                }
                any = true;
            }
            Err(e) => {
                log::warn!("theme catalogue {}: {e}", i.label());
                let tail = if cat.fetched(i) > 0 { " Showing the copy saved earlier." } else { "" };
                cat.problems.push(format!("Couldn't get the {} theme list: {e}.{tail}", i.label()));
            }
        }
    }
    cat.entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
    if any {
        let _ = std::fs::create_dir_all(dir);
        if let Ok(json) = serde_json::to_vec(&cat) {
            let tmp = dir.join("catalog.json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, cache_file(dir));
            }
        }
    }
    cat
}

/// Compare dotted versions numerically ("1.10.0" > "1.9"); text parts compare as text.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let parts = |s: &str| -> Vec<String> {
        s.trim()
            .trim_start_matches(['v', 'V'])
            .split(['.', '-', '_', ' '])
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect()
    };
    let (pa, pb) = (parts(a), parts(b));
    for k in 0..pa.len().max(pb.len()) {
        let (x, y) = (pa.get(k), pb.get(k));
        let o = match (x, y) {
            (None, None) => Ordering::Equal,
            (None, Some(y)) => {
                if y.chars().all(|c| c == '0') {
                    Ordering::Equal
                } else {
                    Ordering::Less
                }
            }
            (Some(x), None) => {
                if x.chars().all(|c| c == '0') {
                    Ordering::Equal
                } else {
                    Ordering::Greater
                }
            }
            (Some(x), Some(y)) => match (x.parse::<u64>(), y.parse::<u64>()) {
                (Ok(m), Ok(n)) => m.cmp(&n),
                _ => x.to_lowercase().cmp(&y.to_lowercase()),
            },
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    Ordering::Equal
}

/// `<theme dir>/install.toml`: where an installed theme came from.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// The list it was installed from (None: imported from a folder / file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<Index>,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Playnite: the package version.
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub page: String,
    /// Playnite: the installer manifest URL; imports: the folder / file.
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub installed: i64,
}

pub const RECORD: &str = "install.toml";

pub fn read_record(theme_dir: &Path) -> Option<Record> {
    let text = std::fs::read_to_string(theme_dir.join(RECORD)).ok()?;
    toml::from_str(&text).ok()
}

pub fn write_record(theme_dir: &Path, r: &Record) -> anyhow::Result<()> {
    std::fs::write(theme_dir.join(RECORD), toml::to_string(r)?)?;
    Ok(())
}

/// A newer version of an installed theme.
#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    /// Theme key (folder).
    pub key: String,
    pub name: String,
    pub index: Index,
    pub id: String,
    pub installed: String,
    pub latest: String,
}

/// What an installed theme would be checked against: (key, name, index, id, version, source).
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub key: String,
    pub name: String,
    pub index: Index,
    pub id: String,
    pub version: String,
    pub source: String,
}

/// The installed themes that can be checked for updates: everything installed from a list,
/// plus Playnite themes imported by hand whose Playnite id is in the list.
pub fn checks(themes: &[installed::InstalledTheme], cat: Option<&Catalog>) -> Vec<Check> {
    let mut out = vec![];
    for t in themes {
        match &t.from {
            installed::From::Catalog(r) => {
                let Some(index) = r.index else { continue };
                let source = cat
                    .and_then(|c| c.find(index, &r.id))
                    .map(|e| e.source.clone())
                    .unwrap_or_else(|| r.source.clone());
                out.push(Check {
                    key: t.key.clone(),
                    name: t.name.clone(),
                    index,
                    id: r.id.clone(),
                    version: r.version.clone(),
                    source,
                });
            }
            installed::From::Playnite { playnite_id, .. } if !playnite_id.is_empty() => {
                let Some(e) = cat.and_then(|c| c.find(Index::Playnite, playnite_id)) else { continue };
                out.push(Check {
                    key: t.key.clone(),
                    name: t.name.clone(),
                    index: Index::Playnite,
                    id: playnite_id.clone(),
                    version: t.version.clone(),
                    source: e.source.clone(),
                });
            }
            _ => {}
        }
    }
    out
}

/// The newest version / commit of one theme (network; blocking).
pub fn latest_version(index: Index, source: &str) -> Result<String, String> {
    match index {
        Index::Playnite => {
            let text = net::get(source, 1 << 20)?;
            pnite::latest_package(&String::from_utf8_lossy(&text))
                .map(|p| p.version)
                .ok_or_else(|| "the theme has no downloadable release".into())
        }
    }
}

/// Whether `latest` is newer than `installed` (versions: greater).
pub fn is_newer(index: Index, installed: &str, latest: &str) -> bool {
    match index {
        Index::Playnite => installed.is_empty() || compare_versions(latest, installed) == Ordering::Greater,
    }
}

/// Check each theme (network, one request each, a short gap between them). Returns the updates
/// found and the problems (theme name: reason).
pub fn find_updates(checks: &[Check]) -> (Vec<Update>, Vec<String>) {
    let mut ups = vec![];
    let mut problems = vec![];
    for (n, c) in checks.iter().enumerate() {
        if n > 0 {
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        match latest_version(c.index, &c.source) {
            Ok(latest) => {
                if is_newer(c.index, &c.version, &latest) {
                    ups.push(Update {
                        key: c.key.clone(),
                        name: c.name.clone(),
                        index: c.index,
                        id: c.id.clone(),
                        installed: c.version.clone(),
                        latest,
                    });
                }
            }
            Err(e) => problems.push(format!("{}: {e}", c.name)),
        }
    }
    (ups, problems)
}

impl Update {
    /// The list entry to install the update from: the list's own when it is loaded, else one
    /// rebuilt from what the installed theme recorded (works offline from the list).
    pub fn entry(&self, cat: Option<&Catalog>, checks: &[Check]) -> Option<Entry> {
        if let Some(e) = cat.and_then(|c| c.find(self.index, &self.id)) {
            return Some(e.clone());
        }
        let c = checks.iter().find(|c| c.key == self.key)?;
        Some(Entry {
            index: c.index,
            id: c.id.clone(),
            name: c.name.clone(),
            author: String::new(),
            description: String::new(),
            kind: Kind::Desktop,
            shots: vec![],
            page: String::new(),
            source: c.source.clone(),
            tags: vec![],
            new_entry: false,
        })
    }
}
