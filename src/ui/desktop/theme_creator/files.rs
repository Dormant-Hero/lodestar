//! The theme's folder: making a new theme from a template, bringing pictures, fonts, sounds and
//! videos into it (copied, so the theme is complete on its own), and saving.
//! Blocking: run on a helper thread (the window starts these through `job`).

use crate::theme::{ThemeFile, builtin};
use std::path::{Component, Path, PathBuf};

/// What a new theme starts from.
#[derive(Clone, Debug, PartialEq)]
pub enum Template {
    /// A built-in theme by key.
    Builtin(String),
    /// A theme folder (an installed user theme).
    Folder(PathBuf),
}

/// Files a copy of a theme leaves behind: where a downloaded theme came from (it is the user's
/// own now) and a Playnite import's options (they would re-import over the user's changes).
const NOT_COPIED: [&str; 3] = [crate::theme::catalog::RECORD, "playnite-options.toml", "playnite-source"];

/// Files bigger than this aren't brought into a theme (videos fit).
pub const MAX_FILE: u64 = 256 << 20;

/// A folder name for a theme called `name`, not yet used in `themes_dir`.
pub fn free_key(themes_dir: &Path, name: &str) -> String {
    let mut base: String = name
        .trim()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '-' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    base = base.trim_matches(['-', '.', ' ']).chars().take(60).collect();
    if base.is_empty() {
        base = "My theme".into();
    }
    let taken = |k: &str| {
        themes_dir.join(k).exists()
            || builtin::SOURCES.iter().any(|(b, _)| b.eq_ignore_ascii_case(k))
            || builtin::RENAMED.iter().any(|(b, _)| b.eq_ignore_ascii_case(k))
    };
    if !taken(&base) {
        return base;
    }
    (2..).map(|n| format!("{base} {n}")).find(|k| !taken(k)).expect("a free name")
}

/// Copy a folder (regular files and folders only: links are left out, never followed).
pub fn copy_dir(from: &Path, to: &Path, skip: &[&str]) -> anyhow::Result<u64> {
    std::fs::create_dir_all(to)?;
    let mut n = 0;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let name = e.file_name();
        let s = name.to_string_lossy();
        if skip.iter().any(|k| *k == s) || s.starts_with(".old-") || s.starts_with('.') && s.contains(".old-") {
            continue;
        }
        let ft = e.file_type()?;
        if ft.is_dir() {
            n += copy_dir(&e.path(), &to.join(&name), &[])?;
        } else if ft.is_file() {
            std::fs::copy(e.path(), to.join(&name))?;
            n += 1;
        }
    }
    Ok(n)
}

/// Make a new theme called `name` in `themes_dir` from `template`; returns its key. The
/// template is never changed.
pub fn create(themes_dir: &Path, template: &Template, name: &str, author: &str) -> anyhow::Result<String> {
    std::fs::create_dir_all(themes_dir)?;
    let key = free_key(themes_dir, name);
    let work = themes_dir.join(format!(".new-{}-{}", std::process::id(), unique_suffix()));
    let _ = std::fs::remove_dir_all(&work);
    let result = (|| -> anyhow::Result<()> {
        let (mut file, raw, based_on) = match template {
            Template::Builtin(k) => {
                let src = builtin::SOURCES
                    .iter()
                    .find(|(b, _)| b == k)
                    .map(|(_, s)| *s)
                    .ok_or_else(|| anyhow::anyhow!("there is no built-in theme called {k}"))?;
                std::fs::create_dir_all(&work)?;
                (ThemeFile::parse(src)?, toml::from_str::<toml::Table>(src)?, k.clone())
            }
            Template::Folder(dir) => {
                let text = std::fs::read_to_string(dir.join("theme.toml"))?;
                copy_dir(dir, &work, &NOT_COPIED)?;
                let f = ThemeFile::parse(&text)?;
                let n = f.theme.name.clone();
                (f, toml::from_str::<toml::Table>(&text)?, n)
            }
        };
        file.theme.name = name.trim().to_string();
        file.theme.author = author.trim().to_string();
        file.theme.version = "1.0.0".into();
        file.theme.origin = "lodestar".into();
        file.theme.controller = true;
        if !based_on.is_empty() {
            file.theme.description = format!("Based on “{based_on}”.");
        }
        let text = file.to_theme_toml(Some(&raw))?;
        crate::fsutil::write_atomic(&work.join("theme.toml"), text.as_bytes())?;
        std::fs::rename(&work, themes_dir.join(&key))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&work);
    }
    result.map(|_| key)
}

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let t =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    t ^ N.fetch_add(1, Ordering::Relaxed)
}

/// What kind of file is brought into a theme (decides the folder and the accepted types).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asset {
    Picture,
    Font,
    Sound,
    Video,
}

impl Asset {
    pub fn folder(self) -> &'static str {
        match self {
            Asset::Picture => "pictures",
            Asset::Font => "fonts",
            Asset::Sound => "sounds",
            Asset::Video => "videos",
        }
    }

    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Asset::Picture => &["png", "jpg", "jpeg", "webp", "gif", "bmp", "svg", "avif", "ico"],
            Asset::Font => &["ttf", "otf", "ttc"],
            Asset::Sound => &["wav", "ogg", "mp3", "flac", "opus"],
            Asset::Video => &["mp4", "webm", "mkv", "mov", "avi", "m4v"],
        }
    }

    pub fn what(self) -> &'static str {
        match self {
            Asset::Picture => "a picture",
            Asset::Font => "a font",
            Asset::Sound => "a sound",
            Asset::Video => "a video",
        }
    }
}

