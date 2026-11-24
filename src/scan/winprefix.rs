//! Wine prefixes on this machine and how to run something inside each.
//!
//! CONTRACT (shared by wine_launchers, addgame's "Detect installed" and future emulation):
//!   pub struct Prefix { pub path: PathBuf, pub origin: String /* "Faugus: Football Life", "Lutris: …",
//!       "Bottles: …", "Heroic", "~/Games/x", "Steam shortcut 123" */, pub runner: Runner }
//!   pub enum Runner { Proton { path: PathBuf }, Wine { path: PathBuf }, Unknown }
//!   pub fn find_prefixes(ctx: &ScanContext) -> Vec<Prefix>   (cheap: config files + directory listings)
//!   pub fn run_in_prefix(prefix: &Prefix, exe: &Path, args: &[String]) -> LaunchSpec
//!       (umu-run / proton run / wine with WINEPREFIX etc. — same rules as addgame::windows_launch)
//!   pub fn registry_values(prefix: &Path, hive: Hive, key: &str) -> Vec<(String, String)>  (system.reg / user.reg)
//!
//! A prefix is a folder holding `drive_c` and `system.reg`. Proton-style prefixes (Steam
//! `compatdata/<id>`, Heroic with Proton) keep it in `<folder>/pfx`; then `Prefix::path` is the
//! `pfx` folder and [`compat_root`] gives the folder Proton / umu want.
//!
//! Where prefixes are found (each source's own config first, so its runner is known):
//! * Faugus: `games.json` `prefix` + `runner` (a `compatibilitytools.d` folder name or
//!   "GE-Proton Latest" / "UMU-Proton Latest"), and every folder of its `default-prefix`
//!   (`config.json`, default `~/Faugus`) with the `default-runner`.
//! * Lutris: `games/*.yml` → `game: prefix:` and `wine: version:` (`runners/wine/<version>/bin/wine`).
//! * Bottles: `bottles/<bottle>/bottle.yml` → `Name:`, `Runner:` (`runners/<runner>/bin/wine`).
//! * Heroic: `GamesConfig/*.json` and `config.json` `defaultSettings` → `winePrefix` +
//!   `wineVersion { bin, type }`; plus `~/Games/Heroic/Prefixes/*` and `…/Prefixes/default/*`.
//! * Steam: `steamapps/compatdata/<appid>/pfx` of non-Steam shortcuts (appid ≥ 2^31) with the
//!   compat tool from `CompatToolMapping` (per shortcut, else the global default, else Proton
//!   Experimental); names from `userdata/*/config/shortcuts.vdf`.
//! * umu: `~/Games/umu/*`; Lodestar's own `~/Games/Lodestar/*`; any prefix in `~/Games/*`; `~/.wine`.
//! * `ScanConfig::extra_prefix_dirs`: a prefix itself or a folder of prefixes.

pub mod registry;

use super::ScanContext;
use crate::model::LaunchSpec;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub use registry::{Hive, RegKey, Registry};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Runner {
    /// A Proton build: the folder holding its `proton` script.
    Proton {
        path: PathBuf,
    },
    /// A Wine binary (absolute path).
    Wine {
        path: PathBuf,
    },
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefix {
    pub path: PathBuf,
    pub origin: String,
    pub runner: Runner,
}

/// Most folders listed from one directory (a hostile / huge folder can't stall the scan).
const MAX_DIR_ENTRIES: usize = 500;

/// `p` holds a Wine prefix (`drive_c` + `system.reg`).
pub fn is_prefix(p: &Path) -> bool {
    p.join("drive_c").is_dir() && p.join("system.reg").is_file()
}

/// The prefix in `p`: `p` itself, or `p/pfx` (Proton-style).
pub fn prefix_in(p: &Path) -> Option<PathBuf> {
    if is_prefix(p) {
        return Some(p.to_path_buf());
    }
    let pfx = p.join("pfx");
    is_prefix(&pfx).then_some(pfx)
}

/// The folder Proton / umu take as `STEAM_COMPAT_DATA_PATH` / `WINEPREFIX`: the parent of a real
/// `pfx` folder (Steam / Heroic layout), else the prefix itself (umu keeps `pfx -> .`).
pub fn compat_root(prefix: &Path) -> PathBuf {
    if prefix.file_name().is_some_and(|n| n == "pfx")
        && let Some(parent) = prefix.parent()
        && std::fs::symlink_metadata(prefix).is_ok_and(|m| m.is_dir())
    {
        return parent.to_path_buf();
    }
    prefix.to_path_buf()
}

/// Short stable hash of a prefix path (6 hex digits, FNV-1a) for telling the same store game in
/// two prefixes apart.
pub fn prefix_hash(p: &Path) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in p.as_os_str().as_encoded_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{:06x}", h & 0x00ff_ffff)
}

