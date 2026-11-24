//! Playnite's add-on database (github.com/JosefNemec/PlayniteAddonDatabase, the list Playnite's
//! own add-on browser reads): one YAML file per add-on under `addons/themes_desktop/` and
//! `addons/themes_fullscreen/`, each pointing at an installer manifest that lists the released
//! `.pthm` packages. Read from the repository's .zip (one download, no API key).

use super::yaml::{self, Y};
use super::{Entry, Index, Kind, Shot};
use crate::emulation::zipread;
use std::path::Path;

/// One add-on entry (a theme) from its YAML text. None: not a theme / not usable.
pub fn parse_entry(text: &str) -> Option<Entry> {
    let y = yaml::parse(text);
    let kind = match y.str("Type")? {
        t if t.eq_ignore_ascii_case("ThemeDesktop") => Kind::Desktop,
        t if t.eq_ignore_ascii_case("ThemeFullscreen") => Kind::Controller,
        _ => return None,
    };
    let id = y.str("AddonId")?.to_string();
    let manifest = y.str("InstallerManifestUrl")?.to_string();
    if !(manifest.starts_with("https://") || manifest.starts_with("http://") || manifest.starts_with("file://")) {
        return None;
    }
    let name = y.str("Name").unwrap_or(&id).to_string();
    let description = y.str("ShortDescription").or_else(|| y.str("Description")).unwrap_or("").to_string();
    let page = y
        .str("SourceUrl")
        .map(str::to_string)
        .or_else(|| match y.get("Links") {
            Some(Y::Map(m)) => m.iter().find_map(|(_, v)| match v {
                Y::Str(s) if s.starts_with("http") => Some(s.clone()),
                _ => None,
            }),
            _ => None,
        })
        .unwrap_or_default();
    let shots = y
        .get("Screenshots")
        .map(|s| s.as_list().to_vec())
        .unwrap_or_default()
        .iter()
        .filter_map(|s| {
            let full = s.str("Image").or_else(|| s.str("Thumbnail"))?.to_string();
            let thumb = s.str("Thumbnail").map(str::to_string).unwrap_or_else(|| full.clone());
            Some(Shot { thumb, full, caption: String::new() })
        })
        .filter(|s| s.full.starts_with("http") || s.full.starts_with("file://"))
        .collect();
    Some(Entry {
        index: Index::Playnite,
        id,
        name,
        author: y.str("Author").unwrap_or("").to_string(),
        description,
        kind,
        shots,
        page,
        source: manifest,
        tags: y.get("Tags").map(Y::strings).unwrap_or_default(),
        new_entry: false,
    })
}

/// Every theme of the database .zip (a GitHub archive: `<repo>-<branch>/addons/…`).
pub fn parse_db_zip(zip: &Path) -> anyhow::Result<Vec<Entry>> {
    let entries = zipread::list_uncached(zip)
        .ok_or_else(|| anyhow::anyhow!("the add-on database download isn't a readable .zip"))?;
    let mut out = vec![];
    for e in &entries {
        let lower = e.name.to_ascii_lowercase();
        let is_theme = lower.contains("addons/themes_desktop/") || lower.contains("addons/themes_fullscreen/");
        if !is_theme || !(lower.ends_with(".yaml") || lower.ends_with(".yml")) {
            continue;
        }
        let Some(bytes) = zipread::read(zip, e, 256 * 1024) else { continue };
        if let Some(entry) = parse_entry(&String::from_utf8_lossy(&bytes)) {
            out.push(entry);
        }
    }
    if out.is_empty() {
        anyhow::bail!("no themes in the Playnite add-on database");
    }
    out.sort_by_key(|e| e.name.to_lowercase());
    out.dedup_by(|a, b| a.id == b.id);
    Ok(out)
}

/// A released package of a theme.
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    pub version: String,
    pub url: String,
    pub date: String,
}

/// The newest package of an installer manifest.
pub fn latest_package(text: &str) -> Option<Package> {
    let y = yaml::parse(text);
    y.get("Packages")?
        .as_list()
        .iter()
        .filter_map(|p| {
            let url = p.str("PackageUrl")?.to_string();
            if !(url.starts_with("https://") || url.starts_with("http://") || url.starts_with("file://")) {
                return None;
            }
            Some(Package {
                version: p.str("Version").unwrap_or("").to_string(),
                url,
                date: p.str("ReleaseDate").unwrap_or("").to_string(),
            })
        })
        .max_by(|a, b| super::compare_versions(&a.version, &b.version).then_with(|| a.date.cmp(&b.date)))
}
