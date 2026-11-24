//! Availability detection for game options (spec §4): cheap, file-based checks plus one cached
//! `gamescope --help`. Never runs games or anything that changes system state.
//!
//! CONTRACT:
//!   `probe() -> Caps` — blocking (call off the UI thread); a few ms plus at most one
//!     `gamescope --help` (2 s timeout), cached by (path, mtime, size) in Lodestar's cache dir.
//!     Pre-probes every Proton dir in the usual compatibilitytools.d folders into `Caps::runners`,
//!     and warms the `runner_caps` cache (bounded) for Steam's own Proton builds and the ones
//!     Lutris / Heroic / Bottles installed. `probe_for_launch()` skips that warming.
//!   `runner_caps(dir) -> Arc<RunnerCaps>` — what the Proton build in `dir` supports; cached per
//!     (dir, mtimes); cheap after the first call. `RunnerCaps::unknown()` when `dir` isn't one.
//!   `wine_caps(wine) -> Arc<RunnerCaps>` — the same file checks for a plain Wine binary (kind W),
//!     cached the same way.
//!   `install_hint(id, distro) -> Option<&'static str>` — one install line for an unavailable option.
//!
//! Every filesystem location goes through [`ProbeRoots`], so tests run [`probe_with`] against a
//! fake tree in a tempdir (see `caps/tests.rs`). Every walk is bounded
//! ([`MAX_DIR_ENTRIES`]) and every whole-file read is small ([`MAX_SMALL_FILE`]); the big runner
//! files (`win32u.so`, `wineserver`) are streamed in chunks and the result is cached.

use super::*;
use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

#[cfg(test)]
mod tests;

// Probing parts in sibling files: gamescope's --help, GPUs / outputs, runners. Their pub items
// are re-exported here; the private ones stay private to caps (pub(super)).
mod gamescope;

mod display;

mod runners;

pub use display::*;
pub use gamescope::*;
pub use runners::*;

/// At most this many entries are looked at per directory listing.
pub const MAX_DIR_ENTRIES: usize = 256;
/// Largest file read whole (manifests, configs, sysfs attributes, scripts).
pub const MAX_SMALL_FILE: u64 = 4 << 20;
/// How long `gamescope --help` may take.
pub const GAMESCOPE_HELP_TIMEOUT: Duration = Duration::from_secs(2);
/// File in Lodestar's cache dir holding the last `gamescope --help` text.
const GAMESCOPE_CACHE_FILE: &str = "gamescope-help.json";
/// Lossless Scaling's Steam appid (lsfg-vk needs its `Lossless.dll`).
const LOSSLESS_APPID: u32 = 993_090;

/// The environment variables a probe looks at (snapshotted by [`ProbeRoots::real`]).
const PROBE_VARS: &[&str] = &[
    "PATH",
    "APPDIR",
    "XDG_CONFIG_HOME",
    "XDG_CONFIG_DIRS",
    "XDG_DATA_HOME",
    "XDG_DATA_DIRS",
    "XDG_RUNTIME_DIR",
    "XDG_CURRENT_DESKTOP",
    "WAYLAND_DISPLAY",
    "GAMESCOPE_WAYLAND_DISPLAY",
    "LSFGVK_CONFIG",
];

// ------------------------------------------------------------------------------------- roots

/// Where a probe looks. [`ProbeRoots::real`] is the running system; tests point everything at
/// a tempdir.
#[derive(Clone, Debug)]
pub struct ProbeRoots {
    /// Filesystem root for the fixed system paths (`/sys`, `/etc`, `/run`, `/dev`, `/proc`,
    /// `/usr`, `/var`, `/lib`). `/` on the real system.
    pub root: PathBuf,
    /// The user's home (`~/.local/share/Steam`, default XDG dirs, Flatpak user installs).
    pub home: PathBuf,
    /// Snapshot of the [`PROBE_VARS`] environment variables. Paths in them are used as given
    /// (they are not re-rooted under `root`).
    pub env: HashMap<String, String>,
    /// Lodestar's cache dir (gamescope `--help` cache); None = no caching.
    pub cache_dir: Option<PathBuf>,
    /// Limit for `gamescope --help`.
    pub gamescope_timeout: Duration,
}

