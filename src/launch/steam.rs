//! Steam around Steam games.
//!
//! * **Quiet start**: when Steam isn't running, a Steam game's launch goes through
//!   `steam -silent …` (`flatpak run com.valvesoftware.Steam -silent …`), so Steam starts in the
//!   background and its library window never shows. Tracking stays Steam's reaper.
//! * **Started by Lodestar**: such a cold start writes `<cache_dir>/steam-started.json`
//!   ([`SteamStarted`]): the launch's tag (every process of that Steam inherits it as
//!   `LODESTAR_GAME_TAG`), Flatpak or not, Steam's log folder and where its cloud log stood. Once
//!   the game runs, the Steam client's main processes are added (pid + start time). A Steam the
//!   user opened has no record, or a record that doesn't match it, and is never closed.
//! * **Closing** (Settings → Launchers "Close Steam when the game ends", off by default): after
//!   the game exited, launcher hygiene waits until Steam is idle — no cloud upload still open in
//!   `logs/cloud_log.txt` since the start, and `cloud_log.txt` / `content_log.txt` unchanged for a
//!   few seconds ([`IdleWatch`]), at most two minutes — then runs `steam -shutdown` (Steam quits
//!   by itself and finishes any sync; never a signal). Never while another Steam game runs (a
//!   reaper is alive), while Lodestar follows another Steam launch, or when "Keep Steam running
//!   for controller support" is on.
//!
//! Why closing is off by default: Steam takes a while to start again, Steam Input layouts, the
//! overlay and friends need it, and many people keep it running anyway; the user turns it on.

use super::hygiene::Proc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// The record file in the cache folder.
pub const RECORD_FILE: &str = "steam-started.json";
const FLATPAK_ID: &str = "com.valvesoftware.Steam";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteamPid {
    pub pid: i32,
    /// Start time in clock ticks since boot (a reused pid isn't taken for Steam).
    pub start: u64,
}

/// A Steam client Lodestar started (cold start for a game).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SteamStarted {
    /// `LODESTAR_GAME_TAG` of the launch that started it (Steam's processes inherit it).
    pub game_tag: String,
    /// Started through `flatpak run com.valvesoftware.Steam`.
    pub flatpak: bool,
    /// Steam's `logs` folder, when found.
    pub logs: Option<PathBuf>,
    /// Size of `cloud_log.txt` when Steam was started (cloud syncs are read from there on).
    pub cloud_offset: u64,
    /// The Steam client's main processes, once the game ran.
    pub pids: Vec<SteamPid>,
    /// Unix seconds.
    pub started_at: u64,
}

pub fn record_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join(RECORD_FILE)
}

pub fn read_record(path: &Path) -> Option<SteamStarted> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_record(path: &Path, r: &SteamStarted) -> anyhow::Result<()> {
    crate::fsutil::write_atomic(path, &serde_json::to_vec_pretty(r)?)?;
    Ok(())
}

pub fn remove_record(path: &Path) {
    let _ = std::fs::remove_file(path);
}

// ---------------------------------------------------------------------------------------------
// Command lines

fn base_name(a: &str) -> &str {
    a.rsplit('/').next().unwrap_or(a)
}

/// Where Steam's own arguments start in a command that runs the Steam client (`steam …` → 0,
/// `flatpak run com.valvesoftware.Steam …` → 2); None for anything else (xdg-open, wrappers).
pub fn steam_args_at(program: &str, args: &[String]) -> Option<usize> {
    match base_name(program) {
        "steam" => Some(0),
        "flatpak" if args.len() >= 2 && args[0] == "run" && args[1] == FLATPAK_ID => Some(2),
        _ => None,
    }
}

/// The arguments with `-silent` in front of Steam's own ones (None: not a Steam client command,
/// or already silent).
pub fn with_silent(program: &str, args: &[String]) -> Option<Vec<String>> {
    let at = steam_args_at(program, args)?;
    if args[at..].iter().any(|a| a == "-silent") {
        return None;
    }
    let mut v = args.to_vec();
    v.insert(at, "-silent".to_string());
    Some(v)
}

