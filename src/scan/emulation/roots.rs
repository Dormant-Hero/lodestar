//! Where ROMs live: the user's ROM folders plus folders found automatically (common layouts in
//! the home folder, ES-DE / EmuDeck settings, drives under /mnt, /media and /run/media, and the
//! game folders the emulators' own settings list), and remembering them between scans.
//!
//! Remembered roots (`<data_dir>/emulation/roots.json`) let a scan tell "the drive is not
//! connected" (return an error, so the library keeps those games as they are) from "the ROMs
//! were deleted". The rules:
//! * a ROM folder the user added that is missing (or an empty mount point that had games) is
//!   always an error, until the user removes or disables it;
//! * a remembered automatic folder that is missing is an error for up to [`MAX_MISSES`] scans
//!   in a row and is then forgotten (its games turn "not installed"), so an old drive never
//!   blocks the emulation library for good;
//! * folders on removable drives (/run/media, /media) never block: unplugging a stick just makes
//!   its games "not installed" until it is back;
//! * a remembered folder that was since ignored, or automatic folders switched off, is forgotten.

use crate::config::{EmulationConfig, RomFolder};
use crate::emulation::paths::host_path;
use crate::emulation::platforms::{self, Platform};
use crate::scan::ScanContext;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Scans a missing automatic folder blocks before it is forgotten.
pub const MAX_MISSES: u32 = 3;
/// Folders looked at below each system root (/mnt, /media, /run/media/$USER) while looking for
/// ROM folders.
const MAX_PROBE_DIRS: usize = 400;

/// How a root's games get their console.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Sub-folders are consoles ("ps2", "gc", "3ds" …).
    Consoles,
    /// Every ROM in it is for this console.
    Platform(&'static Platform),
}

impl Layout {
    /// The console id ("" for a "sub-folders are consoles" folder).
    #[cfg(test)]
    pub fn map_id(&self) -> &'static str {
        match self {
            Layout::Consoles => "",
            Layout::Platform(p) => p.id,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Root {
    /// Folder id: the config key, or "auto-<8 hex>".
    pub id: String,
    pub path: PathBuf,
    /// Where it really is ([`host_path`]: symlinks and Flatpak document-portal paths resolved);
    /// roots are compared by it, so one folder reached two ways is scanned once.
    pub real: PathBuf,
    pub layout: Layout,
    /// The user's folder settings (None: found automatically).
    pub folder: Option<RomFolder>,
    /// The user chose the console (content never moves games to another one).
    pub explicit: bool,
    /// Where it was found, for logs / previews.
    pub how: String,
}

/// The machine the scan looks at (injectable so tests never touch the real /mnt).
#[derive(Clone, Debug, Default)]
pub struct Env {
    /// Drive roots searched for ROM folders (depth ≤ 3 below each).
    pub system: Vec<PathBuf>,
    /// Where removable drives are mounted (/run/media, /media): folders there never block a scan.
    pub removable: Vec<PathBuf>,
    /// `/proc/self/mounts` (network mounts are never walked).
    pub mounts: Option<PathBuf>,
}

impl Env {
    pub fn real(ctx: &ScanContext) -> Env {
        let user = std::env::var("USER")
            .ok()
            .filter(|u| !u.is_empty())
            .or_else(|| ctx.home.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let mut system = vec![PathBuf::from("/mnt"), PathBuf::from("/media")];
        if !user.is_empty() {
            system.push(PathBuf::from("/run/media").join(user));
        }
        Env {
            system,
            removable: vec![PathBuf::from("/run/media"), PathBuf::from("/media")],
            mounts: Some(PathBuf::from("/proc/self/mounts")),
        }
    }
}

/// "auto-<8 hex>" for a path.
pub fn auto_id(path: &Path) -> String {
    format!("auto-{:08x}", crc32fast::hash(path.as_os_str().as_encoded_bytes()))
}

fn hidden(name: &str) -> bool {
    name.starts_with('.') || name.starts_with('$') || name.starts_with('＄') || name.eq_ignore_ascii_case("lost+found")
}

/// Sub-folders of `dir` whose names are consoles.
fn console_children(dir: &Path) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir() || t.is_symlink()))
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            !hidden(&n) && platforms::by_folder_name(&n).is_some()
        })
        .take(2)
        .count()
}

