//! "Scan for games": the background folder walk and the "Detect installed" scan (Windows
//! programs in Wine prefixes, `.desktop` games, Flatpak games). Runs on its own thread; results
//! go to the UI as `ImportMsg`s. Read-only: never writes anything, never starts a program.

use super::detect::*;
use super::{ImportCandidate, ProgramKind, ProtonVersion};
use crate::model::*;
use crate::scan::ScanContext;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub enum ImportMsg {
    Progress(usize),
    Found(Vec<ImportCandidate>),
    Error(String),
    Done,
}

/// Where results go and whether to stop.
pub struct Sink<'a> {
    pub cancel: &'a AtomicBool,
    pub emit: &'a mut dyn FnMut(ImportMsg),
    pub files: usize,
}

impl Sink<'_> {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    fn tick(&mut self) {
        self.files += 1;
        if self.files.is_multiple_of(250) {
            (self.emit)(ImportMsg::Progress(self.files));
        }
    }
    fn found(&mut self, mut v: Vec<ImportCandidate>) {
        if !v.is_empty() {
            v.sort_by_key(|c| c.name.to_lowercase());
            (self.emit)(ImportMsg::Found(v));
        }
    }
}

pub const MAX_DEPTH: usize = 6;
pub const MAX_FILES: usize = 20_000;

/// Anything Lodestar should not offer: installers, uninstallers, redistributables, crash
/// reporters, updaters, launcher helpers and runtimes. `stem` is the file name without extension.
pub fn is_excluded_program(stem: &str) -> bool {
    let n = norm_name(stem);
    const PARTS: &[&str] = &[
        "unins",
        "uninst",
        "setup",
        "install",
        "redist",
        "dxsetup",
        "dxwebsetup",
        "directx",
        "dotnetfx",
        "ndp4",
        "netfx",
        "crashhandler",
        "crashreport",
        "crashpad",
        "crashsender",
        "crashdump",
        "bugreport",
        "bugsplat",
        "errorreport",
        "reporter",
        "updater",
        "autoupdate",
        "update",
        "patcher",
        "cefsubprocess",
        "cefprocess",
        "webhelper",
        "helper",
        "easyanticheat",
        "eacsetup",
        "battleye",
        "beservice",
        "protectedgame",
        "dxdiag",
        "vkcube",
        "vulkaninfo",
        "notification",
        "elevate",
        "createdump",
        "regedit",
        "winecfg",
        "wineconsole",
        "iexplore",
        "wmplayer",
        "steamservice",
        "gameoverlay",
        "qtwebengine",
        "jabswitch",
        "keytool",
        "oalinst",
        "physx",
        "xnafx",
        "dpinst",
        "touchup",
        "repair",
        "cleanup",
        "sandbox",
        "crashpadhandler",
        "vcredist",
        "prereq",
        "redistributable",
        "dotnet",
        "msiexec",
        "unitycrash",
        "uploader",
        "diagnostic",
        "benchmarktool",
    ];
    const EXACT: &[&str] = &[
        "python",
        "pythonw",
        "python3",
        "java",
        "javaw",
        "node",
        "nw",
        "cmd",
        "explorer",
        "7z",
        "7za",
        "unzip",
        "wine",
        "wine64",
        "wineserver",
        "steam",
        "steamcmd",
        "ffmpeg",
        "ffprobe",
        "dosbox",
        "chromesandbox",
        "notepad",
        "conhost",
        "rundll32",
        "regsvr32",
        "mono",
        "gamemoderun",
        "mangohud",
        "lodestar",
        "appimagetool",
        "umurun",
        "faugusrun",
        "winetricks",
        "protontricks",
    ];
    n.is_empty() || EXACT.contains(&n.as_str()) || PARTS.iter().any(|p| n.contains(p))
}

/// Folders never walked (lowercase names).
fn skip_dir(name: &str, parent: &Path) -> bool {
    let l = name.to_lowercase();
    if l.starts_with('.') {
        return true;
    }
    const SKIP: &[&str] = &[
        "__macosx",
        "node_modules",
        "_commonredist",
        "commonredist",
        "redist",
        "redists",
        "redistributables",
        "_redist",
        "__installer",
        "installer",
        "installers",
        "directx",
        "vcredist",
        "dotnet",
        "prerequisites",
        "prereqs",
        "users",
        "programdata",
        "common files",
        "internet explorer",
        "windows media player",
        "windows nt",
        "windows defender",
        "windows mail",
        "windows photo viewer",
        "windows sidebar",
        "windowspowershell",
        "windows portable devices",
        "microsoft.net",
        "msbuild",
        "reference assemblies",
        "dosdevices",
        "shadercache",
        "gstreamer-1.0",
        "compatdata",
        "shadercache",
        "proton_shortcuts",
        "openxr",
        "vrclient",
        "crashreports",
        "crashdumps",
        "logs",
        "cache",
        "__pycache__",
        "site-packages",
        "jre",
        "jdk",
        "mono",
        "dotnet_runtime",
        "easyanticheat",
        "battleye",
        "steamlinuxruntime",
        "steamlinuxruntime_soldier",
        "steamlinuxruntime_sniper",
    ];
    if SKIP.contains(&l.as_str()) {
        return true;
    }
    let parent_name = parent.file_name().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
    // `drive_c/windows`, `Program Files*/Steam` (Proton's stub client), `…/Microsoft*`.
    (l == "windows" && parent_name == "drive_c")
        || (parent_name.starts_with("program files") && (l == "steam" || l.starts_with("microsoft") || l == "umu"))
}