/// `steam -shutdown` / `flatpak run com.valvesoftware.Steam -shutdown`: asks the running client to
/// quit (it finishes cloud syncs first).
pub fn shutdown_argv(flatpak: bool) -> (String, Vec<String>) {
    if flatpak {
        ("flatpak".into(), vec!["run".into(), FLATPAK_ID.into(), "-shutdown".into()])
    } else {
        ("steam".into(), vec!["-shutdown".into()])
    }
}

// ---------------------------------------------------------------------------------------------
// Processes

/// The Steam client's main process: `…/ubuntu12_32/steam`, `…/Steam/steamrt64/steam` (native or
/// Flatpak, seen from the host). Not steamwebhelper, the reaper or a game.
pub fn is_steam_main(cmdline: &[String]) -> bool {
    cmdline.first().is_some_and(|a| base_name(a) == "steam" && (a.contains("ubuntu12_32") || a.contains("Steam/")))
}

/// Steam is still starting: its launcher scripts (`/usr/bin/steam`, `/usr/lib/steam/steam`,
/// `…/Steam/steam.sh`, `flatpak run com.valvesoftware.Steam`) run before the client does (Steam
/// autostarting at login takes a few seconds there).
pub fn is_steam_starting(cmdline: &[String]) -> bool {
    let script = |a: &String| {
        a == "/usr/bin/steam"
            || a == "/usr/lib/steam/steam"
            || a.ends_with("/Steam/steam.sh")
            || a.ends_with("/usr/games/steam")
    };
    let flatpak = cmdline.first().is_some_and(|a| base_name(a) == "flatpak")
        && cmdline.iter().any(|a| a == "run")
        && cmdline.iter().any(|a| a == FLATPAK_ID);
    cmdline.iter().take(2).any(script) || flatpak
}

/// The Steam client runs, or is starting (from `/proc` command lines): Lodestar then doesn't
/// start (or own) it.
pub fn steam_running<'a>(mut cmdlines: impl Iterator<Item = &'a [String]>) -> bool {
    cmdlines.any(|c| is_steam_main(c) || is_steam_starting(c))
}

/// The Steam that runs now is the one the record is about: a live main process carrying the
/// record's launch tag, or one of its recorded processes.
pub fn is_ours(rec: &SteamStarted, procs: &[Proc]) -> bool {
    procs.iter().filter(|p| !p.zombie && is_steam_main(&p.cmdline)).any(|p| {
        (!rec.game_tag.is_empty() && p.tag.as_deref() == Some(rec.game_tag.as_str()))
            || rec.pids.iter().any(|x| x.pid == p.pid && x.start == p.start)
    })
}

/// The main processes of the Steam a record is about, to note once its game runs: the ones
/// carrying the record's launch tag; for a Flatpak Steam (the tag may not be visible in the
/// sandbox) the untagged ones. A Steam without the tag is someone else's (the user opened it
/// while Lodestar started its own, e.g. Steam autostarting at login): none.
pub fn claim_pids(rec: &SteamStarted, procs: &[Proc]) -> Vec<SteamPid> {
    let mains = || procs.iter().filter(|p| !p.zombie && is_steam_main(&p.cmdline));
    let tagged: Vec<SteamPid> = mains()
        .filter(|p| !rec.game_tag.is_empty() && p.tag.as_deref() == Some(rec.game_tag.as_str()))
        .map(|p| SteamPid { pid: p.pid, start: p.start })
        .collect();
    if !tagged.is_empty() || !rec.flatpak {
        return tagged;
    }
    mains().filter(|p| p.tag.is_none()).map(|p| SteamPid { pid: p.pid, start: p.start }).collect()
}

