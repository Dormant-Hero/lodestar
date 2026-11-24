//! Themes: format (`model`), built-in themes (`builtin`), font registration (`fonts`),
//! controller-mode renderer (`render`), the theme lists (`catalog`), Lodestar's console artwork
//! (`console_art`) and the Playnite theme importer (`playnite`).

pub mod builtin;
pub mod catalog;
pub mod console_art;
pub mod fonts;
pub mod model;
pub mod playnite;
pub mod render;

pub use model::*;

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A theme folder name from a theme's name: lower-case, `[a-z0-9-_]`, no leading dot; never empty.
pub fn sanitize_key(name: &str) -> String {
    let mut s = String::new();
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            s.push(ch.to_ascii_lowercase());
        } else if !s.ends_with('-') {
            s.push('-');
        }
    }
    let s = s.trim_matches('-').to_string();
    let s: String = s.chars().take(64).collect();
    if s.is_empty() { "theme".into() } else { s }
}

/// A loaded theme.
#[derive(Clone, Debug)]
pub struct Theme {
    /// Name used in config (`desktop_theme`, `controller.theme`). For user themes this is the
    /// directory name; for built-ins the built-in's name.
    pub key: String,
    pub file: ThemeFile,
    /// Theme directory for relative paths; None for built-ins (their assets are embedded).
    pub dir: Option<PathBuf>,
}

impl Theme {
    /// Resolve a theme-relative path ("art/bg.png", "./_inc/x.svg") to an absolute path.
    /// Absolute paths are returned unchanged. Built-ins have no files, so relative paths → None.
    pub fn resolve(&self, p: &str) -> Option<PathBuf> {
        let p = p.trim();
        if p.is_empty() {
            return None;
        }
        let path = Path::new(p);
        if path.is_absolute() {
            return Some(path.to_path_buf());
        }
        let rel = p.strip_prefix("./").unwrap_or(p);
        self.dir.as_ref().map(|d| d.join(rel))
    }
}

#[derive(Clone, Debug)]
pub struct ThemeInfo {
    pub key: String,
    pub meta: ThemeMeta,
    /// None = built-in.
    pub dir: Option<PathBuf>,
}

pub struct ThemeManager {
    themes_dir: PathBuf,
    infos: Vec<ThemeInfo>,
}

impl ThemeManager {
    pub fn new(themes_dir: PathBuf) -> ThemeManager {
        let mut m = ThemeManager { themes_dir, infos: vec![] };
        m.refresh();
        m
    }

    pub fn themes_dir(&self) -> &Path {
        &self.themes_dir
    }

    /// Re-discover built-in and user themes (`<themes_dir>/<key>/theme.toml`).
    pub fn refresh(&mut self) {
        let mut infos: Vec<ThemeInfo> = builtin::all()
            .into_iter()
            .map(|(key, file)| ThemeInfo { key: key.to_string(), meta: file.theme, dir: None })
            .collect();
        if let Ok(rd) = std::fs::read_dir(&self.themes_dir) {
            let mut user: Vec<ThemeInfo> = rd
                .flatten()
                .filter_map(|e| {
                    let dir = e.path();
                    let text = std::fs::read_to_string(dir.join("theme.toml")).ok()?;
                    let file = match ThemeFile::parse(&text) {
                        Ok(f) => f,
                        Err(err) => {
                            log::warn!("theme {}: {err:#}", dir.display());
                            return None;
                        }
                    };
                    Some(ThemeInfo {
                        key: e.file_name().to_string_lossy().into_owned(),
                        meta: file.theme,
                        dir: Some(dir),
                    })
                })
                .collect();
            user.sort_by_key(|i| i.meta.name.to_lowercase());
            infos.extend(user);
        }
        self.infos = infos;
    }

    pub fn list(&self) -> &[ThemeInfo] {
        &self.infos
    }

    /// Load a theme by key. Never fails: unknown/broken themes fall back to the default built-in.
    pub fn load(&self, key: &str) -> Arc<Theme> {
        self.load_checked(key).0
    }

    /// `load_checked` with the theme's colour scheme `scheme` applied (when it has it).
    pub fn load_with_scheme(&self, key: &str, scheme: Option<&str>) -> (Arc<Theme>, Option<String>) {
        let (t, problem) = self.load_checked(key);
        match scheme.filter(|s| t.file.color_schemes.contains_key(*s)) {
            Some(s) => {
                (Arc::new(Theme { key: t.key.clone(), file: t.file.with_color_scheme(s), dir: t.dir.clone() }), problem)
            }
            None => (t, problem),
        }
    }

