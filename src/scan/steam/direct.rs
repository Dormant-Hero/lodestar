//! Direct launch of an installed Steam game, bypassing Steam's own per-game "Launch Options".
//!
//! `steam -applaunch <appid>` always applies the launch options set inside Steam. A user who wants
//! Lodestar to decide the launch options instead can start from [`direct_launch`]: it resolves the
//! game's executable from appinfo (`config/launch`), its install dir, and — for games Steam runs
//! with Proton — the compat tool and prefix, and returns the full command. The UI can put
//! [`DirectLaunch::command_line`] into `LaunchOverride::command` so `LaunchOverride::options`
//! (`%command%`, `VAR=value`, wrappers) apply to it.
//!
//! Best effort: Steam must still be running for games using Steamworks DRM/APIs, and the game runs
//! outside the Steam Linux Runtime container. Reads a few small files plus one appinfo entry;
//! call it from a background thread or a click handler, not every frame.

// Public API for the launch/editor modules; not every item is wired up yet.

use super::{InstallKind, SteamRoot, appinfo, compat_tools, find_roots, parse_manifest};
use crate::scan::ScanContext;
use crate::scan::vdf;
use anyhow::{Context, Result, anyhow, bail};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectLaunch {
    pub appid: u32,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    /// Compat tool internal name when run through Proton, e.g. "Proton-CachyOS Latest".
    pub compat_tool: Option<String>,
    /// Directory of the compat tool (contains the `proton` script).
    pub compat_tool_dir: Option<PathBuf>,
}

impl DirectLaunch {
    /// Shell-like quoted command (`env VAR=value … program args …`) for `LaunchOverride::command`.
    /// Uses `env(1)` for the variables so it works without a shell.
    pub fn command_line(&self) -> String {
        let mut words: Vec<String> = Vec::new();
        if !self.env.is_empty() {
            words.push("env".into());
            words.extend(self.env.iter().map(|(k, v)| format!("{k}={v}")));
        }
        words.push(self.program.clone());
        words.extend(self.args.iter().cloned());
        words.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ")
    }
}

/// Quote a word for a POSIX-shell-like parser (single quotes when needed).
pub fn quote(w: &str) -> String {
    if !w.is_empty() && w.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_./=:,+@%".contains(&b)) {
        w.to_string()
    } else {
        format!("'{}'", w.replace('\'', "'\\''"))
    }
}