/// `~/x` for paths under `home`.
pub fn tilde(p: &Path, home: &Path) -> String {
    match p.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

fn subdirs(d: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(d) else {
        return vec![];
    };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .take(MAX_DIR_ENTRIES)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) || e.path().is_dir())
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// `~/x`, `$HOME/x` and absolute paths; anything else is `None`.
fn expand_home(s: &str, home: &Path) -> Option<PathBuf> {
    let s = s.trim().trim_matches(|c| c == '\'' || c == '"');
    if let Some(rest) = s.strip_prefix("~/") {
        return Some(home.join(rest));
    }
    if let Some(rest) = s.strip_prefix("$HOME/") {
        return Some(home.join(rest));
    }
    let p = PathBuf::from(s);
    p.is_absolute().then_some(p)
}

fn executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// `name` on `$PATH`.
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| executable(p))
}

// ---------------------------------------------------------------------------------------------
// Proton / compat tool lookup

/// Folders that hold custom compat tools (`compatibilitytools.d` of every Steam root, umu's).
fn compat_tool_dirs(ctx: &ScanContext) -> Vec<PathBuf> {
    let flat = ctx.flatpak_app("com.valvesoftware.Steam");
    let mut v = vec![
        ctx.data_home().join("Steam/compatibilitytools.d"),
        ctx.home.join(".steam/root/compatibilitytools.d"),
        ctx.home.join(".steam/steam/compatibilitytools.d"),
        flat.join("data/Steam/compatibilitytools.d"),
        flat.join(".local/share/Steam/compatibilitytools.d"),
        ctx.data_home().join("umu/compatibilitytools"),
    ];
    for r in super::steam::find_roots(ctx) {
        v.push(r.path.join("compatibilitytools.d"));
    }
    v.push("/usr/share/steam/compatibilitytools.d".into());
    v.push("/usr/local/share/steam/compatibilitytools.d".into());
    let mut seen = vec![];
    v.retain(|d| {
        let c = d.canonicalize().unwrap_or_else(|_| d.clone());
        let new = d.is_dir() && !seen.contains(&c);
        seen.push(c);
        new
    });
    v
}

/// Internal and display names a custom compat tool declares in `compatibilitytool.vdf`.
fn tool_names(dir: &Path) -> Vec<String> {
    let Ok(text) = super::util::read_text(dir.join("compatibilitytool.vdf")) else {
        return vec![];
    };
    let Ok(v) = super::vdf::parse(&text) else {
        return vec![];
    };
    let Some(tools) = v.path(&["compatibilitytools", "compat_tools"]) else {
        return vec![];
    };
    let mut out = vec![];
    for (internal, t) in tools.entries() {
        out.push(internal.clone());
        if let Some(d) = t.get_str("display_name") {
            out.push(d.to_string());
        }
    }
    out
}

/// Steam's internal names of Valve's own Proton builds → their `steamapps/common` folder.
fn valve_proton_dir(name: &str) -> Option<String> {
    let rest = name.strip_prefix("proton_")?;
    Some(match rest {
        "experimental" => "Proton - Experimental".into(),
        "hotfix" => "Proton Hotfix".into(),
        "next" => "Proton Next".into(),
        r if !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()) => {
            if r.len() == 1 || (r.len() == 2 && r.starts_with('1')) {
                format!("Proton {r}.0")
            } else {
                let (maj, min) = r.split_at(1);
                format!("Proton {maj}.{min}")
            }
        }
        _ => return None,
    })
}

