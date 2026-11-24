//! Where each installed emulator (each *instance*: the Flatpak, an AppImage, a portable copy on
//! another drive) really keeps its files: its BIOS folder, Switch keys and firmware, RPCS3's
//! `dev_flash`. Used by the BIOS checks (a BIOS already in the emulator = "Ready"), the copy
//! targets and the firmware-installed checks.
//!
//! The rules, per install:
//! * **Flatpak**: `~/.var/app/<id>/{config,data}`.
//! * **AppImage**: `<AppImage>.home/` is its HOME (`.config`, `.local/share` inside it) and
//!   `<AppImage>.config/` its XDG_CONFIG_HOME (AppImage's own portable mode); else the usual
//!   folders.
//! * **Portable copies**: a marker next to the program (the AppImage file's folder for an
//!   AppImage) switches the emulator to a folder beside it — DuckStation `portable.txt` /
//!   `settings.ini` → that folder; PCSX2 `portable.ini` / `portable.txt` → that folder (or the
//!   sub-folder `portable.txt` names; an AppImage only with `-portable` → `PCSX2/` beside it);
//!   Dolphin `portable.txt` → `User/`; Cemu, Ryujinx, RPCS3 `portable/`; Eden / yuzu
//!   forks, Azahar / Citra, shadPS4 `user/`; melonDS `melonDS.toml`; mGBA `portable.ini`; xemu
//!   `xemu.toml`; RetroArch `retroarch.cfg`.
//! * **Native**: `~/.config` / `~/.local/share` (DuckStation, PCSX2, PPSSPP, Flycast, Dolphin's
//!   settings and RetroArch honour `$XDG_CONFIG_HOME`; Dolphin `~/.dolphin-emu` when it exists).
//!
//! Settings folders (`config_dir`) can differ from these data folders: Dolphin and Flycast keep
//! settings in the config folder and BIOS / data in the data folder.
//!
//! On top of that, folders the emulator's own settings name: DuckStation `[BIOS]
//! SearchDirectory`, PCSX2 `[Folders] Bios`, RetroArch `system_directory`, Eden's
//! `nand_directory`, RPCS3's `vfs.yml` `/dev_flash/` (all in `bios.rs` / here).
//!
//! Also here: the drives (`/mnt/*`, `/media/*`, `/run/media/$USER/*`; tests set their own with
//! [`set_test_drives`]) searched for portable emulator trees and `Emulation/bios` folders.

use super::detect::{Emulator, InstallKind};
use std::path::{Path, PathBuf};

/// How an install's folders were worked out (shown in the report and the BIOS panel).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Where {
    /// `~/.config` / `~/.local/share`.
    Usual,
    /// `~/.var/app/<id>`.
    Flatpak,
    /// `<AppImage>.home` / `<AppImage>.config`.
    AppImageHome,
    /// A portable folder next to the program.
    Portable,
}

impl Where {
    pub fn label(self) -> &'static str {
        match self {
            Where::Usual => "usual folder",
            Where::Flatpak => "Flatpak folder",
            Where::AppImageHome => "the AppImage's own home folder",
            Where::Portable => "portable folder next to the program",
        }
    }
}

/// The folders an install runs with: the HOME and XDG folders it sees and its program's folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Base {
    pub home: PathBuf,
    /// XDG_CONFIG_HOME as the emulator sees it.
    pub config: PathBuf,
    /// XDG_DATA_HOME as the emulator sees it.
    pub data: PathBuf,
    /// Where the program file is (an AppImage's own folder); None for a Flatpak.
    pub exe_dir: Option<PathBuf>,
    pub how: Where,
}

/// `<path>.home` / `<path>.config` (AppImage portable folders).
fn beside(path: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{suffix}", path.display()))
}