/// Folders that hold games rather than being one (descended into, not named after).
fn is_container_dir(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "drive_c" | "program files" | "program files (x86)" | "games" | "gog games" | "steamapps" | "common" | "pfx"
    )
}

/// A runnable file found by the walk.
#[derive(Clone, Debug)]
pub struct Hit {
    pub path: PathBuf,
    pub kind: ProgramKind,
    pub size: u64,
    pub depth: usize,
}

/// Is this file a game program candidate? Cheap checks first (extension, +x) and a 20-byte read
/// only for executable files without a telling extension.
pub fn classify_file(path: &Path, meta: &std::fs::Metadata) -> Option<ProgramKind> {
    use std::os::unix::fs::PermissionsExt;
    let ext = ext_lower(path);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let exec = meta.permissions().mode() & 0o111 != 0;
    let kind = match ext.as_str() {
        "exe" => ProgramKind::Windows,
        "appimage" => ProgramKind::AppImage,
        "sh" if exec => ProgramKind::Script,
        "" | "x86_64" | "x86" | "x64" | "amd64" | "bin" | "64" | "32" if exec => {
            let bytes = head(path, 20).ok()?;
            match sniff(path, &bytes)? {
                k @ (ProgramKind::Linux | ProgramKind::AppImage) => k,
                ProgramKind::Script => ProgramKind::Script,
                _ => return None,
            }
        }
        _ => return None,
    };
    let stem_for_check = if kind == ProgramKind::AppImage { appimage_base_name(stem) } else { stem.to_string() };
    (!is_excluded_program(&stem_for_check)).then_some(kind)
}

/// Walk `dir` (depth-limited, no symlinked folders), collecting hits.
fn walk(dir: &Path, depth: usize, sink: &mut Sink, out: &mut Vec<Hit>) {
    if depth > MAX_DEPTH || sink.files >= MAX_FILES || sink.cancelled() {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<std::fs::DirEntry> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if sink.files >= MAX_FILES || sink.cancelled() {
            return;
        }
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if ft.is_dir() {
            if !skip_dir(&name, dir) {
                walk(&p, depth + 1, sink, out);
            }
            continue;
        }
        sink.tick();
        // Symlinked files are followed (an AppImage linked into ~/Games); folders never are.
        let Ok(meta) = std::fs::metadata(&p) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        if let Some(kind) = classify_file(&p, &meta) {
            out.push(Hit { path: p, kind, size: meta.len(), depth });
        }
    }
}

/// How likely a hit is the game's main program, for the game folder `folder_name`.
pub fn score(h: &Hit, folder_name: &str) -> i64 {
    let stem = h.path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let n = norm_name(&program_name(&h.path, h.kind));
    let ns = norm_name(stem);
    let f = norm_name(folder_name);
    let mut s: i64 = 0;
    if !f.is_empty() && (n == f || ns == f) {
        s += 1000;
    } else if !f.is_empty() && ns.len() >= 3 && (f.contains(&ns) || ns.contains(&f)) {
        s += 500;
    }
    s += match h.kind {
        ProgramKind::Linux | ProgramKind::AppImage => 300,
        ProgramKind::Windows => 250,
        ProgramKind::Script => 100,
        _ => 0,
    };
    s -= 40 * h.depth as i64;
    s += (64 - h.size.max(1).leading_zeros() as i64) * 12; // log2(size)
    for (word, pen) in [
        ("settings", 450),
        ("config", 450),
        ("editor", 350),
        ("server", 350),
        ("dedicated", 350),
        ("benchmark", 300),
        ("tool", 250),
        ("sdk", 300),
        ("debug", 250),
        ("test", 200),
        ("launcher", 120),
        ("win32", 30),
        ("x86", 20),
    ] {
        if ns.contains(word) && !f.contains(word) {
            s -= pen;
        }
    }
    s
}

/// The Wine prefix a Windows program lives in (the folder holding `drive_c`).
pub fn prefix_of(exe: &Path) -> Option<PathBuf> {
    let mut cur = exe.parent();
    while let Some(d) = cur {
        if d.file_name().is_some_and(|n| n == "drive_c") {
            let p = d.parent()?;
            // Proton compat data: `<id>/pfx/drive_c` → WINEPREFIX is `<id>/pfx`, fine for umu too.
            return Some(p.to_path_buf());
        }
        cur = d.parent();
    }
    None
}

