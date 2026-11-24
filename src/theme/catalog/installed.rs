//! The installed themes as Add-ons lists them: built-in or a folder, and where each folder
//! came from (a theme list, a Playnite import, or the user's own).

use super::{Index, Record, read_record};
use crate::theme::model::{Color, Palette, ThemeFile};
use crate::theme::{ThemeInfo, builtin};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub enum From {
    BuiltIn,
    /// Installed from Browse (`install.toml` with an index).
    Catalog(Record),
    /// Imported from a Playnite theme (package / folder path, Playnite id).
    Playnite {
        source: String,
        playnite_id: String,
    },
    /// A Lodestar theme someone put in the themes folder.
    Own,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InstalledTheme {
    pub key: String,
    pub name: String,
    pub author: String,
    pub version: String,
    pub description: String,
    /// Has controller-mode views (can be the controller theme).
    pub controller: bool,
    pub dir: Option<PathBuf>,
    pub from: From,
    /// Background, surface, accent and text colours (for a small preview).
    pub swatch: [Color; 4],
}

impl InstalledTheme {
    pub fn builtin(&self) -> bool {
        self.dir.is_none()
    }

    /// "Built in", "From Playnite's theme list …", "Imported from a Playnite theme (…)" …
    pub fn origin_text(&self) -> String {
        match &self.from {
            From::BuiltIn => "Built in".into(),
            From::Catalog(r) => match r.index {
                Some(Index::Playnite) => "From Playnite's theme list, converted by Lodestar".into(),
                None => "Installed".into(),
            },
            From::Playnite { source, .. } if !source.is_empty() => {
                format!("Imported from a Playnite theme ({})", tidy_path(source))
            }
            From::Playnite { .. } => "Imported from a Playnite theme".into(),
            From::Own => "Lodestar theme in your themes folder".into(),
        }
    }
}

/// `/home/x/Downloads/a.pthm` → `~/Downloads/a.pthm`.
fn tidy_path(p: &str) -> String {
    if let Some(h) = dirs::home_dir()
        && let Ok(rest) = Path::new(p).strip_prefix(&h)
    {
        return format!("~/{}", rest.display());
    }
    p.to_string()
}

#[derive(serde::Deserialize, Default)]
struct PlayniteRecord {
    #[serde(default)]
    source: String,
    #[serde(default)]
    playnite_id: String,
}

fn swatch_of(p: &Palette) -> [Color; 4] {
    [p.background, p.surface_alt, p.accent, p.text]
}

/// Everything the theme manager lists (built-ins first), with origins and colours. Reads each
/// user theme's files: run it on a helper thread.
pub fn list(infos: &[ThemeInfo]) -> Vec<InstalledTheme> {
    let builtins = builtin::all();
    infos
        .iter()
        .map(|i| {
            let palette = match &i.dir {
                None => builtins.iter().find(|(k, _)| *k == i.key).map(|(_, f)| f.palette.clone()),
                Some(dir) => std::fs::read_to_string(dir.join("theme.toml"))
                    .ok()
                    .and_then(|t| ThemeFile::parse(&t).ok())
                    .map(|f| f.palette),
            }
            .unwrap_or_default();
            let from = match &i.dir {
                None => From::BuiltIn,
                Some(dir) => origin_of(dir, &i.meta.origin),
            };
            InstalledTheme {
                key: i.key.clone(),
                name: i.meta.name.clone(),
                author: i.meta.author.clone(),
                version: i.meta.version.clone(),
                description: i.meta.description.clone(),
                controller: i.meta.controller,
                dir: i.dir.clone(),
                from,
                swatch: swatch_of(&palette),
            }
        })
        .collect()
}

fn origin_of(dir: &Path, origin: &str) -> From {
    if let Some(r) = read_record(dir)
        && r.index.is_some()
    {
        return From::Catalog(r);
    }
    match origin {
        "playnite" => {
            let p: PlayniteRecord = std::fs::read_to_string(dir.join("playnite-import.toml"))
                .ok()
                .and_then(|t| toml::from_str(&t).ok())
                .unwrap_or_default();
            From::Playnite { source: p.source, playnite_id: p.playnite_id }
        }
        _ => From::Own,
    }
}

/// Delete an installed theme's folder (`<themes_dir>/<key>`). Refuses built-ins, odd keys and
/// anything outside the themes folder; a symlinked theme only loses the link.
pub fn remove(themes_dir: &Path, key: &str) -> anyhow::Result<()> {
    if key.is_empty() || key.starts_with('.') || key.contains('/') || key.contains('\\') || key == ".." {
        anyhow::bail!("not a theme folder name: {key}");
    }
    let dir = themes_dir.join(key);
    let meta = std::fs::symlink_metadata(&dir).map_err(|e| anyhow::anyhow!("{}: {e}", dir.display()))?;
    if meta.file_type().is_symlink() {
        std::fs::remove_file(&dir)?;
        return Ok(());
    }
    if !meta.is_dir() || !dir.join("theme.toml").is_file() {
        anyhow::bail!("{} isn't a theme folder", dir.display());
    }
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}
