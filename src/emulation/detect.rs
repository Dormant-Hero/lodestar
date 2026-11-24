//! Finding installed emulators: Flatpaks, AppImages, programs in PATH, Steam's RetroArch and its
//! cores, plus the ones the user added in Settings → Emulation.
//!
//! Where it looks is a [`ProbeRoots`] (the real machine's folders, or a fixture home in tests).
//! The automatic part is cached per process; the cache is checked against the modification times
//! of the folders it looked in (cheap `stat`s), and [`invalidate`] (called by
//! `registry::refresh`) throws it away.
//!
//! Instance ids are stable: the definition id for the first install found (Flatpak, then a
//! program in PATH, then an AppImage / unpacked program, then Steam), `<def>-flatpak`,
//! `<def>-program`, `<def>-appimage`, `<def>-steam` for further installs of the same emulator.
//! The same emulator installed twice (a Flatpak and an AppImage) is listed twice; which one a
//! console uses is the user's pick, else the first found (Flatpak > program > AppImage > Steam,
//! see `registry::choose`).
//!
//! AppImages / programs are found in the usual app folders, in folders named like "AppImages" /
//! "Applications" in the home folder and Documents (3 levels down, plus their sub-folders), and
//! through the emulators' own menu entries (`.desktop` files whose `Exec` is an AppImage or
//! program of a known emulator — an AppImage that registered itself can be anywhere). An
//! AppImage without the executable bit is listed as `blocked` ("not allowed to run yet"): it is
//! never chosen automatically, and Settings → Emulation offers "Allow it to run" (`chmod u+x`,
//! only when the user clicks).
//!
//! Portable emulator collections are found too: folders named "Emulators" in the home folder
//! and the user's emulator folders (Settings → Emulation → Emulator folders), each with one
//! sub-folder per emulator (`/mnt/X/Gaming/Emulation/Emulators/Duckstation/Duckstation`); there a
//! program is also recognised by its plain name or as an AppImage by its content. Folders named
//! "Emulators" on the drives aren't searched by themselves (they held old copies and backups
//! nobody meant to use): [`suggested_dirs`] offers them, and [`dirs_to_migrate`] lists the ones
//! a config from before already used. Where each install keeps its files (portable folder,
//! AppImage home, Flatpak) is `roots`; "· portable" in `how` says so.

use super::defs::{self, EmulatorDef};
use crate::config::{EmulationConfig, EmulatorEntry};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

mod flatpak;
pub use flatpak::*;

/// How an emulator is installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallKind {
    Flatpak { app_id: String },
    AppImage { path: PathBuf },
    Binary { path: PathBuf },
}

/// One usable emulator (an installed copy of a definition, or one the user added).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Emulator {
    /// Instance id (config key; stable across runs).
    pub id: String,
    /// Built-in definition id ("" = fully custom).
    pub def: String,
    pub name: String,
    pub kind: InstallKind,
    /// How it was found, for the UI: "Flatpak", "AppImage in ~/Applications", "Added by you" …
    pub how: String,
    /// Added (or changed) by the user.
    pub user: bool,
    /// RetroArch: the cores found.
    pub cores: Vec<PathBuf>,
    /// The user's arguments ("" = the profile's).
    pub args: String,
    /// Found automatically (also when the user changed it: an override of a found one).
    pub auto: bool,
    /// An AppImage / program that isn't allowed to run yet (no executable bit): listed, never
    /// chosen automatically.
    pub blocked: bool,
}

impl Emulator {
    /// The built-in definition it follows.
    pub fn definition(&self) -> Option<&'static EmulatorDef> {
        defs::by_id(&self.def)
    }

    /// The Flatpak app id, when it is one.
    pub fn flatpak_id(&self) -> Option<&str> {
        match &self.kind {
            InstallKind::Flatpak { app_id } => Some(app_id),
            _ => None,
        }
    }

    /// The program file (AppImage / binary).
    pub fn program_path(&self) -> Option<&Path> {
        match &self.kind {
            InstallKind::AppImage { path } | InstallKind::Binary { path } => Some(path),
            InstallKind::Flatpak { .. } => None,
        }
    }

    /// What kind of install it is, for the UI: "Flatpak", "AppImage", "installed program",
    /// "Steam", or "portable" (an AppImage / program that keeps its files in its own folder) —
    /// also for one the user added (that it was added by hand is said where it was found).
    pub fn kind_label(&self) -> &'static str {
        let portable = self.how.contains("· portable");
        match &self.kind {
            InstallKind::Flatpak { .. } => "Flatpak",
            InstallKind::AppImage { .. } | InstallKind::Binary { .. } if portable => "portable",
            InstallKind::AppImage { .. } => "AppImage",
            InstallKind::Binary { .. } if self.how.starts_with("Steam") => "Steam",
            InstallKind::Binary { .. } => "installed program",
        }
    }

    /// "DuckStation — AppImage".
    pub fn label(&self) -> String {
        format!("{} — {}", self.name, self.kind_label())
    }

    /// What tells copies of one emulator apart: "DuckStation — Duckstation.backup" (its program's
    /// folder), "DuckStation — DuckStation.AppImage" (the file, when the folder is just its
    /// name), else [`Emulator::label`].
    pub fn copy_label(&self) -> String {
        let Some(path) = self.program_path() else { return self.label() };
        let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let dir = path.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let plain = |s: &str| s.is_empty() || s.eq_ignore_ascii_case(&self.name);
        match () {
            _ if !plain(&dir) => format!("{} — {dir}", self.name),
            _ if !plain(&file) => format!("{} — {file}", self.name),
            _ => self.label(),
        }
    }

    /// A copy that looks like one kept aside: "old", "backup", "test", "copy" … in its file or
    /// folder name (`RPCS3_old/`, `Duckstation.backup/`). Never chosen automatically over one
    /// that doesn't.
    pub fn looks_old(&self) -> bool {
        let Some(p) = self.program_path() else { return false };
        let file = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let dir = p.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        name_says_kept_aside(&file) || name_says_kept_aside(&dir)
    }

    /// For a sentence: "DuckStation (AppImage)".
    pub fn label_in_text(&self) -> String {
        format!("{} ({})", self.name, self.kind_label())
    }

    /// Its name where emulators are listed together: with its kind ("DuckStation — Flatpak")
    /// only when another of `all` has the same name, else just "DuckStation".
    /// Two of the same kind too (copies of one emulator): its folder ("DuckStation —
    /// Duckstation.backup", [`Emulator::copy_label`]).
    pub fn name_among<'a>(&self, all: impl IntoIterator<Item = &'a Emulator>) -> String {
        let same: Vec<&Emulator> =
            all.into_iter().filter(|o| o.id != self.id && o.name.eq_ignore_ascii_case(&self.name)).collect();
        if same.is_empty() {
            self.name.clone()
        } else if same.iter().any(|o| o.kind_label() == self.kind_label()) {
            self.copy_label()
        } else {
            self.label()
        }
    }
}