impl ProbeRoots {
    /// The running system: `/`, the real home, the process env, Lodestar's cache dir
    /// (`LODESTAR_HOME` honoured through `config::Paths`).
    pub fn real() -> ProbeRoots {
        let paths = crate::config::Paths::detect();
        let env = PROBE_VARS.iter().filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v))).collect();
        ProbeRoots {
            root: PathBuf::from("/"),
            home: paths.home,
            env,
            cache_dir: Some(paths.cache_dir),
            gamescope_timeout: GAMESCOPE_HELP_TIMEOUT,
        }
    }

    /// A fixed system path (`"/sys/class/drm"`) under [`Self::root`].
    pub fn at(&self, abs: &str) -> PathBuf {
        self.root.join(abs.trim_start_matches('/'))
    }

    /// A non-empty environment variable.
    fn var(&self, key: &str) -> Option<&str> {
        self.env.get(key).map(String::as_str).filter(|v| !v.is_empty())
    }

    /// `$XDG_CONFIG_HOME`, else `~/.config`.
    fn config_home(&self) -> PathBuf {
        self.var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| self.home.join(".config"))
    }

    /// `$XDG_DATA_HOME`, else `~/.local/share`.
    fn data_home(&self) -> PathBuf {
        self.var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| self.home.join(".local/share"))
    }

    /// A `:`-list variable, else the defaults (system paths, re-rooted).
    fn dir_list(&self, key: &str, defaults: &[&str]) -> Vec<PathBuf> {
        match self.var(key) {
            Some(v) => v.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect(),
            None => defaults.iter().map(|d| self.at(d)).collect(),
        }
    }

    /// The Vulkan loader's implicit layer folders, in its search order (spec §4.2 "layers").
    pub fn layer_dirs(&self) -> Vec<PathBuf> {
        let mut bases = vec![self.config_home()];
        bases.extend(self.dir_list("XDG_CONFIG_DIRS", &["/etc/xdg"]));
        bases.push(self.at("/etc"));
        bases.push(self.data_home());
        bases.extend(self.dir_list("XDG_DATA_DIRS", &["/usr/local/share", "/usr/share"]));
        let mut out: Vec<PathBuf> = Vec::new();
        for b in bases {
            let d = b.join("vulkan/implicit_layer.d");
            if !out.contains(&d) {
                out.push(d);
            }
        }
        out
    }

    /// `PATH` without the entries inside our own AppImage mount (`$APPDIR`).
    fn search_path(&self) -> Option<OsString> {
        let appdir = self.var("APPDIR").map(PathBuf::from);
        let dirs = std::env::split_paths(self.var("PATH")?)
            .filter(|d| appdir.as_ref().is_none_or(|a| !d.starts_with(a)))
            .collect::<Vec<_>>();
        std::env::join_paths(dirs).ok()
    }

    /// The compatibilitytools.d folders Proton builds live in (Steam's, the system package
    /// folder, and umu's: umu installs UMU-Proton / GE-Proton into
    /// `$XDG_DATA_HOME/Steam/compatibilitytools.d` **[U]**).
    pub fn compat_dirs(&self) -> Vec<PathBuf> {
        vec![
            self.home.join(".local/share/Steam/compatibilitytools.d"),
            self.home.join(".steam/root/compatibilitytools.d"),
            self.at("/usr/share/steam/compatibilitytools.d"),
            self.data_home().join("Steam/compatibilitytools.d"),
        ]
    }
}

// ------------------------------------------------------------------------------------- probe

/// Detect everything on the running system (blocking; see the module CONTRACT).
pub fn probe() -> Caps {
    probe_with(&ProbeRoots::real())
}

/// [`probe`] for a one-shot headless launch (`--play`, `--launch`): the same `Caps`, but without
/// warming the runner cache for other launchers' Proton builds (up to a few MB read per build,
/// which would only delay the game; its own runner is probed on resolve anyway).
pub fn probe_for_launch() -> Caps {
    probe_impl(&ProbeRoots::real(), false)
}