/// A Proton build by name: a `compatibilitytools.d` folder name, a tool's internal / display name,
/// or a Valve internal name (`proton_experimental`, `proton_9`) in a Steam library.
pub fn find_proton(ctx: &ScanContext, name: &str) -> Option<PathBuf> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    if let Some(p) = expand_home(name, &ctx.home).filter(|p| p.join("proton").is_file()) {
        return Some(p);
    }
    let dirs = compat_tool_dirs(ctx);
    for d in &dirs {
        let exact = d.join(name);
        if exact.join("proton").is_file() {
            return Some(exact);
        }
    }
    for d in &dirs {
        for t in subdirs(d) {
            if t.join("proton").is_file()
                && (file_name(&t).eq_ignore_ascii_case(name) || tool_names(&t).iter().any(|n| n == name))
            {
                return Some(t);
            }
        }
    }
    let common = valve_proton_dir(name).or_else(|| name.starts_with("Proton").then(|| name.to_string()))?;
    super::steam::find_roots(ctx)
        .iter()
        .flat_map(|r| super::steam::library_folders(&r.path))
        .map(|lib| lib.join("steamapps/common").join(&common))
        .find(|d| d.join("proton").is_file())
}

/// The newest Proton build whose folder name starts with `family` ("GE-Proton", "UMU-Proton").
fn newest_proton(ctx: &ScanContext, family: &str) -> Option<PathBuf> {
    let fam = family.to_ascii_lowercase();
    crate::addgame::proton_versions(ctx)
        .into_iter()
        .find(|p| file_name(&p.path).to_ascii_lowercase().starts_with(&fam))
        .map(|p| p.path)
}

/// Faugus' runner names → a Proton build.
fn faugus_runner(ctx: &ScanContext, name: &str) -> Runner {
    let path = match name.trim() {
        "" | "UMU-Proton Latest" => newest_proton(ctx, "UMU-Proton"),
        "Linux-Native" => None,
        n if n.starts_with("GE-Proton Latest") || n == "GE-Proton" => newest_proton(ctx, "GE-Proton"),
        n if n.starts_with("Proton-EM Latest") || n == "Proton-EM" => newest_proton(ctx, "Proton-EM"),
        n => find_proton(ctx, n),
    };
    path.map_or(Runner::Unknown, |path| Runner::Proton { path })
}

/// A Wine runner folder (`<dir>/bin/wine`, or a Proton build's `<dir>/files/bin/wine`).
fn wine_in(dir: &Path) -> Option<PathBuf> {
    [dir.join("bin/wine"), dir.join("files/bin/wine"), dir.join("dist/bin/wine")].into_iter().find(|p| p.is_file())
}

// ---------------------------------------------------------------------------------------------
// Prefix discovery

struct Found {
    list: Vec<Prefix>,
    index: HashMap<PathBuf, usize>,
}

impl Found {
    /// Add the prefix in `dir` (itself or `dir/pfx`). The first finder names it; a later one
    /// that knows the runner fills it in.
    fn add(&mut self, dir: &Path, origin: impl Into<String>, runner: Runner) {
        let Some(path) = prefix_in(dir) else { return };
        let canon = path.canonicalize().unwrap_or_else(|_| path.clone());
        if let Some(&i) = self.index.get(&canon) {
            if self.list[i].runner == Runner::Unknown {
                self.list[i].runner = runner;
            }
            return;
        }
        self.index.insert(canon, self.list.len());
        self.list.push(Prefix { path, origin: origin.into(), runner });
    }
}

/// Every Wine prefix the known launchers and folders point at, in discovery order, each once.
pub fn find_prefixes(ctx: &ScanContext) -> Vec<Prefix> {
    let mut f = Found { list: vec![], index: HashMap::new() };
    faugus(ctx, &mut f);
    lutris(ctx, &mut f);
    bottles(ctx, &mut f);
    heroic(ctx, &mut f);
    steam(ctx, &mut f);
    let h = &ctx.home;
    for d in subdirs(&h.join("Games/umu")) {
        f.add(&d, format!("umu: {}", file_name(&d)), Runner::Unknown);
    }
    for d in subdirs(&h.join("Games/Lodestar")) {
        f.add(&d, format!("Lodestar: {}", file_name(&d)), Runner::Unknown);
    }
    for d in subdirs(&h.join("Games")) {
        f.add(&d, tilde(&d, h), Runner::Unknown);
    }
    f.add(&h.join(".wine"), "~/.wine", Runner::Unknown);
    for extra in &ctx.config.extra_prefix_dirs {
        let extra = expand_home(&extra.to_string_lossy(), h).unwrap_or_else(|| extra.clone());
        if prefix_in(&extra).is_some() {
            f.add(&extra, tilde(&extra, h), Runner::Unknown);
        } else {
            for d in subdirs(&extra) {
                f.add(&d, tilde(&d, h), Runner::Unknown);
            }
        }
    }
    f.list
}

