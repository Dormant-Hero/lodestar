//! Steam scanner. See CONTRACT in scan/mod.rs.
//!
//! Per Steam root (native, `~/.steam/*`, Flatpak, Snap, `extra_steam_roots`; deduped by canonical path):
//!   * libraries from `steamapps/libraryfolders.vdf` (+ `config/libraryfolders.vdf`); missing ones skipped,
//!   * `appmanifest_*.acf` -> installed apps,
//!   * `appcache/appinfo.vdf` (only the needed entries) -> type filter (drops Proton, runtimes,
//!     redistributables …) + developer/publisher/year/genres/oslist,
//!   * active user's `localconfig.vdf` -> playtime, last played, Steam launch options,
//!   * active user's `shortcuts.vdf` -> non-Steam shortcuts (source_id `shortcut:<appid>`),
//!   * art from `userdata/<id>/config/grid` (custom, wins) and `appcache/librarycache`.
//!
//! Public helpers for other modules: [`find_roots`], [`direct_launch_for_source_id`] (builds a command
//! that runs a Steam game without going through Steam's own launch options).

use super::ScanContext;
use super::util::{file_if_exists, image_with_stem, read_bytes, read_text};
use super::vdf::{self, Vdf};
use crate::model::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub mod appinfo;
pub mod direct;
pub use direct::direct_launch_for_source_id;

/// Steam64 id of account id 0.
const STEAMID64_BASE: u64 = 76_561_197_960_265_728;

/// Known non-game appids (Proton, runtimes, redistributables, SteamVR …). Used in addition to
/// the appinfo type filter, and as the only filter when appinfo.vdf is unreadable.
pub const TOOL_APPIDS: &[u32] = &[
    7, 753, 760,       // Steam client / screenshots
    228_980,   // Steamworks Common Redistributables
    1_070_560, // Steam Linux Runtime 1.0 (scout)
    1_391_110, // Steam Linux Runtime 2.0 (soldier)
    1_628_350, // Steam Linux Runtime 3.0 (sniper)
    4_183_110, // Steam Linux Runtime 4.0
    1_493_710, // Proton Experimental
    2_180_100, // Proton Hotfix
    1_826_330, // Proton EasyAntiCheat Runtime
    1_161_040, // Proton BattlEye Runtime
    3_658_110, // Proton 10.0
    2_805_730, // Proton 9.0
    2_348_590, // Proton 8.0
    2_230_260, // Proton Next
    1_887_720, // Proton 7.0
    1_580_130, // Proton 6.3
    1_420_170, // Proton 5.13
    1_245_040, // Proton 5.0
    1_113_280, // Proton 4.11
    1_054_830, // Proton 4.2
    996_510,   // Proton 3.16 Beta
    961_940,   // Proton 3.16
    930_400,   // Proton 3.7 Beta
    858_280,   // Proton 3.7
    250_820,   // SteamVR
    1_009_850, // OpenXR Developer Tools for Windows Mixed Reality
    241_100,   // Steam Controller Configs
    1_007,     // Steamworks SDK Redist (old)
];

/// Name prefixes of non-game installs, used when appinfo has no type for the app.
pub const TOOL_NAME_PREFIXES: &[&str] = &[
    "Proton ",
    "Proton-",
    "Steam Linux Runtime",
    "Steamworks Common",
    "Steamworks Shared",
    "SteamVR",
    "Steam Controller Configs",
];

/// How a Steam root was found (decides how the client is started).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallKind {
    Native,
    Flatpak,
    Snap,
    /// From `ScanConfig::extra_steam_roots`.
    Custom,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SteamRoot {
    /// Canonicalised root (contains `steamapps/`, `config/`, `userdata/`).
    pub path: PathBuf,
    pub kind: InstallKind,
}