/// The layout of a folder whose console is not set: its own name, else its sub-folders.
pub fn layout_of(path: &Path) -> Layout {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match platforms::by_folder_name(&name) {
        Some(p) if console_children(path) < 2 => Layout::Platform(p),
        _ => Layout::Consoles,
    }
}

/// A configured folder as a root.
pub fn configured(id: &str, f: &RomFolder) -> Root {
    let explicit = platforms::by_id(&f.platform);
    Root {
        id: id.to_string(),
        path: f.path.clone(),
        real: host_path(&f.path),
        layout: explicit.map(Layout::Platform).unwrap_or_else(|| layout_of(&f.path)),
        folder: Some(f.clone()),
        explicit: explicit.is_some(),
        how: "Added by you".into(),
    }
}

fn expand(home: &Path, s: &str) -> Option<PathBuf> {
    let s = s.trim().trim_matches('"').trim_matches('\'').trim();
    if s.is_empty() {
        return None;
    }
    let hs = home.to_string_lossy();
    let s = s.replace("%HOME%", &hs).replace("$HOME", &hs).replace("${HOME}", &hs);
    let p = if let Some(rest) = s.strip_prefix("~/") {
        home.join(rest)
    } else if s == "~" {
        home.to_path_buf()
    } else {
        PathBuf::from(s)
    };
    p.is_absolute().then_some(p)
}