/// `/home/me/Games/x` → `~/Games/x`.
pub fn tilde(p: &Path, home: &Path) -> String {
    match p.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

pub struct Env<'a> {
    pub protons: &'a [ProtonVersion],
    pub home: &'a Path,
    pub umu: Option<String>,
}

impl Env<'_> {
    /// Launch for a found program: direct for Linux kinds; for Windows, umu-run in the prefix it
    /// is installed in with that prefix's Proton. A Windows program outside any prefix gets
    /// None: the checklist applies the Proton the user picks there and a new prefix
    /// (`default_prefix`).
    fn launch_for(&self, exe: &Path, kind: ProgramKind) -> Option<LaunchSpec> {
        match kind {
            ProgramKind::Windows => {
                let prefix = prefix_of(exe)?;
                let proton = proton_for_prefix(&prefix, self.protons)?;
                Some(windows_launch_with(exe, proton, &prefix, self.umu.clone(), self.home))
            }
            ProgramKind::Script if !is_executable(exe) => Some(LaunchSpec::Command {
                program: "sh".into(),
                args: vec![exe.to_string_lossy().into_owned()],
                cwd: exe.parent().map(Path::to_path_buf),
                env: vec![],
            }),
            _ => Some(LaunchSpec::Command {
                program: exe.to_string_lossy().into_owned(),
                args: vec![],
                cwd: exe.parent().map(Path::to_path_buf),
                env: vec![],
            }),
        }
    }

    fn candidate(&self, h: &Hit, name: String, install_dir: Option<PathBuf>, origin: String) -> ImportCandidate {
        let dir = install_dir.clone().or_else(|| h.path.parent().map(Path::to_path_buf));
        let icon = dir.as_deref().and_then(|d| nearby_icon(d, &h.path, &name));
        ImportCandidate {
            launch: self.launch_for(&h.path, h.kind),
            name,
            path: h.path.clone(),
            kind: h.kind,
            icon,
            install_dir,
            existing: None,
            origin,
        }
    }
}

/// The best hit of one game folder as a candidate (None when the folder has nothing runnable).
fn group_candidate(env: &Env, dir: &Path, hits: &[Hit]) -> Option<ImportCandidate> {
    let folder = dir.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let best = hits.iter().max_by_key(|h| score(h, folder))?;
    let name = if is_generic(folder) { program_name(&best.path, best.kind) } else { prettify_folder(folder) };
    Some(env.candidate(best, name, Some(dir.to_path_buf()), tilde(dir, env.home)))
}

/// Game folder names are usually already proper ("SP Football Life 2027"); only underscores /
/// all-lowercase names get prettified.
fn prettify_folder(s: &str) -> String {
    if s.contains('_') || !s.chars().any(|c| c.is_uppercase()) { prettify(s) } else { s.to_string() }
}

/// Scan a folder: every first-level game folder (below container folders such as `drive_c`,
/// `Program Files`, `Games`) gives at most one candidate — its best program; AppImages are
/// always their own candidate; loose programs directly in a folder form one group.
pub fn scan_folder(env: &Env, root: &Path, sink: &mut Sink) {
    if is_prefix(root) {
        let v = scan_prefix(
            env,
            &Prefix { path: root.to_path_buf(), origin: format!("Wine prefix {}", tilde(root, env.home)) },
            sink,
        );
        sink.found(v);
    } else {
        visit_level(env, root, 0, sink);
    }
    (sink.emit)(ImportMsg::Progress(sink.files));
}

fn visit_level(env: &Env, dir: &Path, depth: usize, sink: &mut Sink) {
    if depth > MAX_DEPTH || sink.cancelled() {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        if depth == 0 {
            (sink.emit)(ImportMsg::Error(format!("Can't read {}", dir.display())));
        }
        return;
    };
    let mut entries: Vec<std::fs::DirEntry> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    let mut loose: Vec<Hit> = vec![];
    let mut out: Vec<ImportCandidate> = vec![];
    for e in entries {
        if sink.files >= MAX_FILES || sink.cancelled() {
            break;
        }
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if ft.is_dir() {
            if skip_dir(&name, dir) {
                continue;
            }
            if is_container_dir(&name) {
                visit_level(env, &p, depth + 1, sink);
                continue;
            }
            // A Wine prefix: its registry and installer shortcuts know its games best.
            if is_prefix(&p) {
                let pfx = Prefix { path: p.clone(), origin: format!("Wine prefix {}", tilde(&p, env.home)) };
                let v = scan_prefix(env, &pfx, sink);
                sink.found(v);
                continue;
            }
            let mut hits = vec![];
            walk(&p, depth + 1, sink, &mut hits);
            let (apps, rest): (Vec<Hit>, Vec<Hit>) = hits.into_iter().partition(|h| h.kind == ProgramKind::AppImage);
            for a in &apps {
                out.push(env.candidate(
                    a,
                    program_name(&a.path, a.kind),
                    a.path.parent().map(Path::to_path_buf),
                    tilde(&p, env.home),
                ));
            }
            if let Some(c) = group_candidate(env, &p, &rest) {
                out.push(c);
            }
            sink.found(std::mem::take(&mut out));
            continue;
        }
        sink.tick();
        let Ok(meta) = std::fs::metadata(&p) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        if let Some(kind) = classify_file(&p, &meta) {
            loose.push(Hit { path: p, kind, size: meta.len(), depth });
        }
    }
    let (apps, rest): (Vec<Hit>, Vec<Hit>) = loose.into_iter().partition(|h| h.kind == ProgramKind::AppImage);
    for a in &apps {
        out.push(env.candidate(a, program_name(&a.path, a.kind), Some(dir.to_path_buf()), tilde(dir, env.home)));
    }
    if let Some(best) = rest.iter().max_by_key(|h| score(h, dir.file_name().and_then(|s| s.to_str()).unwrap_or(""))) {
        let name = program_name(&best.path, best.kind);
        out.push(env.candidate(best, name, Some(dir.to_path_buf()), tilde(dir, env.home)));
    }
    sink.found(out);
}