fn read_json(p: &Path) -> Option<Value> {
    serde_json::from_str(&super::util::read_text(p).ok()?).ok()
}

fn faugus(ctx: &ScanContext, f: &mut Found) {
    let configs = [
        ctx.config_home().join("faugus-launcher/config.json"),
        ctx.flatpak_app(super::faugus::FLATPAK_ID).join("config/faugus-launcher/config.json"),
    ];
    let mut runners: HashMap<String, Runner> = HashMap::new();
    let mut runner = |name: &str| runners.entry(name.to_string()).or_insert_with(|| faugus_runner(ctx, name)).clone();
    for dir in super::faugus::data_dirs(ctx) {
        let Some(v) = read_json(&dir.join("games.json")) else {
            continue;
        };
        for g in v.as_array().map(Vec::as_slice).unwrap_or_default() {
            let s = |k: &str| g.get(k).and_then(Value::as_str).unwrap_or("").trim();
            let Some(p) = expand_home(s("prefix"), &ctx.home) else {
                continue;
            };
            let title = Some(s("title")).filter(|t| !t.is_empty()).map_or_else(|| file_name(&p), str::to_string);
            f.add(&p, format!("Faugus: {title}"), runner(s("runner")));
        }
    }
    let mut bases = vec![];
    for c in &configs {
        let Some(v) = read_json(c) else { continue };
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
        if let Some(p) = expand_home(&s("default-prefix"), &ctx.home) {
            bases.push((p, runner(&s("default-runner"))));
        }
    }
    bases.push((ctx.home.join("Faugus"), Runner::Unknown));
    bases.push((ctx.flatpak_app(super::faugus::FLATPAK_ID).join("data/Faugus"), Runner::Unknown));
    for (base, runner) in bases {
        for d in subdirs(&base) {
            f.add(&d, format!("Faugus: {}", file_name(&d)), runner.clone());
        }
    }
}

/// `section.key` → value from a Lutris game YAML (two-level block mappings, which is all Lutris
/// writes there).
pub(crate) fn lutris_yaml(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut section = String::new();
    for line in text.lines() {
        let indent = line.len() - line.trim_start().len();
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || t.starts_with('-') {
            continue;
        }
        let Some((k, v)) = t.split_once(':') else {
            continue;
        };
        let v = v.trim().trim_matches(|c| c == '\'' || c == '"');
        if indent == 0 {
            section = k.trim().to_string();
            if !v.is_empty() {
                out.insert(section.clone(), v.to_string());
            }
        } else if !v.is_empty() {
            out.entry(format!("{section}.{}", k.trim())).or_insert_with(|| v.to_string());
        }
    }
    out
}

fn lutris(ctx: &ScanContext, f: &mut Found) {
    let fp = ctx.flatpak_app(super::lutris::FLATPAK_ID);
    let installs = [
        (ctx.config_home().join("lutris"), ctx.data_home().join("lutris")),
        (ctx.data_home().join("lutris"), ctx.data_home().join("lutris")),
        (fp.join("config/lutris"), fp.join("data/lutris")),
        (fp.join("data/lutris"), fp.join("data/lutris")),
    ];
    for (config, data) in installs {
        let Ok(rd) = std::fs::read_dir(config.join("games")) else {
            continue;
        };
        let default_version = super::util::read_text(config.join("runners/wine.yml"))
            .ok()
            .and_then(|t| lutris_yaml(&t).remove("wine.version"));
        let mut files: Vec<PathBuf> = rd.flatten().take(2000).map(|e| e.path()).collect();
        files.sort();
        for p in files {
            if p.extension().is_none_or(|e| e != "yml") {
                continue;
            }
            let Ok(text) = super::util::read_text(&p) else {
                continue;
            };
            let y = lutris_yaml(&text);
            let Some(prefix) = y.get("game.prefix").and_then(|s| expand_home(s, &ctx.home)) else {
                continue;
            };
            let version = y.get("wine.version").cloned().or_else(|| default_version.clone());
            let runner = match version.as_deref() {
                None => Runner::Unknown,
                Some(v) if v.eq_ignore_ascii_case("system") => {
                    which("wine").map_or(Runner::Unknown, |path| Runner::Wine { path })
                }
                Some(v) => match wine_in(&data.join("runners/wine").join(v)) {
                    Some(path) => Runner::Wine { path },
                    None => find_proton(ctx, v).map_or(Runner::Unknown, |path| Runner::Proton { path }),
                },
            };
            // `<slug>-<timestamp>.yml`
            let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let slug = match stem.rsplit_once('-') {
                Some((s, ts)) if !s.is_empty() && ts.bytes().all(|b| b.is_ascii_digit()) => s.to_string(),
                _ => stem,
            };
            f.add(&prefix, format!("Lutris: {slug}"), runner);
        }
    }
}