/// Minimal INI: (section, key, value) in file order (repeated keys kept).
pub fn ini(text: &str) -> Vec<(String, String, String)> {
    let mut out = vec![];
    let mut section = String::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with(';') || l.starts_with('#') {
            continue;
        }
        if let Some(s) = l.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s.trim().to_string();
        } else if let Some((k, v)) = l.split_once('=') {
            out.push((section.clone(), k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

fn read(p: &Path) -> Option<String> {
    crate::scan::util::read_text(p).ok()
}

/// ES-DE `<string name="KEY" value="…" />` from es_settings.xml.
fn esde_setting(text: &str, key: &str) -> Option<String> {
    let needle = format!("name=\"{key}\"");
    let line = text.lines().find(|l| l.contains(&needle))?;
    let v = line.split("value=\"").nth(1)?.split('"').next()?;
    (!v.is_empty()).then(|| v.replace("&amp;", "&"))
}

/// ES-DE's downloaded media folder (box art next to the ROM folders), if present.
pub fn esde_media_dir(home: &Path) -> Option<PathBuf> {
    let set = read(&home.join("ES-DE/settings/es_settings.xml")).and_then(|t| esde_setting(&t, "MediaDirectory"));
    let dir = set.and_then(|s| expand(home, &s)).unwrap_or_else(|| home.join("ES-DE/downloaded_media"));
    dir.is_dir().then_some(dir)
}

/// Folders where ROM collections usually live in the home folder.
fn home_layouts(home: &Path) -> Vec<(PathBuf, String)> {
    let mut out: Vec<(PathBuf, String)> =
        ["ROMs", "roms", "Roms", "Emulation/roms", "RetroPie/roms", "retrodeck/roms", "Games/roms", "Games/ROMs"]
            .iter()
            .map(|r| (home.join(r), format!("~/{r}")))
            .collect();
    if let Some(t) = read(&home.join("ES-DE/settings/es_settings.xml"))
        && let Some(p) = esde_setting(&t, "ROMDirectory").and_then(|s| expand(home, &s))
    {
        out.push((p, "ES-DE's ROM folder".into()));
    }
    if let Some(t) = read(&home.join(".config/EmuDeck/settings.sh")) {
        let mut emu = home.join("Emulation").to_string_lossy().into_owned();
        for line in t.lines() {
            let l = line.trim().trim_start_matches("export ").trim();
            if let Some(v) = l.strip_prefix("emulationPath=") {
                emu = v.trim_matches('"').replace("$HOME", &home.to_string_lossy());
            } else if let Some(v) = l.strip_prefix("romsPath=") {
                let v = v.trim_matches('"').replace("$emulationPath", &emu).replace("${emulationPath}", &emu);
                if let Some(p) = expand(home, &v) {
                    out.push((p, "EmuDeck's ROM folder".into()));
                }
            }
        }
    }
    out
}

/// Game folders listed in the emulators' own settings, with the console each emulator is for.
pub fn emulator_dirs(ctx: &ScanContext) -> Vec<(PathBuf, &'static str, String)> {
    let home = &ctx.home;
    let mut out: Vec<(PathBuf, &'static str, String)> = vec![];
    let fp = |id: &str, rest: &str| ctx.flatpak_app(id).join(rest);
    let mut add = |p: Option<PathBuf>, plat: &'static str, how: &str| {
        if let Some(p) = p.filter(|p| p.is_absolute()) {
            let p = PathBuf::from(p.to_string_lossy().trim_end_matches('/'));
            if !out.iter().any(|(q, _, _)| *q == p) {
                out.push((p, plat, how.to_string()));
            }
        }
    };
    // Dolphin: [General] ISOPath0..N.
    for f in [
        home.join(".config/dolphin-emu/Dolphin.ini"),
        home.join(".dolphin-emu/Config/Dolphin.ini"),
        fp("org.DolphinEmu.dolphin-emu", "config/dolphin-emu/Dolphin.ini"),
        fp("io.github.shiiion.primehack", "config/dolphin-emu/Dolphin.ini"),
    ] {
        for (s, k, v) in read(&f).map(|t| ini(&t)).unwrap_or_default() {
            if s.eq_ignore_ascii_case("General")
                && k.starts_with("ISOPath")
                && k[7..].chars().all(|c| c.is_ascii_digit())
                && k.len() > 7
            {
                add(expand(home, &v), "nintendo_gamecube", "Dolphin's game folders");
            }
        }
    }
    // PCSX2 / DuckStation: [GameList] Paths / RecursivePaths (repeated keys).
    let gamelist = [
        (home.join(".config/PCSX2/inis/PCSX2.ini"), "sony_playstation2", "PCSX2's game folders"),
        (fp("net.pcsx2.PCSX2", "config/PCSX2/inis/PCSX2.ini"), "sony_playstation2", "PCSX2's game folders"),
        (home.join(".local/share/duckstation/settings.ini"), "sony_playstation", "DuckStation's game folders"),
        (home.join(".config/duckstation/settings.ini"), "sony_playstation", "DuckStation's game folders"),
        (
            fp("org.duckstation.DuckStation", "config/duckstation/settings.ini"),
            "sony_playstation",
            "DuckStation's game folders",
        ),
    ];
    for (f, plat, how) in gamelist {
        for (s, k, v) in read(&f).map(|t| ini(&t)).unwrap_or_default() {
            if s.eq_ignore_ascii_case("GameList") && (k == "Paths" || k == "RecursivePaths") {
                add(expand(home, &v), plat, how);
            }
        }
    }
    // PPSSPP: pinned folders (and the last folder browsed when it is named like a PSP folder).
    for f in [
        home.join(".config/ppsspp/PSP/SYSTEM/ppsspp.ini"),
        fp("org.ppsspp.PPSSPP", "config/ppsspp/PSP/SYSTEM/ppsspp.ini"),
    ] {
        for (s, k, v) in read(&f).map(|t| ini(&t)).unwrap_or_default() {
            let pinned = s.eq_ignore_ascii_case("PinnedPaths") && k.starts_with("Path");
            let current = s.eq_ignore_ascii_case("General") && k == "CurrentDirectory";
            if pinned || current {
                let Some(p) = expand(home, &v) else { continue };
                let named_psp = p
                    .file_name()
                    .and_then(|n| platforms::by_folder_name(&n.to_string_lossy()))
                    .is_some_and(|p| p.id == "sony_psp");
                if pinned || named_psp {
                    add(Some(p), "sony_psp", "PPSSPP's game folders");
                }
            }
        }
    }
    // Eden / yuzu forks: [UI] Paths\gamedirs\N\path=…
    for (dir, app) in [
        ("eden", "dev.eden_emu.eden"),
        ("yuzu", "org.yuzu_emu.yuzu"),
        ("citron", "org.citron_emu.citron"),
        ("sudachi", "org.sudachi_emu.sudachi"),
        ("suyu", "org.suyu_emu.suyu"),
    ] {
        for f in [home.join(format!(".config/{dir}/qt-config.ini")), fp(app, &format!("config/{dir}/qt-config.ini"))] {
            for (_, k, v) in read(&f).map(|t| ini(&t)).unwrap_or_default() {
                let k = k.replace('/', "\\");
                if k.starts_with("Paths\\gamedirs\\")
                    && k.ends_with("\\path")
                    && !["SDMC", "UserNAND", "SysNAND"].contains(&v.as_str())
                {
                    add(expand(home, &v), "nintendo_switch", "the Switch emulator's game folders");
                }
            }
        }
    }
    // Ryujinx: Config.json "game_dirs".
    for f in [
        home.join(".config/Ryujinx/Config.json"),
        fp("io.github.ryubing.Ryujinx", "config/Ryujinx/Config.json"),
        fp("org.ryujinx.Ryujinx", "config/Ryujinx/Config.json"),
    ] {
        if let Some(v) = read(&f).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
            for d in v.get("game_dirs").and_then(|d| d.as_array()).into_iter().flatten() {
                add(d.as_str().and_then(|s| expand(home, s)), "nintendo_switch", "Ryujinx's game folders");
            }
        }
    }
    // Cemu: settings.xml <GamePaths><Entry>.
    for f in [home.join(".config/Cemu/settings.xml"), fp("info.cemu.Cemu", "config/Cemu/settings.xml")] {
        if let Some(t) = read(&f)
            && let Ok(doc) = roxmltree::Document::parse(&t)
        {
            for gp in doc.descendants().filter(|n| n.has_tag_name("GamePaths")) {
                for e in gp.children().filter(|n| n.has_tag_name("Entry")) {
                    add(e.text().and_then(|s| expand(home, s)), "nintendo_wiiu", "Cemu's game folders");
                }
            }
        }
    }
    // RPCS3: vfs.yml "/games/" (its disc games folder) and games.yml (one folder per game).
    for cfg in [home.join(".config/rpcs3"), fp("net.rpcs3.RPCS3", "config/rpcs3")] {
        if let Some(t) = read(&cfg.join("vfs.yml")) {
            for line in t.lines() {
                if let Some(v) = line.trim().strip_prefix("/games/:") {
                    let v =
                        v.trim().trim_matches('"').replace("$(EmulatorDir)", &format!("{}/", cfg.to_string_lossy()));
                    add(expand(home, &v), "sony_playstation3", "RPCS3's games folder");
                }
            }
        }
        // games.yml lists single game folders; their parent is a games folder only when it holds
        // several of them or is named like one ("ps3") — not ~/Downloads for one game put there.
        if let Some(t) = read(&cfg.join("games.yml")) {
            let mut parents: Vec<PathBuf> = vec![];
            for line in t.lines() {
                if let Some((id, v)) = line.split_once(':')
                    && id.trim().len() == 9
                    && let Some(parent) = expand(home, v).and_then(|p| p.parent().map(Path::to_path_buf))
                {
                    parents.push(parent);
                }
            }
            for p in &parents {
                let named = p
                    .file_name()
                    .and_then(|n| platforms::by_folder_name(&n.to_string_lossy()))
                    .is_some_and(|x| x.id == "sony_playstation3");
                if named || parents.iter().filter(|q| *q == p).count() >= 2 {
                    add(Some(p.clone()), "sony_playstation3", "RPCS3's game list");
                }
            }
        }
    }
    out
}

fn network_mounts(env: &Env) -> Vec<PathBuf> {
    const NET: &[&str] = &[
        "nfs",
        "nfs4",
        "cifs",
        "smb3",
        "smbfs",
        "fuse.sshfs",
        "sshfs",
        "fuse.rclone",
        "fuse.s3fs",
        "davfs",
        "fuse.davfs2",
        "9p",
        "afs",
        "ceph",
        "glusterfs",
        "fuse.gvfsd-fuse",
        "fuse.glusterfs",
    ];
    let Some(text) = env.mounts.as_deref().and_then(|p| std::fs::read_to_string(p).ok()) else { return vec![] };
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let (_, mp, fs) = (it.next()?, it.next()?, it.next()?);
            NET.contains(&fs).then(|| PathBuf::from(mp.replace("\\040", " ").replace("\\011", "\t")))
        })
        .collect()
}