// ---------------------------------------------------------------------------------------------
// Detect installed

/// A Wine prefix and where it was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefix {
    pub path: PathBuf,
    pub origin: String,
}

fn is_prefix(p: &Path) -> bool {
    p.join("drive_c").is_dir()
}

/// Known Wine prefixes: Faugus (`games.json` prefixes, `~/Faugus/*`), Lutris game configs,
/// Bottles, Heroic, `~/Games/*` (two levels), `~/.wine`. Deduplicated by canonical path.
pub fn find_prefixes(ctx: &ScanContext) -> Vec<Prefix> {
    let h = &ctx.home;
    let mut cands: Vec<(PathBuf, String)> = vec![];
    // Faugus
    for dir in crate::scan::faugus::data_dirs(ctx) {
        if let Ok(text) = crate::scan::util::read_text(dir.join("games.json"))
            && let Ok(v) = serde_json::from_str::<Value>(&text)
        {
            for g in v.as_array().map(Vec::as_slice).unwrap_or_default() {
                if let Some(p) = g.get("prefix").and_then(Value::as_str).filter(|p| p.starts_with('/')) {
                    cands.push((PathBuf::from(p), "Faugus prefix".into()));
                }
            }
        }
    }
    for base in [h.join("Faugus"), ctx.flatpak_app(crate::scan::faugus::FLATPAK_ID).join("data/Faugus")] {
        for d in subdirs(&base) {
            cands.push((d, "Faugus prefix".into()));
        }
    }
    // Lutris: `prefix:` in the per-game YAML configs.
    for base in [
        ctx.config_home().join("lutris/games"),
        ctx.data_home().join("lutris/games"),
        ctx.flatpak_app("net.lutris.Lutris").join("config/lutris/games"),
        ctx.flatpak_app("net.lutris.Lutris").join("data/lutris/games"),
    ] {
        let Ok(rd) = std::fs::read_dir(&base) else {
            continue;
        };
        for e in rd.flatten().take(2000) {
            let Ok(text) = crate::scan::util::read_text(e.path()) else {
                continue;
            };
            for line in text.lines() {
                if let Some(v) = line.trim().strip_prefix("prefix:") {
                    let v = v.trim().trim_matches(|c| c == '\'' || c == '"');
                    let p = if let Some(rest) = v.strip_prefix("~/") { h.join(rest) } else { PathBuf::from(v) };
                    if p.is_absolute() {
                        cands.push((p, "Lutris prefix".into()));
                    }
                }
            }
        }
    }
    cands.extend(subdirs(&h.join("Games/lutris")).into_iter().map(|d| (d, "Lutris prefix".into())));
    // Bottles
    for base in [
        ctx.data_home().join("bottles/bottles"),
        ctx.flatpak_app("com.usebottles.bottles").join("data/bottles/bottles"),
    ] {
        cands.extend(subdirs(&base).into_iter().map(|d| (d, "Bottles".into())));
    }
    // Heroic: default prefix folders and per-game `winePrefix`.
    for base in [h.join("Games/Heroic/Prefixes"), h.join("Games/Heroic/Prefixes/default")] {
        cands.extend(subdirs(&base).into_iter().map(|d| (d, "Heroic prefix".into())));
    }
    for cfg in [
        ctx.config_home().join("heroic/GamesConfig"),
        ctx.flatpak_app("com.heroicgameslauncher.hgl").join("config/heroic/GamesConfig"),
    ] {
        let Ok(rd) = std::fs::read_dir(&cfg) else {
            continue;
        };
        for e in rd.flatten().take(2000) {
            let Ok(text) = crate::scan::util::read_text(e.path()) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            for (_, game) in v.as_object().into_iter().flatten() {
                if let Some(p) = game.get("winePrefix").and_then(Value::as_str).filter(|p| p.starts_with('/')) {
                    cands.push((PathBuf::from(p), "Heroic prefix".into()));
                }
            }
        }
    }
    // ~/Games/* and ~/Games/*/*, ~/.wine
    for d in subdirs(&h.join("Games")) {
        let sub = subdirs(&d);
        cands.push((d, "Wine prefix".into()));
        cands.extend(sub.into_iter().map(|s| (s, "Wine prefix".into())));
    }
    cands.push((h.join(".wine"), "Wine prefix".into()));

    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut out = vec![];
    for (p, origin) in cands {
        if !is_prefix(&p) {
            continue;
        }
        let canon = p.canonicalize().unwrap_or_else(|_| p.clone());
        if seen.insert(canon) {
            out.push(Prefix { origin: format!("{origin} {}", tilde(&p, h)), path: p });
        }
    }
    out
}

