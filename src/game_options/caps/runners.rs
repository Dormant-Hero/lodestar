//! Proton / Wine runners: where they are, and what each supports (cached per runner).

use super::*;

// ----------------------------------------------------------------------------------- runners

/// At most this many Proton dirs outside compatibilitytools.d are pre-probed (each first scan
/// reads a few MB).
pub(super) const MAX_EXTRA_RUNNERS: usize = 32;

/// Folders other launchers keep Proton builds in, each holding one build per subfolder:
/// Lutris (`runners/wine`, `runners/proton`), Heroic (`tools/proton`, `tools/wine`) and Bottles
/// (`runners`), native and Flatpak. Only subfolders with a `proton` script count.
pub(super) fn launcher_runner_dirs(r: &ProbeRoots) -> Vec<PathBuf> {
    let h = &r.home;
    let data = r.data_home();
    let config = r.config_home();
    let lutris = [data.join("lutris"), h.join(".var/app/net.lutris.Lutris/data/lutris")];
    let heroic = [config.join("heroic"), h.join(".var/app/com.heroicgameslauncher.hgl/config/heroic")];
    let bottles = [data.join("bottles"), h.join(".var/app/com.usebottles.bottles/data/bottles")];
    let mut out = Vec::new();
    for l in &lutris {
        out.push(l.join("runners/wine"));
        out.push(l.join("runners/proton"));
    }
    for hr in &heroic {
        out.push(hr.join("tools/proton"));
        out.push(hr.join("tools/wine"));
    }
    for b in &bottles {
        out.push(b.join("runners"));
    }
    out
}

/// Steam's own Proton builds (`steamapps/common/Proton …`) in every Steam library. Names are
/// filtered before anything is stat'ed, so a library with many games costs one listing.
pub(super) fn steam_proton_dirs(r: &ProbeRoots) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in steam_roots(r) {
        for lib in crate::scan::steam::library_folders(&root) {
            let Ok(rd) = std::fs::read_dir(lib.join("steamapps/common")) else { continue };
            let mut found: Vec<PathBuf> = rd
                .flatten()
                .take(MAX_DIR_ENTRIES * 16)
                .filter(|e| e.file_name().to_str().is_some_and(|n| n.starts_with("Proton")))
                .map(|e| e.path())
                .collect();
            found.sort();
            out.extend(found);
        }
    }
    out
}

/// Probe every Proton dir in the compatibilitytools.d folders (keys are the paths as listed).
/// Only these go into `Caps::runners`: compose picks `PROTONPATH=GE-Proton` / plain `umu-run`
/// runners by name among its keys (spec §4.2, umu's storage), so a Heroic or Bottles
/// `GE-Proton*` must not be a candidate there. Other launchers' builds are only warmed
/// ([`warm_extra_runners`]).
pub(super) fn pre_probe_runners(r: &ProbeRoots) -> HashMap<PathBuf, Arc<RunnerCaps>> {
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut out = HashMap::new();
    for dir in r.compat_dirs() {
        let Ok(canon) = dir.canonicalize() else { continue };
        if seen.contains(&canon) {
            continue;
        }
        seen.push(canon);
        for (_, path) in list_dir(&dir) {
            if path.join("proton").is_file() {
                let caps = runner_caps(&path);
                out.insert(path, caps);
            }
        }
    }
    out
}

/// Fill [`runner_caps`]'s process cache (not `Caps::runners`) for Steam's own Proton builds and
/// the ones Lutris / Heroic / Bottles installed, at most [`MAX_EXTRA_RUNNERS`] of them, so a
/// game using one doesn't scan its files on the UI thread on first resolve (compose falls back
/// to `runner_caps` for a dir not in `Caps::runners`). Dirs inside compatibilitytools.d are
/// skipped (already probed). Returns how many dirs it probed.
pub(super) fn warm_extra_runners(r: &ProbeRoots) -> usize {
    let compat: Vec<PathBuf> = r.compat_dirs().iter().filter_map(|d| d.canonicalize().ok()).collect();
    let mut seen: Vec<PathBuf> = compat.clone();
    let mut extra: Vec<PathBuf> = steam_proton_dirs(r);
    for dir in launcher_runner_dirs(r) {
        let Ok(canon) = dir.canonicalize() else { continue };
        if seen.contains(&canon) {
            continue;
        }
        seen.push(canon);
        extra.extend(list_dir(&dir).into_iter().map(|(_, p)| p));
    }
    let mut done: Vec<PathBuf> = Vec::new();
    for path in extra {
        if done.len() >= MAX_EXTRA_RUNNERS {
            log::debug!("game options: pre-probed {MAX_EXTRA_RUNNERS} extra Proton dirs, skipping the rest");
            break;
        }
        if !path.join("proton").is_file() {
            continue;
        }
        let canon = path.canonicalize().unwrap_or_else(|_| path.clone());
        if done.contains(&canon) || canon.parent().is_some_and(|p| compat.iter().any(|c| c == p)) {
            continue;
        }
        runner_caps(&path);
        done.push(canon);
    }
    done.len()
}

