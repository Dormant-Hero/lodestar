//! Sharing themes: a theme folder as a .zip, and a .zip someone shared added as a new theme.
//!
//! Export writes a plain zip (deflate via miniz_oxide, one top folder named after the theme).
//! Import never runs anything from the archive: it checks the archive's size and entry count
//! first, unpacks with the same safe unpacker theme downloads use (no absolute paths, no `..`,
//! oversized files left out, links are plain files), makes sure there is a theme.toml Lodestar
//! can read, takes away execute permissions, and only then moves the folder into the themes
//! folder under a name that isn't taken. Blocking: run on a helper thread.

use crate::emulation::zipread;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// An archive bigger than this (unpacked) is refused.
pub const MAX_TOTAL: u64 = 2 << 30;
/// … and one with more files than this.
pub const MAX_FILES: usize = 10_000;

/// The files of a theme folder to share (hidden files and old copies left out), as
/// (path inside the folder with `/`, file).
pub fn theme_files(dir: &Path) -> anyhow::Result<Vec<(String, PathBuf)>> {
    fn walk(base: &Path, d: &Path, out: &mut Vec<(String, PathBuf)>) -> anyhow::Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(d)?.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let ft = e.file_type()?;
            let p = e.path();
            if ft.is_dir() {
                walk(base, &p, out)?;
            } else if ft.is_file() {
                let rel = p.strip_prefix(base)?.to_string_lossy().replace('\\', "/");
                out.push((rel, p));
            }
        }
        Ok(())
    }
    let mut out = vec![];
    walk(dir, dir, &mut out)?;
    Ok(out)
}

/// Size of what `export` would write (files, bytes before compression).
pub fn export_size(dir: &Path) -> (usize, u64) {
    let files = theme_files(dir).unwrap_or_default();
    let bytes = files.iter().map(|(_, p)| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)).sum();
    (files.len(), bytes)
}

/// DOS date / time of now (zip headers).
fn dos_now() -> (u16, u16) {
    let secs = crate::model::now_unix();
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let date = (((y - 1980).clamp(0, 127) as u16) << 9) | ((m as u16) << 5) | d as u16;
    let time = (((rem / 3600) as u16) << 11) | ((((rem % 3600) / 60) as u16) << 5) | (((rem % 60) / 2) as u16);
    (date, time)
}

/// Write the theme folder `dir` as a zip at `dest`, its files under `top/`. Returns the number
/// of files.
pub fn export(dir: &Path, top: &str, dest: &Path) -> anyhow::Result<usize> {
    let files = theme_files(dir)?;
    anyhow::ensure!(files.iter().any(|(r, _)| r == "theme.toml"), "the theme folder has no theme.toml");
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    let (date, time) = dos_now();
    let top = top.trim().trim_matches('/');
    for (rel, path) in &files {
        let data = std::fs::read(path)?;
        anyhow::ensure!(
            data.len() < u32::MAX as usize && out.len() < u32::MAX as usize / 2,
            "the theme is too big for a .zip"
        );
        let crc = crc32fast::hash(&data);
        let packed = miniz_oxide::deflate::compress_to_vec(&data, 6);
        let (method, body): (u16, &[u8]) = if packed.len() < data.len() { (8, &packed) } else { (0, &data) };
        let name = if top.is_empty() { rel.clone() } else { format!("{top}/{rel}") };
        let offset = out.len() as u32;
        // Local header.
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
        out.extend_from_slice(&method.to_le_bytes());
        out.extend_from_slice(&time.to_le_bytes());
        out.extend_from_slice(&date.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(body);
        // Central directory entry.
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&0x031eu16.to_le_bytes()); // made by: Unix, 3.0
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0x0800u16.to_le_bytes());
        central.extend_from_slice(&method.to_le_bytes());
        central.extend_from_slice(&time.to_le_bytes());
        central.extend_from_slice(&date.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(body.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0u8; 8]); // extra, comment, disk, internal attributes
        central.extend_from_slice(&((0o100644u32) << 16).to_le_bytes()); // a plain rw-r--r-- file
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    let n = files.len() as u16;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    let tmp = dest.with_extension(format!("zip.part-{}", std::process::id()));
    let r = (|| -> anyhow::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&out)?;
        f.sync_all()?;
        std::fs::rename(&tmp, dest)?;
        Ok(())
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r.map(|_| files.len())
}

/// What an import did.
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    pub key: String,
    pub name: String,
    pub files: usize,
    /// Files left out (too big).
    pub skipped: usize,
}