fn subdirs(d: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(d) else {
        return vec![];
    };
    let mut v: Vec<PathBuf> =
        rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).take(500).collect();
    v.sort();
    v
}

/// One `Uninstall` registry entry of a prefix.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegApp {
    pub key: String,
    pub display_name: String,
    pub install_location: Option<String>,
    pub display_icon: Option<String>,
    pub uninstall_string: Option<String>,
    pub publisher: Option<String>,
    pub system_component: bool,
}

/// Unescape a Wine `.reg` string value (`\\`, `\"`, `\n`, `\t`, `\xHHHH`).
fn reg_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => {}
            Some('x') => {
                let mut hex = String::new();
                while hex.len() < 4 && it.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                    hex.push(it.next().unwrap_or('0'));
                }
                if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    out.push(ch);
                }
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// `"Name"="value"` / `"Name"=str(2):"value"` / `"Name"=dword:…` → (name, value string).
fn reg_value(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix('"')?;
    // The name ends at the first unescaped quote.
    let mut end = None;
    let mut esc = false;
    for (i, c) in rest.char_indices() {
        match c {
            '\\' if !esc => esc = true,
            '"' if !esc => {
                end = Some(i);
                break;
            }
            _ => esc = false,
        }
    }
    let end = end?;
    let name = reg_unescape(&rest[..end]);
    let val = rest[end + 1..].strip_prefix('=')?;
    let val = val.strip_prefix("str(2):").or_else(|| val.strip_prefix("str(7):")).unwrap_or(val);
    if let Some(v) = val.strip_prefix('"') {
        let v = v.strip_suffix('"').unwrap_or(v);
        return Some((name, reg_unescape(v)));
    }
    if let Some(d) = val.strip_prefix("dword:") {
        return Some((name, u32::from_str_radix(d.trim(), 16).map(|n| n.to_string()).unwrap_or_default()));
    }
    None
}

/// `Uninstall` entries (`Software\…\CurrentVersion\Uninstall\<key>`, also `Wow6432Node`) of a
/// Wine `system.reg` / `user.reg`.
pub fn parse_uninstall_entries(text: &str) -> Vec<RegApp> {
    let mut out: Vec<RegApp> = vec![];
    let mut cur: Option<RegApp> = None;
    let finish = |cur: &mut Option<RegApp>, out: &mut Vec<RegApp>| {
        if let Some(a) = cur.take().filter(|a| !a.display_name.trim().is_empty()) {
            out.push(a);
        }
    };
    for line in text.lines() {
        if let Some(h) = line.strip_prefix('[') {
            finish(&mut cur, &mut out);
            let key = h.split(']').next().unwrap_or("");
            let lower = key.to_ascii_lowercase();
            const MARK: &str = "\\\\microsoft\\\\windows\\\\currentversion\\\\uninstall\\\\";
            if let Some(i) = lower.find(MARK) {
                let sub = &key[i + MARK.len()..];
                if !sub.is_empty() && !sub.contains("\\\\") {
                    cur = Some(RegApp { key: reg_unescape(sub), ..Default::default() });
                }
            }
            continue;
        }
        let Some(app) = cur.as_mut() else { continue };
        let Some((name, value)) = reg_value(line) else {
            continue;
        };
        let opt = |v: String| Some(v.trim().to_string()).filter(|s| !s.is_empty());
        match name.as_str() {
            "DisplayName" => app.display_name = value.trim().to_string(),
            "InstallLocation" => app.install_location = opt(value),
            "Inno Setup: App Path" if app.install_location.is_none() => app.install_location = opt(value),
            "DisplayIcon" => app.display_icon = opt(value),
            "UninstallString" => app.uninstall_string = opt(value),
            "Publisher" => app.publisher = opt(value),
            "SystemComponent" => app.system_component = value == "1",
            _ => {}
        }
    }
    finish(&mut cur, &mut out);
    out
}