/// The HOME / XDG folders of an install.
pub fn base(emu: &Emulator, home: &Path) -> Base {
    match &emu.kind {
        InstallKind::Flatpak { app_id } => {
            let app = home.join(".var/app").join(app_id);
            Base {
                home: home.to_path_buf(),
                config: app.join("config"),
                data: app.join("data"),
                exe_dir: None,
                how: Where::Flatpak,
            }
        }
        InstallKind::AppImage { path } | InstallKind::Binary { path } => {
            let exe_dir = path.parent().map(Path::to_path_buf);
            let side_home = beside(path, ".home");
            let side_cfg = beside(path, ".config");
            let (h, mut how) =
                if side_home.is_dir() { (side_home, Where::AppImageHome) } else { (home.to_path_buf(), Where::Usual) };
            let config = if side_cfg.is_dir() {
                how = Where::AppImageHome;
                side_cfg
            } else {
                h.join(".config")
            };
            let data = h.join(".local/share");
            Base { home: h, config, data, exe_dir, how }
        }
    }
}

/// An emulator's own user folder, and how it was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserDir {
    pub path: PathBuf,
    pub how: Where,
}

/// A portable folder next to the program: `marker` (a file or folder) beside it → `dir`
/// (relative to the program's folder; "" = the program's folder itself).
fn portable(b: &Base, markers: &[&str], dir: &str) -> Option<UserDir> {
    let exe = b.exe_dir.as_ref()?;
    markers
        .iter()
        .any(|m| exe.join(m).exists())
        .then(|| UserDir { path: if dir.is_empty() { exe.clone() } else { exe.join(dir) }, how: Where::Portable })
}

/// The yuzu-family folder name of a definition ("eden" → "eden").
fn yuzu_name(def: &str) -> Option<&'static str> {
    Some(match def {
        "eden" => "eden",
        "citron" => "citron",
        "sudachi" => "sudachi",
        "yuzu" => "yuzu",
        "suyu" => "suyu",
        _ => return None,
    })
}

/// The folder an emulator keeps its own files in (BIOS, keys, NAND, settings), for this install.
/// None for emulators Lodestar doesn't know the folders of.
pub fn user_dir(emu: &Emulator, home: &Path) -> Option<UserDir> {
    let b = base(emu, home);
    let at = |p: PathBuf| Some(UserDir { path: p, how: b.how });
    match emu.def.as_str() {
        "duckstation" => portable(&b, &["portable.txt", "settings.ini"], "").or_else(|| {
            if b.how == Where::Flatpak || beside_dir(emu, ".config") {
                return at(b.config.join("duckstation"));
            }
            // Native: $XDG_CONFIG_HOME/duckstation whenever it is set (DuckStation doesn't check
            // that the folder exists), else ~/.local/share/duckstation (see `xdg_folder`).
            at(xdg_folder(&b, home, "duckstation", b.data.join("duckstation")))
        }),
        "pcsx2" => pcsx2_root(emu, &b, home),
        "dolphin" | "primehack" => portable(&b, &["portable.txt"], "User").or_else(|| match b.how {
            Where::Flatpak => at(b.data.join("dolphin-emu")),
            _ if b.home.join(".dolphin-emu").is_dir() => at(b.home.join(".dolphin-emu")),
            _ => at(b.data.join("dolphin-emu")),
        }),
        // PPSSPP's "memstick next to the program" is Windows-only: on Linux always the config folder.
        "ppsspp" => at(xdg_folder(&b, home, "ppsspp", b.config.join("ppsspp"))),
        "cemu" => portable(&b, &["portable"], "portable").or_else(|| at(b.data.join("Cemu"))),
        "ryujinx" => portable(&b, &["portable"], "portable").or_else(|| at(b.config.join("Ryujinx"))),
        "rpcs3" => portable(&b, &["portable"], "portable").or_else(|| at(b.config.join("rpcs3"))),
        d if yuzu_name(d).is_some() => portable(&b, &["user"], "user").or_else(|| at(b.data.join(yuzu_name(d)?))),
        "azahar" => portable(&b, &["user"], "user").or_else(|| at(b.data.join("azahar-emu"))),
        "citra" => portable(&b, &["user"], "user").or_else(|| at(b.data.join("citra-emu"))),
        "lime3ds" => portable(&b, &["user"], "user").or_else(|| at(b.data.join("lime3ds-emu"))),
        "shadps4" => portable(&b, &["user"], "user").or_else(|| at(b.data.join("shadPS4"))),
        "melonds" => portable(&b, &["melonDS.toml", "melonDS.ini"], "").or_else(|| at(b.config.join("melonDS"))),
        "mgba" => portable(&b, &["portable.ini"], "").or_else(|| at(b.config.join("mgba"))),
        "xemu" => portable(&b, &["xemu.toml"], "").or_else(|| at(b.data.join("xemu/xemu"))),
        "flycast" => at(b.data.join("flycast")),
        "mednafen" => at(b.home.join(".mednafen")),
        "vita3k" => portable(&b, &["portable"], "portable").or_else(|| at(b.data.join("Vita3K/Vita3K"))),
        "retroarch" => {
            let cfg = super::detect::retroarch_cfg(&emu.kind, home);
            let how = if b.exe_dir.as_ref().is_some_and(|d| cfg.starts_with(d)) && b.how == Where::Usual {
                Where::Portable
            } else {
                b.how
            };
            Some(UserDir { path: cfg.parent().unwrap_or(home).to_path_buf(), how })
        }
        _ => None,
    }
}