/// Add the theme in `zip` to `themes_dir` as a new theme (never replacing one).
pub fn import(zip: &Path, themes_dir: &Path) -> anyhow::Result<Imported> {
    let entries = zipread::list_uncached(zip).ok_or_else(|| anyhow::anyhow!("this isn't a .zip Lodestar can read"))?;
    anyhow::ensure!(!entries.is_empty(), "the .zip is empty");
    anyhow::ensure!(entries.len() <= MAX_FILES, "the .zip has too many files ({}) for a theme", entries.len());
    let total: u64 = entries.iter().map(|e| e.size).sum();
    anyhow::ensure!(total <= MAX_TOTAL, "the .zip is too big for a theme ({} MB unpacked)", total >> 20);
    if !entries.iter().any(|e| e.base_name() == "theme.toml") {
        if entries.iter().any(|e| e.base_name().eq_ignore_ascii_case("theme.xml")) {
            anyhow::bail!(
                "this is a theme for another program, not a Lodestar theme (it has no theme.toml) — \
                 you can rebuild its look in the theme creator"
            );
        }
        anyhow::bail!("this .zip has no theme.toml, so it isn't a Lodestar theme");
    }
    std::fs::create_dir_all(themes_dir)?;
    let work = themes_dir.join(format!(
        ".import-{}-{}",
        std::process::id(),
        crc32fast::hash(zip.as_os_str().as_encoded_bytes())
    ));
    let _ = std::fs::remove_dir_all(&work);
    let r = (|| -> anyhow::Result<Imported> {
        let (root, skipped) = extract(zip, &entries, &work)?;
        let root = if root.join("theme.toml").is_file() {
            root
        } else {
            anyhow::bail!("the theme.toml isn't at the top of the .zip (or of its only folder)");
        };
        let text = std::fs::read_to_string(root.join("theme.toml"))?;
        let file = crate::theme::ThemeFile::parse(&text)
            .map_err(|e| anyhow::anyhow!("its theme.toml has a mistake Lodestar can't read past: {e:#}"))?;
        let files = no_exec(&root)?;
        let name =
            if file.theme.name.trim().is_empty() { "Imported theme".to_string() } else { file.theme.name.clone() };
        let key = super::files::free_key(themes_dir, &name);
        std::fs::rename(&root, themes_dir.join(&key))?;
        Ok(Imported { key, name, files, skipped })
    })();
    let _ = std::fs::remove_dir_all(&work);
    r
}

/// Files bigger than this are left out of an import (theme videos fit).
const MAX_FILE: u64 = 256 << 20;

/// A safe relative path for an entry name (None: absolute, `..`, a drive letter, empty).
pub fn safe_rel(name: &str) -> Option<PathBuf> {
    use std::path::Component;
    let name = name.replace('\\', "/");
    if name.starts_with('/') || name.as_bytes().get(1) == Some(&b':') {
        return None;
    }
    let mut out = PathBuf::new();
    for c in Path::new(&name).components() {
        match c {
            Component::Normal(s) => out.push(s),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// Unpack `entries` of `zip` into `dest` (plain files only, every path checked, oversized
/// files left out). Returns the theme's folder (the archive's only top folder, else `dest`)
/// and how many files were left out.
fn extract(zip: &Path, entries: &[zipread::Entry], dest: &Path) -> anyhow::Result<(PathBuf, usize)> {
    std::fs::create_dir_all(dest)?;
    let mut skipped = 0;
    let mut tops = std::collections::BTreeSet::new();
    let mut written = 0;
    for e in entries {
        if e.name.ends_with('/') {
            continue;
        }
        let Some(rel) = safe_rel(&e.name) else {
            anyhow::bail!("the .zip contains an unsafe path ({})", e.name);
        };
        if e.size > MAX_FILE {
            skipped += 1;
            continue;
        }
        let Some(data) = zipread::read(zip, e, MAX_FILE) else {
            skipped += 1;
            continue;
        };
        if let Some(std::path::Component::Normal(t)) = rel.components().next() {
            tops.insert(t.to_os_string());
        }
        let out = dest.join(&rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out, data)?;
        written += 1;
    }
    anyhow::ensure!(written > 0, "the .zip has no files Lodestar can use");
    let root = match tops.iter().next() {
        Some(top) if tops.len() == 1 && dest.join(top).is_dir() => dest.join(top),
        _ => dest.to_path_buf(),
    };
    Ok((root, skipped))
}

/// Take execute permissions off every file (a theme is data); returns the file count.
fn no_exec(dir: &Path) -> anyhow::Result<usize> {
    use std::os::unix::fs::PermissionsExt;
    let mut n = 0;
    for (_, p) in theme_files(dir)? {
        let m = std::fs::metadata(&p)?;
        if m.permissions().mode() & 0o111 != 0 {
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644))?;
        }
        n += 1;
    }
    Ok(n)
}