/// Registry entries that are runtimes, redistributables, Wine / Steam parts or store launchers.
pub fn is_system_app(a: &RegApp) -> bool {
    if a.system_component {
        return true;
    }
    let n = a.display_name.to_lowercase();
    const WORDS: &[&str] = &[
        "microsoft",
        ".net framework",
        ".net runtime",
        ".net host",
        ".net core",
        "visual c++",
        "redistributable",
        "directx",
        "runtime",
        "wine mono",
        "wine gecko",
        "update for",
        "hotfix",
        "security update",
        "service pack",
        "language pack",
        "physx",
        "openal",
        "xna framework",
        "webview2",
        "dotnet",
        "vcredist",
        "epic games launcher",
        "ea app",
        "ubisoft connect",
        "gog galaxy",
        "rockstar games launcher",
        "amazon games",
        "easyanticheat",
        "easy anti-cheat",
        "battleye",
        "steamworks common",
    ];
    // Whole names (or their first word): "Steam" but not "SteamWorld Dig", "Origin" but not "Original Sin".
    const NAMES: &[&str] =
        &["steam", "origin", "uplay", "battle.net", "umu", "wine", "mono", "gecko", "wic", "windows"];
    let first = n.split_whitespace().next().unwrap_or("");
    if NAMES.contains(&n.as_str()) || NAMES.contains(&first) {
        return true;
    }
    WORDS.iter().any(|w| n.contains(w)) || a.key.eq_ignore_ascii_case("wic")
}

/// A Windows path in `prefix` (`C:\Program Files\X\` → `<prefix>/drive_c/Program Files/X`),
/// matched case-insensitively against what exists.
pub fn win_to_unix(prefix: &Path, win: &str) -> Option<PathBuf> {
    let w = win.trim().trim_matches('"');
    // `"C:\x\game.exe",0` / `C:\x\game.exe,0` (icon index)
    let w = match w.rsplit_once(',') {
        Some((p, idx)) if idx.trim().parse::<i32>().is_ok() => p.trim().trim_matches('"'),
        _ => w,
    };
    let (drive, rest) = w.split_once(':')?;
    if drive.len() != 1 {
        return None;
    }
    let d = drive.to_ascii_lowercase();
    let base = if d == "c" {
        prefix.join("drive_c")
    } else {
        let link = prefix.join("dosdevices").join(format!("{d}:"));
        std::fs::read_link(&link).ok().map(|t| if t.is_absolute() { t } else { prefix.join("dosdevices").join(t) })?
    };
    let mut cur = base;
    for comp in rest.split(['\\', '/']).filter(|c| !c.is_empty()) {
        let exact = cur.join(comp);
        if exact.exists() {
            cur = exact;
            continue;
        }
        let found = std::fs::read_dir(&cur)
            .ok()?
            .flatten()
            .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(comp))
            .map(|e| e.path());
        cur = found.unwrap_or(exact);
    }
    Some(cur)
}

/// The first word of an `UninstallString` as a path (`"C:\x\unins000.exe" /SILENT` → `C:\x\unins000.exe`).
fn uninstall_exe(s: &str) -> &str {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix('"') {
        return rest.split('"').next().unwrap_or(rest);
    }
    match s.to_ascii_lowercase().find(".exe") {
        Some(i) => &s[..i + 4],
        None => s,
    }
}

/// The best program below `dir` (a few levels), like the folder scan picks one per game folder.
fn best_exe_in(dir: &Path, name: &str, sink: &mut Sink) -> Option<Hit> {
    let mut hits = vec![];
    let start = sink.files;
    walk_limited(dir, 0, 3, start + 3000, sink, &mut hits);
    let folder = dir.file_name().and_then(|s| s.to_str()).unwrap_or(name);
    hits.into_iter().max_by_key(|h| score(h, folder).max(score(h, name)))
}

fn walk_limited(dir: &Path, depth: usize, max_depth: usize, max_files: usize, sink: &mut Sink, out: &mut Vec<Hit>) {
    if depth > max_depth || sink.files >= max_files || sink.cancelled() {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<std::fs::DirEntry> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if sink.files >= max_files {
            return;
        }
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        if ft.is_dir() {
            if !skip_dir(&e.file_name().to_string_lossy(), dir) {
                walk_limited(&p, depth + 1, max_depth, max_files, sink, out);
            }
            continue;
        }
        sink.tick();
        let Ok(meta) = std::fs::metadata(&p) else {
            continue;
        };
        if meta.is_file()
            && let Some(kind) = classify_file(&p, &meta)
            && kind == ProgramKind::Windows
        {
            out.push(Hit { path: p, kind, size: meta.len(), depth });
        }
    }
}