/// Does `<AppImage><suffix>` exist (an AppImage portable folder)?
fn beside_dir(emu: &Emulator, suffix: &str) -> bool {
    matches!(&emu.kind, InstallKind::AppImage { path } if beside(path, suffix).is_dir())
}

#[cfg(test)]
thread_local! {
    static TEST_XDG_CONFIG: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Tests: the `$XDG_CONFIG_HOME` this thread's lookups see (default: none, never the real one).
#[cfg(test)]
pub fn set_test_xdg_config(v: Option<PathBuf>) {
    TEST_XDG_CONFIG.with(|d| *d.borrow_mut() = v);
}

/// `$XDG_CONFIG_HOME` when it is set, absolute and inside `home` (a test home never sees the
/// real one).
pub(super) fn xdg_config_home(home: &Path) -> Option<PathBuf> {
    #[cfg(test)]
    let v = TEST_XDG_CONFIG.with(|d| d.borrow().clone());
    #[cfg(not(test))]
    let v = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    v.filter(|p| p.is_absolute() && p.starts_with(home))
}

/// The user's `$XDG_CONFIG_HOME` for a native install (Flatpaks and AppImages with their own
/// folders have theirs in `Base`).
fn native_xdg_config(b: &Base, home: &Path) -> Option<PathBuf> {
    (b.how == Where::Usual).then(|| xdg_config_home(home)).flatten()
}

/// `$XDG_CONFIG_HOME/<sub>` for a native install when that variable is set, else `default`. One
/// exception: when that folder doesn't exist but `default` does, the emulator evidently runs
/// without the variable (it is set only where Lodestar runs, e.g. a container's shell), so the
/// folder it has been using wins.
fn xdg_folder(b: &Base, home: &Path, sub: &str, default: PathBuf) -> PathBuf {
    match native_xdg_config(b, home) {
        Some(x) if x.join(sub).is_dir() || !default.is_dir() => x.join(sub),
        _ => default,
    }
}

/// PCSX2's data folder: portable when `portable.ini` / `portable.txt` is next to the program
/// (then `<program folder>/<what portable.txt says>`) or it runs with `-portable`; an AppImage
/// only with `-portable` (its program folder is the mount point), then `<AppImage folder>/PCSX2`;
/// else `$XDG_CONFIG_HOME/PCSX2` (`~/.config/PCSX2`).
fn pcsx2_root(emu: &Emulator, b: &Base, home: &Path) -> Option<UserDir> {
    let flag = emu.args.split_whitespace().any(|a| a == "-portable" || a == "--portable");
    if let (Some(exe), InstallKind::AppImage { .. }) = (&b.exe_dir, &emu.kind) {
        if flag {
            return Some(UserDir { path: exe.join("PCSX2"), how: Where::Portable });
        }
    } else if let Some(exe) = &b.exe_dir
        && (flag || exe.join("portable.ini").exists() || exe.join("portable.txt").exists())
    {
        let sub = std::fs::read_to_string(exe.join("portable.txt")).unwrap_or_default();
        let sub = sub.trim();
        let path = if sub.is_empty() { exe.clone() } else { exe.join(sub) };
        return Some(UserDir { path, how: Where::Portable });
    }
    Some(UserDir { path: xdg_folder(b, home, "PCSX2", b.config.join("PCSX2")), how: b.how })
}

/// The folder an emulator keeps its **settings** in, for the emulators whose RetroAchievements
/// sign-in Lodestar reads (`achievements::ra::emulators`): DuckStation `settings.ini`, PCSX2
/// `inis/`, PPSSPP `PSP/SYSTEM/`, Dolphin's config folder (separate from its data folder unless
/// it keeps everything in one user folder), Flycast `config/flycast` (its data folder holds the
/// BIOS), RetroArch the folder of its `retroarch.cfg`.
pub fn config_dir(emu: &Emulator, home: &Path) -> Option<UserDir> {
    let b = base(emu, home);
    let at = |p: PathBuf| Some(UserDir { path: p, how: b.how });
    match emu.def.as_str() {
        "duckstation" | "retroarch" => user_dir(emu, home),
        "pcsx2" => user_dir(emu, home).map(|u| UserDir { path: u.path.join("inis"), ..u }),
        "ppsspp" => user_dir(emu, home).map(|u| UserDir { path: u.path.join("PSP/SYSTEM"), ..u }),
        "dolphin" | "primehack" => portable(&b, &["portable.txt"], "User/Config").or_else(|| match b.how {
            Where::Flatpak => at(b.config.join("dolphin-emu")),
            _ if b.home.join(".dolphin-emu").is_dir() => at(b.home.join(".dolphin-emu/Config")),
            _ => at(xdg_folder(&b, home, "dolphin-emu", b.config.join("dolphin-emu"))),
        }),
        "flycast" => at(xdg_folder(&b, home, "flycast", b.config.join("flycast"))),
        _ => None,
    }
}

/// A yuzu-family emulator's NAND folder: `nand_directory` in its `qt-config.ini` when the user
/// moved it, else `<user>/nand`.
pub fn yuzu_nand(emu: &Emulator, home: &Path) -> Option<PathBuf> {
    let name = yuzu_name(&emu.def)?;
    let u = user_dir(emu, home)?;
    let ini = if u.how == Where::Portable {
        u.path.join("config/qt-config.ini")
    } else {
        base(emu, home).config.join(name).join("qt-config.ini")
    };
    if let Ok(text) = std::fs::read_to_string(&ini) {
        let mut in_sec = false;
        let (mut default, mut value) = (true, None::<String>);
        for l in text.lines().map(str::trim) {
            if l.starts_with('[') {
                in_sec = l.eq_ignore_ascii_case("[Data%20Storage]") || l.eq_ignore_ascii_case("[Data Storage]");
            } else if in_sec && let Some((k, v)) = l.split_once('=') {
                match k.trim() {
                    "nand_directory\\default" => default = v.trim() == "true",
                    "nand_directory" => value = Some(v.trim().trim_matches('"').to_string()).filter(|v| !v.is_empty()),
                    _ => {}
                }
            }
        }
        if !default && let Some(v) = value {
            let p = super::detect::expand_home(&v, home);
            if p.is_absolute() {
                return Some(p);
            }
        }
    }
    Some(u.path.join("nand"))
}

/// RPCS3's `dev_flash` for this install: `/dev_flash/` in its `vfs.yml` (`$(EmulatorDir)` =
/// its folder, or the `$(EmulatorDir)` the file names) when set, else `<folder>/dev_flash`.
pub fn rpcs3_dev_flash(emu: &Emulator, home: &Path) -> Option<PathBuf> {
    let root = user_dir(emu, home)?.path;
    for vfs in [root.join("vfs.yml"), root.join("config/vfs.yml")] {
        let Ok(text) = std::fs::read_to_string(&vfs) else { continue };
        let value = |key: &str| -> Option<String> {
            text.lines().find_map(|l| {
                let (k, v) = l.split_once(": ").or_else(|| l.trim_end().strip_suffix(':').map(|k| (k, "")))?;
                (k.trim().trim_matches(|c| c == '"' || c == '\'') == key)
                    .then(|| v.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
            })
        };
        let emu_dir =
            value("$(EmulatorDir)").filter(|v| !v.is_empty()).unwrap_or_else(|| format!("{}/", root.display()));
        if let Some(v) = value("/dev_flash/").filter(|v| !v.is_empty()) {
            let v = v.replace("$(EmulatorDir)", &emu_dir);
            let p = super::detect::expand_home(&v, home);
            if p.is_absolute() {
                return Some(PathBuf::from(p.to_string_lossy().trim_end_matches('/').to_string()));
            }
        }
    }
    Some(root.join("dev_flash"))
}

/// BIOS / firmware files an emulator's own settings point at (melonDS `BIOS9Path` …, xemu
/// `bootrom_path` …): they count as being in that emulator.
pub fn config_named_files(emu: &Emulator, home: &Path) -> Vec<PathBuf> {
    let Some(u) = user_dir(emu, home) else { return vec![] };
    let (files, keys): (Vec<PathBuf>, &[&str]) = match emu.def.as_str() {
        "melonds" => (
            vec![u.path.join("melonDS.toml"), u.path.join("melonDS.ini")],
            &["BIOS9Path", "BIOS7Path", "FirmwarePath", "DSiBIOS9Path", "DSiBIOS7Path", "DSiFirmwarePath"],
        ),
        "xemu" => (vec![u.path.join("xemu.toml")], &["bootrom_path", "flashrom_path", "eeprom_path"]),
        "mgba" => (vec![u.path.join("config.ini")], &["bios", "gba.bios", "gb.bios", "gbc.bios", "sgb.bios"]),
        _ => return vec![],
    };
    let mut out = vec![];
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        for l in text.lines() {
            let Some((k, v)) = l.split_once('=') else { continue };
            if !keys.contains(&k.trim()) {
                continue;
            }
            let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
            if v.is_empty() {
                continue;
            }
            let p = super::detect::expand_home(v, home);
            let p = if p.is_absolute() { p } else { u.path.join(p) };
            if p.is_file() && !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Drives
// ---------------------------------------------------------------------------------------------

#[cfg(test)]
thread_local! {
    static TEST_DRIVES: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(vec![]) };
}

/// Tests: the drive folders this thread's BIOS search looks at (default: none, never the real
/// /mnt).
#[cfg(test)]
pub fn set_test_drives(v: Vec<PathBuf>) {
    TEST_DRIVES.with(|d| *d.borrow_mut() = v);
}

/// Where drives are mounted: `/mnt`, `/media`, `/run/media/$USER`.
pub fn drive_bases() -> Vec<PathBuf> {
    #[cfg(test)]
    {
        TEST_DRIVES.with(|d| d.borrow().clone())
    }
    #[cfg(not(test))]
    {
        // Test hook (headless checks): other folders as the mount folders (`:`-separated).
        if let Some(v) = std::env::var_os("LODESTAR_DRIVES") {
            return std::env::split_paths(&v).filter(|p| p.is_absolute()).collect();
        }
        let mut v = vec![PathBuf::from("/mnt"), PathBuf::from("/media")];
        if let Ok(u) = std::env::var("USER")
            && !u.is_empty()
        {
            v.push(PathBuf::from("/run/media").join(u));
        }
        v
    }
}

/// Network mounts (`/proc/self/mounts`): never walked.
pub fn network_mounts() -> Vec<PathBuf> {
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
    if cfg!(test) {
        return vec![];
    }
    let Ok(text) = std::fs::read_to_string("/proc/self/mounts") else { return vec![] };
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let (_, mp, fs) = (it.next()?, it.next()?, it.next()?);
            NET.contains(&fs).then(|| PathBuf::from(mp.replace("\\040", " ").replace("\\011", "\t")))
        })
        .collect()
}

/// Folders on a drive never looked into (system and program folders, game stores).
const DRIVE_SKIP: &[&str] = &[
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "$recycle.bin",
    "system volume information",
    "lost+found",
    "proc",
    "sys",
    "dev",
    "usr",
    "boot",
    "etc",
    "var",
    "node_modules",
    "steamapps",
    "compatdata",
    "shadercache",
    "drive_c",
    "pfx",
    "__macosx",
    "target",
    "snap",
    "appdata",
    "$windows.~bt",
    "recovery",
];

/// Folders whose name matches `want` (lower-case name → true), breadth first below each of
/// `bases`, at most `levels` down and `budget` folders per base (hidden, system and network
/// folders skipped). A match isn't looked into further.
pub fn find_named(bases: &[PathBuf], levels: usize, budget: usize, want: &dyn Fn(&str) -> bool) -> Vec<PathBuf> {
    let net = network_mounts();
    let mut out = vec![];
    for b in bases {
        let mut left = budget;
        let mut queue: std::collections::VecDeque<(PathBuf, usize)> = [(b.clone(), 0)].into();
        while let Some((dir, depth)) = queue.pop_front() {
            if left == 0 {
                break;
            }
            left -= 1;
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            let mut subs: Vec<(String, PathBuf, bool)> = vec![];
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let lname = name.to_lowercase();
                if name.starts_with('.') || DRIVE_SKIP.contains(&lname.as_str()) {
                    continue;
                }
                let Ok(ft) = e.file_type() else { continue };
                let p = e.path();
                if !(ft.is_dir() || (ft.is_symlink() && p.is_dir())) {
                    continue;
                }
                subs.push((lname, p, ft.is_symlink()));
            }
            subs.sort();
            for (lname, p, link) in subs {
                if want(&lname) {
                    if !out.contains(&p) {
                        out.push(p);
                    }
                } else if depth + 1 < levels && !link && !net.iter().any(|m| p.starts_with(m)) {
                    queue.push_back((p, depth + 1));
                }
            }
        }
    }
    out
}