/// [`probe`] against explicit roots.
pub fn probe_with(r: &ProbeRoots) -> Caps {
    probe_impl(r, true)
}

fn probe_impl(r: &ProbeRoots, warm_extra: bool) -> Caps {
    let started = Instant::now();
    let bins = find_bins(r);
    let gamescope = bins.gamescope.as_deref().and_then(|p| gamescope_caps(p, r));
    let layers = find_layers(r);
    let lsfg = lsfg_caps(r, &layers);
    let gpus = find_gpus(r);
    let connectors = find_connectors(r);
    let (kde_hdr_enabled, kde_vrr_never) = kde_output_settings(r, &connectors);
    let caps = Caps {
        probed: true,
        bins,
        gamescope,
        layers,
        lsfg,
        preferred_mode: preferred_mode(&gpus, &connectors),
        gpus,
        nvidia_version: nvidia_version(r),
        ntsync: ntsync_state(r),
        wayland_session: wayland_session(r),
        nested_gamescope: nested_gamescope(r),
        kde_hdr_enabled,
        kde_vrr_never,
        hdr_outputs: pq_outputs(&connectors),
        flatpak_ext: flatpak_extensions(r),
        distro: distro(r),
        runners: pre_probe_runners(r),
    };
    if warm_extra {
        warm_extra_runners(r);
    }
    log::debug!("game options caps probe: {:?}", started.elapsed());
    caps
}

// ----------------------------------------------------------------------------------- helpers

/// Read a small regular file whole (None when missing, not a file, or too large).
fn read_small(p: &Path) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(p).ok()?;
    if !meta.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(p).ok()?.take(MAX_SMALL_FILE + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= MAX_SMALL_FILE).then_some(bytes)
}