fn bottles(ctx: &ScanContext, f: &mut Found) {
    for data in [ctx.data_home().join("bottles"), ctx.flatpak_app(super::bottles::FLATPAK_ID).join("data/bottles")] {
        for b in subdirs(&data.join("bottles")) {
            let text = super::util::read_text(b.join("bottle.yml")).unwrap_or_default();
            let y = lutris_yaml(&text);
            let name = y.get("Name").cloned().unwrap_or_else(|| file_name(&b));
            let runner = match y.get("Runner") {
                Some(r) if r.starts_with("sys-") => which("wine").map_or(Runner::Unknown, |path| Runner::Wine { path }),
                Some(r) => match wine_in(&data.join("runners").join(r)) {
                    Some(path) => Runner::Wine { path },
                    None => find_proton(ctx, r).map_or(Runner::Unknown, |path| Runner::Proton { path }),
                },
                None => Runner::Unknown,
            };
            f.add(&b, format!("Bottles: {name}"), runner);
        }
    }
}

/// Heroic's `{winePrefix, wineVersion: {bin, type}}` → (prefix folder, runner).
fn heroic_entry(ctx: &ScanContext, v: &Value) -> Option<(PathBuf, Runner)> {
    let prefix = expand_home(v.get("winePrefix")?.as_str()?, &ctx.home)?;
    let wv = v.get("wineVersion");
    let bin = wv.and_then(|w| w.get("bin")).and_then(Value::as_str).and_then(|b| expand_home(b, &ctx.home));
    let ty = wv.and_then(|w| w.get("type")).and_then(Value::as_str).unwrap_or("");
    let runner = match (bin, ty) {
        (Some(b), "proton") => b.parent().map_or(Runner::Unknown, |p| Runner::Proton { path: p.to_path_buf() }),
        (Some(b), "wine") => Runner::Wine { path: b },
        _ => Runner::Unknown,
    };
    Some((prefix, runner))
}

fn heroic(ctx: &ScanContext, f: &mut Found) {
    for root in [ctx.config_home().join("heroic"), ctx.flatpak_app(super::heroic::FLATPAK_ID).join("config/heroic")] {
        let Ok(rd) = std::fs::read_dir(root.join("GamesConfig")) else {
            continue;
        };
        let mut files: Vec<PathBuf> = rd.flatten().take(2000).map(|e| e.path()).collect();
        files.sort();
        for p in files {
            if p.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Some(v) = read_json(&p) else { continue };
            for (_, game) in v.as_object().into_iter().flatten() {
                if let Some((prefix, runner)) = heroic_entry(ctx, game) {
                    f.add(&prefix, format!("Heroic: {}", file_name(&prefix)), runner);
                }
            }
        }
        if let Some(v) = read_json(&root.join("config.json"))
            && let Some((prefix, runner)) = v.get("defaultSettings").and_then(|d| heroic_entry(ctx, d))
        {
            f.add(&prefix, format!("Heroic: {}", file_name(&prefix)), runner);
        }
    }
    let base = ctx.home.join("Games/Heroic/Prefixes");
    for d in subdirs(&base.join("default")).into_iter().chain(subdirs(&base)) {
        f.add(&d, format!("Heroic: {}", file_name(&d)), Runner::Unknown);
    }
}