/// All Steam roots that exist, deduplicated by canonical path, most preferred first.
pub fn find_roots(ctx: &ScanContext) -> Vec<SteamRoot> {
    let h = &ctx.home;
    let flatpak = ctx.flatpak_app("com.valvesoftware.Steam");
    let mut cands: Vec<(PathBuf, InstallKind)> = vec![
        (h.join(".local/share/Steam"), InstallKind::Native),
        (h.join(".steam/steam"), InstallKind::Native),
        (h.join(".steam/root"), InstallKind::Native),
        (h.join(".steam/debian-installation"), InstallKind::Native),
        (flatpak.join(".local/share/Steam"), InstallKind::Flatpak),
        (flatpak.join("data/Steam"), InstallKind::Flatpak),
        (h.join("snap/steam/common/.local/share/Steam"), InstallKind::Snap),
    ];
    cands.extend(ctx.config.extra_steam_roots.iter().map(|p| (p.clone(), InstallKind::Custom)));
    let mut out: Vec<SteamRoot> = Vec::new();
    for (p, kind) in cands {
        let Ok(canon) = p.canonicalize() else {
            continue;
        };
        if !(canon.join("steamapps").is_dir() || canon.join("config").is_dir()) {
            continue;
        }
        if out.iter().any(|r| r.path == canon) {
            continue;
        }
        out.push(SteamRoot { path: canon, kind });
    }
    out
}

pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>> {
    let roots = find_roots(ctx);
    let mut out: Vec<ScannedGame> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut first_err: Option<anyhow::Error> = None;
    for root in &roots {
        match scan_root(ctx, &root.path) {
            Ok(games) => {
                for g in games {
                    if seen.insert(g.source_id.clone()) {
                        out.push(g);
                    }
                }
            }
            Err(e) => {
                log::warn!("steam root {}: {e:#}", root.path.display());
                first_err.get_or_insert(e);
            }
        }
    }
    match first_err {
        // Data exists but nothing could be read: let the DB keep its previous state.
        Some(e) if out.is_empty() => Err(e),
        _ => Ok(out),
    }
}

// ---------------------------------------------------------------------------------------------
// Libraries and manifests
// ---------------------------------------------------------------------------------------------

/// Library folders of a root (the root itself first), canonicalised; missing ones skipped.
pub fn library_folders(root: &Path) -> Vec<PathBuf> {
    let mut libs: Vec<PathBuf> = vec![root.to_path_buf()];
    for f in [root.join("steamapps/libraryfolders.vdf"), root.join("config/libraryfolders.vdf")] {
        let Ok(text) = read_text(&f) else {
            continue;
        };
        let v = match vdf::parse(&text) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("{}: {e:#}", f.display());
                continue;
            }
        };
        let Some(lf) = v.get("libraryfolders") else {
            continue;
        };
        for (key, val) in lf.entries() {
            if !key.bytes().all(|b| b.is_ascii_digit()) {
                continue; // "contentstatsid", "TimeNextStatsReport" …
            }
            // New format: "0" { "path" "…" }; old format: "1" "/path".
            let path = match val {
                Vdf::Str(s) => Some(s.as_str()),
                Vdf::Obj(_) => val.get_str("path"),
            };
            if let Some(p) = path.filter(|p| !p.is_empty()) {
                libs.push(PathBuf::from(p));
            }
        }
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for l in libs {
        // Missing / unmounted libraries are skipped quietly.
        let Ok(c) = l.canonicalize() else { continue };
        if c.join("steamapps").is_dir() && !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub appid: u32,
    pub name: String,
    pub installdir: String,
    pub state_flags: u64,
    pub size_on_disk: Option<u64>,
    pub last_played: Option<i64>,
    pub library: PathBuf,
}

impl Manifest {
    pub fn installed(&self) -> bool {
        self.state_flags & 4 != 0
    }
    pub fn install_path(&self) -> Option<PathBuf> {
        (!self.installdir.is_empty()).then(|| self.library.join("steamapps/common").join(&self.installdir))
    }
}

/// Parse one `appmanifest_*.acf`.
pub fn parse_manifest(text: &str, library: &Path) -> anyhow::Result<Manifest> {
    let v = vdf::parse(text)?;
    let st = v.get("AppState").ok_or_else(|| anyhow::anyhow!("no AppState"))?;
    let appid = st
        .get("appid")
        .and_then(Vdf::uint)
        .and_then(|a| u32::try_from(a).ok())
        .ok_or_else(|| anyhow::anyhow!("no appid"))?;
    Ok(Manifest {
        appid,
        name: st.get_str("name").unwrap_or("").trim().to_string(),
        installdir: st.get_str("installdir").unwrap_or("").to_string(),
        state_flags: st.get("StateFlags").and_then(Vdf::uint).unwrap_or(0),
        size_on_disk: st.get("SizeOnDisk").and_then(Vdf::uint).filter(|&s| s > 0),
        last_played: st.get("LastPlayed").and_then(Vdf::int).filter(|&t| t > 0),
        library: library.to_path_buf(),
    })
}