/// How an emulator starts games of some platforms (built-in or the user's).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub platforms: Vec<String>,
    pub extensions: Vec<String>,
    pub args: String,
    pub core: Option<String>,
    pub folder_launch: Vec<String>,
    pub custom: bool,
    /// The user's profile runs its own program (a fork, a test build) instead of the
    /// emulator's; may start with `{EmulatorDir}` or `~/`.
    pub program: Option<PathBuf>,
    /// Working folder (`{EmulatorDir}`, `{InstallDir}`, `{RomDir}`); "" = the program's folder
    /// for a profile's own program, else none.
    pub cwd: String,
    /// How Play follows its games (Default = the launch's usual way).
    pub tracking: crate::model::TrackingMode,
}

impl Profile {
    pub fn supports(&self, platform: &str) -> bool {
        self.platforms.iter().any(|p| p == platform)
    }

    /// Does it open files with this extension?
    pub fn accepts_ext(&self, ext: &str) -> bool {
        self.extensions.iter().any(|x| x.eq_ignore_ascii_case(ext))
    }
}

/// Where detection looks. [`ProbeRoots::system`] is the real machine; tests build one around a
/// fixture home with [`ProbeRoots::under`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeRoots {
    pub home: PathBuf,
    /// System Flatpak installation (`/var/lib/flatpak`).
    pub system_flatpak: PathBuf,
    /// User Flatpak installation (`~/.local/share/flatpak`).
    pub user_flatpak: PathBuf,
    /// `$PATH`.
    pub path: Option<OsString>,
    /// Folders searched for AppImages / unpacked emulators.
    pub app_dirs: Vec<PathBuf>,
    /// System RetroArch core folders (`/usr/lib/libretro` …).
    pub core_dirs: Vec<PathBuf>,
    /// Menu entry folders (`~/.local/share/applications`, `/usr/share/applications` …): an
    /// emulator's `.desktop` file names its AppImage / program.
    pub desktop_dirs: Vec<PathBuf>,
    /// Where drives are mounted (`/mnt`, `/media`, `/run/media/$USER`): folders named
    /// "Emulators" there are suggested ([`suggested_dirs`]), not searched by themselves.
    pub drives: Vec<PathBuf>,
    /// Emulator folders searched like the home folder's "Emulators" folders (with their
    /// sub-folders): the user's list ([`searched_roots`] adds it from the config).
    pub emulator_dirs: Vec<PathBuf>,
}

impl ProbeRoots {
    /// The real machine.
    pub fn system(home: &Path) -> ProbeRoots {
        ProbeRoots {
            home: home.to_path_buf(),
            // Test hook: headless checks run in a dev container that sees the host's Flatpaks
            // under /run/host (LODESTAR_FLATPAK_ROOT=/run/host/var/lib/flatpak).
            system_flatpak: std::env::var_os("LODESTAR_FLATPAK_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/var/lib/flatpak")),
            user_flatpak: home.join(".local/share/flatpak"),
            path: std::env::var_os("PATH"),
            app_dirs: app_dirs(home, Path::new("/opt")),
            core_dirs: [
                "/usr/lib/libretro",
                "/usr/lib64/libretro",
                "/usr/lib/x86_64-linux-gnu/libretro",
                "/usr/local/lib/libretro",
                "/run/current-system/sw/lib/retroarch/cores",
            ]
            .iter()
            .map(PathBuf::from)
            .chain([home.join(".nix-profile/lib/retroarch/cores")])
            .collect(),
            desktop_dirs: {
                let mut v = vec![home.join(".local/share/applications")];
                let xdg = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
                for d in xdg.split(':').filter(|d| !d.is_empty()) {
                    let p = Path::new(d).join("applications");
                    if !v.contains(&p) {
                        v.push(p);
                    }
                }
                v
            },
            drives: super::roots::drive_bases(),
            emulator_dirs: vec![],
        }
    }

    /// Everything under `root` (tests): `root/var/lib/flatpak`, `root/usr/bin` as PATH,
    /// `root/opt`, `root/usr/lib/libretro`; the home is `home`.
    #[cfg(test)]
    pub fn under(root: &Path, home: &Path) -> ProbeRoots {
        ProbeRoots {
            home: home.to_path_buf(),
            system_flatpak: root.join("var/lib/flatpak"),
            user_flatpak: home.join(".local/share/flatpak"),
            path: Some(std::env::join_paths([root.join("usr/bin"), home.join(".local/bin")]).unwrap_or_default()),
            app_dirs: app_dirs(home, &root.join("opt")),
            core_dirs: vec![root.join("usr/lib/libretro")],
            desktop_dirs: vec![home.join(".local/share/applications"), root.join("usr/share/applications")],
            drives: vec![root.join("mnt")],
            emulator_dirs: vec![],
        }
    }
}

/// AppImage / unpacked-program folders (ES-DE's set + EmuDeck's, which `settings.sh` may move).
fn app_dirs(home: &Path, opt: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> =
        ["Applications", "AppImages", ".local/bin", "bin", "Downloads", "Emulation/tools", ".local/share/applications"]
            .iter()
            .map(|d| home.join(d))
            .collect();
    v.push(opt.to_path_buf());
    // EmuDeck: `emusFolder=` / `toolsPath=` in ~/.config/EmuDeck/settings.sh.
    if let Ok(text) = std::fs::read_to_string(home.join(".config/EmuDeck/settings.sh")) {
        for line in text.lines() {
            let line = line.trim().trim_start_matches("export ").trim();
            for key in ["emusFolder=", "toolsPath=", "appFolder="] {
                if let Some(v0) = line.strip_prefix(key) {
                    let raw = v0.trim().trim_matches('"').trim_matches('\'');
                    let p = expand_home(raw, home);
                    if p.is_absolute() && !v.contains(&p) {
                        v.push(p);
                    }
                }
            }
        }
    }
    v
}