/// Cache entry: mtimes of `proton` and `version` when scanned.
pub(super) type RunnerStamp = (Option<SystemTime>, Option<SystemTime>);

/// Process-wide runner cache: canonical Proton dir (or canonical wine binary, [`wine_caps`]) →
/// (stamp when scanned, caps).
pub(super) type RunnerCache = Mutex<HashMap<PathBuf, (RunnerStamp, Arc<RunnerCaps>)>>;

pub(super) fn runner_cache() -> &'static RunnerCache {
    static C: std::sync::OnceLock<RunnerCache> = std::sync::OnceLock::new();
    C.get_or_init(Default::default)
}

/// What the Proton build in `dir` supports (spec §4.2 RunnerCaps). Cached process-wide by
/// (canonical dir, mtime of `proton`, mtime of `version`). A dir without a `proton` script is
/// not a Proton dir → [`RunnerCaps::unknown`] (not cached).
///
/// A cache hit costs two `stat`s: the entry is also stored under `dir` as given, so the
/// per-frame resolve doesn't canonicalize (one syscall per path component) every time.
pub fn runner_caps(dir: &Path) -> Arc<RunnerCaps> {
    let script = dir.join("proton");
    let Ok(meta) = std::fs::metadata(&script) else {
        return Arc::new(RunnerCaps::unknown());
    };
    if !meta.is_file() {
        return Arc::new(RunnerCaps::unknown());
    }
    let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let stamp = (meta.modified().ok(), mtime(&dir.join("version")));
    let cached = |key: &Path| {
        let c = runner_cache().lock().ok()?;
        let (s, caps) = c.get(key)?;
        (*s == stamp).then(|| caps.clone())
    };
    if let Some(caps) = cached(dir) {
        return caps;
    }
    let key = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let caps = cached(&key).unwrap_or_else(|| Arc::new(scan_runner(dir)));
    if let Ok(mut c) = runner_cache().lock() {
        c.insert(key, (stamp, caps.clone()));
        c.insert(dir.to_path_buf(), (stamp, caps.clone()));
    }
    caps
}

/// The uncached scan behind [`runner_caps`].
pub(super) fn scan_runner(dir: &Path) -> RunnerCaps {
    let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let version = read_text(&dir.join("version")).map(|t| parse_version_file(&t)).filter(|v| !v.is_empty());
    let version = version.unwrap_or_else(|| dir_name.to_string());
    let mut vars = HashSet::new();
    if let Some(t) = read_small(&dir.join("proton")) {
        collect_vars(&t, &mut vars);
    }
    for (name, path) in list_dir(&dir.join("protonfixes")) {
        if name.ends_with(".py")
            && let Some(t) = read_small(&path)
        {
            collect_vars(&t, &mut vars);
        }
    }
    let files = dir.join("files");
    let wine = scan_wine_files(&files, &wine_unix_dir(&files).unwrap_or_else(|| files.join("lib/wine/x86_64-unix")));
    RunnerCaps {
        family: runner_family(&version),
        version,
        vars,
        contrib_fsr4: ["files/contrib/amdxcffx64.dll", "contrib/amdxcffx64.dll"].iter().any(|p| dir.join(p).is_file()),
        ..wine
    }
}

/// The 64-bit unix-side library folder of a Wine tree rooted at `root` (the folder holding
/// `bin/wine`): `lib/wine/x86_64-unix`, `lib64/wine/x86_64-unix`, or Debian's multiarch
/// `lib/x86_64-linux-gnu/wine/x86_64-unix`.
pub(super) fn wine_unix_dir(root: &Path) -> Option<PathBuf> {
    ["lib/wine/x86_64-unix", "lib64/wine/x86_64-unix", "lib/x86_64-linux-gnu/wine/x86_64-unix"]
        .iter()
        .map(|d| root.join(d))
        .find(|d| d.is_dir())
}

/// The file checks shared by Proton and plain Wine: NTSync strings in `<root>/bin/wineserver`,
/// `winewayland.so` and the fullscreen-hack FSR string in `win32u.so` (spec §4.2 RunnerCaps).
/// Family Unknown, no vars.
pub(super) fn scan_wine_files(root: &Path, unix_dir: &Path) -> RunnerCaps {
    let ws = file_contains(&root.join("bin/wineserver"), &[b"/dev/ntsync", b"PROTON_NO_NTSYNC"]);
    RunnerCaps {
        ntsync: ws[0],
        wineserver_no_ntsync: ws[1],
        winewayland: unix_dir.join("winewayland.so").is_file(),
        fshack_fsr: file_contains(&unix_dir.join("win32u.so"), &[b"WINE_FULLSCREEN_FSR"])[0],
        ..RunnerCaps::unknown()
    }
}

