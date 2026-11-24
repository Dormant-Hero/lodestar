//! Add Game → Pick a program: what a file is, a good name / icon / folder for it, Proton builds,
//! Wine prefixes and the umu-run command for Windows programs. Everything here is cheap (a few
//! header bytes, one `read_dir`) except `proton_versions` / `flatpak_apps`, which list a handful of
//! directories — call those from a click handler or a background thread, not every frame.

use super::{ProgramInfo, ProgramKind, ProtonVersion};
use crate::config::Paths;
use crate::model::*;
use crate::scan::ScanContext;
use crate::scan::desktop::{self, IconResolver};
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------------------------
// Program detection

/// The first bytes of a regular file (FIFOs / devices are refused: reading one would block).
pub(crate) fn head(path: &Path, n: usize) -> std::io::Result<Vec<u8>> {
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    let mut buf = Vec::with_capacity(n);
    std::fs::File::open(path)?.take(n as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

pub(crate) fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

pub(crate) fn ext_lower(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// Classify by magic bytes first, then by extension. `None` = nothing Lodestar can run.
pub(crate) fn sniff(path: &Path, bytes: &[u8]) -> Option<ProgramKind> {
    let ext = ext_lower(path);
    if ext == "desktop" {
        return Some(ProgramKind::Desktop);
    }
    if bytes.starts_with(b"\x7fELF") {
        // AppImage type 1 / 2: "AI" + version at offset 8 of the ELF header.
        let appimage = bytes.len() >= 11 && &bytes[8..10] == b"AI" && matches!(bytes[10], 1 | 2);
        return Some(if appimage || ext == "appimage" { ProgramKind::AppImage } else { ProgramKind::Linux });
    }
    if ext == "appimage" {
        return Some(ProgramKind::AppImage);
    }
    if bytes.starts_with(b"#!") {
        return Some(ProgramKind::Script);
    }
    if bytes.starts_with(b"MZ") || matches!(ext.as_str(), "exe" | "bat" | "cmd" | "msi" | "com" | "lnk") {
        return Some(ProgramKind::Windows);
    }
    None
}

/// `detect_program` with an explicit scan context (tests point `home` at a fixture tree).
pub fn detect_program_in(ctx: &ScanContext, path: &Path) -> anyhow::Result<ProgramInfo> {
    if path.is_dir() {
        anyhow::bail!("{} is a folder — pick the game's program file", path.display());
    }
    let bytes = head(path, 256).map_err(|e| anyhow::anyhow!("can't read {}: {e}", path.display()))?;
    let Some(kind) = sniff(path, &bytes) else {
        anyhow::bail!(
            "{} is not a program Lodestar can run (pick a Linux program, script, AppImage, .exe or .desktop file)",
            path.display()
        );
    };
    let exe = path.to_path_buf();
    if kind == ProgramKind::Desktop {
        return desktop_info(ctx, path);
    }
    let name = program_name(path, kind);
    let install_dir = path.parent().map(Path::to_path_buf);
    let icon = install_dir.as_deref().and_then(|d| nearby_icon(d, path, &name));
    let launch = match kind {
        ProgramKind::Windows => None,
        ProgramKind::Script if !is_executable(path) => Some(script_launch(path, &bytes)),
        _ => Some(LaunchSpec::Command {
            program: path.to_string_lossy().into_owned(),
            args: vec![],
            cwd: install_dir.clone(),
            env: vec![],
        }),
    };
    Ok(ProgramInfo { kind, name, icon, install_dir, launch, exe })
}

/// A script without the executable bit runs through its shebang interpreter.
fn script_launch(path: &Path, bytes: &[u8]) -> LaunchSpec {
    let first = String::from_utf8_lossy(bytes.split(|b| *b == b'\n').next().unwrap_or_default());
    let mut words: Vec<String> = first.trim_start_matches("#!").split_whitespace().map(str::to_string).collect();
    if words.is_empty() {
        words.push("sh".into());
    }
    let program = words.remove(0);
    words.push(path.to_string_lossy().into_owned());
    LaunchSpec::Command { program, args: words, cwd: path.parent().map(Path::to_path_buf), env: vec![] }
}

fn desktop_info(ctx: &ScanContext, path: &Path) -> anyhow::Result<ProgramInfo> {
    let e = desktop::read_entry_file(path)
        .ok_or_else(|| anyhow::anyhow!("{} is not a valid .desktop file", path.display()))?;
    let (launch, _windows) =
        desktop::entry_launch(&e).ok_or_else(|| anyhow::anyhow!("{} has no Exec= line", path.display()))?;
    let name = e
        .string("Name")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| prettify(&stem(path)));
    let mut install_dir = match &launch {
        LaunchSpec::Command { cwd: Some(c), .. } => Some(c.clone()),
        _ => None,
    };
    if install_dir.is_none()
        && let LaunchSpec::Command { program, .. } = &launch
        && Path::new(program).is_absolute()
        && !program.starts_with("/usr/")
        && !program.starts_with("/bin/")
    {
        install_dir = Path::new(program).parent().map(Path::to_path_buf);
    }
    let icon = e.string("Icon").and_then(|i| IconResolver::default().resolve(ctx, &i)).map(|(p, _)| p);
    let kind = if is_flatpak_launch(&launch).is_some() { ProgramKind::Flatpak } else { ProgramKind::Desktop };
    Ok(ProgramInfo { kind, name, icon, install_dir, launch: Some(launch), exe: path.to_path_buf() })
}

/// `flatpak run [opts] <app.id>` → the app id.
pub(crate) fn is_flatpak_launch(l: &LaunchSpec) -> Option<String> {
    let LaunchSpec::Command { program, args, .. } = l else {
        return None;
    };
    if basename(program) != "flatpak" {
        return None;
    }
    let mut it = args.iter().skip_while(|a| a.as_str() != "run");
    it.next()?;
    it.find(|a| !a.starts_with('-')).cloned()
}

pub(crate) fn basename(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}

fn stem(path: &Path) -> String {
    path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string()
}

/// Lowercase letters and digits only: "Baldur's Gate 3™" → "baldursgate3".
pub fn norm_name(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Folder / file names that say nothing about the game.
const GENERIC: &[&str] = &[
    "game",
    "games",
    "launcher",
    "launch",
    "start",
    "play",
    "run",
    "main",
    "app",
    "application",
    "client",
    "bin",
    "bin32",
    "bin64",
    "binaries",
    "win32",
    "win64",
    "x64",
    "x86",
    "x8664",
    "amd64",
    "linux",
    "linux64",
    "windows",
    "exe",
    "release",
    "shipping",
    "data",
    "program",
    "default",
    "appimage",
    "setup",
];

pub(crate) fn is_generic(s: &str) -> bool {
    let n = norm_name(s);
    n.is_empty() || n.len() <= 1 || GENERIC.contains(&n.as_str()) || n.chars().all(|c| c.is_ascii_digit())
}

/// Folder names never used as a game name (go further up).
const CONTAINERS: &[&str] = &[
    "drivec",
    "programfiles",
    "programfilesx86",
    "gog games",
    "goggames",
    "steamapps",
    "common",
    "pfx",
    "home",
    "downloads",
    "desktop",
    "documents",
    "opt",
    "usr",
    "local",
    "applications",
    "appimages",
    "tmp",
];

fn is_container(s: &str) -> bool {
    CONTAINERS.contains(&norm_name(s).as_str())
}

/// Suffixes of executables that are not part of the name.
const NAME_SUFFIXES: &[&str] = &[
    "-win64-shipping",
    "-win32-shipping",
    "-shipping",
    "_x86_64",
    ".x86_64",
    "-x86_64",
    "_x64",
    ".x64",
    "-x64",
    "_64",
    "_x86",
    ".x86",
    "-x86",
    ".bin",
    "_linux",
    "-linux",
    ".linux",
];

/// A suggested game name for a program file.
pub fn program_name(path: &Path, kind: ProgramKind) -> String {
    let raw = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
    let base = match kind {
        ProgramKind::AppImage => appimage_base_name(&raw),
        ProgramKind::Linux => {
            // `hollow_knight.x86_64`: the "extension" is an architecture, not part of the name.
            let mut s = raw.clone();
            for suf in NAME_SUFFIXES {
                if s.len() > suf.len() && s.to_ascii_lowercase().ends_with(suf) {
                    s.truncate(s.len() - suf.len());
                    break;
                }
            }
            s
        }
        _ => {
            let mut s = stem(path);
            let lower = s.to_ascii_lowercase();
            for suf in NAME_SUFFIXES {
                if lower.len() > suf.len() && lower.ends_with(suf) {
                    s.truncate(s.len() - suf.len());
                    break;
                }
            }
            s
        }
    };
    let pretty = strip_generic_words(&prettify(&base));
    let folder = folder_name(path.parent());
    if !is_generic(&pretty) {
        // "Witcher3" in "The Witcher 3": the folder has the fuller name.
        if let Some(f) = folder.filter(|f| {
            let (nf, np) = (norm_name(f), norm_name(&pretty));
            np.len() >= 4 && nf.len() > np.len() && nf.contains(&np)
        }) {
            return f;
        }
        return pretty;
    }
    folder.unwrap_or(if pretty.is_empty() { raw } else { pretty })
}

/// "FL 2027 start" → "FL 2027", "Game Launcher" → "Game Launcher" (kept when nothing else is left).
fn strip_generic_words(s: &str) -> String {
    let mut words: Vec<&str> = s.split_whitespace().collect();
    const TRAILING: &[&str] =
        &["start", "launcher", "launch", "x64", "x86", "win64", "win32", "64bit", "32bit", "shipping", "exe", "bin"];
    while words.len() > 1 && words.last().is_some_and(|w| TRAILING.contains(&norm_name(w).as_str())) {
        words.pop();
    }
    words.join(" ")
}

/// The closest ancestor folder whose name says something ("Game/Binaries/Win64" → "Game").
pub(crate) fn folder_name(mut dir: Option<&Path>) -> Option<String> {
    let mut steps = 0;
    while let Some(d) = dir {
        let n = d.file_name()?.to_str()?;
        if is_container(n) || steps > 5 {
            return None;
        }
        if !is_generic(n) {
            return Some(prettify(n));
        }
        dir = d.parent();
        steps += 1;
    }
    None
}

/// "Heroic-2.15.2-x86_64.AppImage" → "Heroic", "Some_Game-v1.0-linux.AppImage" → "Some_Game".
pub fn appimage_base_name(file: &str) -> String {
    let s = file.strip_suffix(".AppImage").or_else(|| file.strip_suffix(".appimage")).unwrap_or(file);
    let s = if s.to_ascii_lowercase().ends_with(".appimage") { &s[..s.len() - 9] } else { s };
    let mut out: Vec<&str> = vec![];
    for tok in s.split(['-', '_', ' ', '.']) {
        let t = tok.to_ascii_lowercase();
        let version = t.starts_with(|c: char| c.is_ascii_digit())
            || (t.starts_with('v') && t[1..].starts_with(|c: char| c.is_ascii_digit()));
        let arch = matches!(
            t.as_str(),
            "x86"
                | "x64"
                | "amd64"
                | "aarch64"
                | "arm64"
                | "armhf"
                | "i386"
                | "i686"
                | "linux"
                | "linux64"
                | "appimage"
                | "64bit"
                | "32bit"
                | "release"
                | "stable"
                | "latest"
                | "portable"
        ) || t == "x86_64";
        if (version || arch) && !out.is_empty() {
            break;
        }
        if !tok.is_empty() && !arch {
            out.push(tok);
        }
    }
    if out.is_empty() { s.to_string() } else { out.join(" ") }
}

/// "hollow_knight" → "Hollow Knight", "Super-Game" → "Super Game"; mixed-case names are kept.
pub fn prettify(s: &str) -> String {
    let spaced: String = if s.contains(' ') {
        s.replace('_', " ")
    } else {
        s.chars().map(|c| if c == '_' || c == '-' || c == '.' { ' ' } else { c }).collect()
    };
    let words: Vec<&str> = spaced.split_whitespace().collect();
    let all_lower = !spaced.chars().any(|c| c.is_uppercase());
    words
        .iter()
        .map(|w| {
            if all_lower {
                let mut c = w.chars();
                match c.next() {
                    Some(f) => f.to_uppercase().chain(c).collect(),
                    None => String::new(),
                }
            } else {
                w.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// An image next to the program named like the game (or a plain `icon.png`).
pub fn nearby_icon(dir: &Path, exe: &Path, name: &str) -> Option<PathBuf> {
    let want = [norm_name(&stem(exe)), norm_name(name), norm_name(dir.file_name()?.to_str().unwrap_or(""))];
    let mut best: Option<(u8, PathBuf)> = None;
    let mut look = |d: &Path, base_rank: u8| {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten().take(500) {
            let p = e.path();
            let ext = ext_lower(&p);
            if !matches!(ext.as_str(), "png" | "ico" | "svg" | "jpg" | "jpeg" | "webp" | "bmp") {
                continue;
            }
            let s = norm_name(&stem(&p));
            let rank = if want.iter().any(|w| !w.is_empty() && *w == s) {
                3
            } else if matches!(s.as_str(), "icon" | "gameicon" | "appicon" | "logo") {
                2
            } else {
                continue;
            };
            let rank = rank * 2 + base_rank + u8::from(ext == "png");
            if best.as_ref().is_none_or(|(r, _)| rank > *r) && crate::scan::util::file_if_exists(&p).is_some() {
                best = Some((rank, p));
            }
        }
    };
    look(dir, 1);
    look(&dir.join("icons"), 0);
    best.map(|(_, p)| p)
}

// ---------------------------------------------------------------------------------------------
// Flatpak apps

/// An installed Flatpak app (for Add Game → Pick a program → Flatpak).
#[derive(Clone, Debug)]
pub struct FlatpakApp {
    pub id: String,
    /// Exported with `Categories=…Game…`.
    pub game: bool,
    pub info: ProgramInfo,
}

/// Installed Flatpak apps (user + system), games first, then by name. Reads the exported
/// `.desktop` files (no `flatpak` process is started). Tens of ms: call off the UI thread.
pub fn flatpak_apps(ctx: &ScanContext) -> Vec<FlatpakApp> {
    let mut dirs = vec![ctx.data_home().join("flatpak/exports/share/applications")];
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    dirs.extend(
        ctx.xdg_data_dirs.iter().filter(|d| d.to_string_lossy().contains("flatpak")).map(|d| d.join("applications")),
    );
    let mut icons = IconResolver::default();
    let mut out: Vec<FlatpakApp> = vec![];
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| ext_lower(p) == "desktop").collect();
        files.sort();
        for f in files {
            let Some(e) = desktop::read_entry_file(&f) else {
                continue;
            };
            if e.get("Type") != Some("Application") || e.bool("NoDisplay") || e.bool("Hidden") {
                continue;
            }
            let Some((launch, _)) = desktop::entry_launch(&e) else {
                continue;
            };
            let Some(id) = is_flatpak_launch(&launch) else {
                continue;
            };
            if out.iter().any(|a| a.id == id) {
                continue;
            }
            let name = e.string("Name").unwrap_or_else(|| id.clone());
            let game = e.list("Categories").iter().any(|c| c.eq_ignore_ascii_case("Game"));
            let icon = e.string("Icon").and_then(|i| icons.resolve(ctx, &i)).map(|(p, _)| p);
            let launch = LaunchSpec::Command {
                program: "flatpak".into(),
                args: vec!["run".into(), id.clone()],
                cwd: None,
                env: vec![],
            };
            out.push(FlatpakApp {
                id: id.clone(),
                game,
                info: ProgramInfo {
                    kind: ProgramKind::Flatpak,
                    name,
                    icon,
                    install_dir: None,
                    launch: Some(launch),
                    exe: f,
                },
            });
        }
    }
    out.sort_by(|a, b| b.game.cmp(&a.game).then_with(|| a.info.name.to_lowercase().cmp(&b.info.name.to_lowercase())));
    out
}

// ---------------------------------------------------------------------------------------------
// Proton

impl ProtonVersion {
    /// "CachyOS", "GE", "Experimental", "UMU", "Proton-EM" or "Valve".
    pub fn flavor(&self) -> &'static str {
        let n = self.name.to_ascii_lowercase();
        if n.contains("cachyos") {
            "CachyOS"
        } else if n.contains("ge-proton") || n.contains("proton-ge") || n.starts_with("ge") || n.contains(" ge") {
            "GE"
        } else if n.contains("experimental") {
            "Experimental"
        } else if n.contains("umu") {
            "UMU"
        } else if n.contains("proton-em") || n.contains("proton em") {
            "Proton-EM"
        } else {
            "Valve"
        }
    }
}

/// Every Proton build found: `compatibilitytools.d` of every Steam root (native, `~/.steam`,
/// Flatpak, system `/usr/share/steam`), `steamapps/common/Proton*` of every library, umu's own
/// folder. Each must contain a `proton` script. Newest first (by the build's `version` file,
/// else the script's mtime).
pub fn proton_versions(ctx: &ScanContext) -> Vec<ProtonVersion> {
    let flat = ctx.flatpak_app("com.valvesoftware.Steam");
    let mut tool_dirs: Vec<PathBuf> = vec![
        ctx.data_home().join("Steam/compatibilitytools.d"),
        ctx.home.join(".steam/root/compatibilitytools.d"),
        ctx.home.join(".steam/steam/compatibilitytools.d"),
        flat.join("data/Steam/compatibilitytools.d"),
        flat.join(".local/share/Steam/compatibilitytools.d"),
        ctx.data_home().join("umu/compatibilitytools"),
        PathBuf::from("/usr/share/steam/compatibilitytools.d"),
        PathBuf::from("/usr/local/share/steam/compatibilitytools.d"),
    ];
    let mut common_dirs: Vec<PathBuf> = vec![];
    for root in crate::scan::steam::find_roots(ctx) {
        tool_dirs.push(root.path.join("compatibilitytools.d"));
        for lib in crate::scan::steam::library_folders(&root.path) {
            common_dirs.push(lib.join("steamapps/common"));
        }
    }
    let mut found: Vec<(i64, ProtonVersion)> = vec![];
    let mut seen: Vec<PathBuf> = vec![];
    let mut add = |dir: PathBuf, fallback_name: String| {
        if !dir.join("proton").is_file() {
            return;
        }
        let canon = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        if seen.contains(&canon) {
            return;
        }
        seen.push(canon);
        let name = display_name(&dir).unwrap_or(fallback_name);
        found.push((build_time(&dir), ProtonVersion { name, path: dir }));
    };
    for td in tool_dirs {
        let Ok(rd) = std::fs::read_dir(&td) else {
            continue;
        };
        let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        v.sort();
        for d in v {
            let n = d.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            add(d, n);
        }
    }
    for cd in common_dirs {
        let Ok(rd) = std::fs::read_dir(&cd) else {
            continue;
        };
        let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        v.sort();
        for d in v {
            let n = d.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            if n.starts_with("Proton") {
                add(d, n);
            }
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    found.into_iter().map(|(_, p)| p).collect()
}

/// `display_name` from `compatibilitytool.vdf`.
fn display_name(dir: &Path) -> Option<String> {
    let text = crate::scan::util::read_text(dir.join("compatibilitytool.vdf")).ok()?;
    let v = crate::scan::vdf::parse(&text).ok()?;
    let tools = v.path(&["compatibilitytools", "compat_tools"])?;
    tools.entries().iter().find_map(|(_, t)| t.get_str("display_name").map(str::to_string)).filter(|s| !s.is_empty())
}

/// Build time: first word of `version` ("1792530497 cachyos-11.0-…"), else the script's mtime.
fn build_time(dir: &Path) -> i64 {
    crate::scan::util::read_text(dir.join("version"))
        .ok()
        .and_then(|t| t.split_whitespace().next().and_then(|w| w.parse::<i64>().ok()))
        .filter(|t| *t > 1_000_000_000)
        .or_else(|| crate::scan::util::mtime(&dir.join("proton")))
        .unwrap_or(0)
}

/// The Proton build a prefix was last run with, when it can be told (`config_info` names the
/// build's `files/` folder; else `version` names its flavour). Falls back to the newest.
pub fn proton_for_prefix<'a>(prefix: &Path, protons: &'a [ProtonVersion]) -> Option<&'a ProtonVersion> {
    if let Ok(text) = crate::scan::util::read_text(prefix.join("config_info")) {
        for line in text.lines() {
            if let Some(i) = line.find("/files/") {
                let root = Path::new(&line[..i]);
                let canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
                if let Some(p) =
                    protons.iter().find(|p| p.path == root || p.path.canonicalize().is_ok_and(|c| c == canon))
                {
                    return Some(p);
                }
            }
        }
    }
    if let Ok(v) = crate::scan::util::read_text(prefix.join("version")) {
        let v = v.trim().to_ascii_lowercase();
        let flavor = if v.contains("cachyos") {
            Some("CachyOS")
        } else if v.contains("ge-proton") || v.starts_with("ge") {
            Some("GE")
        } else {
            None
        };
        if let Some(f) = flavor
            && let Some(p) = protons.iter().find(|p| p.flavor() == f)
        {
            return Some(p);
        }
    }
    protons.first()
}

// ---------------------------------------------------------------------------------------------
// Prefixes and the Windows launch

/// A folder name safe for any filesystem: "Baldur's Gate 3: Deluxe" → "Baldur's Gate 3 Deluxe".
pub fn safe_folder_name(name: &str) -> String {
    let s: String =
        name.chars()
            .map(|c| {
                if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                    ' '
                } else {
                    c
                }
            })
            .collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.trim_matches('.').trim().to_string();
    if s.is_empty() { "Game".into() } else { s }
}

/// A new Wine prefix folder for a game: `~/Games/Lodestar/<name>`.
pub fn default_prefix(paths: &Paths, name: &str) -> PathBuf {
    paths.home.join("Games/Lodestar").join(safe_folder_name(name))
}

/// `name` in `$PATH`.
pub(crate) fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| is_executable(p))
}

/// The umu-run to use: on `$PATH` (as the bare name), else the one Faugus bundles.
pub(crate) fn find_umu(home: &Path) -> Option<String> {
    if which("umu-run").is_some() {
        return Some("umu-run".into());
    }
    [
        home.join(".local/share/faugus-launcher/umu-run"),
        home.join(".var/app/io.github.Faugus.faugus-launcher/data/faugus-launcher/umu-run"),
    ]
    .into_iter()
    .find(|p| is_executable(p))
    .map(|p| p.to_string_lossy().into_owned())
}

/// Steam root a Proton build belongs to (for `STEAM_COMPAT_CLIENT_INSTALL_PATH`).
fn steam_root_of(proton: &Path, home: &Path) -> PathBuf {
    let s = proton.to_string_lossy();
    for marker in ["/compatibilitytools.d/", "/steamapps/common/"] {
        if let Some(i) = s.find(marker)
            && !s[..i].starts_with("/usr")
        {
            return PathBuf::from(&s[..i]);
        }
    }
    home.join(".local/share/Steam")
}

/// The command that runs a Windows program with Proton in `prefix`:
/// `umu-run <exe>` with `WINEPREFIX`, `GAMEID=umu-default`, `PROTONPATH` (umu on PATH, else
/// Faugus' bundled one), else Proton's own `proton run <exe>` with `STEAM_COMPAT_DATA_PATH` /
/// `STEAM_COMPAT_CLIENT_INSTALL_PATH`. `.msi` files go through `msiexec /i`. Working folder:
/// the program's folder.
pub fn windows_launch(exe: &Path, proton: &ProtonVersion, prefix: &Path) -> LaunchSpec {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    windows_launch_with(exe, proton, prefix, find_umu(&home), &home)
}

pub(crate) fn windows_launch_with(
    exe: &Path,
    proton: &ProtonVersion,
    prefix: &Path,
    umu: Option<String>,
    home: &Path,
) -> LaunchSpec {
    let exe_s = exe.to_string_lossy().into_owned();
    let target: Vec<String> =
        if ext_lower(exe) == "msi" { vec!["msiexec".into(), "/i".into(), exe_s] } else { vec![exe_s] };
    let cwd = exe.parent().map(Path::to_path_buf);
    let prefix_s = prefix.to_string_lossy().into_owned();
    let mut env = vec![
        ("WINEPREFIX".to_string(), prefix_s.clone()),
        ("GAMEID".to_string(), "umu-default".to_string()),
        ("PROTONPATH".to_string(), proton.path.to_string_lossy().into_owned()),
    ];
    match umu {
        Some(program) => LaunchSpec::Command { program, args: target, cwd, env },
        None => {
            env.push(("STEAM_COMPAT_DATA_PATH".into(), prefix_s));
            env.push((
                "STEAM_COMPAT_CLIENT_INSTALL_PATH".into(),
                steam_root_of(&proton.path, home).to_string_lossy().into_owned(),
            ));
            let mut args = vec!["run".to_string()];
            args.extend(target);
            LaunchSpec::Command { program: proton.path.join("proton").to_string_lossy().into_owned(), args, cwd, env }
        }
    }
}
