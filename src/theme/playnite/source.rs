//! Where a Playnite theme's files come from: a theme folder or a `.pthm` package (a zip).
//! Both are read the same way: a list of `/`-separated relative paths and a bounded read.
//! Nothing is ever extracted wholesale; only the files the import uses are read.

use crate::emulation::zipread;
use std::path::{Path, PathBuf};

/// Most files a theme may have before we stop listing (themes have a few hundred at most).
const MAX_FILES: usize = 5000;
/// Biggest small text file read from a theme (theme.yaml, thememodifier.yaml, themeExtras.yaml).
pub(crate) const YAML_LIMIT: u64 = 256 * 1024;

pub(crate) enum Source {
    Dir(PathBuf),
    Zip { zip: PathBuf, prefix: String, entries: Vec<zipread::Entry> },
}

impl Source {
    /// Open a theme folder (or any file inside one, e.g. its theme.yaml) or a .pthm / .zip
    /// package. The theme root is the folder holding `theme.yaml` (the shallowest one).
    pub fn open(path: &Path) -> anyhow::Result<Source> {
        let meta = std::fs::metadata(path).map_err(|e| anyhow::anyhow!("can't open {}: {e}", path.display()))?;
        if meta.is_file() {
            if is_package(path) {
                return Source::open_zip(path);
            }
            // A file inside a theme folder (the picker may hand us theme.yaml).
            if let Some(dir) = path.parent() {
                return Source::open_dir(dir);
            }
            anyhow::bail!("{} is not a Playnite theme", path.display());
        }
        Source::open_dir(path)
    }

    fn open_dir(dir: &Path) -> anyhow::Result<Source> {
        if dir.join("theme.yaml").is_file() {
            return Ok(Source::Dir(dir.to_path_buf()));
        }
        // A folder that holds one theme a level or two down (an unpacked download).
        let mut found: Vec<PathBuf> = vec![];
        walk(dir, 2, &mut |p| {
            if p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("theme.yaml")) {
                found.push(p.parent().unwrap_or(dir).to_path_buf());
            }
        });
        found.sort_by_key(|p| p.components().count());
        match found.into_iter().next() {
            Some(root) => Ok(Source::Dir(root)),
            None => {
                anyhow::bail!("{} has no theme.yaml — choose a Playnite theme folder or a .pthm file", dir.display())
            }
        }
    }

    fn open_zip(zip: &Path) -> anyhow::Result<Source> {
        let entries = zipread::list_uncached(zip)
            .ok_or_else(|| anyhow::anyhow!("{} is not a readable theme package", zip.display()))?;
        let manifest = entries
            .iter()
            .filter(|e| e.base_name().eq_ignore_ascii_case("theme.yaml"))
            .min_by_key(|e| e.name.matches('/').count())
            .ok_or_else(|| anyhow::anyhow!("{} has no theme.yaml inside", zip.display()))?;
        let prefix = manifest.name[..manifest.name.len() - manifest.base_name().len()].to_string();
        let entries: Vec<zipread::Entry> =
            entries.into_iter().filter(|e| e.name.starts_with(&prefix)).take(MAX_FILES).collect();
        Ok(Source::Zip { zip: zip.to_path_buf(), prefix, entries })
    }

    /// What to call the source in messages.
    pub fn display(&self) -> String {
        match self {
            Source::Dir(d) => d.display().to_string(),
            Source::Zip { zip, .. } => zip.display().to_string(),
        }
    }

    /// Every file, as (relative path, size).
    pub fn files(&self) -> Vec<(String, u64)> {
        match self {
            Source::Dir(root) => {
                let mut out = vec![];
                walk(root, 6, &mut |p| {
                    if out.len() >= MAX_FILES {
                        return;
                    }
                    let Ok(rel) = p.strip_prefix(root) else { return };
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    if rel.split('/').any(|c| c.starts_with('.')) {
                        return;
                    }
                    let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
                    out.push((rel, size));
                });
                out.sort();
                out
            }
            Source::Zip { prefix, entries, .. } => {
                entries.iter().map(|e| (e.name[prefix.len()..].to_string(), e.size)).collect()
            }
        }
    }

    /// The bytes of one file (None: missing, unreadable or bigger than `limit`).
    pub fn read(&self, rel: &str, limit: u64) -> Option<Vec<u8>> {
        if rel.split('/').any(|c| c == ".." || c.is_empty()) {
            return None;
        }
        match self {
            Source::Dir(root) => {
                let p = root.join(rel);
                // Stay inside the theme (no symlinks pointing elsewhere).
                let real = p.canonicalize().ok()?;
                if !real.starts_with(root.canonicalize().ok()?) {
                    return None;
                }
                let m = std::fs::metadata(&real).ok()?;
                if !m.is_file() || m.len() > limit {
                    return None;
                }
                std::fs::read(&real).ok()
            }
            Source::Zip { zip, prefix, entries } => {
                let full = format!("{prefix}{rel}");
                let e = entries.iter().find(|e| e.name == full)?;
                zipread::read(zip, e, limit)
            }
        }
    }

    /// Put one file at `dest`: a hard link when this is a theme's kept copy inside Lodestar's
    /// own themes folder (re-applying theme options), else a copy. False: not there / too big.
    pub fn put(&self, rel: &str, dest: &Path, limit: u64) -> bool {
        if let Source::Dir(root) = self
            && root.file_name().is_some_and(|n| n == super::options::SNAPSHOT)
            && !rel.split('/').any(|c| c == ".." || c.is_empty())
        {
            let p = root.join(rel);
            if std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_file() && m.len() <= limit)
                && std::fs::hard_link(&p, dest).is_ok()
            {
                return true;
            }
        }
        match self.read(rel, limit) {
            Some(b) => std::fs::write(dest, b).is_ok(),
            None => false,
        }
    }

    pub fn read_text(&self, rel: &str, limit: u64) -> Option<String> {
        let b = self.read(rel, limit)?;
        let s = String::from_utf8_lossy(&b).into_owned();
        Some(s.trim_start_matches('\u{feff}').to_string())
    }
}

fn is_package(p: &Path) -> bool {
    p.extension()
        .and_then(|x| x.to_str())
        .is_some_and(|x| x.eq_ignore_ascii_case("pthm") || x.eq_ignore_ascii_case("zip"))
}

/// Visit the files under `dir` (not following symlinked folders), `depth` levels deep.
fn walk(dir: &Path, depth: usize, f: &mut dyn FnMut(&Path)) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut items: Vec<_> = rd.flatten().collect();
    items.sort_by_key(|e| e.file_name());
    for e in items {
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        if ft.is_dir() {
            if depth > 0 && !e.file_name().to_string_lossy().starts_with('.') {
                walk(&p, depth - 1, f);
            }
        } else if ft.is_file() {
            f(&p);
        }
    }
}
