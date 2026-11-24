//! Flatpak permissions: where an app is installed and which host folders its sandbox can see
//! (its own `filesystems=` plus `flatpak override`s), so a launch adds `--filesystem` only when
//! the ROM is out of sight.

use super::*;

/// Where a Flatpak app is installed: `Some("system" | "user")`.
pub fn flatpak_installation(roots: &ProbeRoots, app_id: &str) -> Option<&'static str> {
    if roots.system_flatpak.join("app").join(app_id).is_dir() {
        Some("system")
    } else if roots.user_flatpak.join("app").join(app_id).is_dir() {
        Some("user")
    } else {
        None
    }
}

/// `filesystems=` of the `[Context]` group of a Flatpak key file (metadata / override).
pub(super) fn context_filesystems(text: &str) -> Vec<String> {
    let mut in_ctx = false;
    let mut out = vec![];
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_ctx = l == "[Context]";
            continue;
        }
        if in_ctx && let Some(v) = l.strip_prefix("filesystems=") {
            out.extend(v.split(';').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string));
        }
    }
    out
}

/// The app's effective `filesystems` permissions: the installed metadata, then the system and
/// user overrides (`global` and per app). `!x` entries remove `x`. Empty when not installed.
pub fn flatpak_filesystems(roots: &ProbeRoots, app_id: &str) -> Vec<String> {
    let inst = match flatpak_installation(roots, app_id) {
        Some("user") => &roots.user_flatpak,
        Some(_) => &roots.system_flatpak,
        None => return vec![],
    };
    let mut files = vec![inst.join("app").join(app_id).join("current/active/metadata")];
    for r in [&roots.system_flatpak, &roots.user_flatpak] {
        files.push(r.join("overrides/global"));
        files.push(r.join("overrides").join(app_id));
    }
    let base = |s: &str| -> String {
        let s = s.trim_start_matches('!');
        match s.rsplit_once(':') {
            Some((b, "ro" | "rw" | "create")) => b.to_string(),
            _ => s.to_string(),
        }
    };
    let mut out: Vec<String> = vec![];
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        for e in context_filesystems(&text) {
            let b = base(&e);
            out.retain(|x| base(x) != b);
            if !e.starts_with('!') {
                out.push(e);
            }
        }
    }
    out
}

/// XDG user folder of `xdg-download` etc. (from `~/.config/user-dirs.dirs`, else the default).
pub(super) fn xdg_user_dir(home: &Path, name: &str) -> Option<PathBuf> {
    let (var, default) = match name {
        "xdg-desktop" => ("XDG_DESKTOP_DIR", "Desktop"),
        "xdg-documents" => ("XDG_DOCUMENTS_DIR", "Documents"),
        "xdg-download" => ("XDG_DOWNLOAD_DIR", "Downloads"),
        "xdg-music" => ("XDG_MUSIC_DIR", "Music"),
        "xdg-pictures" => ("XDG_PICTURES_DIR", "Pictures"),
        "xdg-public-share" => ("XDG_PUBLICSHARE_DIR", "Public"),
        "xdg-templates" => ("XDG_TEMPLATES_DIR", "Templates"),
        "xdg-videos" => ("XDG_VIDEOS_DIR", "Videos"),
        "xdg-config" => return Some(home.join(".config")),
        "xdg-data" => return Some(home.join(".local/share")),
        "xdg-cache" => return Some(home.join(".cache")),
        _ => return None,
    };
    if let Ok(text) = std::fs::read_to_string(home.join(".config/user-dirs.dirs")) {
        for line in text.lines() {
            if let Some(v) = line.trim().strip_prefix(var).and_then(|r| r.strip_prefix('=')) {
                return Some(expand_home(v.trim().trim_matches('"'), home));
            }
        }
    }
    Some(home.join(default))
}

/// Do these `filesystems` permissions let the app see `path` (read-only is enough)?
/// `host` covers everything but the OS folders, `home` the home folder, `xdg-*` / `~/x` /
/// absolute entries their folder.
pub fn fs_covers(filesystems: &[String], home: &Path, path: &Path) -> bool {
    const OS: &[&str] = &["/usr", "/etc", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/app", "/proc", "/sys", "/dev"];
    let os_path = OS.iter().any(|d| path.starts_with(d));
    filesystems.iter().any(|e| {
        if e.starts_with('!') {
            return false;
        }
        let b = match e.rsplit_once(':') {
            Some((b, "ro" | "rw" | "create")) => b,
            _ => e.as_str(),
        };
        match b {
            "host" => !os_path,
            "host-all" => true,
            "host-os" => ["/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64"].iter().any(|d| path.starts_with(d)),
            "host-etc" => path.starts_with("/etc"),
            "home" | "~" => path.starts_with(home),
            _ => {
                let dir = if let Some(rest) = b.strip_prefix("~/") {
                    home.join(rest)
                } else if b.starts_with('/') {
                    PathBuf::from(b)
                } else if b.starts_with("xdg-") {
                    let (name, sub) = b.split_once('/').unwrap_or((b, ""));
                    match xdg_user_dir(home, name) {
                        Some(d) if sub.is_empty() => d,
                        Some(d) => d.join(sub),
                        None => return false,
                    }
                } else {
                    return false;
                };
                path.starts_with(&dir)
            }
        }
    })
}

/// Process-wide: each Flatpak's permissions, per home (filled by detection, read when building
/// launch commands so that never touches the disk twice).
pub(super) type AccessMap = HashMap<(PathBuf, String), Vec<String>>;
pub(super) static ACCESS: Mutex<Option<AccessMap>> = Mutex::new(None);

pub(super) fn remember_access(home: &Path, app_id: &str, fs: Vec<String>) {
    ACCESS.lock().get_or_insert_with(HashMap::new).insert((home.to_path_buf(), app_id.to_string()), fs);
}

/// A Flatpak's `filesystems` permissions as last detected (read from the real installation the
/// first time when detection hasn't seen it).
pub fn flatpak_access(home: &Path, app_id: &str) -> Vec<String> {
    let key = (home.to_path_buf(), app_id.to_string());
    if let Some(v) = ACCESS.lock().as_ref().and_then(|m| m.get(&key)) {
        return v.clone();
    }
    let fs = flatpak_filesystems(&ProbeRoots::system(home), app_id);
    remember_access(home, app_id, fs.clone());
    fs
}