/// Folders on drives whose sub-folders are consoles (≥ 2 of them), depth ≤ 3 below each
/// system root.
fn drive_roots(env: &Env) -> Vec<PathBuf> {
    let net = network_mounts(env);
    let mut found = vec![];
    for sys in &env.system {
        // Each system root has its own budget: a big drive under /mnt (a Windows partition)
        // must not use up the search of /run/media, where USB drives appear.
        let mut probed = 0usize;
        let mut queue: Vec<(PathBuf, u32)> = vec![(sys.clone(), 0)];
        while let Some((dir, depth)) = queue.pop() {
            if probed >= MAX_PROBE_DIRS {
                break;
            }
            probed += 1;
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            let mut subs = vec![];
            let mut consoles = 0;
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if hidden(&n) || !e.file_type().is_ok_and(|t| t.is_dir()) {
                    continue;
                }
                if platforms::by_folder_name(&n).is_some() {
                    consoles += 1;
                } else {
                    subs.push(e.path());
                }
            }
            if consoles >= 2 && depth > 0 {
                found.push(dir);
                continue;
            }
            if depth < 3 {
                subs.sort();
                for s in subs.into_iter().rev() {
                    if !net.iter().any(|m| s.starts_with(m)) {
                        queue.push((s, depth + 1));
                    }
                }
            }
        }
    }
    found
}

