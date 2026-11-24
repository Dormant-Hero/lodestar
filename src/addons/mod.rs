//! External add-ons.
//!
//! An add-on is a folder in `<data_dir>/addons/<id>/` with `addon.toml` and an executable / script in any
//! language. Lodestar runs it as a separate process — never loads code into itself — so a broken add-on
//! can't crash or hang Lodestar and never needs a library from the system.
//!
//! PROTOCOL: `<exec> <command>` with a JSON request on stdin and a JSON reply on stdout (stderr → log),
//! per-call timeout from the manifest (default 30 s, kill on timeout). Environment:
//! LODESTAR_ADDON_ID, LODESTAR_ADDON_DATA (a writable folder for the add-on), LODESTAR_ADDON_SETTINGS (JSON),
//! LODESTAR_VERSION. Commands:
//!   list      → {"games":[{"id","name","installed",?"install_dir",?"launch":{"program","args"[],"cwd"}|{"url"},
//!                ?"cover",?"hero",?"logo",?"icon" (paths or URLs), ?"playtime_secs",?"last_played",
//!                ?"description",?"developer",?"publisher",?"genres"[],?"release_year",?"platform"}]}
//!   launch    {"id"} → {"launch": …} (Lodestar runs and tracks it) or {"handled": true}
//!   install / uninstall {"id"} → {"ok": true} or {"error": "…"}
//!   search    {"name"} → {"results":[{"id","name",?"year"}]}            (metadata add-ons)
//!   fetch     {"id"} | {"name"} → metadata fields as in `list` + art URLs (metadata add-ons)
//!
//! CONTRACT (UI + scanner call these):
//!   Manifest, AddonKind, SettingField, SettingKind, AddonInfo
//!   AddonManager::new(addons_dir), refresh(), list() -> &[AddonInfo], get(id)
//!   install_from(&Path /* folder or .zip */) -> Result<String /*id*/>  (validates, copies, never runs it)
//!   remove(id), call(id, command, serde_json::Value request, &AddonsConfig) -> Result<serde_json::Value>
//!   addons_dir()
//!
//! DETAILS (beyond the contract):
//! - Any reply may be `{"error": "…"}` (also with a non-zero exit status): the call fails with that message.
//! - Extra environment: LODESTAR_ADDON_DIR (the add-on folder, also the working directory) and
//!   LODESTAR_ADDON_COMMAND. Settings are strings ("true" / "false" for toggles), manifest defaults
//!   filled in for keys the user hasn't set.
//! - `call` blocks for up to the timeout: never call it on the UI thread. `AddonManager` is `Clone`,
//!   so clone it into a worker thread.
//! - The outcome of every call is remembered per add-on (`last_error`, `status`), across threads
//!   (the scanner's calls show in the UI's manager too).
//! - A library game without `launch` is started by running `<exec> launch <game id>` directly as the
//!   game process (env LODESTAR_ADDON_LAUNCH=1, empty stdin): the add-on starts the game itself,
//!   ideally with `exec`, so Lodestar tracks it. See [`AddonManager::direct_launch`].
//! - `set_no_launch(true)` (screenshot runs) refuses `launch` / `install` / `uninstall` calls.

pub mod install;
pub mod manifest;
pub mod metadata;
pub mod run;

#[cfg(test)]
mod tests;

use crate::config::AddonsConfig;
use crate::model::LaunchSpec;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AddonKind {
    #[default]
    Library,
    Metadata,
    Generic,
}

