//! Downloading a RomM game: which ROM folder it goes to, and the download itself.
//!
//! Where (`pick_target`), for the game's console:
//!   1. a ROM folder the user set to that console (Settings → Emulation);
//!   2. a console sub-folder ("snes", "ps2" …) of a "sub-folders are consoles" ROM folder;
//!   3. a folder found automatically for that console (an emulator's own games folder);
//!   4. `<download folder>/<console folder>` when the user picked a download folder before;
//!   5. otherwise the user is asked once (`Target::Ask`, with a suggestion).
//!
//! The download: one file → `GET /api/roms/{id}/content/{fs_name}` into `<dir>/<fs_name>`;
//! a game of several files (multi-disc, cue + bins) → each file
//! (`GET /api/roms/{file id}/files/content/{name}`) into `<dir>/<fs_name>/…`, or, from a server
//! without that endpoint, the server's zip of it as `<dir>/<fs_name>.zip`. Every file is written
//! to `<name>.part` and renamed when complete; cancel / failure removes what was written.
//! Existing files are never overwritten. Names from the server never leave the target folder.

use super::{ApiError, RommGame, Server};
use crate::emulation::platforms::{self, Platform};
use crate::scan::emulation::roots::{Layout, Root};
use serde_json::Value;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Shared between the download thread and the UI.
#[derive(Debug, Default)]
pub struct Progress {
    pub done: AtomicU64,
    /// Bytes expected (0 = unknown).
    pub total: AtomicU64,
    pub cancel: AtomicBool,
}

impl Progress {
    pub fn fraction(&self) -> Option<f32> {
        let t = self.total.load(Ordering::Relaxed);
        (t > 0).then(|| (self.done.load(Ordering::Relaxed) as f64 / t as f64).clamp(0.0, 1.0) as f32)
    }
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Where a download goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Into `dir` (created when missing). `add_folder`: the download folder is not one of the
    /// ROM folders yet; add it to Settings → Emulation so the scan finds the game.
    Dir { dir: PathBuf, add_folder: Option<PathBuf> },
    /// No folder for this console: ask where (once), suggesting `suggest`.
    Ask { suggest: PathBuf },
}

/// The folder name for a console inside a "sub-folders are consoles" folder: its ES-DE name
/// when that maps back to it ("snes", "ps2", "gc" …), else its first folder alias.
pub fn console_folder_name(p: &Platform) -> String {
    p.es.iter()
        .chain(p.folders.iter())
        .find(|n| platforms::by_folder_name(n).is_some_and(|x| x.id == p.id))
        .map(|s| s.to_string())
        .unwrap_or_else(|| p.id.to_string())
}

fn hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// A console sub-folder of `dir` for `p`.
fn console_child(dir: &Path, p: &Platform) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            !hidden(&n) && platforms::by_folder_name(&n).is_some_and(|x| x.id == p.id)
        })
        .map(|e| e.path())
        .collect();
    found.sort();
    found.into_iter().next()
}

/// `dir` is scanned as part of one of `roots` (so no new ROM folder is needed).
pub fn covered(roots: &[Root], dir: &Path) -> bool {
    let real = crate::emulation::paths::host_path(dir);
    roots.iter().any(|r| {
        let Ok(rest) = real.strip_prefix(&r.real) else { return false };
        match (r.layout, rest.components().next()) {
            (_, None) => true,
            (Layout::Platform(_), Some(_)) => true,
            (Layout::Consoles, Some(first)) => {
                platforms::by_folder_name(&first.as_os_str().to_string_lossy()).is_some()
            }
        }
    })
}

/// The download folder the user chose, as a "sub-folders are consoles" folder: picking a
/// console's own folder ("~/ROMs/snes") means its parent.
pub fn download_root(chosen: &Path) -> PathBuf {
    let name = chosen.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match (platforms::by_folder_name(&name), chosen.parent()) {
        (Some(_), Some(parent)) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => chosen.to_path_buf(),
    }
}