/// Windows programs installed in one prefix: the registry's Uninstall entries (name, folder,
/// icon exe) plus top-level folders of `Program Files*` / `Games` / `GOG Games` the registry
/// doesn't know.
pub fn scan_prefix(env: &Env, pfx: &Prefix, sink: &mut Sink) -> Vec<ImportCandidate> {
    let root = &pfx.path;
    let mut out: Vec<ImportCandidate> = vec![];
    let mut covered: Vec<PathBuf> = vec![];
    let mut apps: Vec<RegApp> = vec![];
    for f in ["system.reg", "user.reg"] {
        if let Ok(text) = crate::scan::util::read_text(root.join(f)) {
            apps.extend(parse_uninstall_entries(&text));
        }
    }
    let shortcuts = prefix_shortcuts(root);
    let proton = proton_for_prefix(root, env.protons);
    let mk = |env: &Env, h: &Hit, name: String, dir: PathBuf, sc_icon: Option<PathBuf>| {
        let icon = nearby_icon(&dir, &h.path, &name).or(sc_icon);
        ImportCandidate {
            launch: proton.map(|p| windows_launch_with(&h.path, p, root, env.umu.clone(), env.home)),
            name,
            path: h.path.clone(),
            kind: ProgramKind::Windows,
            icon,
            install_dir: Some(dir),
            existing: None,
            origin: pfx.origin.clone(),
        }
    };
    let hit_of = |p: PathBuf| {
        let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        Hit { path: p, kind: ProgramKind::Windows, size, depth: 0 }
    };
    // The main program of a game folder: the installer's shortcut named like the game, else
    // the registry's icon exe or another shortcut, else the best program found in the folder.
    let choose =
        |dir: &Path, name: &str, icon_exe: Option<PathBuf>, sink: &mut Sink| -> Option<(Hit, Option<PathBuf>)> {
            let folder = dir.file_name().and_then(|s| s.to_str()).unwrap_or(name);
            let mut opts: Vec<(i64, Hit, Option<PathBuf>)> = vec![];
            for sc in shortcuts.iter().filter(|sc| sc.exe.starts_with(dir)) {
                let bonus = if norm_name(&sc.name) == norm_name(name) { 2500 } else { 1500 };
                let h = hit_of(sc.exe.clone());
                opts.push((score(&h, folder) + bonus, h, sc.icon.clone()));
            }
            if let Some(p) = icon_exe {
                let h = hit_of(p);
                opts.push((score(&h, folder) + 1500, h, None));
            }
            if let Some((_, h, icon)) = opts.into_iter().max_by_key(|o| o.0) {
                return Some((h, icon));
            }
            best_exe_in(dir, name, sink).map(|h| (h, None))
        };
    for a in apps.iter().filter(|a| !is_system_app(a)) {
        if sink.cancelled() {
            return out;
        }
        let icon_exe = a
            .display_icon
            .as_deref()
            .filter(|i| i.to_ascii_lowercase().contains(".exe"))
            .and_then(|i| win_to_unix(root, i))
            .filter(|p| p.is_file() && ext_lower(p) == "exe")
            .filter(|p| !is_excluded_program(p.file_stem().and_then(|s| s.to_str()).unwrap_or("")));
        let dir = a
            .install_location
            .as_deref()
            .and_then(|l| win_to_unix(root, l))
            .filter(|d| d.is_dir())
            .or_else(|| {
                a.uninstall_string
                    .as_deref()
                    .and_then(|u| win_to_unix(root, uninstall_exe(u)))
                    .and_then(|p| p.parent().map(Path::to_path_buf))
                    .filter(|d| d.is_dir())
            })
            .or_else(|| icon_exe.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)));
        let Some(dir) = dir else { continue };
        // Don't offer the prefix's drive or a whole Program Files folder as a game.
        if dir == *root
            || dir == root.join("drive_c")
            || dir.components().count() < 4
            || dir.file_name().is_some_and(|n| is_container_dir(&n.to_string_lossy()))
            || covered.contains(&dir)
        {
            continue;
        }
        let Some((hit, sc_icon)) = choose(&dir, &a.display_name, icon_exe, sink) else {
            continue;
        };
        covered.push(dir.clone());
        out.push(mk(env, &hit, a.display_name.clone(), dir, sc_icon));
    }
    let drive = root.join("drive_c");
    for top in ["Program Files", "Program Files (x86)", "Games", "GOG Games"] {
        let base = drive.join(top);
        for d in subdirs(&base) {
            if sink.cancelled() {
                return out;
            }
            let folder = d.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            if skip_dir(&folder, &base) || covered.iter().any(|c| c.starts_with(&d) || d.starts_with(c)) {
                continue;
            }
            if let Some((hit, sc_icon)) = choose(&d, &folder, None, sink) {
                // The installer's shortcut name is the nicest name when it runs this program.
                let name = shortcuts
                    .iter()
                    .find(|sc| sc.exe == hit.path && !is_generic(&sc.name))
                    .map(|sc| sc.name.clone())
                    .unwrap_or_else(|| prettify_folder(&folder));
                covered.push(d.clone());
                out.push(mk(env, &hit, name, d, sc_icon));
            }
        }
    }
    out
}