/// Split an argument string the way Steam/a shell would for simple cases
/// (whitespace, single/double quotes, backslash escapes).
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut have = false;
    let mut chars = s.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') if matches!(chars.peek(), Some('"' | '\\')) => {
                cur.extend(chars.next());
            }
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                have = true;
            }
            (None, '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                    have = true;
                }
            }
            (None, c) if c.is_whitespace() => {
                if have || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    have = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if have || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Libraries of a root (root first) that contain `appmanifest_<appid>.acf`.
fn find_manifest(root: &Path, appid: u32) -> Option<super::Manifest> {
    super::library_folders(root).into_iter().find_map(|lib| {
        let text = crate::scan::util::read_text(lib.join(format!("steamapps/appmanifest_{appid}.acf"))).ok()?;
        parse_manifest(&text, &lib).ok().filter(|m| m.installed())
    })
}

/// Map a compat tool internal name to its directory.
fn compat_tool_dir(roots: &[SteamRoot], name: &str) -> Option<PathBuf> {
    // Custom tools: compatibilitytools.d/<dir>/compatibilitytool.vdf declares internal names.
    let mut dirs: Vec<PathBuf> = roots.iter().map(|r| r.path.join("compatibilitytools.d")).collect();
    dirs.push("/usr/share/steam/compatibilitytools.d".into());
    dirs.push("/usr/local/share/steam/compatibilitytools.d".into());
    for d in &dirs {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        for ent in rd.flatten() {
            let p = ent.path();
            let manifest = if p.is_dir() { p.join("compatibilitytool.vdf") } else { p.clone() };
            let Ok(text) = crate::scan::util::read_text(&manifest) else {
                continue;
            };
            let Ok(v) = vdf::parse(&text) else { continue };
            let Some(tools) = v.path(&["compatibilitytools", "compat_tools"]) else {
                continue;
            };
            for (internal, t) in tools.entries() {
                let display = t.get_str("display_name").unwrap_or("");
                if internal != name && display != name {
                    continue;
                }
                let base = manifest.parent().unwrap_or(d);
                let dir = match t.get_str("install_path").map(str::trim) {
                    None | Some("" | ".") => base.to_path_buf(),
                    Some(ip) => base.join(ip),
                };
                if dir.join("proton").is_file() {
                    return Some(dir);
                }
            }
        }
    }
    // Valve's Proton builds live in steamapps/common.
    let common_name = valve_proton_dir(name)?;
    roots
        .iter()
        .flat_map(|r| super::library_folders(&r.path))
        .map(|lib| lib.join("steamapps/common").join(&common_name))
        .find(|d| d.join("proton").is_file())
}

/// "proton_experimental" -> "Proton - Experimental", "proton_9" -> "Proton 9.0",
/// "proton_63" -> "Proton 6.3", "proton_513" -> "Proton 5.13", "proton_hotfix" -> "Proton Hotfix".
fn valve_proton_dir(name: &str) -> Option<String> {
    let rest = name.strip_prefix("proton_")?;
    Some(match rest {
        "experimental" => "Proton - Experimental".into(),
        "hotfix" => "Proton Hotfix".into(),
        "next" => "Proton Next".into(),
        r if r.bytes().all(|b| b.is_ascii_digit()) && !r.is_empty() => {
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

fn entry_matches_os(e: &appinfo::LaunchEntry, want_windows: bool) -> bool {
    let os = e.oslist.as_str();
    if os.is_empty() {
        return true;
    }
    os.split(',').any(|o| o.trim() == if want_windows { "windows" } else { "linux" })
}

/// Pick the default launch entry for the OS.
fn pick_entry(entries: &[appinfo::LaunchEntry], want_windows: bool) -> Option<&appinfo::LaunchEntry> {
    // "none" is Steam's plain "Launch" type (used e.g. for Linux entries), not "disabled".
    let primary = |e: &appinfo::LaunchEntry| matches!(e.kind.as_str(), "default" | "" | "none");
    let secondary_ok = |e: &appinfo::LaunchEntry| !matches!(e.kind.as_str(), "server" | "editor" | "manual");
    entries
        .iter()
        .filter(|e| {
            !e.executable.trim().is_empty()
                && e.betakey.is_empty()
                && entry_matches_os(e, want_windows)
                && (primary(e) || secondary_ok(e))
        })
        // Lowest score wins; ties keep Steam's order.
        .min_by_key(|e| (!primary(e), e.oslist.is_empty()))
}

/// Shortcut `LaunchOptions` are part of the shortcut's command: without `%command%` they are
/// the program's arguments; with it, only the words after `%command%` are (wrappers and
/// `VAR=value` before it belong to Steam's launch options and are left to the override).
fn shortcut_args(opts: &str) -> Vec<String> {
    let words = split_args(opts);
    match words.iter().position(|w| w == "%command%") {
        Some(i) => words[i + 1..].to_vec(),
        None => words,
    }
}

/// Resolve a command that starts `appid` without Steam's launch options. See module docs.
pub fn direct_launch(ctx: &ScanContext, appid: u32) -> Result<DirectLaunch> {
    let roots = find_roots(ctx);
    if roots.is_empty() {
        bail!("Steam is not installed");
    }
    let (root, m) = roots
        .iter()
        .find_map(|r| find_manifest(&r.path, appid).map(|m| (r, m)))
        .ok_or_else(|| anyhow!("app {appid} is not installed"))?;
    let install = m.install_path().ok_or_else(|| anyhow!("app {appid} has no install dir"))?;
    if !install.is_dir() {
        bail!("install dir {} is missing", install.display());
    }
    let wanted: HashSet<u32> = [appid].into();
    let infos = appinfo::read_root(&root.path, &wanted).context("reading Steam's app cache")?;
    let info = infos.get(&appid).ok_or_else(|| anyhow!("app {appid} not in Steam's app cache"))?;

    let compat = compat_tools(&root.path);
    let has_linux = info.launch.iter().any(|e| entry_matches_os(e, false) && !e.oslist.is_empty())
        || info.oslist.iter().any(|o| o == "linux");
    let tool = compat.get(&appid).or_else(|| if has_linux { None } else { compat.get(&0) }).cloned();
    let want_windows = tool.is_some() || !has_linux;
    let entry = pick_entry(&info.launch, want_windows)
        .ok_or_else(|| anyhow!("no launch entry for {}", if want_windows { "Windows" } else { "Linux" }))?;

    let rel = entry.executable.trim().replace('\\', "/");
    let exe = install.join(rel.trim_start_matches('/'));
    let cwd = if entry.workingdir.trim().is_empty() {
        exe.parent().map(Path::to_path_buf).unwrap_or_else(|| install.clone())
    } else {
        install.join(entry.workingdir.trim().replace('\\', "/").trim_start_matches('/'))
    };
    let game_args = split_args(&entry.arguments);
    let job = Job {
        appid,
        exe,
        cwd,
        args: game_args,
        tool: want_windows.then(|| tool.unwrap_or_else(|| "proton_experimental".into())),
        compat_data: m.library.join("steamapps/compatdata").join(appid.to_string()),
        install,
        shortcut: false,
    };
    assemble(&roots, root, job)
}

/// Everything needed to build a direct command.
struct Job {
    appid: u32,
    exe: PathBuf,
    cwd: PathBuf,
    args: Vec<String>,
    /// Compat tool to run through (None = native).
    tool: Option<String>,
    compat_data: PathBuf,
    install: PathBuf,
    /// Non-Steam shortcut: no SteamAppId/SteamGameId (they would point at a fake app).
    shortcut: bool,
}

fn assemble(roots: &[SteamRoot], root: &SteamRoot, job: Job) -> Result<DirectLaunch> {
    let id = job.appid.to_string();
    let mut env: Vec<(String, String)> = if job.shortcut {
        vec![]
    } else {
        vec![("SteamAppId".to_string(), id.clone()), ("SteamGameId".to_string(), id.clone())]
    };
    match job.tool {
        Some(tool) => {
            let dir = compat_tool_dir(roots, &tool)
                .ok_or_else(|| anyhow!("compat tool {tool:?} not found; install it or set a custom command"))?;
            env.push(("STEAM_COMPAT_DATA_PATH".into(), job.compat_data.display().to_string()));
            env.push(("STEAM_COMPAT_CLIENT_INSTALL_PATH".into(), root.path.display().to_string()));
            env.push(("STEAM_COMPAT_INSTALL_PATH".into(), job.install.display().to_string()));
            env.push(("STEAM_COMPAT_APP_ID".into(), id));
            let mut args = vec!["waitforexitandrun".to_string(), job.exe.display().to_string()];
            args.extend(job.args);
            Ok(DirectLaunch {
                appid: job.appid,
                program: dir.join("proton").display().to_string(),
                args,
                cwd: job.cwd,
                env,
                compat_tool: Some(tool),
                compat_tool_dir: Some(dir),
            })
        }
        None => {
            if root.kind == InstallKind::Flatpak {
                log::info!("direct launch of a Flatpak Steam game runs it outside the Flatpak sandbox");
            }
            Ok(DirectLaunch {
                appid: job.appid,
                program: job.exe.display().to_string(),
                args: job.args,
                cwd: job.cwd,
                env,
                compat_tool: None,
                compat_tool_dir: None,
            })
        }
    }
}

/// Direct command for a non-Steam shortcut (`shortcut:<appid>`): its `Exe` and `StartDir`
/// without the shortcut's Steam launch options; `.exe` targets run through the compat tool Steam
/// would use (per-shortcut mapping, else the global default, else Proton Experimental).
pub fn direct_launch_shortcut(ctx: &ScanContext, appid: u32) -> Result<DirectLaunch> {
    let roots = find_roots(ctx);
    for root in &roots {
        let Some(user) = super::active_user(&root.path) else {
            continue;
        };
        let Ok(bytes) = crate::scan::util::read_bytes(user.join("config/shortcuts.vdf")) else {
            continue;
        };
        let Ok(list) = super::parse_shortcuts(&bytes) else {
            continue;
        };
        let Some(sc) = list.into_iter().find(|s| s.appid == appid) else {
            continue;
        };
        let exe = PathBuf::from(super::unquote(&sc.exe));
        let start = super::unquote(&sc.start_dir);
        let cwd = if start.is_empty() {
            exe.parent().map(Path::to_path_buf).unwrap_or_default()
        } else {
            PathBuf::from(start)
        };
        let is_windows = exe
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("exe") || e.eq_ignore_ascii_case("bat"));
        let compat = compat_tools(&root.path);
        let tool = if is_windows {
            Some(compat.get(&appid).or(compat.get(&0)).cloned().unwrap_or_else(|| "proton_experimental".into()))
        } else {
            compat.get(&appid).cloned()
        };
        let job = Job {
            appid,
            exe,
            cwd: cwd.clone(),
            args: shortcut_args(&sc.launch_options),
            tool,
            compat_data: root.path.join("steamapps/compatdata").join(appid.to_string()),
            install: cwd,
            shortcut: true,
        };
        return assemble(&roots, root, job);
    }
    bail!("Steam shortcut {appid} not found")
}

/// [`direct_launch`] or [`direct_launch_shortcut`] by Lodestar `source_id`
/// (`"<appid>"` or `"shortcut:<appid>"`).
pub fn direct_launch_for_source_id(ctx: &ScanContext, source_id: &str) -> Result<DirectLaunch> {
    if let Some(id) = source_id.strip_prefix("shortcut:") {
        let id: u32 = id.parse().context("bad shortcut id")?;
        direct_launch_shortcut(ctx, id)
    } else {
        let id: u32 = source_id.parse().context("bad Steam appid")?;
        direct_launch(ctx, id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_quote() {
        assert_eq!(split_args(r#"-a "b c" 'd e' f\ g"#), vec!["-a", "b c", "d e", "f g"]);
        assert_eq!(split_args(r#""" x"#), vec!["", "x"]);
        assert_eq!(split_args("   "), Vec::<String>::new());
        assert_eq!(quote("/a/b-c.exe"), "/a/b-c.exe");
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(quote(""), "''");
    }

    #[test]
    fn valve_proton_names() {
        assert_eq!(valve_proton_dir("proton_experimental").as_deref(), Some("Proton - Experimental"));
        assert_eq!(valve_proton_dir("proton_9").as_deref(), Some("Proton 9.0"));
        assert_eq!(valve_proton_dir("proton_10").as_deref(), Some("Proton 10.0"));
        assert_eq!(valve_proton_dir("proton_63").as_deref(), Some("Proton 6.3"));
        assert_eq!(valve_proton_dir("proton_513").as_deref(), Some("Proton 5.13"));
        assert_eq!(valve_proton_dir("GE-Proton9-1"), None);
    }

    #[test]
    fn picks_os_entry() {
        let e = |exe: &str, os: &str, kind: &str| appinfo::LaunchEntry {
            executable: exe.into(),
            oslist: os.into(),
            kind: kind.into(),
            ..Default::default()
        };
        let v = vec![
            e("game.sh", "linux", "default"),
            e("bin\\game.exe", "windows", "default"),
            e("tool.exe", "windows", "option1"),
        ];
        assert_eq!(pick_entry(&v, false).unwrap().executable, "game.sh");
        assert_eq!(pick_entry(&v, true).unwrap().executable, "bin\\game.exe");
        // "none" is a normal launch type; explicit OS beats any-OS; options lose to defaults.
        let v = vec![e("any.sh", "", "default"), e("opt.sh", "linux", "option1"), e("TombRaider.sh", "linux", "none")];
        assert_eq!(pick_entry(&v, false).unwrap().executable, "TombRaider.sh");
        assert!(pick_entry(&[e("srv", "linux", "server")], false).is_none());
        assert_eq!(pick_entry(&[e("o.exe", "", "option2")], true).unwrap().executable, "o.exe");
    }

    #[test]
    fn shortcut_args_split() {
        assert_eq!(shortcut_args("--game football-life-2026"), vec!["--game", "football-life-2026"]);
        assert_eq!(shortcut_args("mangohud gamescope -f -- %command% -windowed"), vec!["-windowed"]);
        assert!(shortcut_args("").is_empty());
    }
}