fn hidden_name(p: &Path) -> bool {
    p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

/// Folders named like "AppImages" / "Applications" (any case) in the home folder and Documents,
/// a few levels down (~/Documents/programs/appimages), and their sub-folders — looked for at
/// each detection (new folders count).
fn named_app_dirs(roots: &ProbeRoots) -> Vec<PathBuf> {
    let home = &roots.home;
    let mut v: Vec<PathBuf> = vec![];
    for base in [home.to_path_buf(), home.join("Documents")] {
        let mut found = vec![];
        let mut budget = 2000;
        app_named(&base, 3, &mut budget, &mut found);
        for d in found {
            let subs: Vec<PathBuf> = list_dir(&d).into_iter().filter(|p| p.is_dir() && !hidden_name(p)).collect();
            for x in std::iter::once(d).chain(subs) {
                if !v.contains(&x) && !roots.app_dirs.contains(&x) {
                    v.push(x);
                }
            }
        }
    }
    v
}

/// Portable emulator collections: the emulator folders (`roots.emulator_dirs`) and every folder
/// named "Emulators" in the home folder, each with its sub-folders (one per emulator:
/// `/mnt/X/Gaming/Emulation/Emulators/Duckstation/`). Programs there are matched by their
/// plain names too ("Duckstation", an AppImage without the .AppImage ending).
fn tree_app_dirs(roots: &ProbeRoots) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = vec![];
    for d in roots.emulator_dirs.iter().cloned().chain(super::roots::home_emulator_trees(&roots.home)) {
        for x in with_subs(&d) {
            if !v.contains(&x) && !roots.app_dirs.contains(&x) {
                v.push(x);
            }
        }
    }
    v
}

/// A folder and its sub-folders (one level, hidden ones left out).
fn with_subs(d: &Path) -> Vec<PathBuf> {
    let subs = list_dir(d).into_iter().filter(|p| p.is_dir() && !hidden_name(p));
    std::iter::once(d.to_path_buf()).chain(subs).collect()
}

/// The probe roots with the user's emulator folders and, while a config from before hasn't had
/// its one-time check yet (`emulator_dirs_checked`), every folder named "Emulators" on the drives
/// as before — so nothing an older version found goes missing in the meantime.
pub fn searched_roots(roots: &ProbeRoots, cfg: &EmulationConfig) -> ProbeRoots {
    let mut r = roots.clone();
    let legacy = if cfg.emulator_dirs_checked { vec![] } else { super::roots::drive_emulator_trees(&roots.drives) };
    for d in cfg.emulator_dirs.iter().cloned().chain(legacy) {
        if !d.as_os_str().is_empty() && !r.emulator_dirs.contains(&d) {
            r.emulator_dirs.push(d);
        }
    }
    r
}

/// Does this folder (or one of its sub-folders) hold a program of a known emulator? Each folder
/// is read once (drives can be slow).
fn holds_emulators(dir: &Path) -> bool {
    let files: Vec<PathBuf> = with_subs(dir).iter().flat_map(|d| list_dir(d)).collect();
    files.iter().any(|f| defs::all().iter().any(|d| tree_file(d, f).is_some()))
}

/// Is `d` one of `list`, or inside one of them?
fn listed(list: &[PathBuf], d: &Path) -> bool {
    list.iter().any(|x| d.starts_with(x))
}

/// Folders named "Emulators" on the drives that hold emulators and that the user hasn't added
/// or dismissed: Settings → Emulation offers "Add this folder". Nothing while emulators aren't
/// found automatically, and before the one-time check (they're all searched then anyway).
pub fn suggested_dirs(cfg: &EmulationConfig, roots: &ProbeRoots) -> Vec<PathBuf> {
    if !cfg.auto_emulators || !cfg.emulator_dirs_checked {
        return vec![];
    }
    super::roots::drive_emulator_trees(&roots.drives)
        .into_iter()
        .filter(|t| !listed(&cfg.emulator_dirs, t) && !cfg.dismissed_emulator_dirs.contains(t))
        .filter(|t| holds_emulators(t))
        .collect()
}

/// The one-time check for configs from before: the folders named "Emulators" on the drives that
/// emulators came from (found there automatically, or added by hand from there). They become
/// emulator folders, so nothing the older version found disappears.
pub fn dirs_to_migrate(cfg: &EmulationConfig, roots: &ProbeRoots) -> Vec<PathBuf> {
    super::roots::drive_emulator_trees(&roots.drives)
        .into_iter()
        .filter(|t| !listed(&cfg.emulator_dirs, t))
        .filter(|t| {
            let by_hand = cfg.emulators.values().any(|e| e.path.is_absolute() && e.path.starts_with(t));
            by_hand || (cfg.auto_emulators && holds_emulators(t))
        })
        .collect()
}

/// An AppImage by its content (ELF with the `AI\x02` mark), whatever its name.
fn is_appimage_file(p: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 11];
    std::fs::File::open(p).and_then(|mut f| f.read_exact(&mut head)).is_ok()
        && head[..4] == *b"\x7fELF"
        && head[8..11] == *b"AI\x02"
}

/// Folders named "appimages" / "applications" / "appimage" (any case) up to `levels` below
/// `base` (hidden and program folders skipped; at most `budget` folders looked at).
fn app_named(base: &Path, levels: usize, budget: &mut usize, out: &mut Vec<PathBuf>) {
    const SKIP: &[&str] = &["node_modules", "target", "steamapps", "compatdata", "drive_c", "pfx", "snap"];
    for p in list_dir(base) {
        if *budget == 0 {
            return;
        }
        let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        if name.starts_with('.') || SKIP.contains(&name.as_str()) || !p.is_dir() || p.is_symlink() {
            continue;
        }
        *budget -= 1;
        if matches!(name.as_str(), "appimages" | "appimage" | "applications") {
            out.push(p);
        } else if levels > 1 {
            app_named(&p, levels - 1, budget, out);
        }
    }
}

/// `~/x`, `$HOME/x`, `${HOME}/x` → `<home>/x`.
pub(crate) fn expand_home(s: &str, home: &Path) -> PathBuf {
    for pre in ["~/", "$HOME/", "${HOME}/"] {
        if let Some(rest) = s.strip_prefix(pre) {
            return home.join(rest);
        }
    }
    if s == "~" || s == "$HOME" {
        return home.to_path_buf();
    }
    PathBuf::from(s)
}

// ---------------------------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------------------------