impl AddonKind {
    pub fn label(self) -> &'static str {
        match self {
            AddonKind::Library => "Library",
            AddonKind::Metadata => "Metadata source",
            AddonKind::Generic => "Generic",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SettingKind {
    #[default]
    Text,
    /// Shown masked (API keys, tokens).
    Secret,
    Folder,
    Toggle,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
pub struct SettingField {
    pub key: String,
    pub label: String,
    pub kind: SettingKind,
    pub default: String,
    pub help: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub author: String,
    pub version: String,
    pub description: String,
    pub kind: AddonKind,
    /// Program to run, relative to the add-on folder.
    pub exec: String,
    pub timeout_secs: u64,
    /// What it says it needs ("network", "reads ~/.config/foo", …) — shown before enabling.
    pub permissions: Vec<String>,
    pub settings: Vec<SettingField>,
}

#[derive(Clone, Debug)]
pub struct AddonInfo {
    pub manifest: Manifest,
    pub dir: PathBuf,
    /// Problem with this add-on (bad manifest, missing executable …).
    pub error: Option<String>,
    /// Absolute path of the executable (None when `error` is set).
    pub exec_path: Option<PathBuf>,
    /// Error of the most recent call this session (scan, metadata, install …), at refresh time.
    /// [`AddonManager::last_error`] is always current.
    pub last_error: Option<String>,
}

impl AddonInfo {
    pub fn is_usable(&self) -> bool {
        self.error.is_none() && self.exec_path.is_some()
    }
}

/// Outcome of the latest call of one add-on.
#[derive(Clone, Debug)]
pub struct CallStatus {
    pub command: String,
    pub error: Option<String>,
    pub at: Instant,
    pub took: Duration,
}

/// Latest call status per add-on folder, shared by every manager in the process.
static STATUS: parking_lot::Mutex<BTreeMap<PathBuf, CallStatus>> = parking_lot::Mutex::new(BTreeMap::new());

#[derive(Clone)]
pub struct AddonManager {
    dir: PathBuf,
    addons: Vec<AddonInfo>,
    no_launch: bool,
}

/// Commands that start, install or remove something (refused in no-launch runs).
const LAUNCHING_COMMANDS: &[&str] = &["launch", "install", "uninstall"];

impl AddonManager {
    pub fn new(addons_dir: PathBuf) -> AddonManager {
        AddonManager { dir: addons_dir, addons: vec![], no_launch: false }
    }
    pub fn addons_dir(&self) -> &Path {
        &self.dir
    }

    /// Screenshot / test runs: refuse calls that start, install or remove anything.
    pub fn set_no_launch(&mut self, no_launch: bool) {
        self.no_launch = no_launch;
    }

    /// Re-read the add-ons folder (a few small files; cheap enough for the UI thread).
    pub fn refresh(&mut self) {
        let mut out = vec![];
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            self.addons = out;
            return;
        };
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue; // staging / work folders
            }
            let dir = entry.path();
            // A symlinked add-on folder is allowed (developing an add-on in place).
            if !dir.is_dir() {
                continue;
            }
            out.push(load_info(&dir, &name));
        }
        out.sort_by(|a, b| {
            a.manifest.name.to_lowercase().cmp(&b.manifest.name.to_lowercase()).then(a.manifest.id.cmp(&b.manifest.id))
        });
        self.addons = out;
    }

    pub fn list(&self) -> &[AddonInfo] {
        &self.addons
    }
    pub fn get(&self, id: &str) -> Option<&AddonInfo> {
        self.addons.iter().find(|a| a.manifest.id == id)
    }

    /// Enabled, valid add-ons of `kind`.
    pub fn enabled_of_kind(&self, kind: AddonKind, cfg: &AddonsConfig) -> Vec<&AddonInfo> {
        self.addons
            .iter()
            .filter(|a| a.manifest.kind == kind && a.is_usable() && is_enabled(&a.manifest.id, cfg))
            .collect()
    }

    /// Install from a folder or .zip: validated and copied into `<addons_dir>/<id>/`, never run.
    /// Updating needs the same id, name and author; another add-on with the same id must be
    /// removed first. Returns the id. The add-on starts disabled (enabling is the caller's job).
    pub fn install_from(&mut self, src: &Path) -> anyhow::Result<String> {
        let m = install::install(&self.dir, src)?;
        self.refresh();
        Ok(m.id)
    }

    /// Delete the add-on's folder (its data folder and settings are kept for a reinstall).
    pub fn remove(&mut self, id: &str) -> anyhow::Result<()> {
        if !manifest::valid_id(id) {
            anyhow::bail!("invalid add-on id \"{id}\"");
        }
        let path = self.dir.join(id);
        let meta = std::fs::symlink_metadata(&path).map_err(|_| anyhow::anyhow!("add-on \"{id}\" isn't installed"))?;
        if meta.file_type().is_symlink() {
            std::fs::remove_file(&path)?;
        } else {
            std::fs::remove_dir_all(&path).map_err(|e| anyhow::anyhow!("removing \"{id}\": {e}"))?;
        }
        STATUS.lock().remove(&path);
        self.refresh();
        Ok(())
    }