    /// Like `load`, plus a message for the user when `key` couldn't be used and the default
    /// built-in was loaded instead ("Theme “x” not found, using Lodestar"). An empty key quietly
    /// means the default. Keys match case-insensitively when there is no exact match (config
    /// files say `theme = "shelf"` for the built-in "Shelf").
    pub fn load_checked(&self, key: &str) -> (Arc<Theme>, Option<String>) {
        let key = builtin::current_key(key.trim());
        let info = self
            .infos
            .iter()
            .find(|i| i.key == key)
            .or_else(|| self.infos.iter().find(|i| i.key.eq_ignore_ascii_case(key)))
            .or_else(|| self.infos.iter().find(|i| i.meta.name.trim().eq_ignore_ascii_case(key)));
        let mut problem = if key.is_empty() { None } else { Some(format!("Theme “{key}” not found")) };
        if let Some(info) = info {
            match &info.dir {
                None => {
                    if let Some((k, file)) = builtin::all().into_iter().find(|(k, _)| *k == info.key) {
                        return (Arc::new(Theme { key: k.to_string(), file, dir: None }), None);
                    }
                }
                Some(dir) => match std::fs::read_to_string(dir.join("theme.toml"))
                    .map_err(anyhow::Error::from)
                    .and_then(|t| ThemeFile::parse(&t))
                {
                    Ok(file) => {
                        return (Arc::new(Theme { key: info.key.clone(), file, dir: Some(dir.clone()) }), None);
                    }
                    Err(e) => {
                        log::warn!("loading theme {key}: {e:#}");
                        problem = Some(format!("Theme “{key}” couldn't be loaded"));
                    }
                },
            }
        }
        let (k, file) = builtin::all().into_iter().next().expect("at least one built-in theme");
        let problem = problem.filter(|_| !k.eq_ignore_ascii_case(key)).map(|p| format!("{p}, using {k}"));
        if let Some(p) = &problem {
            log::warn!("{p}");
        }
        (Arc::new(Theme { key: k.to_string(), file, dir: None }), problem)
    }
}

/// Move a freshly built theme folder `new` to `dest`, replacing what is there without a moment
/// where neither exists: the old folder is renamed aside first, put back if the move fails,
/// and deleted only once the new one is in place.
pub(crate) fn replace_dir(new: &Path, dest: &Path) -> std::io::Result<()> {
    if !dest.exists() {
        return std::fs::rename(new, dest);
    }
    let name = dest.file_name().and_then(|n| n.to_str()).unwrap_or("theme");
    let aside = dest.with_file_name(format!(
        ".{name}.old-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
    ));
    std::fs::rename(dest, &aside)?;
    if let Err(e) = std::fs::rename(new, dest) {
        if let Err(back) = std::fs::rename(&aside, dest) {
            log::warn!("couldn't put {} back: {back}", dest.display());
        }
        return Err(e);
    }
    if let Err(e) = std::fs::remove_dir_all(&aside) {
        log::warn!("couldn't remove {}: {e}", aside.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_a_theme_folder_keeps_the_old_one_when_the_move_fails() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("mytheme");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("theme.toml"), "old").unwrap();
        // The new folder is missing: the move fails and the earlier import is still there.
        assert!(replace_dir(&dir.path().join(".missing"), &dest).is_err());
        assert_eq!(std::fs::read_to_string(dest.join("theme.toml")).unwrap(), "old");
        // A real replacement swaps it and leaves nothing aside.
        let new = dir.path().join(".new");
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("theme.toml"), "new").unwrap();
        replace_dir(&new, &dest).unwrap();
        assert_eq!(std::fs::read_to_string(dest.join("theme.toml")).unwrap(), "new");
        let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, [std::ffi::OsString::from("mytheme")]);
        // Into an empty place.
        let fresh = dir.path().join(".fresh");
        std::fs::create_dir_all(&fresh).unwrap();
        replace_dir(&fresh, &dir.path().join("other")).unwrap();
        assert!(dir.path().join("other").is_dir());
    }

    #[test]
    fn theme_keys_match_case_insensitively_and_report_fallbacks() {
        let dir = tempfile::tempdir().unwrap();
        let m = ThemeManager::new(dir.path().to_path_buf());
        let builtins: Vec<String> = m.list().iter().map(|i| i.key.clone()).collect();
        let default = builtins[0].clone();
        for k in &builtins {
            let (t, msg) = m.load_checked(&k.to_lowercase());
            assert_eq!((&t.key, msg), (k, None));
            let (t, msg) = m.load_checked(&k.to_uppercase());
            assert_eq!((&t.key, msg), (k, None));
        }
        let (t, msg) = m.load_checked("no-such-theme");
        assert_eq!(t.key, default);
        let msg = msg.expect("fallback reported");
        assert!(msg.contains("no-such-theme") && msg.contains(&default), "{msg}");
        // Empty = the default, quietly.
        assert_eq!(m.load_checked("").1, None);
        // A broken user theme is reported too.
        std::fs::create_dir_all(dir.path().join("Broken")).unwrap();
        std::fs::write(dir.path().join("Broken/theme.toml"), "[theme]\nname = \"Broken\"\n").unwrap();
        let mut m = m;
        m.refresh();
        if m.list().iter().any(|i| i.key == "Broken") {
            std::fs::write(dir.path().join("Broken/theme.toml"), "this is = = not toml").unwrap();
            let (t, msg) = m.load_checked("broken");
            assert_eq!(t.key, default);
            assert!(msg.is_some_and(|m| m.contains("couldn't be loaded")));
        }
    }

    #[test]
    fn theme_folder_names_are_tidy_and_never_empty() {
        assert_eq!(super::sanitize_key("  Harbour Night (v2) "), "harbour-night-v2");
        assert_eq!(super::sanitize_key(".hidden"), "hidden");
        assert_eq!(super::sanitize_key("***"), "theme");
        assert_eq!(super::sanitize_key(&"x".repeat(100)).len(), 64);
    }
}