fn ext_of(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// A short content tag for file names (same file → same name; another file → another name, so a
/// changed picture never shows the old one from the image cache).
fn content_tag(bytes: &[u8]) -> String {
    format!("{:08x}", crc32fast::hash(bytes))
}

fn safe_stem(p: &Path) -> String {
    let s: String = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '-' })
        .take(40)
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "file".into() } else { s }
}

/// Copy `src` into the theme folder `dir` (`pictures/`, `fonts/` …) and return the path the
/// theme refers to it by ("pictures/sunset-1a2b3c4d.jpg"). A file already inside the theme
/// folder is used where it is.
pub fn bring_in(dir: &Path, src: &Path, kind: Asset) -> anyhow::Result<String> {
    let ext = ext_of(src);
    if !kind.extensions().contains(&ext.as_str()) {
        anyhow::bail!("{} isn't {} Lodestar can use ({})", file_name(src), kind.what(), kind.extensions().join(", "));
    }
    if let Some(rel) = inside(dir, src) {
        return Ok(rel);
    }
    let meta = std::fs::metadata(src)?;
    if !meta.is_file() {
        anyhow::bail!("{} isn't a file", file_name(src));
    }
    if meta.len() > MAX_FILE {
        anyhow::bail!("{} is too big (over {} MB)", file_name(src), MAX_FILE >> 20);
    }
    let bytes = std::fs::read(src)?;
    let rel = format!("{}/{}-{}.{ext}", kind.folder(), safe_stem(src), content_tag(&bytes));
    let dest = dir.join(&rel);
    if !dest.is_file() {
        crate::fsutil::write_atomic(&dest, &bytes)?;
    }
    Ok(rel)
}

/// The pictures a theme can give each console or collection (`[collection_art.<key>]`), with
/// what the creator calls them.
pub const ART_SLOTS: [(&str, &str); 4] =
    [("card", "Card"), ("hero", "Background"), ("mark", "Name badge"), ("device", "Device")];

/// Picture types a console picture can be.
pub const CONSOLE_EXTS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "svg"];

/// The `[collection_art]` key for a collection: the bare platform id for a console, else the
/// collection key ("steam", "favorites", "tag:RPG" …).
pub fn art_key(c: &crate::library::Collection) -> String {
    match &c.kind {
        crate::library::CollectionKind::Platform(id) => id.clone(),
        _ => c.key(),
    }
}

/// A file-name-safe form of an art key.
fn key_file(key: &str) -> String {
    key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(80).collect()
}

/// Copy a console's picture for `slot` into `consoles/<slot>/<key>-<tag>.<ext>` and return the
/// theme path (the caller writes it into `[collection_art]`).
pub fn copy_console_picture(dir: &Path, slot: &str, key: &str, src: &Path) -> anyhow::Result<String> {
    let ext = ext_of(src);
    if !CONSOLE_EXTS.contains(&ext.as_str()) {
        anyhow::bail!("{} isn't a picture Lodestar can use here ({})", file_name(src), CONSOLE_EXTS.join(", "));
    }
    let meta = std::fs::metadata(src)?;
    if meta.len() > MAX_FILE {
        anyhow::bail!("{} is too big", file_name(src));
    }
    let bytes = std::fs::read(src)?;
    let rel = format!("consoles/{slot}/{}-{}.{ext}", key_file(key), content_tag(&bytes));
    let dest = dir.join(&rel);
    if !dest.is_file() {
        crate::fsutil::write_atomic(&dest, &bytes)?;
    }
    Ok(rel)
}

/// `p` inside `dir`: the theme path for it.
fn inside(dir: &Path, p: &Path) -> Option<String> {
    let dir = dir.canonicalize().ok()?;
    let p = p.canonicalize().ok()?;
    let rel = p.strip_prefix(&dir).ok()?;
    if rel.components().all(|c| matches!(c, Component::Normal(_))) {
        Some(rel.to_string_lossy().replace('\\', "/"))
    } else {
        None
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

/// Save the theme's text (temporary file, fsync, rename: a crash never leaves half a file).
pub fn save(dir: &Path, text: &str) -> anyhow::Result<()> {
    crate::fsutil::write_atomic(&dir.join("theme.toml"), text.as_bytes())?;
    Ok(())
}

/// A theme path (relative to `dir`) that exists, or a binding / absolute path (not checked).
pub fn theme_file_exists(dir: &Path, path: &str) -> bool {
    let p = path.trim();
    if p.is_empty() || p.contains('{') || p.starts_with("system:") || p.starts_with("builtin:") {
        return true;
    }
    let pp = Path::new(p);
    if pp.is_absolute() {
        return pp.exists();
    }
    if let Some(home) = p.strip_prefix("~/") {
        return dirs::home_dir().is_some_and(|h| h.join(home).exists());
    }
    dir.join(p.strip_prefix("./").unwrap_or(p)).exists()
}

/// Files in a theme folder of one kind (for pickers), as theme paths.
pub fn list_assets(dir: &Path, kind: Asset) -> Vec<String> {
    let mut out = vec![];
    fn walk(base: &Path, d: &Path, exts: &[&str], out: &mut Vec<String>, depth: usize) {
        if depth > 4 || out.len() > 500 {
            return;
        }
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if !e.file_name().to_string_lossy().starts_with('.') {
                    walk(base, &p, exts, out, depth + 1);
                }
            } else if ft.is_file()
                && exts.contains(&ext_of(&p).as_str())
                && let Ok(rel) = p.strip_prefix(base)
            {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    walk(dir, dir, kind.extensions(), &mut out, 0);
    out.sort();
    out
}