/// [`read_small`] as text (invalid UTF-8 replaced).
fn read_text(p: &Path) -> Option<String> {
    read_small(p).map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// A trimmed one-line attribute (sysfs).
fn read_attr(p: &Path) -> Option<String> {
    read_text(p).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// `0x1002` / `1002` → 0x1002.
fn parse_hex_u16(s: &str) -> Option<u16> {
    u16::from_str_radix(s.trim().trim_start_matches("0x").trim_start_matches("0X"), 16).ok()
}

/// Entries of a directory sorted by name, at most [`MAX_DIR_ENTRIES`] of them.
fn list_dir(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = rd
        .flatten()
        .take(MAX_DIR_ENTRIES)
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
        .collect();
    out.sort();
    out
}

/// Which of `needles` occur in the file, streamed in chunks (for large binaries).
fn file_contains(path: &Path, needles: &[&[u8]]) -> Vec<bool> {
    let mut found = vec![false; needles.len()];
    let Ok(mut f) = std::fs::File::open(path) else {
        return found;
    };
    let keep = needles.iter().map(|n| n.len()).max().unwrap_or(1).saturating_sub(1);
    let mut buf: Vec<u8> = Vec::with_capacity(CHUNK + keep);
    let mut chunk = vec![0u8; CHUNK];
    loop {
        let n = match f.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        buf.extend_from_slice(&chunk[..n]);
        for (i, needle) in needles.iter().enumerate() {
            if !found[i] && contains(&buf, needle) {
                found[i] = true;
            }
        }
        if found.iter().all(|f| *f) {
            break;
        }
        // Keep the tail so a needle split across two chunks is still seen.
        let cut = buf.len().saturating_sub(keep);
        buf.drain(..cut);
    }
    found
}

/// Read size for [`file_contains`].
const CHUNK: usize = 256 << 10;

/// Byte-substring search.
fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

// -------------------------------------------------------------------------------------- bins

/// Helper programs on PATH; hits under `$APPDIR` are ignored.
fn find_bins(r: &ProbeRoots) -> Bins {
    let path = r.search_path();
    let find = |name: &str| crate::launch::find_in_path_with(name, path.as_deref());
    Bins {
        gamescope: find("gamescope"),
        mangohud: find("mangohud"),
        mangoapp: find("mangoapp"),
        gamemoderun: find("gamemoderun"),
        game_performance: find("game-performance"),
        powerprofilesctl: find("powerprofilesctl"),
        tlpctl: find("tlpctl"),
        obs_gamecapture: find("obs-gamecapture"),
        systemd_inhibit: find("systemd-inhibit"),
    }
}

// ------------------------------------------------------------------------------------ layers

/// `layer.name` prefixes the generic "Vulkan layers" list leaves out: layers other options
/// handle, Steam's own, and loader plumbing (spec §4.2 "layers").
#[cfg(test)]
pub const LAYER_EXCLUDE_PREFIXES: &[&str] = &[
    "VK_LAYER_MANGOHUD_",
    "VK_LAYER_OBS_vkcapture",
    "VK_LAYER_LSFGVK",
    "VK_LAYER_LS_frame_generation",
    "VK_LAYER_FROG_gamescope_wsi",
    "VK_LAYER_hdr_wsi",
    "VK_LAYER_VALVE_",
    "VK_LAYER_MESA_device_select",
    "VK_LAYER_NV_optimus",
];

/// Whether a layer gets its own row in the generic "Vulkan layers" list. `Caps::layers` holds
/// every layer (other options look up OBS, lsfg, HDR WSI there); the UI filters with this.
#[cfg(test)]
pub fn is_generic_layer(name: &str) -> bool {
    !LAYER_EXCLUDE_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Every implicit layer, one entry per enable/disable env key (32- and 64-bit manifests of the
/// same layer are grouped; the 64-bit one is kept). Earlier folders win for duplicates.
fn find_layers(r: &ProbeRoots) -> Vec<LayerManifest> {
    let mut out: Vec<(LayerManifest, bool)> = Vec::new();
    for dir in r.layer_dirs() {
        for (name, path) in list_dir(&dir) {
            if !name.ends_with(".json") {
                continue;
            }
            let Some(text) = read_text(&path) else { continue };
            for (layer, is32) in parse_layer_manifest(&text, &path) {
                let key = layer_group_key(&layer);
                match out.iter_mut().find(|(l, _)| layer_group_key(l) == key) {
                    Some(slot) if slot.1 && !is32 => *slot = (layer, is32),
                    Some(_) => {}
                    None => out.push((layer, is32)),
                }
            }
        }
    }
    out.into_iter().map(|(l, _)| l).collect()
}

/// Grouping key: the env keys, or the name when the layer has none.
fn layer_group_key(l: &LayerManifest) -> (Option<String>, Option<String>, Option<String>) {
    let enable = l.enable_env.as_ref().map(|(k, _)| k.clone());
    let name = (enable.is_none() && l.disable_env.is_none()).then(|| l.name.clone());
    (enable, l.disable_env.clone(), name)
}

/// The layers in one manifest (`"layer": {…}` or `"layers": […]`) with a 32-bit guess each.
/// Invalid JSON yields nothing.
pub fn parse_layer_manifest(text: &str, file: &Path) -> Vec<(LayerManifest, bool)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        log::debug!("{}: not a layer manifest", file.display());
        return Vec::new();
    };
    let entries: Vec<&serde_json::Value> = match (v.get("layer"), v.get("layers")) {
        (Some(l), _) => vec![l],
        (None, Some(serde_json::Value::Array(ls))) => ls.iter().collect(),
        _ => Vec::new(),
    };
    let file_name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
    entries
        .into_iter()
        .filter_map(|l| {
            let name = l.get("name")?.as_str()?.to_string();
            let first = |key: &str| {
                l.get(key)?.as_object()?.iter().next().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("1").to_string()))
            };
            let lib = l.get("library_path").and_then(|p| p.as_str()).unwrap_or("");
            let is32 = looks_32bit(&name, file_name, lib);
            let layer = LayerManifest {
                description: l.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string(),
                enable_env: first("enable_environment"),
                disable_env: first("disable_environment").map(|(k, _)| k),
                file: file.to_path_buf(),
                name,
            };
            Some((layer, is32))
        })
        .collect()
}