/// Folders looked at in the home folder while looking for ROM folders.
const MAX_HOME_DIRS: usize = 3000;

/// Home sub-folders never searched for ROM folders (any case): game stores and Wine prefixes,
/// development trees, media.
const HOME_SKIP: &[&str] = &[
    "node_modules",
    "steamapps",
    "steamlibrary",
    "steam",
    "compatdata",
    "shadercache",
    "drive_c",
    "prefixes",
    "pfx",
    "wine",
    "wineprefixes",
    "bottles",
    "heroic",
    "lutris",
    "faugus",
    "snap",
    "go",
    "android",
    "androidstudioprojects",
    "venv",
    "target",
    "build",
    "dist",
    "__pycache__",
    "site-packages",
    "trash",
    "pictures",
    "music",
    "videos",
    "photos",
    "flatpak",
    "applications",
    "appimages",
    "bios",
    "bios files",
    "saves",
    "screenshots",
];

/// Entries that mark a folder as something else (a code project, a Wine prefix, a Steam
/// library): it is not searched.
const NOT_ROMS_MARKERS: &[&str] = &[
    ".git",
    "Cargo.toml",
    "package.json",
    "CMakeLists.txt",
    "Makefile",
    "meson.build",
    "pyproject.toml",
    "setup.py",
    "system.reg",
    "user.reg",
    "drive_c",
    "steamapps",
    "libraryfolders.vdf",
];