/// Lower-case glob match (`*` any run, `?` one character).
pub(crate) fn glob_match(pat: &str, name: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut ni, mut star, mut mark) = (0, 0, None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Can a file start? Some(true): executable; Some(false): an ELF / AppImage without the
/// executable bit (a fresh download: "not allowed to run yet"); None: not a program.
fn runnable_state(p: &Path) -> Option<bool> {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    let m = std::fs::metadata(p).ok()?;
    if !m.is_file() {
        return None;
    }
    if m.permissions().mode() & 0o111 != 0 {
        return Some(true);
    }
    let mut magic = [0u8; 4];
    (std::fs::File::open(p).and_then(|mut f| f.read_exact(&mut magic)).is_ok() && magic == *b"\x7fELF").then_some(false)
}

fn runnable(p: &Path) -> bool {
    runnable_state(p).is_some()
}

/// "Allow it to run": add the owner's executable bit (`chmod u+x`). Only on the user's click.
pub fn allow_to_run(p: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perm = std::fs::metadata(p)?.permissions();
    perm.set_mode(perm.mode() | 0o100);
    std::fs::set_permissions(p, perm)
}

fn tilde(p: &Path, home: &Path) -> String {
    match p.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

fn list_dir(d: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(d) else { return vec![] };
    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    v.sort();
    v
}

/// One AppImage / program found for a definition.
#[derive(Clone, Debug)]
struct AppFile {
    path: PathBuf,
    appimage: bool,
    how: String,
    /// No executable bit yet.
    blocked: bool,
}

/// Does a file name match one of a definition's AppImage patterns (single-file patterns) or
/// its program names?
fn matches_def_file(def: &EmulatorDef, name: &str) -> bool {
    def.appimage.iter().any(|pat| !pat.contains('/') && glob_match(pat, name))
        || def.binaries.iter().any(|b| b.eq_ignore_ascii_case(name))
}

/// Programs named by emulators' menu entries: (definition id, program path, entry file). An
/// AppImage that registered itself (`org.duckstation.DuckStation.desktop` with
/// `Exec="/…/DuckStation-x64.AppImage" %f`) is found wherever it is.
fn desktop_programs(roots: &ProbeRoots) -> Vec<(&'static str, PathBuf, PathBuf)> {
    let mut out: Vec<(&'static str, PathBuf, PathBuf)> = vec![];
    for dir in &roots.desktop_dirs {
        for f in list_dir(dir) {
            let fname = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let Some(stem) = fname.strip_suffix(".desktop") else { continue };
            // Cheap first look: only entries that mention an emulator.
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            let Some(entry) = crate::scan::desktop::DesktopEntry::parse(&text) else { continue };
            let Some(exec) = entry.get("Exec") else { continue };
            let argv = crate::scan::desktop::parse_exec(exec);
            let mut it = argv.iter();
            let mut prog = it.next();
            if prog.is_some_and(|p| p == "env" || p.ends_with("/env")) {
                prog = it.find(|a| !a.contains('='));
            }
            let Some(prog) = prog.map(PathBuf::from) else { continue };
            if !prog.is_absolute() || prog.file_name().is_some_and(|n| n == "flatpak") {
                continue; // PATH programs and Flatpaks are found their own way
            }
            let pname = prog.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let def = defs::all()
                .iter()
                .find(|d| matches_def_file(d, &pname) || d.flatpak.iter().any(|id| id.eq_ignore_ascii_case(stem)));
            if let Some(d) = def
                && runnable(&prog)
                && !out.iter().any(|(_, p, _)| *p == prog)
            {
                out.push((d.id, prog, f.clone()));
            }
        }
    }
    out
}

/// Programs of a definition in portable collections (`trees`: folders and their sub-folders):
/// by its plain name, or an AppImage named after it.
fn tree_files(def: &EmulatorDef, trees: &[PathBuf], home: &Path) -> Vec<AppFile> {
    let mut out = vec![];
    for dir in trees {
        for e in list_dir(dir) {
            if let Some(ok) = tree_file(def, &e) {
                let what = if ok.1 { "AppImage" } else { "Program" };
                out.push(AppFile {
                    path: e.clone(),
                    appimage: ok.1,
                    how: format!("{what} in {}", tilde(dir, home)),
                    blocked: !ok.0,
                });
            }
        }
    }
    out
}

/// Is `e` (in a portable collection) a program of `def`? Some((allowed to run, is an AppImage)).
fn tree_file(def: &EmulatorDef, e: &Path) -> Option<(bool, bool)> {
    let name = e.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let lname = name.to_lowercase();
    let by_pattern = def.appimage.iter().any(|pat| !pat.contains('/') && glob_match(pat, &name));
    let by_name = def.binaries.iter().any(|b| b.eq_ignore_ascii_case(&name)) || lname == def.id;
    let ai_named = (lname.starts_with(def.id) || def.binaries.iter().any(|b| lname.starts_with(&b.to_lowercase())))
        && !lname.ends_with(".so")
        && !lname.contains(".so.");
    if !(by_pattern || by_name || ai_named) {
        return None;
    }
    let ok = runnable_state(e)?;
    let is_ai = lname.ends_with(".appimage") || is_appimage_file(e);
    if !(by_pattern || by_name) && !is_ai {
        return None;
    }
    Some((ok, is_ai))
}

/// AppImages / unpacked programs of a definition in the app folders and its menu entries.
fn find_app_files(
    def: &EmulatorDef,
    roots: &ProbeRoots,
    named: &[PathBuf],
    trees: &[PathBuf],
    from_menu: &[(&'static str, PathBuf, PathBuf)],
) -> Vec<AppFile> {
    let mut out = tree_files(def, trees, &roots.home);
    for dir in roots.app_dirs.iter().chain(named).filter(|d| !trees.contains(d)) {
        let entries = list_dir(dir);
        if entries.is_empty() {
            continue;
        }
        for pat in def.appimage {
            match pat.split_once('/') {
                None => {
                    for e in &entries {
                        let name = e.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        if glob_match(pat, &name)
                            && let Some(ok) = runnable_state(e)
                        {
                            let is_ai = name.to_lowercase().ends_with(".appimage");
                            let what = if is_ai { "AppImage" } else { "Program" };
                            out.push(AppFile {
                                path: e.clone(),
                                appimage: is_ai,
                                how: format!("{what} in {}", tilde(dir, &roots.home)),
                                blocked: !ok,
                            });
                        }
                    }
                }
                Some((dpat, fpat)) => {
                    for e in entries.iter().filter(|e| e.is_dir()) {
                        let dname = e.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        if !glob_match(dpat, &dname) {
                            continue;
                        }
                        for f in list_dir(e) {
                            let fname = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            if glob_match(fpat, &fname)
                                && let Some(ok) = runnable_state(&f)
                            {
                                out.push(AppFile {
                                    path: f.clone(),
                                    appimage: false,
                                    how: format!("Program in {}", tilde(e, &roots.home)),
                                    blocked: !ok,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    // Menu entries first: "(from its menu entry)" tells where it was found.
    let mut menu = vec![];
    for (_, p, _) in from_menu.iter().filter(|(id, _, _)| *id == def.id) {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let is_ai = name.to_lowercase().ends_with(".appimage");
        let what = if is_ai { "AppImage" } else { "Program" };
        let dir = p.parent().unwrap_or(Path::new("/"));
        menu.push(AppFile {
            path: p.clone(),
            appimage: is_ai,
            how: format!("{what} in {} (from its menu entry)", tilde(dir, &roots.home)),
            blocked: runnable_state(p) == Some(false),
        });
    }
    menu.append(&mut out);
    let mut out = menu;
    let mut seen = std::collections::HashSet::new();
    out.retain(|f| seen.insert(f.path.canonicalize().unwrap_or_else(|_| f.path.clone())));
    for f in out.iter_mut().filter(|f| f.blocked) {
        f.how.push_str(" — not allowed to run yet");
    }
    out
}

/// `key = "value"` from a RetroArch config.
fn ra_cfg_value(cfg: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(cfg).ok()?;
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').to_string()).filter(|v| !v.is_empty() && v != "default")
    })
}

/// RetroArch's config file for an install: the Flatpak's; for an AppImage its `.home` /
/// `.config` folder or a `retroarch.cfg` next to it (portable); for a program a `retroarch.cfg`
/// next to it (Steam's RetroArch, portable copies); else RetroArch's own search:
/// `$XDG_CONFIG_HOME/retroarch/retroarch.cfg` (`~/.config/…`), then `~/.retroarch.cfg`.
pub fn retroarch_cfg(kind: &InstallKind, home: &Path) -> PathBuf {
    match kind {
        InstallKind::Flatpak { app_id } => home.join(".var/app").join(app_id).join("config/retroarch/retroarch.cfg"),
        InstallKind::AppImage { path } | InstallKind::Binary { path } => {
            let mut cands: Vec<PathBuf> = vec![];
            if matches!(kind, InstallKind::AppImage { .. }) {
                cands.push(PathBuf::from(format!("{}.home", path.display())).join(".config/retroarch/retroarch.cfg"));
                cands.push(PathBuf::from(format!("{}.config", path.display())).join("retroarch/retroarch.cfg"));
            }
            if let Some(d) = path.parent()
                && !matches!(d.to_str(), Some("/usr/bin" | "/bin" | "/usr/local/bin" | "/usr/games"))
            {
                cands.push(d.join("retroarch.cfg"));
            }
            let usual = super::roots::xdg_config_home(home)
                .unwrap_or_else(|| home.join(".config"))
                .join("retroarch/retroarch.cfg");
            cands.push(usual.clone());
            // RetroArch run without that $XDG_CONFIG_HOME (set only where Lodestar runs).
            cands.push(home.join(".config/retroarch/retroarch.cfg"));
            cands.push(home.join(".retroarch.cfg"));
            cands.into_iter().find(|c| c.is_file()).unwrap_or(usual)
        }
    }
}

/// A RetroArch config path value: `~` is home, `:` is the config folder.
pub(crate) fn ra_path(v: &str, cfg: &Path, home: &Path) -> PathBuf {
    if let Some(rest) = v.strip_prefix(":/") {
        return cfg.parent().unwrap_or(Path::new("/")).join(rest);
    }
    expand_home(v, home)
}

/// RetroArch's cores for one install (`*_libretro.so`, first folder wins per name).
fn retroarch_cores(kind: &InstallKind, roots: &ProbeRoots) -> Vec<PathBuf> {
    let home = &roots.home;
    let cfg = retroarch_cfg(kind, home);
    let mut dirs: Vec<PathBuf> = vec![];
    if let Some(v) = ra_cfg_value(&cfg, "libretro_directory") {
        dirs.push(ra_path(&v, &cfg, home));
    }
    match kind {
        InstallKind::Flatpak { app_id } => {
            // The sandbox can't see /usr/lib/libretro: only its own cores.
            dirs.push(home.join(".var/app").join(app_id).join("config/retroarch/cores"));
        }
        InstallKind::AppImage { path } => {
            dirs.push(PathBuf::from(format!("{}.home", path.display())).join(".config/retroarch/cores"));
            dirs.push(home.join(".config/retroarch/cores"));
        }
        InstallKind::Binary { path } if path.to_string_lossy().contains("steamapps/common/RetroArch") => {
            // Steam: its DLC cores in <install>/cores only.
            dirs.push(path.parent().unwrap_or(Path::new("/")).join("cores"));
        }
        InstallKind::Binary { path } => {
            if let Some(d) = path.parent() {
                dirs.push(d.join("cores"));
            }
            dirs.push(home.join(".config/retroarch/cores"));
            dirs.push(home.join("snap/retroarch/current/.config/retroarch/cores"));
            dirs.extend(roots.core_dirs.iter().cloned());
        }
    }
    let mut out: Vec<PathBuf> = vec![];
    let mut names = std::collections::HashSet::new();
    for d in dirs {
        for f in list_dir(&d) {
            let Some(name) = f.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
            if name.ends_with("_libretro.so") && f.is_file() && names.insert(name) {
                out.push(f);
            }
        }
    }
    out
}

/// Steam's RetroArch (app 1118310) in every Steam library.
fn steam_retroarch(ctx: &crate::scan::ScanContext) -> Vec<PathBuf> {
    let mut out = vec![];
    for root in crate::scan::steam::find_roots(ctx) {
        for lib in crate::scan::steam::library_folders(&root.path) {
            let bin = lib.join("steamapps/common/RetroArch/retroarch");
            if runnable(&bin) && !out.contains(&bin) {
                out.push(bin);
            }
        }
    }
    out
}

/// One found install before ids are given out.
struct Found {
    kind: InstallKind,
    how: String,
    suffix: &'static str,
    blocked: bool,
}

/// The automatic part: every installed copy of every built-in definition.
pub fn detect_auto(ctx: &crate::scan::ScanContext, roots: &ProbeRoots) -> Vec<Emulator> {
    let mut out: Vec<Emulator> = vec![];
    let from_menu = desktop_programs(roots);
    let named = named_app_dirs(roots);
    let trees = tree_app_dirs(roots);
    for def in defs::all() {
        let mut found: Vec<Found> = vec![];
        for app in def.flatpak {
            if let Some(inst) = flatpak_installation(roots, app) {
                remember_access(&roots.home, app, flatpak_filesystems(roots, app));
                let how = if inst == "user" { "Flatpak (installed for you only)" } else { "Flatpak" };
                found.push(Found {
                    kind: InstallKind::Flatpak { app_id: app.to_string() },
                    how: how.into(),
                    suffix: "flatpak",
                    blocked: false,
                });
                break; // the current id wins over legacy ones
            }
        }
        let app_files = find_app_files(def, roots, &named, &trees, &from_menu);
        let canon: Vec<PathBuf> =
            app_files.iter().map(|f| f.path.canonicalize().unwrap_or_else(|_| f.path.clone())).collect();
        if let Some(p) = def
            .binaries
            .iter()
            .find_map(|b| crate::launch::find_in_path_with(b, roots.path.as_deref()))
            .filter(|p| !canon.contains(&p.canonicalize().unwrap_or_else(|_| p.clone())))
        {
            let how = format!("Program in PATH ({})", tilde(&p, &roots.home));
            found.push(Found { kind: InstallKind::Binary { path: p }, how, suffix: "program", blocked: false });
        }
        for f in app_files {
            let (kind, suffix) = if f.appimage {
                (InstallKind::AppImage { path: f.path }, "appimage")
            } else {
                (InstallKind::Binary { path: f.path }, "program")
            };
            found.push(Found { kind, how: f.how, suffix, blocked: f.blocked });
        }
        if def.id == "retroarch" {
            for p in steam_retroarch(ctx) {
                found.push(Found {
                    kind: InstallKind::Binary { path: p },
                    how: "Steam".into(),
                    suffix: "steam",
                    blocked: false,
                });
            }
        }
        for f in found {
            let mut id = if out.iter().any(|e| e.def == def.id) {
                format!("{}-{}", def.id, f.suffix)
            } else {
                def.id.to_string()
            };
            let base = id.clone();
            let mut n = 2;
            while out.iter().any(|e| e.id == id) {
                id = format!("{base}-{n}");
                n += 1;
            }
            let cores = if def.id == "retroarch" { retroarch_cores(&f.kind, roots) } else { vec![] };
            let mut how = f.how;
            let tag = where_tag(def.id, &f.kind, &roots.home);
            let at = how.find(" — not allowed").unwrap_or(how.len());
            how.insert_str(at, tag);
            out.push(Emulator {
                id,
                def: def.id.to_string(),
                name: def.name.to_string(),
                kind: f.kind,
                how,
                user: false,
                cores,
                args: String::new(),
                auto: true,
                blocked: f.blocked,
            });
        }
    }
    out
}

/// " · portable" / " · own home folder" when an install keeps its files next to it (`roots`).
fn where_tag(def: &str, kind: &InstallKind, home: &Path) -> &'static str {
    let probe = Emulator {
        id: String::new(),
        def: def.to_string(),
        name: String::new(),
        kind: kind.clone(),
        how: String::new(),
        user: false,
        cores: vec![],
        args: String::new(),
        auto: true,
        blocked: false,
    };
    match super::roots::user_dir(&probe, home).map(|u| u.how) {
        Some(super::roots::Where::Portable) => " · portable",
        Some(super::roots::Where::AppImageHome) => " · own home folder",
        _ => "",
    }
}

/// Modification times of everything detection looked at (a changed one = detect again).
fn signature(roots: &ProbeRoots) -> Vec<Option<SystemTime>> {
    let mut dirs: Vec<PathBuf> = vec![roots.system_flatpak.join("app"), roots.user_flatpak.join("app")];
    dirs.extend(roots.app_dirs.iter().cloned());
    if let Some(p) = &roots.path {
        dirs.extend(std::env::split_paths(p));
    }
    dirs.extend(roots.core_dirs.iter().cloned());
    dirs.extend(roots.desktop_dirs.iter().cloned());
    dirs.extend(named_app_dirs(roots));
    dirs.extend(tree_app_dirs(roots));
    dirs.push(roots.home.join(".config/retroarch/cores"));
    dirs.push(roots.home.join(".var/app/org.libretro.RetroArch/config/retroarch/cores"));
    dirs.push(roots.home.join(".local/share/Steam/steamapps/common"));
    dirs.iter().map(|d| std::fs::metadata(d).and_then(|m| m.modified()).ok()).collect()
}

type AutoCache = HashMap<ProbeRoots, (Vec<Option<SystemTime>>, Vec<Emulator>)>;
static AUTO: Mutex<Option<AutoCache>> = Mutex::new(None);

impl std::hash::Hash for ProbeRoots {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.home.hash(h);
        self.system_flatpak.hash(h);
    }
}

/// Forget the cached detection (the next `detect` looks again).
pub fn invalidate() {
    *AUTO.lock() = None;
}

/// `detect_auto`, cached while nothing it looked at changed.
fn detect_auto_cached(ctx: &crate::scan::ScanContext, roots: &ProbeRoots) -> Vec<Emulator> {
    let sig = signature(roots);
    if let Some((s, v)) = AUTO.lock().as_ref().and_then(|m| m.get(roots))
        && *s == sig
    {
        return v.clone();
    }
    let v = detect_auto(ctx, roots);
    AUTO.lock().get_or_insert_with(HashMap::new).insert(roots.clone(), (sig, v.clone()));
    v
}

/// Emulators found automatically (when `ctx.emulation.auto_emulators`) plus the user's, without
/// the hidden ones. May touch the filesystem: call off the UI thread.
pub fn detect(ctx: &crate::scan::ScanContext) -> Vec<Emulator> {
    detect_with(ctx, &ProbeRoots::system(&ctx.home))
}

/// [`detect`] looking in `roots` (plus the emulator folders of the config, see
/// [`searched_roots`]).
pub fn detect_with(ctx: &crate::scan::ScanContext, roots: &ProbeRoots) -> Vec<Emulator> {
    let cfg = &ctx.emulation;
    let roots = &searched_roots(roots, cfg);
    let mut list = if cfg.auto_emulators { detect_auto_cached(ctx, roots) } else { vec![] };
    list.retain(|e| !cfg.hidden_emulators.contains(&e.id));
    merge_user(&mut list, cfg, roots);
    list
}

fn entry_kind(e: &EmulatorEntry) -> Option<InstallKind> {
    let k = e.kind.trim().to_ascii_lowercase();
    if k == "flatpak" || (k.is_empty() && !e.app_id.trim().is_empty()) {
        return (!e.app_id.trim().is_empty()).then(|| InstallKind::Flatpak { app_id: e.app_id.trim().to_string() });
    }
    if e.path.as_os_str().is_empty() {
        return None;
    }
    let is_ai =
        k == "appimage" || (k.is_empty() && e.path.extension().is_some_and(|x| x.eq_ignore_ascii_case("appimage")));
    Some(if is_ai {
        InstallKind::AppImage { path: e.path.clone() }
    } else {
        InstallKind::Binary { path: e.path.clone() }
    })
}

/// The same install (the same program file, or the same Flatpak)?
fn same_install(a: &InstallKind, b: &InstallKind) -> bool {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    match (a, b) {
        (InstallKind::Flatpak { app_id: x }, InstallKind::Flatpak { app_id: y }) => x == y,
        (InstallKind::Flatpak { .. }, _) | (_, InstallKind::Flatpak { .. }) => false,
        _ => match (a, b) {
            (
                InstallKind::AppImage { path: x } | InstallKind::Binary { path: x },
                InstallKind::AppImage { path: y } | InstallKind::Binary { path: y },
            ) => x == y || canon(x) == canon(y),
            _ => false,
        },
    }
}

/// The built-in emulator a program is (its Flatpak id, its AppImage / program name, an
/// unpacked program's folder): the longest matching name wins ("dosbox-staging" over "dosbox").
pub fn identify(kind: &InstallKind) -> Option<&'static EmulatorDef> {
    let path = match kind {
        InstallKind::Flatpak { app_id } => {
            return defs::all().iter().find(|d| d.flatpak.iter().any(|f| f.eq_ignore_ascii_case(app_id)));
        }
        InstallKind::AppImage { path } | InstallKind::Binary { path } => path,
    };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let lname = name.to_lowercase();
    let parent =
        path.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut best: Option<(usize, &'static EmulatorDef)> = None;
    for d in defs::all() {
        let mut score = 0;
        if matches_def_file(d, &name) {
            score = d.id.len().max(1) + 100;
        }
        for pat in d.appimage {
            if let Some((dp, fp)) = pat.split_once('/')
                && glob_match(dp, &parent)
                && glob_match(fp, &name)
            {
                score = score.max(d.id.len() + 100);
            }
        }
        // "DuckStation.AppImage", "rpcs3-v0.0.43", "Duckstation": the name starts with its own.
        let keys = std::iter::once(d.id.to_string()).chain(d.binaries.iter().map(|b| b.to_lowercase()));
        for k in keys {
            let rest = lname.strip_prefix(&k);
            if rest.is_some_and(|r| r.is_empty() || r.starts_with(['.', '-', '_', ' '])) {
                score = score.max(k.len());
            }
        }
        if score > 0 && best.is_none_or(|(b, _)| score > b) {
            best = Some((score, d));
        }
    }
    best.map(|(_, d)| d)
}

/// A name for a program nothing else names: "pcsx2-v2.2.0-linux-appimage-x64-Qt" → "pcsx2".
pub fn name_from_program(kind: &InstallKind) -> String {
    match kind {
        InstallKind::Flatpak { app_id } => app_id.rsplit('.').next().unwrap_or(app_id).to_string(),
        InstallKind::AppImage { path } | InstallKind::Binary { path } => {
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            stem.split(['-', '_', '.', ' ']).next().filter(|s| !s.is_empty()).unwrap_or(&stem).to_string()
        }
    }
}

/// An emulator the user added: the name they gave it. Older versions saved the name of the
/// built-in emulator it was based on ("Eden" for a Ryujinx found with Locate… on the Switch row):
/// such a name is replaced by the program's own (Ryujinx) when the program clearly is another
/// emulator. An empty name: the program's, else the built-in one's.
pub fn user_name(entry_name: &str, follows: &str, kind: &InstallKind, key: &str) -> String {
    let typed = entry_name.trim();
    let based = defs::by_id(follows);
    let program = identify(kind);
    if typed.is_empty() {
        return program.or(based).map(|d| d.name.to_string()).unwrap_or_else(|| key.to_string());
    }
    if let (Some(b), Some(p)) = (based, program)
        && typed.eq_ignore_ascii_case(b.name)
        && p.id != b.id
    {
        return p.name.to_string();
    }
    typed.to_string()
}

/// What an emulator the user added is based on. Older versions based a program found with
/// Locate… on a console's suggested emulator and named it after that one ("Eden" for a Ryujinx):
/// when the saved name is still that emulator's and the program clearly is another one that runs
/// the same consoles, it starts like that other one (Ryujinx's arguments, not Eden's).
pub fn user_def<'a>(entry_name: &str, follows: &'a str, kind: &InstallKind) -> &'a str {
    let (Some(b), Some(p)) = (defs::by_id(follows), identify(kind)) else { return follows };
    let same_consoles =
        b.profiles.iter().flat_map(|x| x.platforms).all(|pf| p.profiles.iter().any(|y| y.platforms.contains(pf)));
    if p.id != b.id && entry_name.trim().eq_ignore_ascii_case(b.name) && same_consoles {
        return p.id;
    }
    follows
}

/// Apply `config.emulation.emulators`: same key as a found one = change it, else add it (and a
/// found one with the very same program goes: it is the user's now).
fn merge_user(list: &mut Vec<Emulator>, cfg: &EmulationConfig, roots: &ProbeRoots) {
    for (key, entry) in &cfg.emulators {
        if !list.iter().any(|e| e.id == *key)
            && let Some(k) = entry_kind(entry)
        {
            list.retain(|e| !(e.auto && !e.user && same_install(&e.kind, &k)));
        }
    }
    for (key, entry) in &cfg.emulators {
        let follows = defs::by_id(entry.follows.trim()).map(|d| d.id);
        let kind = entry_kind(entry);
        if let Some(e) = list.iter_mut().find(|e| e.id == *key) {
            if let Some(k) = kind {
                e.kind = k;
            }
            // What it was saved as based on (before `user_def` may correct it).
            let based = follows.map_or_else(|| e.def.clone(), str::to_string);
            e.def = user_def(&entry.name, &based, &e.kind).to_string();
            if !entry.name.trim().is_empty() {
                e.name = user_name(&entry.name, &based, &e.kind, key);
            }
            e.args = entry.args.clone();
            e.user = true;
            e.how = "Changed by you".into();
            e.blocked = e.program_path().is_some_and(|p| runnable_state(p) == Some(false));
            if let InstallKind::Flatpak { app_id } = &e.kind {
                remember_access(&roots.home, app_id, flatpak_filesystems(roots, app_id));
            }
            if e.def == "retroarch" {
                e.cores = retroarch_cores(&e.kind, roots);
            }
            continue;
        }
        let Some(kind) = kind else {
            log::warn!("emulation: emulator {key} has no program or Flatpak set; skipped");
            continue;
        };
        let based = follows.unwrap_or("");
        let def = user_def(&entry.name, based, &kind);
        // Named from what it was saved as based on: "Eden" saved for a Ryujinx program is Ryujinx.
        let name = user_name(&entry.name, based, &kind, key);
        let cores = if def == "retroarch" { retroarch_cores(&kind, roots) } else { vec![] };
        let blocked = match &kind {
            InstallKind::Flatpak { app_id } => {
                remember_access(&roots.home, app_id, flatpak_filesystems(roots, app_id));
                false
            }
            InstallKind::AppImage { path } | InstallKind::Binary { path } => runnable_state(path) == Some(false),
        };
        let mut how = format!("Added by you{}", where_tag(def, &kind, &roots.home));
        if blocked {
            how.push_str(" — not allowed to run yet");
        }
        list.push(Emulator {
            id: key.clone(),
            def: def.to_string(),
            name,
            kind,
            how,
            user: true,
            cores,
            args: entry.args.clone(),
            auto: false,
            blocked,
        });
    }
}

/// The full path of a RetroArch core among `cores` ("mgba", "mgba_libretro", "mgba_libretro.so").
pub fn find_core<'a>(cores: &'a [PathBuf], name: &str) -> Option<&'a PathBuf> {
    let stem = name.trim().trim_end_matches(".so").trim_end_matches(".dll");
    let stem = stem.rsplit('/').next().unwrap_or(stem);
    let stem = stem.strip_suffix("_libretro").unwrap_or(stem);
    cores.iter().find(|c| {
        c.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix("_libretro.so")).is_some_and(|n| n == stem)
    })
}

/// An emulator's usable profiles: built-in ones (only those that can work, e.g. RetroArch
/// profiles whose core is present) plus the user's.
pub fn profiles(e: &Emulator, cfg: &EmulationConfig) -> Vec<Profile> {
    let mut out: Vec<Profile> = vec![];
    if let Some(def) = e.definition() {
        for p in def.profiles {
            let core = match p.core {
                Some(c) => match find_core(&e.cores, c) {
                    Some(path) => Some(path.to_string_lossy().into_owned()),
                    None => continue, // core not installed
                },
                None => None,
            };
            out.push(Profile {
                id: p.id.to_string(),
                name: p.name.to_string(),
                platforms: p.platforms.iter().map(|s| s.to_string()).collect(),
                extensions: p.extensions.iter().map(|s| s.to_string()).collect(),
                args: p.args.to_string(),
                core,
                folder_launch: p.folder_launch.iter().map(|s| s.to_string()).collect(),
                custom: false,
                ..Default::default()
            });
        }
    }
    for (key, pe) in &cfg.profiles {
        let (emu, pid) = match key.split_once('/') {
            Some((a, b)) => (a, b),
            None => (pe.emulator.as_str(), key.as_str()),
        };
        if emu != e.id && pe.emulator != e.id {
            continue;
        }
        let pid = pid.trim();
        if pid.is_empty() {
            continue;
        }
        let core = (!pe.core.trim().is_empty()).then(|| {
            find_core(&e.cores, &pe.core)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| pe.core.trim().to_string())
        });
        let lower = |v: &[String]| -> Vec<String> {
            v.iter().map(|s| s.trim().trim_start_matches('.').to_ascii_lowercase()).filter(|s| !s.is_empty()).collect()
        };
        if let Some(p) = out.iter_mut().find(|p| p.id == pid) {
            if !pe.name.trim().is_empty() {
                p.name = pe.name.trim().to_string();
            }
            if !pe.platforms.is_empty() {
                p.platforms = pe.platforms.clone();
            }
            if !pe.extensions.is_empty() {
                p.extensions = lower(&pe.extensions);
            }
            if !pe.args.trim().is_empty() {
                p.args = pe.args.clone();
            }
            if core.is_some() {
                p.core = core;
            }
            p.program = own_program(&pe.program);
            p.cwd = pe.cwd.trim().to_string();
            p.tracking = pe.tracking;
            p.custom = true;
        } else {
            out.push(Profile {
                id: pid.to_string(),
                name: if pe.name.trim().is_empty() { pid.to_string() } else { pe.name.trim().to_string() },
                platforms: pe.platforms.clone(),
                extensions: lower(&pe.extensions),
                args: if pe.args.trim().is_empty() { "{rom}".into() } else { pe.args.clone() },
                core,
                folder_launch: vec![],
                custom: true,
                program: own_program(&pe.program),
                cwd: pe.cwd.trim().to_string(),
                tracking: pe.tracking,
            });
        }
    }
    out
}

/// A profile's own program (None = the emulator's).
fn own_program(p: &Path) -> Option<PathBuf> {
    let s = p.to_string_lossy();
    (!s.trim().is_empty()).then(|| PathBuf::from(s.trim()))
}

/// Does a file or folder name call itself a copy kept aside? Whole words only ("RPCS3_old",
/// "Duckstation.backup", "rpcs3-test", "Copy of PCSX2"): a substring test took "rpcs3-latest"
/// (la-test), "PPSSPPGold" (G-old) and "preview" builds for old copies, so the current build lost
/// the automatic choice. Words are split as [`name_words`] does.
pub(crate) fn name_says_kept_aside(name: &str) -> bool {
    const WORDS: &[&str] = &["old", "older", "backup", "bak", "bkp", "test", "copy", "prev", "previous"];
    name_words(name).iter().any(|w| WORDS.contains(&w.as_str()))
}

/// Does a folder name say its contents were put aside ("Backups - Modding - Resources",
/// "unused BIOS", "PCSX2_old", "Archive")? Whole words, any case (see [`name_words`]). The BIOS
/// search never looks into such folders by itself (only when the user chose one).
pub(crate) fn name_says_set_aside(name: &str) -> bool {
    const WORDS: &[&str] = &[
        "old",
        "older",
        "backup",
        "backups",
        "bak",
        "bkp",
        "unused",
        "archive",
        "archived",
        "archives",
        "prev",
        "previous",
        "obsolete",
        "deprecated",
    ];
    name_words(name).iter().any(|w| WORDS.contains(&w.as_str()))
}

/// The lower-case words of a name: split at anything not a letter or digit, between letters and
/// digits, and at case changes ("RPCS3Old" → rpcs, 3, old; "PPSSPPGold" → ppsspp, gold).
pub(crate) fn name_words(name: &str) -> Vec<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut words: Vec<String> = vec![];
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if let Some(&prev) = i.checked_sub(1).and_then(|j| chars.get(j)) {
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            let boundary = (prev.is_alphabetic() != c.is_alphabetic() && prev.is_alphanumeric())
                || (prev.is_lowercase() && c.is_uppercase())
                || (prev.is_uppercase() && c.is_uppercase() && next_lower);
            if boundary && !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
        }
        cur.extend(c.to_lowercase());
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

#[cfg(test)]
pub(crate) mod tests;