/// Guess whether a manifest is the 32-bit copy (name `…_x86` / `…32`, file `….x86.json`,
/// `i386` / `i686` / `lib32` in the file or library path).
fn looks_32bit(name: &str, file_name: &str, library_path: &str) -> bool {
    let n = name.to_ascii_lowercase();
    let f = file_name.to_ascii_lowercase();
    let l = library_path.to_ascii_lowercase();
    let arch32 = |s: &str| s.contains("i386") || s.contains("i686") || s.contains("lib32");
    n.ends_with("_x86")
        || n.ends_with("32")
        || arch32(&f)
        || arch32(&l)
        || (f.contains("x86") && !f.contains("x86_64"))
        || f.contains("32.json")
}

// -------------------------------------------------------------------------------------- lsfg

/// lsfg-vk: version from its layer (`VK_LAYER_LSFGVK_frame_generation…` = 2,
/// `VK_LAYER_LS_frame_generation` = 1 **[U]**), profiles from its conf.toml, and Lossless.dll.
/// None when no lsfg-vk layer is installed.
fn lsfg_caps(r: &ProbeRoots, layers: &[LayerManifest]) -> Option<LsfgCaps> {
    let version = layers.iter().find_map(|l| {
        if l.name.starts_with("VK_LAYER_LSFGVK_frame_generation") {
            Some(2)
        } else if l.name == "VK_LAYER_LS_frame_generation" {
            Some(1)
        } else {
            None
        }
    })?;
    let conf = r.var("LSFGVK_CONFIG").map(PathBuf::from).unwrap_or_else(|| r.config_home().join("lsfg-vk/conf.toml"));
    let parsed = read_text(&conf).map(|t| parse_lsfg_conf(&t)).unwrap_or_default();
    let dll = parsed.dll.filter(|p| p.is_file()).or_else(|| find_lossless_dll(r));
    Some(LsfgCaps { version, profiles: parsed.profiles, dll })
}

/// What Lodestar reads from lsfg-vk's conf.toml.
#[derive(Debug, Default, PartialEq)]
pub struct LsfgConf {
    /// v2 `[[profile]]` (name, active_in); v1 `[[game]]` entries become (exe, [exe]).
    pub profiles: Vec<(String, Vec<String>)>,
    /// `[global] dll`, when set.
    pub dll: Option<PathBuf>,
}