/// A Steam game still runs (its reaper `… SteamLaunch AppId=<id> -- …` is alive): its AppId.
pub fn other_game(procs: &[Proc]) -> Option<u32> {
    procs.iter().filter(|p| !p.zombie).find_map(|p| super::track::reaper_appid(&p.cmdline))
}

// ---------------------------------------------------------------------------------------------
// Logs

/// Steam's `logs` folder: native `~/.steam/steam` (→ `~/.local/share/Steam`), Flatpak
/// `~/.var/app/com.valvesoftware.Steam/{.local/share,data}/Steam`.
pub fn logs_dir(home: &Path, flatpak: bool) -> Option<PathBuf> {
    let roots: Vec<PathBuf> = if flatpak {
        let app = home.join(".var/app").join(FLATPAK_ID);
        vec![app.join(".local/share/Steam"), app.join("data/Steam"), app.join(".steam/steam")]
    } else {
        vec![home.join(".steam/steam"), home.join(".local/share/Steam"), home.join(".steam/root")]
    };
    roots.into_iter().map(|r| r.join("logs")).find(|d| d.is_dir())
}

pub const CLOUD_LOG: &str = "cloud_log.txt";
pub const CONTENT_LOG: &str = "content_log.txt";

/// Size and change time of the logs that show Steam working after a game (cloud sync,
/// downloads / updates): a change means Steam isn't idle yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogSig(pub Vec<(u64, Option<SystemTime>)>);

impl LogSig {
    pub fn read(logs: &Path) -> LogSig {
        LogSig(
            [CLOUD_LOG, CONTENT_LOG]
                .iter()
                .map(|f| match std::fs::metadata(logs.join(f)) {
                    Ok(m) => (m.len(), m.modified().ok()),
                    Err(_) => (0, None),
                })
                .collect(),
        )
    }
}

pub fn cloud_log_len(logs: &Path) -> u64 {
    std::fs::metadata(logs.join(CLOUD_LOG)).map(|m| m.len()).unwrap_or(0)
}