/// See the module docs. `roots`: the ROM folders as the emulation scanner sees them
/// (user-configured first).
pub fn pick_target(roots: &[Root], p: &Platform, download_dir: Option<&Path>, home: &Path) -> Target {
    let enabled = |r: &&Root| r.folder.as_ref().is_none_or(|f| f.enabled);
    let user = || roots.iter().filter(enabled).filter(|r| r.folder.is_some());
    let auto = || roots.iter().filter(|r| r.folder.is_none());
    let same = |r: &Root| matches!(r.layout, Layout::Platform(x) if x.id == p.id);
    if let Some(r) = user().find(|r| same(r)) {
        return Target::Dir { dir: r.path.clone(), add_folder: None };
    }
    for r in user().chain(auto()).filter(|r| r.layout == Layout::Consoles) {
        if let Some(d) = console_child(&r.path, p) {
            return Target::Dir { dir: d, add_folder: None };
        }
    }
    if let Some(r) = auto().find(|r| same(r)) {
        return Target::Dir { dir: r.path.clone(), add_folder: None };
    }
    if let Some(dl) = download_dir.filter(|d| d.is_absolute()) {
        let root = download_root(dl);
        let add_folder = (!covered(roots, &root.join(console_folder_name(p)))).then(|| root.clone());
        return Target::Dir { dir: root.join(console_folder_name(p)), add_folder };
    }
    let suggest = user()
        .chain(auto())
        .find(|r| r.layout == Layout::Consoles)
        .map(|r| r.path.clone())
        .unwrap_or_else(|| home.join("ROMs"));
    Target::Ask { suggest }
}

/// One path component from the server, if it is a plain name.
fn safe_name(n: &str) -> Option<&str> {
    let n = n.trim_matches('/');
    (!n.is_empty() && n != "." && n != ".." && !n.contains('/') && !n.contains('\0') && !n.contains('\\')).then_some(n)
}