/// Parse lsfg-vk's conf.toml (`active_in` may be a string or a list). Invalid TOML → empty.
pub fn parse_lsfg_conf(text: &str) -> LsfgConf {
    let Ok(v) = text.parse::<toml::Table>() else {
        return LsfgConf::default();
    };
    let strings = |v: Option<&toml::Value>| -> Vec<String> {
        match v {
            Some(toml::Value::String(s)) => vec![s.clone()],
            Some(toml::Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
            _ => Vec::new(),
        }
    };
    let tables = |key: &str| -> Vec<toml::Table> {
        v.get(key).and_then(|p| p.as_array()).into_iter().flatten().filter_map(|t| t.as_table().cloned()).collect()
    };
    let mut profiles: Vec<(String, Vec<String>)> = tables("profile")
        .iter()
        .filter_map(|t| Some((t.get("name")?.as_str()?.to_string(), strings(t.get("active_in")))))
        .collect();
    profiles.extend(
        tables("game").iter().filter_map(|t| t.get("exe")?.as_str().map(|e| (e.to_string(), vec![e.to_string()]))),
    );
    let dll = v
        .get("global")
        .and_then(|g| g.get("dll"))
        .and_then(|d| d.as_str())
        .filter(|d| !d.is_empty())
        .map(PathBuf::from);
    LsfgConf { profiles, dll }
}

/// Steam roots under the home (native, `~/.steam/*`, Flatpak, Snap), canonical and deduped.
fn steam_roots(r: &ProbeRoots) -> Vec<PathBuf> {
    let h = &r.home;
    let mut out: Vec<PathBuf> = Vec::new();
    for c in [
        h.join(".local/share/Steam"),
        h.join(".steam/steam"),
        h.join(".steam/root"),
        h.join(".steam/debian-installation"),
        h.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
        h.join(".var/app/com.valvesoftware.Steam/data/Steam"),
        h.join("snap/steam/common/.local/share/Steam"),
    ] {
        if let Ok(c) = c.canonicalize()
            && c.join("steamapps").is_dir()
            && !out.contains(&c)
        {
            out.push(c);
        }
    }
    out
}

/// `Lossless.dll` in the Steam library that has Lossless Scaling installed
/// (`appmanifest_993090.acf`; folder from its `installdir`, else "Lossless Scaling" **[U]**).
fn find_lossless_dll(r: &ProbeRoots) -> Option<PathBuf> {
    let manifest = format!("steamapps/appmanifest_{LOSSLESS_APPID}.acf");
    for root in steam_roots(r) {
        for lib in crate::scan::steam::library_folders(&root) {
            let Some(text) = read_text(&lib.join(&manifest)) else { continue };
            let dir = crate::scan::steam::parse_manifest(&text, &lib)
                .ok()
                .and_then(|m| m.install_path())
                .unwrap_or_else(|| lib.join("steamapps/common/Lossless Scaling"));
            let dll = dir.join("Lossless.dll");
            if dll.is_file() {
                return Some(dll);
            }
        }
    }
    None
}

// ------------------------------------------------------------------------------ system state

/// NTSync: `/dev/ntsync` readable → Usable; an `ntsync.ko*` for the running kernel → NotLoaded;
/// else Missing. A `/dev/ntsync` that exists but isn't readable counts as not usable (Wine
/// can't open it either) and falls through to the module check.
///
/// The module is looked up in the kernel's `modules.dep` / `modules.builtin` (they list the
/// in-tree module as well as DKMS (`updates/dkms/`) and out-of-tree (`extra/`) builds), and,
/// when those can't be read, in the folders such modules are installed to (one level each).
fn ntsync_state(r: &ProbeRoots) -> NtsyncState {
    if readable(&r.at("/dev/ntsync")) {
        return NtsyncState::Usable;
    }
    let Some(release) = read_attr(&r.at("/proc/sys/kernel/osrelease")) else {
        return NtsyncState::Missing;
    };
    let is_ntsync = |n: &str| n.starts_with("ntsync.ko");
    let has_module = ["/lib/modules", "/usr/lib/modules"].iter().any(|base| {
        let kdir = r.at(base).join(&release);
        let listed = ["modules.dep", "modules.builtin"].iter().any(|f| {
            read_text(&kdir.join(f)).is_some_and(|t| {
                t.lines().any(|l| {
                    let path = l.split(':').next().unwrap_or("").trim();
                    Path::new(path).file_name().and_then(|n| n.to_str()).is_some_and(is_ntsync)
                })
            })
        });
        listed
            || ["kernel/drivers/misc", "updates", "updates/dkms", "extra"]
                .iter()
                .any(|d| list_dir(&kdir.join(d)).iter().any(|(n, _)| is_ntsync(n)))
    });
    if has_module { NtsyncState::NotLoaded } else { NtsyncState::Missing }
}

/// `access(path, R_OK)`.
fn readable(p: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `c` is a valid NUL-terminated string that outlives the call.
    unsafe { libc::access(c.as_ptr(), libc::R_OK) == 0 }
}

/// `WAYLAND_DISPLAY` is set and names a socket (absolute, or under `XDG_RUNTIME_DIR`).
fn wayland_session(r: &ProbeRoots) -> bool {
    use std::os::unix::fs::FileTypeExt;
    let Some(display) = r.var("WAYLAND_DISPLAY") else {
        return false;
    };
    let sock = if display.starts_with('/') {
        PathBuf::from(display)
    } else {
        match r.var("XDG_RUNTIME_DIR") {
            Some(run) => Path::new(run).join(display),
            None => return false,
        }
    };
    std::fs::metadata(sock).is_ok_and(|m| m.file_type().is_socket())
}

/// Lodestar runs inside gamescope: `GAMESCOPE_WAYLAND_DISPLAY` set, or `XDG_CURRENT_DESKTOP`
/// (a `:`-list) contains `gamescope`.
fn nested_gamescope(r: &ProbeRoots) -> bool {
    r.var("GAMESCOPE_WAYLAND_DISPLAY").is_some()
        || r.var("XDG_CURRENT_DESKTOP").is_some_and(|d| d.split(':').any(|p| p.eq_ignore_ascii_case("gamescope")))
}

/// Distro family from `/etc/os-release` (else `/usr/lib/os-release`) `ID` and `ID_LIKE`.
fn distro(r: &ProbeRoots) -> Distro {
    read_text(&r.at("/etc/os-release"))
        .or_else(|| read_text(&r.at("/usr/lib/os-release")))
        .map(|t| parse_os_release(&t))
        .unwrap_or_default()
}

/// `ID` / `ID_LIKE` → [`Distro`] (Arch, CachyOS, Manjaro … → Arch; Fedora, Nobara … → Fedora;
/// Debian, Ubuntu, Mint … → Debian).
pub fn parse_os_release(text: &str) -> Distro {
    let mut ids: Vec<String> = Vec::new();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=')
            && (k.trim() == "ID" || k.trim() == "ID_LIKE")
        {
            ids.extend(
                v.trim().trim_matches(|c| c == '"' || c == '\'').split_whitespace().map(str::to_ascii_lowercase),
            );
        }
    }
    let has = |names: &[&str]| ids.iter().any(|i| names.contains(&i.as_str()));
    if has(&["arch", "cachyos", "manjaro", "endeavouros"]) {
        Distro::Arch
    } else if has(&["fedora", "nobara", "bazzite"]) {
        Distro::Fedora
    } else if has(&["debian", "ubuntu"]) {
        Distro::Debian
    } else {
        Distro::Other
    }
}