/// What Steam's cloud log says since `offset` (the whole file when it was rotated meanwhile; at
/// most its last 2 MB).
pub fn cloud_text_since(logs: &Path, offset: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    const MAX: u64 = 2 << 20;
    let Ok(mut f) = std::fs::File::open(logs.join(CLOUD_LOG)) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let from = if offset > len { 0 } else { offset }.max(len.saturating_sub(MAX));
    let mut buf = vec![];
    if f.seek(SeekFrom::Start(from)).is_err() || f.take(MAX).read_to_end(&mut buf).is_err() {
        return String::new();
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// Apps whose cloud upload is still open in `text` (Steam's `cloud_log.txt`): a
/// `[AppID N] Starting sync (up,…)` without a later `[AppID N] Upload complete…` (`… in build
/// list` when nothing had to go up, `…, result OK` after uploading).
pub fn cloud_pending(text: &str) -> Vec<u32> {
    let mut open: Vec<u32> = vec![];
    for line in text.lines() {
        let Some(i) = line.find("[AppID ") else { continue };
        let rest = &line[i + 7..];
        let Some(j) = rest.find(']') else { continue };
        let Ok(app) = rest[..j].trim().parse::<u32>() else { continue };
        let msg = rest[j + 1..].trim_start();
        if let Some(flags) = msg.strip_prefix("Starting sync (") {
            let flags = flags.split(')').next().unwrap_or("");
            if flags.split(',').any(|f| f.trim() == "up") && !open.contains(&app) {
                open.push(app);
            }
        } else if msg.starts_with("Upload complete") {
            open.retain(|a| *a != app);
        }
    }
    open
}

// ---------------------------------------------------------------------------------------------
// Waiting for Steam to be idle

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SteamTiming {
    /// How often Steam is looked at while waiting.
    pub poll: Duration,
    /// Never sooner than this after the wait began (Steam starts its exit sync).
    pub min_wait: Duration,
    /// The logs must have been unchanged this long.
    pub quiet: Duration,
    /// Close anyway after this (`-shutdown` lets Steam finish its sync itself).
    pub max_wait: Duration,
}

impl Default for SteamTiming {
    fn default() -> Self {
        SteamTiming {
            poll: Duration::from_secs(1),
            min_wait: Duration::from_secs(3),
            quiet: Duration::from_secs(5),
            max_wait: Duration::from_secs(120),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Idle {
    /// Look again in `poll`.
    Wait,
    /// Nothing open, logs quiet.
    Ready,
    /// Waited `max_wait`.
    TimedOut,
}

/// Is Steam done after the game? Fed what the logs look like at each poll (a fake clock in tests).
#[derive(Clone, Debug)]
pub struct IdleWatch {
    since: Instant,
    last_change: Instant,
    last: Option<LogSig>,
}

impl IdleWatch {
    pub fn new(now: Instant) -> IdleWatch {
        IdleWatch { since: now, last_change: now, last: None }
    }

    pub fn poll(&mut self, now: Instant, t: &SteamTiming, sig: LogSig, pending: &[u32]) -> Idle {
        if self.last.as_ref().is_some_and(|l| *l != sig) {
            self.last_change = now;
        }
        self.last = Some(sig);
        let waited = now.saturating_duration_since(self.since);
        if waited >= t.max_wait {
            return Idle::TimedOut;
        }
        if waited >= t.min_wait && pending.is_empty() && now.saturating_duration_since(self.last_change) >= t.quiet {
            return Idle::Ready;
        }
        Idle::Wait
    }
}

// ---------------------------------------------------------------------------------------------
// The launch side

/// For a Steam client command when Steam isn't running: the same command starting it quietly.
pub fn cold_start(r: &super::Resolved) -> Option<super::Resolved> {
    // Only Steam client commands look at /proc.
    steam_args_at(&r.program, &r.args)?;
    let procs = super::track::read_procs(true);
    if steam_running(procs.iter().filter(|p| !p.zombie).filter_map(|p| p.cmdline.as_deref())) {
        return None;
    }
    let args = with_silent(&r.program, &r.args)?;
    Some(super::Resolved { args, ..r.clone() })
}

/// The record for a cold start through `program` by the launch tagged `tag`.
pub fn new_record(program: &str, tag: &str, home: Option<&Path>, now_unix: u64) -> SteamStarted {
    let flatpak = base_name(program) == "flatpak";
    let logs = home.and_then(|h| logs_dir(h, flatpak));
    SteamStarted {
        game_tag: tag.to_string(),
        flatpak,
        cloud_offset: logs.as_deref().map(cloud_log_len).unwrap_or(0),
        logs,
        pids: vec![],
        started_at: now_unix,
    }
}

/// Remember that Lodestar starts Steam for the launch `tag` (never in unit tests).
pub fn note_cold_start(program: &str, tag: &str) {
    if cfg!(test) {
        return;
    }
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let rec = new_record(program, tag, dirs::home_dir().as_deref(), now);
    let path = record_path(&crate::config::Paths::detect().cache_dir);
    match write_record(&path, &rec) {
        Ok(()) => log::info!("Steam isn't running: starting it quietly for this game (recorded in {})", path.display()),
        Err(e) => log::warn!("couldn't record that Lodestar starts Steam: {e:#}"),
    }
}

/// Run `steam -shutdown` in the background (never in unit tests).
pub fn shutdown(flatpak: bool) {
    if cfg!(test) {
        return;
    }
    let (program, args) = shutdown_argv(flatpak);
    let mut cmd = std::process::Command::new(&program);
    cmd.args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .env_remove(super::track::TAG_VAR);
    match cmd.spawn() {
        Ok(mut child) => {
            log::info!("asked Steam to quit ({program} {})", args.join(" "));
            let _ = std::thread::Builder::new().name("steam-shutdown".into()).spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => log::warn!("couldn't run {program} -shutdown: {e}"),
    }
}

#[cfg(test)]
mod tests;