/// [`find_named`] on the drives: each drive below the mount folders (`/mnt/FastSSD`,
/// `/run/media/me/USB`) is searched on its own `budget` (one big drive never uses up the
/// others'), `levels` counted from the mount folder. Network mounts are left out.
pub fn find_on_drives(mounts: &[PathBuf], levels: usize, budget: usize, want: &dyn Fn(&str) -> bool) -> Vec<PathBuf> {
    let net = network_mounts();
    let mut out = vec![];
    for m in mounts {
        let Ok(rd) = std::fs::read_dir(m) else { continue };
        let mut drives: Vec<(String, PathBuf)> = rd
            .flatten()
            .map(|e| (e.file_name().to_string_lossy().to_lowercase(), e.path()))
            .filter(|(n, p)| !n.starts_with('.') && !DRIVE_SKIP.contains(&n.as_str()) && p.is_dir())
            .filter(|(_, p)| !net.iter().any(|n| p.starts_with(n)))
            .collect();
        drives.sort();
        for (name, d) in drives {
            if want(&name) {
                out.push(d);
            } else if levels > 1 {
                for p in find_named(std::slice::from_ref(&d), levels - 1, budget, want) {
                    if !out.contains(&p) {
                        out.push(p);
                    }
                }
            }
        }
    }
    out
}

fn is_emulators_name(n: &str) -> bool {
    n == "emulators"
}

/// Folders named "Emulators" on the drives (5 levels down from the mount folders).
pub fn drive_emulator_trees(drives: &[PathBuf]) -> Vec<PathBuf> {
    find_on_drives(drives, 5, 800, &is_emulators_name)
}

/// Folders named "Emulators" in the home folder (3 levels down: `~/Emulation/Emulators`).
pub fn home_emulator_trees(home: &Path) -> Vec<PathBuf> {
    find_named(&[home.to_path_buf()], 3, 1500, &is_emulators_name)
}

#[cfg(test)]
mod tests;