/// An installer's shortcut as Proton records it (`drive_c/proton_shortcuts/*.desktop`:
/// `Path=` the program's folder, `StartupWMClass=` its exe name in lowercase, icons beside).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub name: String,
    pub exe: PathBuf,
    pub icon: Option<PathBuf>,
}

pub fn prefix_shortcuts(prefix: &Path) -> Vec<Shortcut> {
    let dir = prefix.join("drive_c/proton_shortcuts");
    let mut out = vec![];
    for f in subfiles(&dir) {
        if ext_lower(&f) != "desktop" {
            continue;
        }
        let Some(e) = crate::scan::desktop::read_entry_file(&f) else {
            continue;
        };
        let (Some(name), Some(path), Some(class)) = (e.string("Name"), e.string("Path"), e.string("StartupWMClass"))
        else {
            continue;
        };
        let folder = PathBuf::from(path);
        let Some(exe) = subfiles(&folder).into_iter().find(|p| {
            p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(&class)) && ext_lower(p) == "exe"
        }) else {
            continue;
        };
        if is_excluded_program(exe.file_stem().and_then(|s| s.to_str()).unwrap_or("")) || is_excluded_program(&name) {
            continue;
        }
        let icon = e.string("Icon").and_then(|i| {
            ["256x256", "128x128", "96x96", "64x64", "48x48", "32x32"].iter().find_map(|sz| {
                crate::scan::util::file_if_exists(dir.join("icons").join(sz).join("apps").join(format!("{i}.png")))
            })
        });
        out.push(Shortcut { name, exe, icon });
    }
    out
}

fn subfiles(d: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(d) else {
        return vec![];
    };
    let mut v: Vec<PathBuf> =
        rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_file())).map(|e| e.path()).take(2000).collect();
    v.sort();
    v
}

/// `.desktop` games (incl. Flatpak exports) as candidates. Shortcuts into other launchers are
/// left out (those launchers' scanners bring the games themselves).
pub fn desktop_candidates(ctx: &ScanContext) -> Vec<ImportCandidate> {
    let Ok(games) = crate::scan::desktop::scan(ctx) else {
        return vec![];
    };
    games
        .into_iter()
        .filter(|g| g.duplicate_of.is_none() && g.installed)
        .map(|g| {
            let flatpak = g.launch.as_ref().and_then(is_flatpak_launch);
            let path = match (&flatpak, &g.launch) {
                (None, Some(LaunchSpec::Command { program, .. })) => PathBuf::from(program),
                _ => PathBuf::from(&g.source_id),
            };
            ImportCandidate {
                name: g.name,
                path,
                kind: if flatpak.is_some() { ProgramKind::Flatpak } else { ProgramKind::Desktop },
                launch: g.launch,
                icon: g.art.icon,
                install_dir: g.install_dir,
                existing: None,
                origin: if flatpak.is_some() { "Flatpak".into() } else { format!("Desktop entry {}", g.source_id) },
            }
        })
        .collect()
}

/// The whole "Detect installed" scan.
pub fn scan_installed(env: &Env, ctx: &ScanContext, sink: &mut Sink) {
    for pfx in find_prefixes(ctx) {
        if sink.cancelled() {
            return;
        }
        let v = scan_prefix(env, &pfx, sink);
        sink.found(v);
    }
    if sink.cancelled() {
        return;
    }
    let d = desktop_candidates(ctx);
    sink.files += d.len();
    sink.found(d);
    (sink.emit)(ImportMsg::Progress(sink.files));
}

/// Run a scan on its own thread. Messages arrive on the returned channel; `cancel` stops it.
pub fn spawn(
    src: super::ImportSource,
    folder: Option<PathBuf>,
    ctx: ScanContext,
    cancel: Arc<AtomicBool>,
    repaint: egui::Context,
) -> crossbeam_channel::Receiver<ImportMsg> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let spawned = std::thread::Builder::new().name("import-scan".into()).spawn(move || {
        let protons = proton_versions(&ctx);
        let env = Env { protons: &protons, home: &ctx.home, umu: find_umu(&ctx.home) };
        let mut emit = |m: ImportMsg| {
            let _ = tx.send(m);
            repaint.request_repaint();
        };
        let mut sink = Sink { cancel: &cancel, emit: &mut emit, files: 0 };
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match src {
            super::ImportSource::Folder => match &folder {
                Some(f) => scan_folder(&env, f, &mut sink),
                None => (sink.emit)(ImportMsg::Error("No folder chosen".into())),
            },
            super::ImportSource::Installed => scan_installed(&env, &ctx, &mut sink),
        }));
        if r.is_err() {
            (sink.emit)(ImportMsg::Error("The scan stopped unexpectedly".into()));
        }
        (sink.emit)(ImportMsg::Done);
    });
    if let Err(e) = spawned {
        log::warn!("import scan thread: {e}");
    }
    rx
}