// ----------------------------------------------------------------------------------- flatpak

/// The user and system Flatpak installs' `runtime/` folders.
fn flatpak_runtime_dirs(r: &ProbeRoots) -> [PathBuf; 2] {
    [r.home.join(".local/share/flatpak/runtime"), r.at("/var/lib/flatpak/runtime")]
}

/// Installed Vulkan-layer extensions (ids containing `.VulkanLayer.`, which covers MangoHud,
/// OBSVkCapture, lsfgvk, gamescope, vkBasalt) → their x86_64 branches with an `active` deploy.
fn flatpak_extensions(r: &ProbeRoots) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for base in flatpak_runtime_dirs(r) {
        for (id, path) in list_dir(&base) {
            if !id.contains(".VulkanLayer.") {
                continue;
            }
            for (branch, bpath) in list_dir(&path.join("x86_64")) {
                if bpath.join("active").exists() {
                    let branches = out.entry(id.clone()).or_default();
                    if !branches.contains(&branch) {
                        branches.push(branch);
                    }
                }
            }
        }
    }
    out
}

/// A Flatpak app's runtime (`org.gnome.Platform`, `48`) from its deployed `metadata` `runtime=` line
/// (user install first). Not wired into the options yet (the per-app extension branch check).
#[cfg(test)]
pub fn flatpak_app_runtime_with(r: &ProbeRoots, app_id: &str) -> Option<(String, String)> {
    if app_id.is_empty() || app_id.contains('/') {
        return None;
    }
    let apps = [r.home.join(".local/share/flatpak/app"), r.at("/var/lib/flatpak/app")];
    apps.iter().find_map(|base| {
        let meta = read_text(&base.join(app_id).join("current/active/metadata"))?;
        let rt = meta.lines().find_map(|l| l.trim().strip_prefix("runtime="))?;
        let mut parts = rt.split('/');
        let id = parts.next()?.to_string();
        let branch = parts.nth(1)?.to_string();
        Some((id, branch))
    })
}

// ------------------------------------------------------------------------------ install hints