/// A relative path from the server, if it stays inside its folder.
fn safe_rel(rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel.trim_start_matches('/'));
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Normal(s) => out.push(safe_name(&s.to_string_lossy())?),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// Free bytes where `dir` lives (None: unknown).
fn free_space(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let mut probe = dir;
    while !probe.exists() {
        probe = probe.parent()?;
    }
    let c = std::ffi::CString::new(probe.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: c is a valid NUL-terminated path; s is a plain struct the call fills.
    let rc = unsafe { libc::statvfs(c.as_ptr(), &mut s) };
    (rc == 0).then(|| s.f_bavail as u64 * s.f_frsize as u64)
}

fn check_space(dir: &Path, need: u64) -> Result<(), ApiError> {
    if let Some(free) = free_space(dir)
        && need > 0
        && free < need + (64 << 20)
    {
        return Err(ApiError::Disk(format!(
            "not enough free space in {} ({} free, {} needed)",
            dir.display(),
            human(free),
            human(need)
        )));
    }
    Ok(())
}

pub(crate) fn human(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{b} B") } else { format!("{v:.1} {}", U[i]) }
}

fn part_of(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// GET `path` into `dest` (via `dest.part`). Err(Http(404)) is passed on for fallbacks.
fn fetch_to(server: &Server, path: &str, dest: &Path, prog: &Progress) -> Result<(), ApiError> {
    if prog.cancelled() {
        return Err(ApiError::Cancelled);
    }
    let mut resp = server.get(path, true)?;
    let status = resp.status().as_u16();
    if status != 200 {
        return Err(super::status_error(status));
    }
    // A sign-in page in front of the server (Authelia, Authentik …) answers 200 with HTML.
    let html = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.trim_start().to_ascii_lowercase().starts_with("text/html"));
    if html {
        return Err(ApiError::Plain("The RomM server sent a web page instead of the game. Is a sign-in page (Authelia, Authentik …) in front of RomM? Let Lodestar's address through it, or use RomM's own address".into()));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ApiError::Disk(format!("{}: {e}", parent.display())))?;
    }
    let part = part_of(dest);
    let res = (|| -> Result<(), ApiError> {
        let mut f = std::fs::File::create(&part).map_err(|e| ApiError::Disk(format!("{}: {e}", part.display())))?;
        let mut reader = resp.body_mut().with_config().limit(u64::MAX).reader();
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            if prog.cancelled() {
                return Err(ApiError::Cancelled);
            }
            let n = reader.read(&mut buf).map_err(|e| ApiError::Unreachable(super::short_err(&e.to_string())))?;
            if n == 0 {
                break;
            }
            f.write_all(&buf[..n]).map_err(|e| ApiError::Disk(e.to_string()))?;
            prog.done.fetch_add(n as u64, Ordering::Relaxed);
        }
        f.sync_all().map_err(|e| ApiError::Disk(e.to_string()))?;
        Ok(())
    })();
    match res {
        Ok(()) => std::fs::rename(&part, dest).map_err(|e| {
            let _ = std::fs::remove_file(&part);
            ApiError::Disk(e.to_string())
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

/// Download `g` into `dir`. Returns the game's file (or folder) there.
pub fn download(server: &Server, g: &RommGame, dir: &Path, prog: &Progress) -> Result<PathBuf, ApiError> {
    let name = safe_name(&g.fs_name).ok_or_else(|| ApiError::BadReply("odd file name".into()))?;
    prog.total.store(g.size, Ordering::Relaxed);
    check_space(dir, g.size)?;
    if !g.multi {
        let dest = dir.join(name);
        // A file of that name is never overwritten (it may be the user's own copy).
        if dest.exists() {
            return Ok(dest);
        }
        let size_ok = |p: &Path| g.size == 0 || std::fs::metadata(p).is_ok_and(|m| m.len() == g.size);
        fetch_to(server, &format!("/api/roms/{}/content/{}", g.id, crate::art::net::encode(name)), &dest, prog)?;
        if !size_ok(&dest) {
            let _ = std::fs::remove_file(&dest);
            return Err(ApiError::Plain("The download stopped before the whole game arrived. Try again".into()));
        }
        return Ok(dest);
    }
    let folder = dir.join(name);
    match download_files(server, g, &folder, prog) {
        Err(ApiError::Http(404)) | Err(ApiError::Http(405)) => {
            // An older server: the game as one zip.
            let zip = dir.join(format!("{name}.zip"));
            if zip.exists() {
                return Ok(zip);
            }
            prog.done.store(0, Ordering::Relaxed);
            let path = format!("/api/roms/{}/content/{}", g.id, crate::art::net::encode(&format!("{name}.zip")));
            fetch_to(server, &path, &zip, prog)?;
            Ok(zip)
        }
        Err(e) => Err(e),
        Ok(()) => Ok(folder),
    }
}

/// Each file of a multi-file game into `folder` (made by us: removed again on failure when we
/// created it and it is empty).
fn download_files(server: &Server, g: &RommGame, folder: &Path, prog: &Progress) -> Result<(), ApiError> {
    let detail = server.get_json(&format!("/api/roms/{}", g.id))?;
    let rom_path = detail.get("full_path").and_then(Value::as_str).unwrap_or("").trim_end_matches('/').to_string();
    let files: Vec<(u64, String, PathBuf, u64)> = detail
        .get("files")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|f| {
                    let id = f.get("id")?.as_u64()?;
                    let fname = safe_name(f.get("file_name")?.as_str()?)?.to_string();
                    // Relative to the game's folder: from full paths, else the file's own folder.
                    let full = f.get("full_path").and_then(Value::as_str).unwrap_or("");
                    let rel = full
                        .strip_prefix(&rom_path)
                        .filter(|r| !rom_path.is_empty() && r.starts_with('/'))
                        .and_then(safe_rel)
                        .unwrap_or_else(|| PathBuf::from(&fname));
                    let size = f.get("file_size_bytes").and_then(Value::as_u64).unwrap_or(0);
                    Some((id, fname, rel, size))
                })
                .collect()
        })
        .unwrap_or_default();
    if files.is_empty() {
        // Nothing listed per file: the zip route.
        return Err(ApiError::Http(404));
    }
    let total: u64 = files.iter().map(|f| f.3).sum();
    if total > 0 {
        prog.total.store(total, Ordering::Relaxed);
    }
    let created = !folder.exists();
    let res = (|| {
        for (id, fname, rel, _) in &files {
            let dest = folder.join(rel);
            if dest.exists() {
                continue;
            }
            fetch_to(server, &format!("/api/roms/{id}/files/content/{}", crate::art::net::encode(fname)), &dest, prog)?;
        }
        Ok(())
    })();
    if res.is_err() && created {
        // Only what this download wrote: the folder was new.
        let _ = std::fs::remove_dir_all(folder);
    }
    res
}