    /// Run `command` of an enabled add-on and return its reply (see the protocol above). Blocks up
    /// to the add-on's timeout: call from a worker thread.
    pub fn call(
        &self,
        id: &str,
        command: &str,
        request: serde_json::Value,
        cfg: &AddonsConfig,
    ) -> anyhow::Result<serde_json::Value> {
        let info = self.get(id).ok_or_else(|| anyhow::anyhow!("add-on \"{id}\" isn't installed"))?;
        if let Some(e) = &info.error {
            anyhow::bail!("{e}");
        }
        if !is_enabled(id, cfg) {
            anyhow::bail!("add-on \"{}\" is disabled", info.manifest.name);
        }
        if command.is_empty() || !command.bytes().all(|b| b.is_ascii_lowercase() || b == b'-' || b == b'_') {
            anyhow::bail!("invalid add-on command \"{command}\"");
        }
        if self.no_launch && LAUNCHING_COMMANDS.contains(&command) {
            anyhow::bail!("add-on {command} is disabled in this run");
        }
        let exec = info.exec_path.clone().ok_or_else(|| anyhow::anyhow!("no executable"))?;
        let settings = settings_for(&info.manifest, cfg);
        let env = self.env_for(info, command, &settings, true);
        let t0 = Instant::now();
        let res = run::invoke(run::Invocation {
            id,
            exec: &exec,
            cwd: &info.dir,
            command,
            stdin: serde_json::to_vec(&request).unwrap_or_else(|_| b"{}".to_vec()),
            env,
            timeout: Duration::from_secs(info.manifest.timeout_secs.max(1)),
        });
        // The status (shown next to the add-on) names the command; the returned error the add-on too.
        let status_error = res.as_ref().err().map(|e| format!("{command}: {e:#}"));
        STATUS.lock().insert(
            info.dir.clone(),
            CallStatus { command: command.to_string(), error: status_error, at: Instant::now(), took: t0.elapsed() },
        );
        let res = res.map_err(|e| anyhow::anyhow!("{}: {command}: {e:#}", info.manifest.name));
        if let Err(e) = &res {
            log::warn!("addon {e:#}");
        }
        res
    }

    /// Error of the add-on's most recent call (None after a successful one).
    pub fn last_error(&self, id: &str) -> Option<String> {
        self.status(id)?.error
    }
    pub fn status(&self, id: &str) -> Option<CallStatus> {
        let info = self.get(id)?;
        STATUS.lock().get(&info.dir).cloned()
    }

    /// `install` / `uninstall` (or any command taking `{"id"}`) for one of the add-on's games;
    /// Ok when the reply is `{"ok": true}` (or any reply without an error). Blocks: worker thread only.
    pub fn game_command(&self, addon: &str, command: &str, game_id: &str, cfg: &AddonsConfig) -> anyhow::Result<()> {
        let reply = self.call(addon, command, serde_json::json!({ "id": game_id }), cfg)?;
        if reply.get("ok").and_then(|v| v.as_bool()) == Some(false) {
            anyhow::bail!("{command} was refused");
        }
        Ok(())
    }

    /// Ask the add-on how to start a game (`launch` command): Some(spec) for Lodestar to run,
    /// None when the add-on started it itself (`{"handled": true}`). Blocks: worker thread only.
    #[cfg(test)]
    pub fn resolve_launch(&self, addon: &str, game_id: &str, cfg: &AddonsConfig) -> anyhow::Result<Option<LaunchSpec>> {
        let reply = self.call(addon, "launch", serde_json::json!({ "id": game_id }), cfg)?;
        if let Some(l) = reply.get("launch") {
            let dir = self.get(addon).map(|a| a.dir.clone()).unwrap_or_default();
            return parse_launch(l, &dir)
                .map(Some)
                .ok_or_else(|| anyhow::anyhow!("launch reply has no program or url"));
        }
        if reply.get("handled").and_then(|v| v.as_bool()) == Some(true) {
            return Ok(None);
        }
        anyhow::bail!("launch reply has neither \"launch\" nor \"handled\"")
    }

    /// The launch stored for a library game whose `list` entry had no `launch`: the add-on run
    /// directly as the game process, `<exec> launch <game id>`, with LODESTAR_ADDON_LAUNCH=1.
    /// Secret settings are left out (this spec is stored in the library database).
    pub fn direct_launch(&self, info: &AddonInfo, game_id: &str, cfg: &AddonsConfig) -> Option<LaunchSpec> {
        let exec = info.exec_path.as_ref()?;
        let mut settings = settings_for(&info.manifest, cfg);
        for f in &info.manifest.settings {
            if f.kind == SettingKind::Secret {
                settings.remove(&f.key);
            }
        }
        let mut env = self.env_for(info, "launch", &settings, false);
        env.push(("LODESTAR_ADDON_LAUNCH".into(), "1".into()));
        Some(LaunchSpec::Command {
            program: exec.to_string_lossy().into_owned(),
            args: vec!["launch".into(), game_id.to_string()],
            cwd: Some(info.dir.clone()),
            env,
        })
    }