/// Does the console folder hold something that looks like a game of it (a file with one of its
/// extensions, or for PS3 / Wii U a game folder)? Looks at most 2 levels and a few hundred
/// entries.
fn has_rom_like(dir: &Path, p: &Platform) -> bool {
    let mut queue = vec![(dir.to_path_buf(), 0u32)];
    let mut seen = 0usize;
    while let Some((d, depth)) = queue.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            seen += 1;
            if seen > 400 {
                return false;
            }
            let n = e.file_name().to_string_lossy().into_owned();
            if hidden(&n) {
                continue;
            }
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            if is_dir {
                if p.folder_games && depth == 0 {
                    return true;
                }
                if depth < 1 {
                    queue.push((e.path(), depth + 1));
                }
                continue;
            }
            let ext = n.rsplit_once('.').map(|(_, x)| x.to_ascii_lowercase()).unwrap_or_default();
            if !ext.is_empty()
                && !["txt", "bin", "img", "dat", "xml", "json"].contains(&ext.as_str())
                && p.extensions.contains(&ext.as_str())
            {
                return true;
            }
            // .bin dumps count with their cue sheet only ("Game.cue").
            if ext == "cue" && p.extensions.contains(&"cue") {
                return true;
            }
        }
    }
    false
}

/// ROM folders anywhere in the home folder (≤ 3 levels deep): a folder with at least two
/// sub-folders named like consoles ("ps2", "PS2 Games", "Nintendo Switch ROMs"), one of which
/// really holds games. Hidden folders, game stores / Wine prefixes, code projects and media are
/// skipped, `known` roots are not entered, and the search stops after [`MAX_HOME_DIRS`] folders.
pub fn home_roots(home: &Path, known: &[PathBuf]) -> Vec<(PathBuf, String)> {
    let mut found = vec![];
    if !home.is_absolute() {
        return found;
    }
    let mut probed = 0usize;
    let mut queue: Vec<(PathBuf, u32)> = vec![(home.to_path_buf(), 0)];
    while let Some((dir, depth)) = queue.pop() {
        if probed >= MAX_HOME_DIRS {
            log::debug!("emulation: home folder search stopped after {MAX_HOME_DIRS} folders");
            break;
        }
        probed += 1;
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut subs = vec![];
        let mut consoles: Vec<(PathBuf, &'static Platform)> = vec![];
        let mut marked = false;
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            // (The home folder itself may hold a dotfiles `.git` or a Makefile: still searched.)
            if depth > 0 && NOT_ROMS_MARKERS.contains(&n.as_str()) {
                marked = true;
                break;
            }
            if hidden(&n) || !e.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            if let Some(p) = platforms::by_folder_name(&n) {
                consoles.push((e.path(), p));
            } else if !HOME_SKIP.iter().any(|x| x.eq_ignore_ascii_case(&n)) {
                subs.push(e.path());
            }
        }
        if marked {
            continue;
        }
        if depth > 0 && consoles.len() >= 2 && consoles.iter().any(|(d, p)| has_rom_like(d, p)) {
            let shown = match dir.strip_prefix(home) {
                Ok(r) => format!("~/{}", r.display()),
                Err(_) => dir.display().to_string(),
            };
            found.push((dir, format!("Found in {shown}")));
            continue;
        }
        if depth < 3 {
            subs.sort();
            for s in subs.into_iter().rev() {
                if !known.iter().any(|k| s.starts_with(k)) {
                    queue.push((s, depth + 1));
                }
            }
        }
    }
    found
}

/// Does a walk of the root at `outer` (with `layout`) reach the folder `inner` with the same
/// console? A "consoles" root only walks its console-named sub-folders: a folder an emulator
/// lists inside it under another name ("Games/Dolphin") stays a root of its own (the outer walk
/// leaves it to that root).
fn covers(outer: &Path, layout: Layout, inner: &Path) -> bool {
    let Ok(rest) = inner.strip_prefix(outer) else { return false };
    match (layout, rest.components().next()) {
        (_, None) => true,
        (Layout::Platform(_), Some(_)) => true,
        (Layout::Consoles, Some(first)) => {
            let n = first.as_os_str().to_string_lossy();
            !hidden(&n) && platforms::by_folder_name(&n).is_some()
        }
    }
}