/// All manifests in one library. `Err` only when `steamapps/` exists but cannot be listed.
fn library_manifests(lib: &Path) -> anyhow::Result<Vec<Manifest>> {
    let dir = lib.join("steamapps");
    let rd = std::fs::read_dir(&dir).map_err(|e| anyhow::anyhow!("{}: {e}", dir.display()))?;
    let mut out = Vec::new();
    for ent in rd.flatten() {
        let name = ent.file_name();
        let Some(name) = name.to_str() else { continue };
        if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
            continue;
        }
        let p = ent.path();
        let Ok(text) = read_text(&p) else {
            continue;
        };
        match parse_manifest(&text, lib) {
            Ok(m) => out.push(m),
            Err(e) => log::debug!("{}: {e:#}", p.display()),
        }
    }
    Ok(out)
}

/// Manifests across all libraries of a root; one entry per appid (installed copy preferred).
pub fn root_manifests(root: &Path) -> anyhow::Result<Vec<Manifest>> {
    let libs = library_folders(root);
    let mut by_id: HashMap<u32, Manifest> = HashMap::new();
    let mut order: Vec<u32> = Vec::new();
    for lib in &libs {
        let ms = match library_manifests(lib) {
            Ok(ms) => ms,
            Err(e) if lib == root => return Err(e),
            Err(e) => {
                log::warn!("steam library: {e:#}");
                continue;
            }
        };
        for m in ms {
            match by_id.get(&m.appid) {
                Some(old) if old.installed() || !m.installed() => {}
                Some(_) => {
                    by_id.insert(m.appid, m);
                }
                None => {
                    order.push(m.appid);
                    by_id.insert(m.appid, m);
                }
            }
        }
    }
    Ok(order.into_iter().filter_map(|id| by_id.remove(&id)).collect())
}

// ---------------------------------------------------------------------------------------------
// Users
// ---------------------------------------------------------------------------------------------

/// `userdata/<accountid>` of the most recent user of this root.
pub fn active_user(root: &Path) -> Option<PathBuf> {
    let userdata = root.join("userdata");
    if let Ok(text) = read_text(root.join("config/loginusers.vdf"))
        && let Ok(v) = vdf::parse(&text)
        && let Some(users) = v.get("users")
    {
        let mut best: Option<(bool, i64, u64)> = None;
        for (id, u) in users.entries() {
            let Ok(sid) = id.trim().parse::<u64>() else {
                continue;
            };
            let recent = u.get("MostRecent").and_then(Vdf::int) == Some(1);
            let ts = u.get("Timestamp").and_then(Vdf::int).unwrap_or(0);
            if best.is_none_or(|(r, t, _)| (recent, ts) > (r, t)) {
                best = Some((recent, ts, sid));
            }
        }
        if let Some((_, _, sid)) = best {
            let account = sid.checked_sub(STEAMID64_BASE).unwrap_or(sid & 0xFFFF_FFFF);
            let dir = userdata.join(account.to_string());
            if dir.is_dir() {
                return Some(dir);
            }
        }
    }
    // Fallback: the user dir whose localconfig.vdf changed last.
    std::fs::read_dir(&userdata)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| n != "0" && n.bytes().all(|b| b.is_ascii_digit())))
        .filter_map(|e| {
            let p = e.path();
            let t = super::util::mtime(&p.join("config/localconfig.vdf"))?;
            Some((t, p))
        })
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocalApp {
    pub playtime_mins: Option<u64>,
    pub last_played: Option<i64>,
    pub launch_options: Option<String>,
}

/// `UserLocalConfigStore/Software/Valve/Steam/apps` of a localconfig.vdf.
pub fn parse_localconfig(text: &str) -> anyhow::Result<HashMap<u32, LocalApp>> {
    let v = vdf::parse(text)?;
    let mut out = HashMap::new();
    let store = v.get("UserLocalConfigStore").unwrap_or(&v);
    let Some(apps) = store.path(&["Software", "Valve", "Steam", "apps"]) else {
        return Ok(out);
    };
    for (k, a) in apps.entries() {
        let Ok(id) = k.parse::<u32>() else { continue };
        if !matches!(a, Vdf::Obj(_)) {
            continue;
        }
        let la = LocalApp {
            playtime_mins: a.get("Playtime").and_then(Vdf::uint),
            last_played: a.get("LastPlayed").and_then(Vdf::int).filter(|&t| t > 0),
            launch_options: a.get_str("LaunchOptions").map(str::trim).filter(|s| !s.is_empty()).map(str::to_string),
        };
        out.insert(id, la);
    }
    Ok(out)
}