/// The install command for a Flatpak Vulkan-layer extension on an app's runtime branch.
#[cfg(test)]
pub fn flatpak_ext_hint(ext_id: &str, branch: &str) -> String {
    format!("flatpak install flathub {ext_id}//{branch}")
}

/// One install line for an unavailable option on `distro` (spec §4.3); Ubuntu uses the Debian
/// column, `Other` gets a distro-neutral line. None for options nothing can be installed for.
pub fn install_hint(id: OptionId, distro: Distro) -> Option<&'static str> {
    use Distro as D;
    use OptionId as O;
    let hint = match (id, distro) {
        (O::Gamescope, D::Arch) => "sudo pacman -S gamescope",
        (O::Gamescope, D::Fedora) => "sudo dnf install gamescope",
        (O::Gamescope, D::Debian) => "sudo apt install gamescope (Ubuntu 26.04 or newer; not in 24.04)",
        (O::Gamescope, D::Other) => "Install the gamescope package",
        (O::Mangohud, D::Arch) => "sudo pacman -S mangohud lib32-mangohud",
        (O::Mangohud, D::Fedora) => "sudo dnf install mangohud",
        (O::Mangohud, D::Debian) => "sudo apt install mangohud",
        (O::Mangohud, D::Other) => "Install the mangohud package (it includes mangoapp)",
        (O::Gamemode, D::Arch) => "sudo pacman -S gamemode lib32-gamemode",
        (O::Gamemode, D::Fedora) => "sudo dnf install gamemode",
        (O::Gamemode, D::Debian) => "sudo apt install gamemode",
        (O::Gamemode, D::Other) => "Install the gamemode package",
        (O::PerformanceProfile, D::Arch) => {
            "CachyOS: sudo pacman -S cachyos-settings power-profiles-daemon; Arch: sudo pacman -S power-profiles-daemon"
        }
        (O::PerformanceProfile, D::Fedora) => {
            "Install power-profiles-daemon (it replaces tuned-ppd, which has no powerprofilesctl)"
        }
        (O::PerformanceProfile, D::Debian) => "sudo apt install power-profiles-daemon",
        (O::PerformanceProfile, D::Other) => "Install power-profiles-daemon",
        (O::KeepAwake, _) => "systemd-inhibit comes with systemd",
        (O::ObsCapture, D::Arch) => "Install obs-vkcapture from the AUR",
        (O::ObsCapture, D::Fedora) => "sudo dnf install obs-studio-plugin-vkcapture",
        (O::ObsCapture, D::Debian) => "Build obs-vkcapture from source, or use the Flatpak OBS plugin",
        (O::ObsCapture, D::Other) => "Install obs-vkcapture",
        (O::Lsfg, D::Arch) => "Install lsfg-vk or lsfg-vk-bin from the AUR, plus Lossless Scaling on Steam",
        (O::Lsfg, _) => "Install the lsfg-vk tarball from lsfg-vk.dev, plus Lossless Scaling on Steam",
        (O::Ntsync, D::Arch) => "Needs Linux 6.14 or newer: install ntsync-autoload on Arch (built in on CachyOS)",
        (O::Ntsync, D::Fedora) => "Fedora 44: install wine-ntsync, or run sudo modprobe ntsync",
        (O::Ntsync, _) => "sudo modprobe ntsync (needs Linux 6.14 or newer)",
        (
            O::Wayland
            | O::HdrForce
            | O::WineFsr
            | O::WineFsrStrength
            | O::Fsr4Upgrade
            | O::DlssUpgrade
            | O::XessUpgrade
            | O::UpscalerIndicator
            | O::SdlInput,
            D::Arch,
        ) => "Install GE-Proton (ProtonUp-Qt) or proton-cachyos",
        (
            O::Wayland
            | O::HdrForce
            | O::WineFsr
            | O::WineFsrStrength
            | O::Fsr4Upgrade
            | O::DlssUpgrade
            | O::XessUpgrade
            | O::UpscalerIndicator
            | O::SdlInput,
            _,
        ) => "Install GE-Proton (ProtonUp-Qt)",
        _ => return None,
    };
    Some(hint)
}