/// What the plain Wine at `wine` (the resolved, absolute wine binary) supports, for launch kind
/// W (spec §2.1 `winewayland.so`, §2.3 `win32u.so` FSR string, §2.2 wineserver).
///
/// * A wine inside a Proton dir (`<dir>/files/bin/wine*` with `<dir>/proton`) gets that dir's
///   [`runner_caps`] (spec §3.2).
/// * Otherwise the Wine root is the folder above `bin/` (tried for the path as given and with
///   symlinks resolved, so `/usr/bin/wine` → `/usr`); the result has family Unknown and no vars
///   (`PROTON_*` don't apply to plain Wine), and `version` is the root's folder name.
/// * A path that isn't a file, or a tree without a unix library folder, gives
///   [`RunnerCaps::unknown`] with every file check false.
///
/// Cached in the same process-wide cache as [`runner_caps`], by (canonical wine path, mtimes of
/// wine and win32u.so); cheap after the first call.
pub fn wine_caps(wine: &Path) -> Arc<RunnerCaps> {
    if !wine.is_file() {
        return Arc::new(RunnerCaps::unknown());
    }
    let canon = wine.canonicalize().unwrap_or_else(|_| wine.to_path_buf());
    let roots: Vec<PathBuf> =
        [wine, canon.as_path()].iter().filter_map(|p| Some(p.parent()?.parent()?.to_path_buf())).collect();
    if let Some(files) = roots.iter().find(|r| r.file_name().is_some_and(|n| n == "files"))
        && let Some(dir) = files.parent()
        && dir.join("proton").is_file()
    {
        return runner_caps(dir);
    }
    let Some((root, unix_dir)) = roots.iter().find_map(|r| Some((r.clone(), wine_unix_dir(r)?))) else {
        return Arc::new(RunnerCaps::unknown());
    };
    let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let stamp = (mtime(&canon), mtime(&unix_dir.join("win32u.so")));
    if let Ok(c) = runner_cache().lock()
        && let Some((s, caps)) = c.get(&canon)
        && *s == stamp
    {
        return caps.clone();
    }
    let version = root.file_name().and_then(|n| n.to_str()).unwrap_or("wine").to_string();
    let caps = Arc::new(RunnerCaps { version, ..scan_wine_files(&root, &unix_dir) });
    if let Ok(mut c) = runner_cache().lock() {
        c.insert(canon, (stamp, caps.clone()));
    }
    caps
}

/// The `version` file is "<build timestamp> <name>" ("1793687165 GE-Proton11-12"); returns the
/// name (the whole first line when there is no timestamp).
pub fn parse_version_file(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    match line.split_once(char::is_whitespace) {
        Some((ts, rest)) if ts.bytes().all(|b| b.is_ascii_digit()) => rest.trim().to_string(),
        _ => line.to_string(),
    }
}

/// "GE-Proton…" → Ge, contains "cachyos" → Cachyos, otherwise Valve (UMU-Proton, Proton 10 …).
pub fn runner_family(version: &str) -> RunnerFamily {
    if version.starts_with("GE-Proton") {
        RunnerFamily::Ge
    } else if version.to_ascii_lowercase().contains("cachyos") {
        RunnerFamily::Cachyos
    } else {
        RunnerFamily::Valve
    }
}

/// Add every `PROTON_[A-Z0-9_]+` and `DXVK_[A-Z_]+` token in `text` to `out` (trailing `_`
/// trimmed, so `"PROTON_" + name` adds nothing).
pub(super) fn collect_vars(text: &[u8], out: &mut HashSet<String>) {
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    for (prefix, digits) in [(&b"PROTON_"[..], true), (&b"DXVK_"[..], false)] {
        let mut i = 0;
        while i + prefix.len() <= text.len() {
            if &text[i..i + prefix.len()] != prefix || (i > 0 && is_word(text[i - 1])) {
                i += 1;
                continue;
            }
            let mut end = i + prefix.len();
            while end < text.len()
                && (text[end].is_ascii_uppercase() || text[end] == b'_' || (digits && text[end].is_ascii_digit()))
            {
                end += 1;
            }
            let word_end = end;
            while end > i + prefix.len() && text[end - 1] == b'_' {
                end -= 1;
            }
            // A token that runs on into lowercase letters (`PROTON_foo`) isn't a variable name.
            let clean = word_end >= text.len() || !is_word(text[word_end]);
            if end > i + prefix.len() && clean {
                out.insert(String::from_utf8_lossy(&text[i..end]).into_owned());
            }
            i = word_end.max(i + 1);
        }
    }
}