/// Compat tool chosen per app in `config/config.vdf` (`CompatToolMapping`); key 0 = global default.
pub fn compat_tools(root: &Path) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    let Ok(text) = read_text(root.join("config/config.vdf")) else {
        return out;
    };
    let Ok(v) = vdf::parse(&text) else { return out };
    let store = v.get("InstallConfigStore").unwrap_or(&v);
    let Some(map) = store.path(&["Software", "Valve", "Steam", "CompatToolMapping"]) else {
        return out;
    };
    for (k, e) in map.entries() {
        if let (Ok(id), Some(name)) = (k.parse::<u32>(), e.get_str("name"))
            && !name.trim().is_empty()
        {
            out.insert(id, name.trim().to_string());
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Scan one root
// ---------------------------------------------------------------------------------------------

fn is_tool_name(name: &str) -> bool {
    TOOL_NAME_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Keep/drop decision for an app. `has_cover` is only consulted for applications.
fn keep_app(appid: u32, name: &str, info: Option<&appinfo::AppInfo>, has_cover: bool) -> bool {
    if TOOL_APPIDS.contains(&appid) {
        return false;
    }
    match info.and_then(|i| i.kind.as_deref()) {
        Some("game" | "demo" | "mod" | "beta") => true,
        // Software (Wallpaper Engine, Blender …): only when Steam shows it with box art.
        Some("application") => has_cover && !is_tool_name(name),
        Some(
            "tool" | "config" | "dlc" | "music" | "video" | "series" | "episode" | "hardware" | "advertising"
            | "driver" | "plugin" | "comic" | "media",
        ) => false,
        _ => !is_tool_name(name),
    }
}

fn platform_for(appid: u32, info: Option<&appinfo::AppInfo>, compat: &HashMap<u32, String>) -> Option<String> {
    if compat.contains_key(&appid) {
        return Some("windows".into());
    }
    let os = &info?.oslist;
    if os.iter().any(|o| o == "linux") {
        Some("linux".into())
    } else if os.is_empty() || os.iter().any(|o| o == "windows") {
        // Steam treats a missing oslist as Windows-only (old titles such as Fallout: New Vegas).
        Some("windows".into())
    } else {
        None
    }
}

fn apply_info(g: &mut ScannedGame, info: Option<&appinfo::AppInfo>) {
    let Some(i) = info else { return };
    g.meta.developer.clone_from(&i.developer);
    g.meta.publisher.clone_from(&i.publisher);
    g.meta.release_year = i.release_year;
    g.meta.controller = i.controller;
    g.meta.deck = i.deck;
    g.meta.players.clone_from(&i.players);
    g.meta.genres.clone_from(&i.genres);
    if g.name.trim().is_empty()
        && let Some(n) = &i.name
    {
        g.name.clone_from(n);
    }
}

fn apply_local(g: &mut ScannedGame, local: Option<&LocalApp>) {
    let Some(l) = local else { return };
    if let Some(m) = l.playtime_mins {
        g.playtime_secs = Some(m * 60);
    }
    g.last_played = g.last_played.max(l.last_played);
    g.source_launch_options.clone_from(&l.launch_options);
}

fn scan_root(ctx: &ScanContext, root: &Path) -> anyhow::Result<Vec<ScannedGame>> {
    let manifests = root_manifests(root)?;
    let user = active_user(root);
    let local: HashMap<u32, LocalApp> = user
        .as_ref()
        .and_then(|u| read_text(u.join("config/localconfig.vdf")).ok())
        .map(|t| {
            parse_localconfig(&t).unwrap_or_else(|e| {
                log::warn!("steam localconfig.vdf: {e:#}");
                HashMap::new()
            })
        })
        .unwrap_or_default();
    let grid = user.as_ref().map(|u| u.join("config/grid")).filter(|g| g.is_dir());
    let compat = compat_tools(root);

    let installed_ids: HashSet<u32> = manifests.iter().map(|m| m.appid).collect();
    let mut wanted: HashSet<u32> = installed_ids.clone();
    if ctx.config.include_uninstalled {
        wanted.extend(local.keys().copied());
    }
    wanted.retain(|id| !TOOL_APPIDS.contains(id));
    let info = appinfo::read(&root.join("appcache/appinfo.vdf"), &wanted).unwrap_or_else(|e| {
        log::warn!("steam appinfo.vdf unreadable, using fallback filters: {e:#}");
        HashMap::new()
    });
    let art_ctx =
        ArtCtx { cache: root.join("appcache/librarycache"), grid: grid.clone(), icon_dirs: hicolor_app_dirs(ctx) };

    let mut out = Vec::new();
    for m in &manifests {
        let i = info.get(&m.appid);
        let name = if m.name.is_empty() { i.and_then(|i| i.name.clone()).unwrap_or_default() } else { m.name.clone() };
        if name.is_empty() {
            continue;
        }
        if TOOL_APPIDS.contains(&m.appid) {
            continue;
        }
        let art = art_ctx.app_art(m.appid);
        if !keep_app(m.appid, &name, i, art.cover.is_some()) {
            continue;
        }
        let mut g = ScannedGame::new(Source::Steam, m.appid.to_string(), name);
        g.installed = m.installed();
        g.install_dir = m.install_path().filter(|p| p.is_dir());
        g.launch = Some(LaunchSpec::Steam { appid: m.appid });
        g.art = art;
        g.size_bytes = m.size_on_disk;
        g.last_played = m.last_played;
        g.platform = platform_for(m.appid, i, &compat);
        apply_info(&mut g, i);
        apply_local(&mut g, local.get(&m.appid));
        out.push(g);
    }

    if ctx.config.include_uninstalled {
        let mut ids: Vec<u32> = local.keys().copied().filter(|id| !installed_ids.contains(id)).collect();
        ids.sort_unstable();
        for id in ids {
            let Some(i) = info.get(&id) else { continue };
            if i.kind.as_deref() != Some("game") || TOOL_APPIDS.contains(&id) {
                continue;
            }
            let Some(name) = i.name.clone() else { continue };
            let mut g = ScannedGame::new(Source::Steam, id.to_string(), name);
            g.installed = false;
            g.launch = Some(LaunchSpec::Steam { appid: id });
            g.art = art_ctx.app_art(id);
            g.platform = platform_for(id, Some(i), &compat);
            apply_info(&mut g, Some(i));
            apply_local(&mut g, local.get(&id));
            out.push(g);
        }
    }

    if let Some(u) = &user {
        let sc = u.join("config/shortcuts.vdf");
        match read_bytes(&sc) {
            Ok(bytes) => match parse_shortcuts(&bytes) {
                Ok(list) => out.extend(list.into_iter().map(|s| s.into_game(&art_ctx))),
                Err(e) => log::warn!("{}: {e:#}", sc.display()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("{}: {e}", sc.display()),
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Non-Steam shortcuts
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shortcut {
    pub appid: u32,
    pub name: String,
    pub exe: String,
    pub start_dir: String,
    pub launch_options: String,
    pub icon: String,
    pub hidden: bool,
    pub last_played: Option<i64>,
    pub tags: Vec<String>,
}

impl Shortcut {
    /// `steam://rungameid/<this>` launches the shortcut.
    pub fn game_id64(&self) -> u64 {
        ((self.appid as u64) << 32) | 0x0200_0000
    }

    fn into_game(self, art: &ArtCtx) -> ScannedGame {
        let mut g = ScannedGame::new(Source::Steam, format!("shortcut:{}", self.appid), self.name.clone());
        g.launch = Some(LaunchSpec::Url(format!("steam://rungameid/{}", self.game_id64())));
        g.art = art.grid_art(self.appid);
        if g.art.icon.is_none() && !self.icon.is_empty() {
            g.art.icon = file_if_exists(unquote(&self.icon));
        }
        let exe = unquote(&self.exe);
        let start = unquote(&self.start_dir);
        if !start.is_empty() && Path::new(start).is_absolute() && Path::new(start).is_dir() {
            g.install_dir = Some(PathBuf::from(start));
        }
        g.platform = Some(if exe.to_ascii_lowercase().ends_with(".exe") { "windows" } else { "linux" }.into());
        g.last_played = self.last_played;
        let opts = self.launch_options.trim();
        if !opts.is_empty() {
            g.source_launch_options = Some(opts.to_string());
        }
        // A Faugus game added to Steam: let the orchestrator drop this duplicate.
        if Path::new(exe).file_name().and_then(|f| f.to_str()) == Some("faugus-launcher")
            && let Some(id) = arg_after(opts, "--game")
        {
            g.duplicate_of = Some((Source::Faugus, id.to_string()));
        }
        g
    }
}

fn unquote(s: &str) -> &str {
    let s = s.trim();
    s.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(s)
}

fn arg_after<'a>(opts: &'a str, flag: &str) -> Option<&'a str> {
    let mut it = opts.split_whitespace();
    while let Some(w) = it.next() {
        if w == flag {
            return it.next().map(|v| v.trim_matches('"')).filter(|v| !v.is_empty());
        }
        if let Some(v) = w.strip_prefix(flag).and_then(|r| r.strip_prefix('=')) {
            return Some(v.trim_matches('"')).filter(|v| !v.is_empty());
        }
    }
    None
}

/// CRC-32 (IEEE), used to derive legacy shortcut appids.
fn crc32(data: &[u8]) -> u32 {
    let mut c: u32 = 0xFFFF_FFFF;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

/// Appid Steam assigns to a shortcut without a stored one: `crc32(exe + name) | 0x80000000`.
pub fn legacy_shortcut_appid(exe: &str, name: &str) -> u32 {
    crc32(format!("{exe}{name}").as_bytes()) | 0x8000_0000
}

/// Parse `shortcuts.vdf` (binary KeyValues).
pub fn parse_shortcuts(bytes: &[u8]) -> anyhow::Result<Vec<Shortcut>> {
    let v = vdf::parse_binary(bytes)?;
    let Some(list) = v.get("shortcuts") else {
        return Ok(vec![]);
    };
    let mut out = Vec::new();
    for (_, e) in list.entries() {
        let s = |k: &str| e.get_str(k).unwrap_or("").to_string();
        let name = s("AppName").trim().to_string();
        if name.is_empty() {
            continue;
        }
        let exe = s("Exe");
        let appid = match e.get("appid").and_then(Vdf::int) {
            // Stored as int32; negative values are the high-bit u32 ids.
            Some(a) if a != 0 => a as i32 as u32,
            _ => legacy_shortcut_appid(&exe, &name),
        };
        out.push(Shortcut {
            appid,
            name,
            exe,
            start_dir: s("StartDir"),
            launch_options: s("LaunchOptions"),
            icon: s("icon"),
            hidden: e.get("IsHidden").and_then(Vdf::int).is_some_and(|h| h != 0),
            last_played: e.get("LastPlayTime").and_then(Vdf::int).filter(|&t| t > 0),
            tags: e
                .get("tags")
                .map(|t| t.entries().iter().filter_map(|(_, v)| v.str().map(str::to_string)).collect())
                .unwrap_or_default(),
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Art
// ---------------------------------------------------------------------------------------------

struct ArtCtx {
    /// `appcache/librarycache`
    cache: PathBuf,
    /// `userdata/<id>/config/grid` (custom art)
    grid: Option<PathBuf>,
    /// `~/.local/share/icons/hicolor/<size>/apps` dirs (largest first) where Steam installs
    /// `steam_icon_<appid>.png` for its desktop shortcuts.
    icon_dirs: Vec<PathBuf>,
}

fn hicolor_app_dirs(ctx: &ScanContext) -> Vec<PathBuf> {
    let base = ctx.data_home().join("icons/hicolor");
    ["256x256", "192x192", "128x128", "96x96", "64x64", "48x48"]
        .iter()
        .map(|s| base.join(s).join("apps"))
        .filter(|d| d.is_dir())
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Cover,
    Hero,
    Logo,
    Banner,
}

/// Classify a librarycache file name: (slot, priority; lower is better).
fn classify(name: &str) -> Option<(Slot, u8)> {
    let (stem, ext) = name.rsplit_once('.')?;
    if !matches!(ext.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png" | "webp") {
        return None;
    }
    Some(match stem {
        "library_600x900_2x" => (Slot::Cover, 0),
        "library_600x900" => (Slot::Cover, 1),
        "library_capsule_2x" => (Slot::Cover, 2),
        "library_capsule" => (Slot::Cover, 3),
        "library_hero_2x" => (Slot::Hero, 0),
        "library_hero" => (Slot::Hero, 1),
        "logo_2x" => (Slot::Logo, 0),
        "logo" => (Slot::Logo, 1),
        "header_2x" | "library_header_2x" => (Slot::Banner, 0),
        "header" | "library_header" => (Slot::Banner, 1),
        _ => return None,
    })
}

fn is_sha1_name(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Icons in the appid dir are `<sha1>.jpg` (32x32, ~1–5 KB).
const ICON_MAX_BYTES: u64 = 64 * 1024;

#[derive(Default)]
struct Found {
    best: [Option<(u8, u8, PathBuf)>; 4], // (priority, depth, path) per slot
    icon: Option<PathBuf>,
}

impl Found {
    fn offer(&mut self, slot: Slot, prio: u8, depth: u8, path: PathBuf) {
        let i = slot as usize;
        if self.best[i].as_ref().is_none_or(|(p, d, _)| (prio, depth) < (*p, *d)) {
            self.best[i] = Some((prio, depth, path));
        }
    }
    fn take(&mut self, slot: Slot) -> Option<PathBuf> {
        self.best[slot as usize].take().and_then(|(_, _, p)| file_if_exists(p))
    }
}

impl ArtCtx {
    /// Custom grid art for an appid (Steam's "Set custom artwork").
    fn grid_art(&self, appid: u32) -> Art {
        let Some(g) = &self.grid else {
            return Art::default();
        };
        Art {
            cover: image_with_stem(g, &format!("{appid}p")),
            hero: image_with_stem(g, &format!("{appid}_hero")),
            logo: image_with_stem(g, &format!("{appid}_logo")),
            icon: image_with_stem(g, &format!("{appid}_icon")),
            banner: image_with_stem(g, &format!("{appid}")),
        }
    }

    fn app_art(&self, appid: u32) -> Art {
        let mut art = self.grid_art(appid);
        if art.icon.is_none() {
            let name = format!("steam_icon_{appid}.png");
            art.icon = self.icon_dirs.iter().find_map(|d| file_if_exists(d.join(&name)));
        }
        art.fill_from(&self.cache_art(appid));
        if art.cover.is_none() || art.hero.is_none() || art.logo.is_none() || art.banner.is_none() {
            // Old flat layout: librarycache/<appid>_library_600x900.jpg …
            let flat = |stem: &str| image_with_stem(&self.cache, &format!("{appid}_{stem}"));
            let old = Art {
                cover: flat("library_600x900_2x").or_else(|| flat("library_600x900")),
                hero: flat("library_hero"),
                logo: flat("logo"),
                icon: flat("icon"),
                banner: flat("header"),
            };
            art.fill_from(&old);
        }
        art
    }

    /// New layout: `librarycache/<appid>/` with files at the top or one level down in
    /// `<sha1>/` subdirectories.
    fn cache_art(&self, appid: u32) -> Art {
        let dir = self.cache.join(appid.to_string());
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return Art::default();
        };
        let mut f = Found::default();
        let mut subdirs = Vec::new();
        for ent in rd.flatten() {
            let Ok(ft) = ent.file_type() else { continue };
            let name = ent.file_name();
            let Some(name) = name.to_str() else { continue };
            if ft.is_dir() {
                subdirs.push(ent.path());
            } else if let Some((slot, prio)) = classify(name) {
                f.offer(slot, prio, 0, ent.path());
            } else if f.icon.is_none()
                && let Some((stem, ext)) = name.rsplit_once('.')
                && is_sha1_name(stem)
                && matches!(ext, "jpg" | "png" | "ico")
                && ent.metadata().is_ok_and(|m| m.len() > 0 && m.len() <= ICON_MAX_BYTES)
            {
                f.icon = Some(ent.path());
            }
        }
        for sd in subdirs {
            let Ok(rd) = std::fs::read_dir(&sd) else {
                continue;
            };
            for ent in rd.flatten() {
                let name = ent.file_name();
                if let Some((slot, prio)) = name.to_str().and_then(classify) {
                    f.offer(slot, prio, 1, ent.path());
                }
            }
        }
        Art {
            cover: f.take(Slot::Cover),
            hero: f.take(Slot::Hero),
            logo: f.take(Slot::Logo),
            banner: f.take(Slot::Banner),
            icon: f.icon,
        }
    }
}

#[cfg(test)]
mod tests;