fn under_any(p: &Path, list: &[PathBuf]) -> bool {
    list.iter().any(|i| p.starts_with(i))
}

/// Every root to scan: the enabled configured folders, then (when `auto_folders`) the automatic
/// ones that are not inside another root or ignored.
///
/// Roots are compared as real folders ([`Root::real`]): a folder an emulator lists through the
/// Flatpak document portal (`/run/user/1000/doc/…/ps2`) is the drive's `ps2` folder. Overlaps:
/// * a folder found automatically inside another root is dropped — the outer one wins and its
///   folder → console mapping applies (also when the outer one is found later: the inner one
///   is replaced);
/// * a folder the user added inside an automatic one keeps its own settings (console, emulator,
///   exclusions): it is scanned as its own root and the outer walk leaves that sub-folder to it;
/// * the same folder configured twice is scanned once (the first entry).
pub fn collect(ctx: &ScanContext, env: &Env) -> Vec<Root> {
    let cfg: &EmulationConfig = &ctx.emulation;
    let mut roots: Vec<Root> = vec![];
    for (id, f) in cfg.rom_folders.iter().filter(|(_, f)| f.enabled && !f.path.as_os_str().is_empty()) {
        let r = configured(id, f);
        if let Some(o) = roots.iter().find(|o| o.real == r.real) {
            log::info!(
                "emulation: ROM folder {} is the same folder as {}; scanned once",
                r.path.display(),
                o.path.display()
            );
            continue;
        }
        roots.push(r);
    }
    if !cfg.auto_folders {
        return roots;
    }
    let ignored = &cfg.ignored_folders;
    let push_auto = |roots: &mut Vec<Root>, p: PathBuf, layout: Layout, how: String| {
        let real = host_path(&p);
        if under_any(&real, ignored) || under_any(&p, ignored) {
            return;
        }
        // Inside (or at) another root that covers it: nothing to add.
        if roots.iter().any(|r| covers(&r.real, r.layout, &real)) {
            return;
        }
        // Holds automatic roots found earlier that it covers: the outer folder replaces them.
        roots.retain(|r| {
            let inner = r.folder.is_none() && covers(&real, layout, &r.real);
            if inner {
                log::info!("emulation: {} is inside {}; scanned as part of it", r.path.display(), real.display());
            }
            !inner
        });
        roots.push(Root { id: auto_id(&real), path: real.clone(), real, layout, folder: None, explicit: false, how });
    };
    for (p, how) in home_layouts(&ctx.home) {
        if p.is_dir() && console_children(&p) >= 1 {
            push_auto(&mut roots, p, Layout::Consoles, how);
        }
    }
    for p in drive_roots(env) {
        let how = format!("Drive folder {}", p.display());
        push_auto(&mut roots, p, Layout::Consoles, how);
    }
    for (p, how) in home_roots(&ctx.home, &roots.iter().map(|r| r.real.clone()).collect::<Vec<_>>()) {
        push_auto(&mut roots, p, Layout::Consoles, how);
    }
    for (p, plat, how) in emulator_dirs(ctx) {
        if !p.is_dir() {
            continue;
        }
        let layout = match layout_of(&p) {
            Layout::Platform(x) => Layout::Platform(x),
            Layout::Consoles if console_children(&p) >= 1 => Layout::Consoles,
            Layout::Consoles => match platforms::by_id(plat) {
                Some(x) => Layout::Platform(x),
                None => continue,
            },
        };
        // A settings folder that holds other roots would be walked as the wrong console.
        let real = host_path(&p);
        if matches!(layout, Layout::Platform(_)) && roots.iter().any(|r| r.real.starts_with(&real) && r.real != real) {
            continue;
        }
        push_auto(&mut roots, p, layout, how);
    }
    roots
}