fn steam(ctx: &ScanContext, f: &mut Found) {
    for root in super::steam::find_roots(ctx) {
        let compat = super::steam::compat_tools(&root.path);
        let mut names: HashMap<u32, String> = HashMap::new();
        for user in subdirs(&root.path.join("userdata")).into_iter().take(16) {
            let Ok(bytes) = super::util::read_bytes(user.join("config/shortcuts.vdf")) else {
                continue;
            };
            for s in super::steam::parse_shortcuts(&bytes).unwrap_or_default() {
                names.entry(s.appid).or_insert(s.name);
            }
        }
        for lib in super::steam::library_folders(&root.path) {
            for d in subdirs(&lib.join("steamapps/compatdata")) {
                let Ok(appid) = file_name(&d).parse::<u64>() else {
                    continue;
                };
                if !(1 << 31..1 << 32).contains(&appid) {
                    continue;
                }
                let appid = appid as u32;
                let tool = compat.get(&appid).or(compat.get(&0)).map_or("proton_experimental", String::as_str);
                let runner = find_proton(ctx, tool).map_or(Runner::Unknown, |path| Runner::Proton { path });
                let origin = match names.get(&appid) {
                    Some(n) => format!("Steam shortcut: {n}"),
                    None => format!("Steam shortcut {appid}"),
                };
                f.add(&d, origin, runner);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Runners and launching

/// The prefix was made by Proton (it writes `version` / `config_info` next to `pfx`).
fn proton_made(prefix: &Path) -> bool {
    let root = compat_root(prefix);
    root.join("version").is_file() || root.join("config_info").is_file()
}

/// Fill in `Runner::Unknown`: the Proton build that made the prefix (by its `config_info` /
/// `version`), else `wine` on `$PATH` for a plain Wine prefix, else the newest Proton. Lists
/// Proton builds at most once for all prefixes.
pub fn resolve_runners(ctx: &ScanContext, prefixes: &mut [Prefix]) {
    let mut protons: Option<Vec<crate::addgame::ProtonVersion>> = None;
    let wine = which("wine");
    for p in prefixes.iter_mut().filter(|p| p.runner == Runner::Unknown) {
        let list = protons.get_or_insert_with(|| crate::addgame::proton_versions(ctx));
        let proton = || crate::addgame::proton_for_prefix(&compat_root(&p.path), list).map(|v| v.path.clone());
        p.runner = if proton_made(&p.path) { proton().map(|path| Runner::Proton { path }) } else { None }
            .or_else(|| wine.clone().map(|path| Runner::Wine { path }))
            .or_else(|| proton().map(|path| Runner::Proton { path }))
            .unwrap_or(Runner::Unknown);
    }
}

/// The umu-run to use: on `$PATH` (bare name), else the one Faugus bundles.
pub fn find_umu(home: &Path) -> Option<String> {
    if which("umu-run").is_some() {
        return Some("umu-run".into());
    }
    [
        home.join(".local/share/faugus-launcher/umu-run"),
        home.join(".var/app/io.github.Faugus.faugus-launcher/data/faugus-launcher/umu-run"),
    ]
    .into_iter()
    .find(|p| executable(p))
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

/// The command that runs `exe` (a Unix path to a Windows program; `.msi` via `msiexec /i`) with
/// `args` inside `prefix`, working folder = the program's folder:
/// * Proton: `umu-run exe args` with `WINEPREFIX=<compat root>`, `GAMEID=umu-default`,
///   `PROTONPATH=<build>` when umu-run exists (on `$PATH` or Faugus' bundled one), else
///   `<build>/proton run exe args` with `STEAM_COMPAT_DATA_PATH` / `STEAM_COMPAT_CLIENT_INSTALL_PATH`.
/// * Wine: `<wine> exe args` with `WINEPREFIX`.
/// * Unknown: `wine` from `$PATH`, else umu-run with its default Proton, else plain `wine`.
///
/// Builds a spec only — nothing is started here.
pub fn run_in_prefix(prefix: &Prefix, exe: &Path, args: &[String]) -> LaunchSpec {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    run_in_prefix_with(prefix, exe, args, find_umu(&home), which("wine"), &home)
}

/// [`run_in_prefix`] for the home of a scan context (umu-run / Steam root looked up there, so a
/// fixture home gives the same command on every machine).
pub fn run_in_prefix_ctx(ctx: &ScanContext, prefix: &Prefix, exe: &Path, args: &[String]) -> LaunchSpec {
    run_in_prefix_with(prefix, exe, args, find_umu(&ctx.home), which("wine"), &ctx.home)
}

pub(crate) fn run_in_prefix_with(
    prefix: &Prefix,
    exe: &Path,
    args: &[String],
    umu: Option<String>,
    path_wine: Option<PathBuf>,
    home: &Path,
) -> LaunchSpec {
    let exe_s = exe.to_string_lossy().into_owned();
    let mut target: Vec<String> = if exe.extension().is_some_and(|e| e.eq_ignore_ascii_case("msi")) {
        vec!["msiexec".into(), "/i".into(), exe_s]
    } else {
        vec![exe_s]
    };
    target.extend(args.iter().cloned());
    let cwd = exe.parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf);
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let wine_cmd = |wine: String| LaunchSpec::Command {
        program: wine,
        args: target.clone(),
        cwd: cwd.clone(),
        env: vec![("WINEPREFIX".into(), s(&prefix.path))],
    };
    match &prefix.runner {
        Runner::Wine { path } => wine_cmd(s(path)),
        Runner::Proton { path } => {
            let root = s(&compat_root(&prefix.path));
            let mut env = vec![
                ("WINEPREFIX".to_string(), root.clone()),
                ("GAMEID".to_string(), "umu-default".to_string()),
                ("PROTONPATH".to_string(), s(path)),
            ];
            match umu {
                Some(program) => LaunchSpec::Command { program, args: target, cwd, env },
                None => {
                    env.push(("STEAM_COMPAT_DATA_PATH".into(), root));
                    env.push(("STEAM_COMPAT_CLIENT_INSTALL_PATH".into(), s(&steam_root_of(path, home))));
                    let mut a = vec!["run".to_string()];
                    a.extend(target);
                    LaunchSpec::Command { program: s(&path.join("proton")), args: a, cwd, env }
                }
            }
        }
        Runner::Unknown => match (path_wine, umu) {
            (Some(w), _) => wine_cmd(s(&w)),
            (None, Some(program)) => LaunchSpec::Command {
                program,
                args: target.clone(),
                cwd: cwd.clone(),
                env: vec![
                    ("WINEPREFIX".into(), s(&compat_root(&prefix.path))),
                    ("GAMEID".into(), "umu-default".into()),
                ],
            },
            (None, None) => wine_cmd("wine".into()),
        },
    }
}

// ---------------------------------------------------------------------------------------------
// Paths and registry

/// A Windows path in `prefix` (`C:\Program Files\X\` or `C:/Program Files/X`) as a Unix path,
/// matched case-insensitively against what exists (components that don't exist are kept as
/// written). Other drives follow `dosdevices/<d>:`.
pub fn win_to_unix(prefix: &Path, win: &str) -> Option<PathBuf> {
    let w = win.trim().trim_matches('"');
    let (drive, rest) = w.split_once(':')?;
    if drive.len() != 1 || !drive.as_bytes()[0].is_ascii_alphabetic() {
        return None;
    }
    let d = drive.to_ascii_lowercase();
    let base = if d == "c" {
        prefix.join("drive_c")
    } else {
        let link = prefix.join("dosdevices").join(format!("{d}:"));
        let t = std::fs::read_link(&link).ok()?;
        if t.is_absolute() { t } else { prefix.join("dosdevices").join(t) }
    };
    let mut cur = base;
    let mut exists = true;
    for comp in rest.split(['\\', '/']).filter(|c| !c.is_empty() && *c != ".") {
        let exact = cur.join(comp);
        if !exists || exact.exists() {
            cur = exact;
            continue;
        }
        let found = std::fs::read_dir(&cur).ok().and_then(|rd| {
            rd.flatten()
                .take(4096)
                .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(comp))
                .map(|e| e.path())
        });
        exists = found.is_some();
        cur = found.unwrap_or(exact);
    }
    Some(cur)
}

/// A Unix path as the Windows path Wine shows for it: `C:\…` inside `drive_c`, else `Z:\…`.
pub fn unix_to_win(prefix: &Path, p: &Path) -> String {
    let (drive, rest) = match p.strip_prefix(prefix.join("drive_c")) {
        Ok(rest) => ("C:", rest),
        Err(_) => ("Z:", p.strip_prefix("/").unwrap_or(p)),
    };
    let rest = rest.to_string_lossy().replace('/', "\\");
    format!("{drive}\\{rest}")
}

/// Values of `key` (`Software\X\Y`, `HKLM\…` / `HKCU\…` prefixes allowed, case-insensitive) in
/// the prefix's `system.reg` / `user.reg`, as text (DWORDs in decimal). Empty when missing.
#[cfg(test)]
pub fn registry_values(prefix: &Path, hive: Hive, key: &str) -> Vec<(String, String)> {
    let want = registry::norm_key(key);
    let reg = Registry::load(prefix, hive, |k| k == want);
    reg.key(key).map(|k| k.values.iter().map(|(n, v)| (n.clone(), v.as_text())).collect()).unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod tests;