    /// The add-on's own writable folder (`LODESTAR_ADDON_DATA`).
    pub fn data_dir(&self, id: &str) -> PathBuf {
        run::data_dir_for(&self.dir, id)
    }

    fn env_for(
        &self,
        info: &AddonInfo,
        command: &str,
        settings: &BTreeMap<String, String>,
        create: bool,
    ) -> Vec<(String, String)> {
        let id = &info.manifest.id;
        let data = self.data_dir(id);
        if create && let Err(e) = std::fs::create_dir_all(&data) {
            log::warn!("addon {id}: data folder {}: {e}", data.display());
        }
        vec![
            ("LODESTAR_ADDON_ID".into(), id.clone()),
            ("LODESTAR_ADDON_DIR".into(), info.dir.to_string_lossy().into_owned()),
            ("LODESTAR_ADDON_DATA".into(), data.to_string_lossy().into_owned()),
            ("LODESTAR_ADDON_SETTINGS".into(), serde_json::to_string(settings).unwrap_or_else(|_| "{}".into())),
            ("LODESTAR_ADDON_COMMAND".into(), command.to_string()),
            ("LODESTAR_VERSION".into(), env!("CARGO_PKG_VERSION").into()),
        ]
    }
}

fn load_info(dir: &Path, folder: &str) -> AddonInfo {
    let last_error = STATUS.lock().get(dir).and_then(|s| s.error.clone());
    let broken = |manifest: Manifest, e: String| AddonInfo {
        manifest,
        dir: dir.to_path_buf(),
        error: Some(e),
        exec_path: None,
        last_error: last_error.clone(),
    };
    let fallback = Manifest { id: folder.to_string(), name: folder.to_string(), ..Default::default() };
    let m = match manifest::load(dir) {
        Ok(m) => m,
        Err(e) => return broken(fallback, format!("{e:#}")),
    };
    if m.id != folder {
        let e = format!("folder name \"{folder}\" doesn't match the add-on id \"{}\"", m.id);
        return broken(Manifest { id: folder.to_string(), ..m }, e);
    }
    match manifest::resolve_exec(dir, &m.exec) {
        Ok(exec) => AddonInfo { manifest: m, dir: dir.to_path_buf(), error: None, exec_path: Some(exec), last_error },
        Err(e) => broken(m, format!("{e:#}")),
    }
}

pub fn is_enabled(id: &str, cfg: &AddonsConfig) -> bool {
    cfg.enabled.iter().any(|e| e == id)
}

/// The add-on's settings: the user's values over the manifest defaults. Keys the manifest doesn't
/// declare are passed too (an add-on may have dropped a field; harmless).
pub fn settings_for(m: &Manifest, cfg: &AddonsConfig) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = m.settings.iter().map(|f| (f.key.clone(), f.default.clone())).collect();
    if let Some(user) = cfg.settings.get(&m.id) {
        for (k, v) in user {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}

/// `"<addon id>:<game id>"` → (addon id, game id).
pub fn split_source_id(source_id: &str) -> Option<(&str, &str)> {
    let (a, g) = source_id.split_once(':')?;
    (manifest::valid_id(a) && !g.is_empty()).then_some((a, g))
}

/// A reply `launch` object: `{"program", "args", "cwd", "env"}` or `{"url"}`. A relative program
/// with a `/` (`./run.sh`, `bin/game`) is resolved against the add-on folder; a bare name is looked
/// up in PATH at launch.
pub fn parse_launch(v: &serde_json::Value, addon_dir: &Path) -> Option<LaunchSpec> {
    if let Some(url) = v.get("url").and_then(|u| u.as_str()).map(str::trim).filter(|u| !u.is_empty()) {
        return url.contains(':').then(|| LaunchSpec::Url(url.to_string()));
    }
    let program = v.get("program")?.as_str()?.trim();
    if program.is_empty() {
        return None;
    }
    let program = if !program.starts_with('/') && program.contains('/') {
        addon_dir.join(program).to_string_lossy().into_owned()
    } else {
        program.to_string()
    };
    let args = v
        .get("args")
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let cwd = v.get("cwd").and_then(|c| c.as_str()).filter(|c| c.starts_with('/')).map(PathBuf::from);
    let env = v
        .get("env")
        .and_then(|e| e.as_object())
        .map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
        .unwrap_or_default();
    Some(LaunchSpec::Command { program, args, cwd, env })
}

impl crate::app::AppState {
    /// Re-read the add-ons folder (the add-ons dialog calls this when it opens / after changes).
    pub fn refresh_addons(&mut self) {
        self.addons.refresh();
        self.metadata.set_addons(&self.addons, &self.config.addons);
    }
}