// ---------------------------------------------------------------------------------------------
// Remembered roots

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Remembered {
    pub path: PathBuf,
    pub id: String,
    pub auto: bool,
    /// Games found there in the last scan that saw it.
    pub games: usize,
    /// Scans in a row that missed it.
    pub misses: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RootsFile {
    pub version: u32,
    pub roots: Vec<Remembered>,
}

impl RootsFile {
    pub fn load(path: Option<&Path>) -> RootsFile {
        path.and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    pub fn save(&self, path: Option<&Path>) {
        if let Some(p) = path
            && let Ok(json) = serde_json::to_vec_pretty(self)
        {
            super::identify::save_cache(p, &json);
        }
    }
}

/// The plain message for a folder that is not there.
pub fn missing_message(p: &Path) -> String {
    format!("ROM folder {} isn't available — is the drive connected?", p.display())
}

/// A folder that is gone, or an empty folder (an unmounted mount point) that had games.
fn absent(p: &Path, had_games: bool) -> bool {
    match std::fs::read_dir(p) {
        Err(_) => true,
        Ok(mut rd) => had_games && rd.next().is_none(),
    }
}

/// Check the roots against what was remembered: Err(message) when a folder that should be
/// there is not (the remembered list is updated either way: misses counted, stale entries
/// forgotten).
pub fn check(roots: &[Root], mem: &mut RootsFile, cfg: &EmulationConfig, env: &Env) -> Result<(), String> {
    let mut errors: Vec<String> = vec![];
    for r in roots.iter().filter(|r| r.folder.is_some()) {
        let had = mem.roots.iter().any(|m| m.path == r.path && m.games > 0);
        if absent(&r.path, had) {
            errors.push(missing_message(&r.path));
        }
    }
    let found: HashSet<&Path> = roots.iter().map(|r| r.path.as_path()).collect();
    let found_real: Vec<&Path> = roots.iter().map(|r| r.real.as_path()).collect();
    let configured: HashSet<&Path> = cfg.rom_folders.values().filter(|f| f.enabled).map(|f| f.path.as_path()).collect();
    mem.roots.retain_mut(|m| {
        if found.contains(m.path.as_path()) {
            return true;
        }
        if !m.auto {
            // A configured folder: kept while it is configured (missing ones were reported above).
            return configured.contains(m.path.as_path());
        }
        if !cfg.auto_folders
            || under_any(&m.path, &cfg.ignored_folders)
            || under_any(&m.path, &env.removable)
            || m.games == 0
        {
            return false;
        }
        // An emulator's document-portal view of a folder, or a folder now scanned as part of
        // another root (0.7.0 listed both): never "is the drive connected?".
        if crate::emulation::paths::looks_like_portal(&m.path)
            || found_real.iter().any(|f| host_path(&m.path).starts_with(f))
        {
            return false;
        }
        if !absent(&m.path, true) {
            // Still there but no longer a ROM folder (consoles removed): its games are gone.
            return false;
        }
        m.misses += 1;
        if m.misses > MAX_MISSES {
            return false;
        }
        errors.push(missing_message(&m.path));
        true
    });
    match errors.into_iter().next() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// After a successful scan: remember the roots that had games.
pub fn remember(mem: &mut RootsFile, scanned: &[(Root, usize)]) {
    mem.version = 1;
    for (r, games) in scanned {
        mem.roots.retain(|m| m.path != r.path);
        // Folders found automatically are remembered even before they have games (Settings →
        // Emulation lists them); "had games" (for "is the drive connected?") is `games > 0`.
        if *games > 0 || r.folder.is_none() {
            mem.roots.push(Remembered {
                path: r.path.clone(),
                id: r.id.clone(),
                auto: r.folder.is_none(),
                games: *games,
                misses: 0,
            });
        }
    }
    mem.roots.sort_by(|a, b| a.path.cmp(&b.path));
}
