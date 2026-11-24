//! Session tracking for launched games: one monitor thread that polls /proc only while
//! something is tracked, plus one blocked waiter thread per spawned child (reaps it, costs nothing).

use super::LaunchEvent;
use crate::model::GameId;
pub use crate::procfs::{ProcInfo, boot_time, clock_ticks, read_procs, read_procs_in, started_ago};
use crate::procfs::{env_entries, read_environ, read_stat};
#[cfg(test)]
pub(crate) use crate::procfs::{fake, parse_stat};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Running while any process in the child's session / process group (or a descendant we saw)
    /// exists. Handles launchers that fork the game and exit (wine, proton, umu, scripts).
    Session,
    /// Running while Steam's reaper (`SteamLaunch … AppId=<appid>`) exists.
    Steam { appid: u32 },
    /// Only the started process itself (TrackingMode::Process).
    Process,
    /// Running while the session runs or any process started since the launch has its
    /// executable, working directory or program path inside the install folder (launchers that
    /// hand off to the real game; TrackingMode::Folder). The folder comes with `Msg::Track`.
    Folder,
    /// Like `Folder`, but ONLY processes inside the install folder count, never the launch's own
    /// session: games started through a store client that stays open (Battle.net, EA app,
    /// Ubisoft Connect …). The game is over when nothing runs from its folder, even though the
    /// client keeps running, and Stop closes just the game (found testing Brawlhalla through
    /// Ubisoft Connect on a real machine).
    GameFolder,
    /// Only reap the child (xdg-open, TrackingMode::None).
    Untracked,
}

pub enum Msg {
    Track {
        id: GameId,
        child: Child,
        mode: Mode,
        log: PathBuf,
        /// Value of TAG_VAR in the launched process' environment.
        tag: String,
        /// Install folder for `Mode::Folder`.
        folder: Option<PathBuf>,
        /// The launcher already sent `Started` (the game runs from the moment it is spawned);
        /// otherwise the monitor sends it once it sees the game.
        announced: bool,
    },
    /// Follow a game that was already running when Lodestar started (see [`find_running`]):
    /// there is no child to reap and no session id to match, only the processes found.
    Adopt(Adopt),
    Stop {
        id: GameId,
    },
    ChildExited {
        token: u64,
        status: Option<ExitStatus>,
    },
}

/// What the monitor needs to follow an adopted game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adopt {
    pub id: GameId,
    /// `Session` (follow the tagged processes and their children), `Process` (only the launch's
    /// first process, as `TrackingMode::Process` does), `Folder` / `GameFolder` (also / only the
    /// install folder) or `Steam` (the reaper).
    pub mode: Mode,
    /// TAG_VAR value to keep following (empty for Steam).
    pub tag: String,
    pub folder: Option<PathBuf>,
    /// (pid, start) of the game's processes found.
    pub known: HashSet<(i32, u64)>,
    /// Earliest start of those (clock ticks since boot): only processes started since then can
    /// belong to the game (folder matches, tags).
    pub since: u64,
    /// How long ago the game started (playtime counts from then).
    pub age: Duration,
}

/// A game counts as "failed" when its command exits non-zero this soon.
const FAIL_WINDOW: Duration = Duration::from_secs(10);

struct Entry {
    token: u64,
    id: GameId,
    mode: Mode,
    /// Session id / process group id of the child (== its pid thanks to setsid()).
    sid: i32,
    /// Start time of the child, to notice its pid (== sid) being reused after it exited.
    child_start: Option<u64>,
    /// The session id now belongs to someone else: follow only known processes.
    sid_reused: bool,
    child_done: Option<Option<ExitStatus>>,
    created: Instant,
    started: Option<Instant>,
    /// (pid, starttime) of processes seen belonging to the game.
    known: HashSet<(i32, u64)>,
    /// Steam reaper pids last seen.
    reapers: Vec<i32>,
    /// When a started Steam game's reapers last disappeared (see STEAM_GRACE).
    gone_since: Option<Instant>,
    /// `TAG_VAR=<tag>` marks every process of this launch (inherited environment).
    tag: String,
    /// (pid, start) of processes whose environment was checked and did not carry the tag.
    untagged: HashSet<(i32, u64)>,
    log: PathBuf,
    stopping: bool,
    /// When SIGTERM was sent; asking again a while after that sends SIGKILL.
    term_sent: Option<Instant>,
    /// SIGKILL already sent by the automatic escalation.
    killed: bool,
    /// `Mode::Folder`: the install folder and what is known about processes in it.
    folder: Option<PathBuf>,
    /// (pid, start) of processes found inside / outside the folder (each looked at once).
    in_folder: HashSet<(i32, u64)>,
    not_in_folder: HashSet<(i32, u64)>,
    /// A process inside the folder was seen at some point.
    folder_seen: bool,
    /// Picked up already running (`Msg::Adopt`): no child of ours, only `known` processes.
    adopted: bool,
}

/// A second stop request at least this long after the SIGTERM escalates to SIGKILL (a quick
/// double press doesn't).
const KILL_AFTER: Duration = Duration::from_secs(3);

/// After Stop, anything of the game still alive this long after SIGTERM gets SIGKILL without
/// the user having to press Stop again (Wine's winedevice.exe ignores SIGTERM once its
/// wineserver is gone — found stopping a Proton game).
/// How long to wait for a game started through a store client (Battle.net, EA, Ubisoft …) to
/// appear: the client may update, ask for a login, or need the user to press Play.
const CLIENT_GAME_WAIT: Duration = Duration::from_secs(600);

const AUTO_KILL_AFTER: Duration = Duration::from_secs(6);

/// Environment variable set on every launched game; its value identifies the launch.
/// Wine/Proton put the game and wineserver in sessions of their own, reparented away from the
/// launcher, so session and parent links alone lose them (found launching a Proton game
/// directly: Stop ended the launcher while the game kept running).
pub const TAG_VAR: &str = "LODESTAR_GAME_TAG";

/// Steam can run short-lived steps under the game's own AppId before the real game starts
/// (seat checkout, first-run setup, install scripts) and some games restart themselves through
/// Steam. So a Steam game only counts as exited once no reaper for its AppId has been seen for
/// this long. Found on a real launch: CHRONO TRIGGER's first reaper ended after ~14 s, the game
/// itself started a few seconds later.
#[cfg(not(test))]
const STEAM_GRACE: Duration = Duration::from_secs(30);
#[cfg(test)]
const STEAM_GRACE: Duration = Duration::from_secs(2);

/// `Mode::Folder`: the game only counts as exited once nothing of it (session or folder) has
/// been seen for this long, so a launcher that hands off to the game through some other process
/// (a service, a restart) isn't taken for the end of the session.
#[cfg(not(test))]
const FOLDER_GRACE: Duration = Duration::from_secs(10);
#[cfg(test)]
const FOLDER_GRACE: Duration = Duration::from_secs(1);

impl Entry {
    /// Session/process-group id to match, unless it was reused.
    fn session(&self) -> Option<i32> {
        (!self.sid_reused).then_some(self.sid)
    }
}

pub fn spawn(ctx: egui::Context, ev_tx: Sender<LaunchEvent>, every: Duration, steam_timeout: Duration) -> Sender<Msg> {
    let (tx, rx) = unbounded();
    let tx2 = tx.clone();
    let r = std::thread::Builder::new()
        .name("launch-monitor".into())
        .spawn(move || Monitor { ctx, ev_tx, tx: tx2, every, steam_timeout, entries: vec![], next_token: 1 }.run(rx));
    if let Err(e) = r {
        log::error!("couldn't start the launch monitor thread: {e}");
    }
    tx
}

struct Monitor {
    ctx: egui::Context,
    ev_tx: Sender<LaunchEvent>,
    tx: Sender<Msg>,
    every: Duration,
    steam_timeout: Duration,
    entries: Vec<Entry>,
    next_token: u64,
}

impl Monitor {
    fn run(mut self, rx: Receiver<Msg>) {
        let mut last_scan = Instant::now();
        loop {
            let msg = if self.entries.is_empty() {
                // Nothing tracked: sleep until the next launch.
                match rx.recv() {
                    Ok(m) => Some(m),
                    Err(_) => return,
                }
            } else {
                let wait = (last_scan + self.every).saturating_duration_since(Instant::now());
                match rx.recv_timeout(wait) {
                    Ok(m) => Some(m),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            };
            let mut scan_now = false;
            if let Some(m) = msg {
                scan_now = self.handle(m);
            }
            if !self.entries.is_empty() && (scan_now || last_scan.elapsed() >= self.every) {
                self.scan();
                last_scan = Instant::now();
            }
        }
    }

    /// Returns true when a scan should happen right away.
    fn handle(&mut self, m: Msg) -> bool {
        match m {
            Msg::Track { id, mut child, mode, log, tag, folder, announced } => {
                let token = self.next_token;
                self.next_token += 1;
                let sid = child.id() as i32;
                let tx = self.tx.clone();
                let waiter = std::thread::Builder::new().name(format!("reap-{sid}")).spawn(move || {
                    let status = child.wait().ok();
                    let _ = tx.send(Msg::ChildExited { token, status });
                });
                if let Err(e) = waiter {
                    log::warn!("couldn't start a waiter thread: {e}");
                }
                if mode == Mode::Untracked {
                    return false;
                }
                // Relaunching the same game while it runs: follow the newest session.
                self.entries.retain(|e| e.id != id);
                self.entries.push(Entry {
                    token,
                    id,
                    mode,
                    sid,
                    child_start: read_stat(sid).map(|p| p.start),
                    sid_reused: false,
                    child_done: None,
                    created: Instant::now(),
                    started: None,
                    known: HashSet::new(),
                    reapers: vec![],
                    gone_since: None,
                    tag,
                    untagged: HashSet::new(),
                    log,
                    stopping: false,
                    term_sent: None,
                    killed: false,
                    folder,
                    in_folder: HashSet::new(),
                    not_in_folder: HashSet::new(),
                    folder_seen: false,
                    adopted: false,
                });
                if announced && let Some(e) = self.entries.last_mut() {
                    e.started = Some(e.created);
                }
                true
            }
            Msg::Adopt(a) => {
                if self.entries.iter().any(|e| e.id == a.id) {
                    log::info!("game {} is already followed; not adopting it again", a.id);
                    return false;
                }
                let token = self.next_token;
                self.next_token += 1;
                self.entries.push(adopted_entry(token, a, Instant::now()));
                true
            }
            Msg::ChildExited { token, status } => {
                let Some(e) = self.entries.iter_mut().find(|e| e.token == token) else {
                    return false;
                };
                e.child_done = Some(status);
                if let (Mode::Steam { .. }, Some(st)) = (e.mode, status)
                    && !st.success()
                {
                    log::warn!("steam command for game {} exited with {st}", e.id);
                }
                matches!(e.mode, Mode::Session | Mode::Process | Mode::Folder | Mode::GameFolder)
            }
            Msg::Stop { id } => {
                let Some(i) = self.entries.iter().position(|e| e.id == id) else {
                    return false;
                };
                if matches!(self.entries[i].mode, Mode::Steam { .. }) && self.entries[i].started.is_none() {
                    // Still waiting for Steam (updates, login …): just stop waiting.
                    self.entries.remove(i);
                    self.emit(LaunchEvent::Exited { id, secs: 0 });
                    return false;
                }
                let procs = read_procs(matches!(self.entries[i].mode, Mode::Steam { .. }));
                if matches!(self.entries[i].mode, Mode::Folder | Mode::GameFolder) {
                    let e = &mut self.entries[i];
                    let alive = folder_procs(e, &procs, Path::new("/proc"));
                    e.reapers = alive;
                }
                let e = &mut self.entries[i];
                let sig = match e.term_sent {
                    Some(t) if t.elapsed() >= KILL_AFTER => libc::SIGKILL,
                    Some(_) => return false, // just asked; give it time
                    None => libc::SIGTERM,
                };
                e.stopping = true;
                e.term_sent.get_or_insert_with(Instant::now);
                stop_entry(e, &procs, sig);
                // Still waiting for a store client (login, update …): end the launch's own helper
                // (`lodestar --client-launch …`) too, or it would hand the client the request
                // later (one game at a time switched to another game meanwhile). Only that
                // process: the client it started runs in a process group of its own and stays.
                if e.mode == Mode::GameFolder
                    && e.started.is_none()
                    && e.child_done.is_none()
                    && !e.sid_reused
                    && e.child_start.is_some()
                    && read_stat(e.sid).is_some_and(|p| Some(p.start) == e.child_start && !p.zombie)
                {
                    log::info!("ending the launch helper of game {} (pid {})", e.id, e.sid);
                    // SAFETY: kill(2) on our own child, checked to be the same process above.
                    unsafe { libc::kill(e.sid, sig) };
                }
                true
            }
        }
    }

    fn emit(&self, ev: LaunchEvent) {
        let _ = self.ev_tx.send(ev);
        self.ctx.request_repaint();
    }

    fn scan(&mut self) {
        let want_cmdline = self.entries.iter().any(|e| matches!(e.mode, Mode::Steam { .. }));
        let procs = read_procs(want_cmdline);
        let mut events = vec![];
        let timeout = self.steam_timeout;
        let now = Instant::now();
        self.entries.retain_mut(|e| {
            let done = match e.mode {
                Mode::Session => update_session(e, &procs, &mut events),
                Mode::Process => update_process(e, &procs, &mut events),
                Mode::Folder | Mode::GameFolder => update_folder(e, &procs, &mut events, now, timeout),
                Mode::Steam { appid } => update_steam(e, appid, &procs, &mut events, now, timeout),
                Mode::Untracked => true,
            };
            if !done && e.stopping && !e.killed && e.term_sent.is_some_and(|t| t.elapsed() >= AUTO_KILL_AFTER) {
                log::info!("game {} ignored the stop request; killing what is left", e.id);
                e.killed = true;
                stop_entry(e, &procs, libc::SIGKILL);
            }
            !done
        });
        for ev in events {
            self.emit(ev);
        }
    }
}

/// `path` with the home directory shown as `~` (shorter toasts).
fn tilde(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if !rest.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

/// Returns true when the session is over.
fn update_session(e: &mut Entry, procs: &[ProcInfo], events: &mut Vec<LaunchEvent>) -> bool {
    if !e.sid_reused
        && e.child_done.is_some()
        && let Some(start) = e.child_start
        && procs.iter().any(|p| p.pid == e.sid && p.start != start)
    {
        e.sid_reused = true;
    }
    adopt_tagged(e, procs, Path::new("/proc"));
    let alive = session_members(e.session(), &e.known, procs);
    e.known = alive.iter().map(|p| (p.pid, p.start)).collect();
    if !alive.is_empty() {
        return false;
    }
    let Some(status) = e.child_done else {
        // The child is gone from /proc listings only as a zombie; wait for its waiter thread.
        return false;
    };
    finish_entry(e, status, e.created.elapsed(), events);
    true
}

/// The session is over: `Failed` first when the command failed right away, then `Exited`.
fn finish_entry(e: &Entry, status: Option<ExitStatus>, ran: Duration, events: &mut Vec<LaunchEvent>) {
    let secs = ran.as_secs_f64().round() as u64;
    if let Some(st) = status
        && !st.success()
        && !e.stopping
        && e.created.elapsed() < FAIL_WINDOW
    {
        let how = match st.code() {
            Some(c) => format!("exited with code {c}"),
            None => format!("{st}"),
        };
        let tail = log_tail(&e.log).map(|t| format!(": {t}")).unwrap_or_default();
        events.push(LaunchEvent::Failed {
            id: e.id,
            error: format!("{how}{tail} (log: {})", tilde(&e.log, dirs::home_dir().as_deref())),
        });
    }
    events.push(LaunchEvent::Exited { id: e.id, secs });
}

/// `Mode::Process`: over when the started process is (an adopted one: when its known process,
/// the launch's first one, is gone; leftovers it started don't count, as for a launch).
fn update_process(e: &mut Entry, procs: &[ProcInfo], events: &mut Vec<LaunchEvent>) -> bool {
    if e.adopted && procs.iter().any(|p| !p.zombie && e.known.contains(&(p.pid, p.start))) {
        return false;
    }
    let Some(status) = e.child_done else {
        return false;
    };
    finish_entry(e, status, e.created.elapsed(), events);
    true
}

/// `Mode::Folder`: running while the launch's session or anything inside the install folder
/// runs. Returns true when the session is over.
fn update_folder(
    e: &mut Entry,
    procs: &[ProcInfo],
    events: &mut Vec<LaunchEvent>,
    now: Instant,
    timeout: Duration,
) -> bool {
    if !e.sid_reused
        && e.child_done.is_some()
        && let Some(start) = e.child_start
        && procs.iter().any(|p| p.pid == e.sid && p.start != start)
    {
        e.sid_reused = true;
    }
    adopt_tagged(e, procs, Path::new("/proc"));
    let session = session_members(e.session(), &e.known, procs);
    e.known = session.iter().map(|p| (p.pid, p.start)).collect();
    let in_folder = folder_procs(e, procs, Path::new("/proc"));
    let game_only = e.mode == Mode::GameFolder;
    let alive = !in_folder.is_empty() || (!game_only && !session.is_empty());
    e.reapers = in_folder;
    if alive {
        e.gone_since = None;
        // Not announced by the launcher (started through Steam …): the game is up once
        // something of it shows up in its folder.
        if e.started.is_none() && !e.reapers.is_empty() {
            e.started = Some(now);
            events.push(LaunchEvent::Started { id: e.id });
        }
        return false;
    }
    match e.started {
        Some(t) => {
            // The launcher's own process may still be a zombie waiting for its waiter thread.
            // (Not for a store client: it stays open after the game.)
            if e.child_done.is_none() && !game_only {
                return false;
            }
            let gone = *e.gone_since.get_or_insert(now);
            if !e.stopping && now.duration_since(gone) < FOLDER_GRACE {
                return false;
            }
            let status = e.child_done.flatten();
            // Only a launch that never got as far as the folder can have failed.
            let status = if e.folder_seen { None } else { status };
            finish_entry(e, status, gone.saturating_duration_since(t), events);
            true
        }
        None => {
            // A store client may need the user to press Play (Battle.net ignores launch
            // requests in some versions), log in or update first: give it longer.
            let timeout = if game_only { timeout.max(CLIENT_GAME_WAIT) } else { timeout };
            if e.stopping || now.duration_since(e.created) > timeout {
                log::info!("game {} never showed up in its folder; no longer tracking it", e.id);
                events.push(LaunchEvent::Exited { id: e.id, secs: 0 });
                true
            } else {
                false
            }
        }
    }
}

/// Live processes started since the launch whose executable, working directory or program path
/// (argv[0], also Wine's `Z:\…` form) is inside the entry's folder. Each process is looked at
/// once (a few readlinks), so this stays cheap while a game runs.
fn folder_procs(e: &mut Entry, procs: &[ProcInfo], proc_root: &Path) -> Vec<i32> {
    let Some(dir) = e.folder.clone() else {
        return vec![];
    };
    let since = e.child_start.unwrap_or(0);
    let present: HashSet<(i32, u64)> = procs.iter().map(|p| (p.pid, p.start)).collect();
    e.in_folder.retain(|k| present.contains(k));
    e.not_in_folder.retain(|k| present.contains(k));
    let me = std::process::id() as i32;
    let mut out = vec![];
    for p in procs {
        let key = (p.pid, p.start);
        if p.zombie || p.start < since || p.pid == me {
            continue;
        }
        if e.in_folder.contains(&key) {
            out.push(p.pid);
            continue;
        }
        if e.not_in_folder.contains(&key) {
            continue;
        }
        if runs_from(proc_root, p.pid, None, &dir) {
            e.in_folder.insert(key);
            e.folder_seen = true;
            out.push(p.pid);
        } else {
            e.not_in_folder.insert(key);
        }
    }
    out
}

/// Process `pid`'s executable, working directory or program path (argv[0], also Wine's `Z:\…`
/// form) is inside `dir` (a few readlinks). `cmdline`: the process' command line when already
/// read (else it is read here).
pub(super) fn runs_from(proc_root: &Path, pid: i32, cmdline: Option<&[String]>, dir: &Path) -> bool {
    let base = proc_root.join(pid.to_string());
    let inside = |l: Option<PathBuf>| l.is_some_and(|l| is_inside(&l, dir));
    let argv0 = || match cmdline {
        Some(c) => unix_path_of(c.first()?),
        None => {
            let b = std::fs::read(base.join("cmdline")).ok()?;
            let first = b.split(|&c| c == 0).next()?;
            unix_path_of(&String::from_utf8_lossy(first))
        }
    };
    inside(std::fs::read_link(base.join("exe")).ok())
        || inside(std::fs::read_link(base.join("cwd")).ok())
        || inside(argv0())
}

/// `path` is `dir` or below it (component-wise, so `/games/foo2` isn't inside `/games/foo`).
pub(super) fn is_inside(path: &Path, dir: &Path) -> bool {
    !dir.as_os_str().is_empty() && dir != Path::new("/") && path.starts_with(dir)
}

/// A program path from argv[0]: absolute Unix paths as they are, Wine's `Z:\path\game.exe`
/// mapped back to `/path/game.exe`; anything else (relative, other drives) is None.
pub(super) fn unix_path_of(s: &str) -> Option<PathBuf> {
    if s.starts_with('/') {
        return Some(PathBuf::from(s));
    }
    let rest = s.strip_prefix("Z:").or_else(|| s.strip_prefix("z:"))?;
    let p = rest.replace('\\', "/");
    p.starts_with('/').then(|| PathBuf::from(p))
}

fn update_steam(
    e: &mut Entry,
    appid: u32,
    procs: &[ProcInfo],
    events: &mut Vec<LaunchEvent>,
    now: Instant,
    timeout: Duration,
) -> bool {
    let needle = format!("AppId={appid}");
    e.reapers = procs
        .iter()
        .filter(|p| !p.zombie && p.cmdline.as_ref().is_some_and(|c| is_reaper_for(c, &needle)))
        .map(|p| p.pid)
        .collect();
    match (e.reapers.is_empty(), e.started) {
        (false, None) => {
            e.started = Some(now);
            e.gone_since = None;
            events.push(LaunchEvent::Started { id: e.id });
            false
        }
        (false, Some(_)) => {
            e.gone_since = None;
            false
        }
        (true, Some(t)) => {
            let gone = *e.gone_since.get_or_insert(now);
            // A stop the user asked for needs no grace period.
            if !e.stopping && now.duration_since(gone) < STEAM_GRACE {
                return false;
            }
            events.push(LaunchEvent::Exited {
                id: e.id,
                secs: gone.saturating_duration_since(t).as_secs_f64().round() as u64,
            });
            true
        }
        (true, None) => {
            if now.duration_since(e.created) > timeout {
                log::info!("game {} never showed up in Steam; no longer tracking it", e.id);
                events.push(LaunchEvent::Exited { id: e.id, secs: 0 });
                true
            } else {
                false
            }
        }
    }
}

/// Steam's reaper: `…/reaper SteamLaunch AppId=<id> -- <game command…>`, also when it is itself
/// started through a wrapper (`steam-launch-wrapper -- …/reaper SteamLaunch AppId=<id> -- …`).
/// `AppId=` must belong to that `SteamLaunch` (before the next `--`), so a game whose own
/// arguments happen to contain `SteamLaunch` or `AppId=…` isn't mistaken for a reaper.
fn is_reaper_for(cmdline: &[String], needle: &str) -> bool {
    cmdline
        .iter()
        .enumerate()
        .any(|(i, a)| a == "SteamLaunch" && cmdline[i + 1..].iter().take_while(|w| *w != "--").any(|w| w == needle))
}

/// Add processes started since the launch whose environment carries this launch's tag to
/// `e.known` (session_members then follows them and their children). Each new process'
/// environment is read once; only processes started after the launch are looked at.
fn adopt_tagged(e: &mut Entry, procs: &[ProcInfo], proc_root: &Path) {
    if e.tag.is_empty() {
        return;
    }
    let since = e.child_start.unwrap_or(0);
    let present: HashSet<(i32, u64)> = procs.iter().map(|p| (p.pid, p.start)).collect();
    e.untagged.retain(|k| present.contains(k));
    for p in procs {
        let key = (p.pid, p.start);
        if p.zombie || p.start < since || e.known.contains(&key) || e.untagged.contains(&key) {
            continue;
        }
        let tagged = read_environ(proc_root, p.pid)
            .is_some_and(|env| env_entries(&env).any(|(k, v)| k == TAG_VAR.as_bytes() && v == e.tag.as_bytes()));
        if tagged {
            e.known.insert(key);
        } else {
            e.untagged.insert(key);
        }
    }
}

/// Processes that belong to the session `sid`: same session or process group, previously seen
/// ones, and descendants of any of those (catches children that called setsid() themselves).
fn session_members<'a>(sid: Option<i32>, known: &HashSet<(i32, u64)>, procs: &'a [ProcInfo]) -> Vec<&'a ProcInfo> {
    let mut children: HashMap<i32, Vec<usize>> = HashMap::new();
    for (i, p) in procs.iter().enumerate() {
        children.entry(p.ppid).or_default().push(i);
    }
    let mut member = vec![false; procs.len()];
    let mut stack: Vec<usize> = procs
        .iter()
        .enumerate()
        .filter(|(_, p)| sid.is_some_and(|s| p.sid == s || p.pgrp == s) || known.contains(&(p.pid, p.start)))
        .map(|(i, _)| i)
        .collect();
    while let Some(i) = stack.pop() {
        if member[i] {
            continue;
        }
        member[i] = true;
        if let Some(c) = children.get(&procs[i].pid) {
            stack.extend(c.iter().copied().filter(|&j| !member[j]));
        }
    }
    procs.iter().zip(member).filter(|(p, m)| *m && !p.zombie).map(|(p, _)| p).collect()
}

fn descendants<'a>(roots: &[i32], procs: &'a [ProcInfo]) -> Vec<&'a ProcInfo> {
    let mut children: HashMap<i32, Vec<&ProcInfo>> = HashMap::new();
    for p in procs {
        children.entry(p.ppid).or_default().push(p);
    }
    let mut out: Vec<&ProcInfo> = vec![];
    let mut seen: HashSet<i32> = HashSet::new();
    let mut stack: Vec<i32> = roots.to_vec();
    while let Some(pid) = stack.pop() {
        if !seen.insert(pid) {
            continue;
        }
        if let Some(c) = children.get(&pid) {
            for p in c {
                out.push(p);
                stack.push(p.pid);
            }
        }
    }
    out
}

fn stop_entry(e: &Entry, procs: &[ProcInfo], sig: libc::c_int) {
    let me = std::process::id() as i32;
    // SAFETY: getpgrp/getsid/kill are plain syscalls.
    let (my_pgrp, my_sid) = unsafe { (libc::getpgrp(), libc::getsid(0)) };
    let (groups, pids) = stop_targets(e, procs, Myself { pid: me, pgrp: my_pgrp, sid: my_sid });
    for g in groups {
        unsafe { libc::kill(-g, sig) };
    }
    for p in pids {
        unsafe { libc::kill(p, sig) };
    }
    log::info!("sent signal {sig} to game {}", e.id);
}

/// Lodestar's own pid, process group and session (never signalled).
#[derive(Clone, Copy, Debug)]
struct Myself {
    pid: i32,
    pgrp: i32,
    sid: i32,
}

/// What Stop signals for `e`: whole process groups (each only when every live member is the
/// game's) and single pids, sorted. Never pid / group 0 or 1, nor Lodestar itself.
fn stop_targets(e: &Entry, procs: &[ProcInfo], my: Myself) -> (Vec<i32>, Vec<i32>) {
    let Myself { pid: me, pgrp: my_pgrp, sid: my_sid } = my;
    let mut groups: HashSet<i32> = HashSet::new();
    let mut pids: HashSet<i32> = HashSet::new();
    match e.mode {
        // Only the game itself: the store client it was started through stays open.
        Mode::GameFolder => {
            for p in procs.iter().filter(|p| e.reapers.contains(&p.pid)) {
                pids.insert(p.pid);
            }
            for p in descendants(&e.reapers, procs) {
                pids.insert(p.pid);
            }
        }
        Mode::Session | Mode::Process | Mode::Folder => {
            for p in session_members(e.session(), &e.known, procs) {
                groups.insert(p.pgrp);
                pids.insert(p.pid);
            }
            if e.mode == Mode::Folder {
                for p in procs.iter().filter(|p| e.reapers.contains(&p.pid)) {
                    pids.insert(p.pid);
                }
                for p in descendants(&e.reapers, procs) {
                    pids.insert(p.pid);
                }
            }
        }
        Mode::Steam { .. } => {
            for p in procs.iter().filter(|p| e.reapers.contains(&p.pid)) {
                groups.insert(p.pgrp);
                pids.insert(p.pid);
            }
            for p in descendants(&e.reapers, procs) {
                pids.insert(p.pid);
            }
        }
        Mode::Untracked => {}
    }
    // Signal a whole process group (catches processes forked since the scan) only when every
    // live member of it is the game's: Steam's reaper may share the Steam client's group, and a
    // game process may have joined some unrelated group.
    let mut out_groups: Vec<i32> = groups
        .into_iter()
        .filter(|&g| {
            let ours = procs.iter().filter(|p| p.pgrp == g && !p.zombie).all(|p| pids.contains(&p.pid));
            ours && g > 1 && g != my_pgrp && g != my_sid && g != me
        })
        .collect();
    let mut out_pids: Vec<i32> = pids.into_iter().filter(|&p| p > 1 && p != me).collect();
    out_groups.sort_unstable();
    out_pids.sort_unstable();
    (out_groups, out_pids)
}

/// Last meaningful line of a log file (bounded read).
fn log_tail(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(4096);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let line = text.lines().rev().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("# "))?;
    Some(line.chars().take(200).collect())
}

// ----- games that already run (Lodestar restarted while they play) -----

/// The monitor's entry for an adopted game: no child to reap (`child_done` is set), no session
/// id to match (`sid_reused`, so session matching follows only the known processes and their
/// children: session / group 0 would be every kernel thread), started `a.age` before `now`.
fn adopted_entry(token: u64, a: Adopt, now: Instant) -> Entry {
    let start = now.checked_sub(a.age).unwrap_or(now);
    let steam = matches!(a.mode, Mode::Steam { .. });
    let mut reapers: Vec<i32> = if steam { a.known.iter().map(|k| k.0).collect() } else { vec![] };
    reapers.sort_unstable();
    // `Process`: `known` is the launch's first process only, the game ends with it.
    Entry {
        token,
        id: a.id,
        mode: a.mode,
        sid: 0,
        child_start: Some(a.since),
        sid_reused: true,
        child_done: Some(None),
        created: start,
        started: Some(start),
        known: if steam { HashSet::new() } else { a.known },
        reapers,
        gone_since: None,
        tag: a.tag,
        untagged: HashSet::new(),
        log: PathBuf::new(),
        stopping: false,
        term_sent: None,
        killed: false,
        folder: a.folder,
        in_folder: HashSet::new(),
        not_in_folder: HashSet::new(),
        folder_seen: false,
        adopted: true,
    }
}

/// Something running that this Lodestar didn't start (see [`find_running`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub what: FoundKind,
    /// (pid, start) of its processes, sorted.
    pub procs: Vec<(i32, u64)>,
    /// Earliest start of those (clock ticks since boot).
    pub since: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FoundKind {
    /// Processes whose environment carries `TAG_VAR=<tag>` of a launch by another Lodestar run.
    /// `foreign`: that Lodestar still runs (another window, a `--play` shortcut) and follows them.
    Tagged { id: GameId, tag: String, foreign: bool },
    /// Steam's reaper for this AppId.
    Steam { appid: u32 },
}

/// A launch tag `<lodestar pid>-<game id>-<nanos>` → (lodestar pid, game id).
pub fn parse_tag(tag: &str) -> Option<(i32, GameId)> {
    let mut it = tag.splitn(3, '-');
    let pid = it.next()?.parse().ok()?;
    let id = it.next()?.parse().ok()?;
    it.next()?.parse::<u128>().ok()?;
    Some((pid, id))
}

/// The AppId of Steam's reaper command line (`… SteamLaunch AppId=<id> -- …`, as
/// [`is_reaper_for`] matches it).
pub(crate) fn reaper_appid(cmdline: &[String]) -> Option<u32> {
    cmdline.iter().enumerate().find_map(|(i, a)| {
        if a != "SteamLaunch" {
            return None;
        }
        cmdline[i + 1..]
            .iter()
            .take_while(|w| *w != "--")
            .find_map(|w| w.strip_prefix("AppId=")?.parse::<u32>().ok())
            .filter(|&n| n != 0)
    })
}

/// `TAG_VAR` in process `pid`'s environment (None: not set, or not readable: another user's).
fn tag_of(proc_root: &Path, pid: i32) -> Option<String> {
    let env = read_environ(proc_root, pid)?;
    env_entries(&env)
        .find_map(|(k, v)| (k == TAG_VAR.as_bytes()).then_some(v))
        .map(|v| String::from_utf8_lossy(v).into_owned())
}

/// `pid` is a live Lodestar process (by its executable or argv[0]; `procs` with command lines).
fn is_lodestar(procs: &[ProcInfo], proc_root: &Path, pid: i32) -> bool {
    let Some(p) = procs.iter().find(|p| p.pid == pid && !p.zombie) else {
        return false;
    };
    let named = |s: &Path| {
        s.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.to_ascii_lowercase().starts_with("lodestar"))
    };
    std::fs::read_link(proc_root.join(pid.to_string()).join("exe")).ok().is_some_and(|e| named(&e))
        || p.cmdline.as_ref().and_then(|c| c.first()).is_some_and(|a| named(Path::new(a)))
}

/// Games running that this Lodestar (pid `me`) didn't start this run: processes tagged by an
/// earlier (or another) Lodestar run, and Steam reapers. `procs` must come with command lines.
/// Tags of `me` are skipped (this run follows its own launches, or already saw them end).
/// Reads each process' environment once (a few ms for a desktop's few hundred processes).
pub fn find_running(procs: &[ProcInfo], proc_root: &Path, me: i32) -> Vec<Found> {
    use std::collections::BTreeMap;
    let mut tagged: BTreeMap<String, Vec<(i32, u64)>> = BTreeMap::new();
    let mut steam: BTreeMap<u32, Vec<(i32, u64)>> = BTreeMap::new();
    for p in procs {
        if p.zombie || p.pid == me {
            continue;
        }
        if let Some(appid) = p.cmdline.as_deref().and_then(reaper_appid) {
            steam.entry(appid).or_default().push((p.pid, p.start));
        }
        if let Some(tag) = tag_of(proc_root, p.pid)
            && parse_tag(&tag).is_some_and(|(lpid, _)| lpid != me)
        {
            tagged.entry(tag).or_default().push((p.pid, p.start));
        }
    }
    let found = |what: FoundKind, mut procs: Vec<(i32, u64)>| {
        procs.sort_unstable();
        let since = procs.iter().map(|k| k.1).min().unwrap_or(0);
        Found { what, procs, since }
    };
    let mut out: Vec<Found> = steam.into_iter().map(|(appid, v)| found(FoundKind::Steam { appid }, v)).collect();
    for (tag, v) in tagged {
        let Some((lpid, id)) = parse_tag(&tag) else { continue };
        let foreign = is_lodestar(procs, proc_root, lpid);
        out.push(found(FoundKind::Tagged { id, tag, foreign }, v));
    }
    out
}

/// Something that looks like a game runs that Lodestar may not follow (the background
/// controller's safety check, input/gate.rs): a Steam reaper (any AppId, in the library or not),
/// a process tagged by another Lodestar run (`found`, from [`find_running`]), a Windows program
/// (Wine / Proton: argv[0] ends in `.exe`), a known emulator, a game-streaming / cloud-gaming
/// app, or a program running from one of `game_dirs` (installed games' folders). Errs on the
/// side of "a game runs": then only the Guide button reaches an unfocused Lodestar.
pub fn others_running(found: &[Found], procs: &[ProcInfo], proc_root: &Path, me: i32, game_dirs: &[PathBuf]) -> bool {
    if !found.is_empty() {
        return true;
    }
    procs.iter().filter(|p| !p.zombie && p.pid != me).any(|p| {
        let argv = p.cmdline.as_deref().unwrap_or_default();
        if reaper_appid(argv).is_some() || argv.first().is_some_and(|a| looks_like_game_program(a)) {
            return true;
        }
        !game_dirs.is_empty()
            && std::fs::read_link(proc_root.join(p.pid.to_string()).join("exe"))
                .is_ok_and(|exe| game_dirs.iter().any(|d| exe.starts_with(d)))
    })
}

/// argv[0] of a Windows program (Wine / Proton), a known emulator, or a game-streaming /
/// cloud-gaming app.
fn looks_like_game_program(argv0: &str) -> bool {
    use std::sync::OnceLock;
    static EMULATORS: OnceLock<HashSet<String>> = OnceLock::new();
    // Wine shows Windows paths (`C:\Games\x.exe`, `Z:\…`): the name after the last separator.
    let name = argv0.rsplit(['/', '\\']).next().unwrap_or(argv0).to_ascii_lowercase();
    if name.ends_with(".exe") {
        // Wine's own helpers live as long as any Windows program does (and a few seconds more);
        // the program itself is what counts.
        const WINE_HELPERS: &[&str] = &[
            "services.exe",
            "winedevice.exe",
            "plugplay.exe",
            "explorer.exe",
            "rpcss.exe",
            "svchost.exe",
            "tabtip.exe",
            "conhost.exe",
            "start.exe",
            "rundll32.exe",
            "winemenubuilder.exe",
            "wineboot.exe",
            "mscorsvw.exe",
        ];
        return !WINE_HELPERS.contains(&name.as_str());
    }
    // Game streaming (Moonlight, PS Remote Play) and cloud-gaming apps: while one runs, the pad
    // plays a game on another machine.
    const STREAMING: &[&str] = &["moonlight", "moonlight-qt", "chiaki", "chiaki-ng"];
    let emulators = EMULATORS.get_or_init(|| {
        crate::emulation::defs::all()
            .iter()
            .flat_map(|d| d.binaries.iter().copied())
            .chain(STREAMING.iter().copied())
            .chain(
                crate::cloud::Service::ALL.iter().flat_map(|s| s.apps().iter().flat_map(|a| a.procs.iter().copied())),
            )
            .map(|b| b.to_ascii_lowercase())
            .collect()
    });
    emulators.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pids no real process has here (above the usual pid_max), so nothing a test does can
    /// reach a real process even by mistake.
    const FAKE: i32 = 4_100_000;

    /// The background controller's safety check: Steam reapers, Windows programs (but not Wine's
    /// own helpers), emulators and programs from an installed game's folder count as a game.
    #[test]
    fn others_running_sees_games_started_elsewhere() {
        use fake::P;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proc");
        let me = FAKE + 99;
        fake::boot(&root, 1_700_000_000);
        fake::add(&root, P::new(me, 10, &["/usr/bin/lodestar"]));
        fake::add(&root, P::new(FAKE + 1, 20, &["/usr/bin/plasmashell"]));
        fake::add(&root, P::new(FAKE + 2, 30, &["C:\\windows\\system32\\services.exe"]));
        fake::add(&root, P::new(FAKE + 3, 30, &["/usr/bin/firefox", "--new-window"]));
        let procs = read_procs_in(&root, true);
        let none = |dirs: &[PathBuf], procs: &[ProcInfo]| !others_running(&[], procs, &root, me, dirs);
        assert!(none(&[], &procs), "desktop programs and Wine helpers aren't games");

        let with = |p: P| {
            let d = tempfile::tempdir().unwrap();
            let r = d.path().join("proc");
            fake::boot(&r, 1_700_000_000);
            let exe = r.join(p.pid.to_string()).join("exe");
            fake::add(&r, p);
            (d, r, exe)
        };
        for argv in [
            &["/home/u/.local/share/Steam/ubuntu12_32/reaper", "SteamLaunch", "AppId=3999999901", "--", "x"][..],
            &["Z:\\home\\u\\Games\\Witcher\\witcher3.exe"][..],
            &["/app/bin/pcsx2-qt", "-batch"][..],
            &["/tmp/.mount_x/usr/bin/RPCS3"][..],
            &["/app/bin/moonlight", "stream", "desk", "Steam"][..],
            &["/opt/BoosteroidGamesS.R.L./bin/Boosteroid"][..],
        ] {
            let (_d, r, _exe) = with(P::new(FAKE + 5, 40, argv));
            let procs = read_procs_in(&r, true);
            assert!(others_running(&[], &procs, &r, me, &[]), "{argv:?}");
        }
        // A native game started from a desktop shortcut: its program is in an installed game's
        // folder.
        let game = dir.path().join("Games/Celeste");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("Celeste.bin.x86_64"), b"").unwrap();
        let (_d, r, exe) = with(P::new(FAKE + 6, 50, &["./Celeste.bin.x86_64"]));
        std::os::unix::fs::symlink(game.join("Celeste.bin.x86_64"), &exe).unwrap();
        let procs = read_procs_in(&r, true);
        assert!(others_running(&[], &procs, &r, me, std::slice::from_ref(&game)));
        assert!(!others_running(&[], &procs, &r, me, &[dir.path().join("Games/Other")]));
        // Lodestar itself never counts, nor do zombies.
        let (_d, r, _exe) = with(P::new(me, 60, &["C:\\x\\game.exe"]));
        assert!(!others_running(&[], &read_procs_in(&r, true), &r, me, &[]));
        // What find_running reported (tags of another Lodestar run, reapers) counts as well.
        let f = Found { what: FoundKind::Steam { appid: 7 }, procs: vec![], since: 0 };
        assert!(others_running(&[f], &procs, &r, me, &[]));
    }

    #[test]
    fn finds_games_tagged_by_an_earlier_lodestar_and_steam_reapers() {
        use fake::P;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let me = FAKE + 99;
        let dead = FAKE + 1; // the Lodestar that launched the game is gone
        let live = FAKE + 2; // another Lodestar (a `--play` shortcut) still runs
        let tag_dead = format!("{TAG_VAR}={dead}-11-1700000000000000000");
        let tag_live = format!("{TAG_VAR}={live}-12-1700000000000000001");
        let tag_me = format!("{TAG_VAR}={me}-13-1700000000000000002");
        fake::boot(root, 1_700_000_000);
        fake::add(root, P::new(me, 10, &["/usr/bin/lodestar"]));
        fake::add(root, P::new(live, 20, &["/home/u/bin/lodestar", "--play", "12"]));
        // Game 11: a launcher that forked the game (both tagged), started at ticks 500 / 520.
        fake::add(root, P::new(FAKE + 10, 520, &["/g/game.exe"]).env(&[&tag_dead, "A=1"]));
        fake::add(root, P::new(FAKE + 11, 500, &["/g/launcher"]).env(&["X=1", &tag_dead]));
        fake::add(root, P::new(FAKE + 12, 600, &["/g12/run"]).env(&[&tag_live]));
        // This Lodestar's own launch: it follows those itself.
        fake::add(root, P::new(FAKE + 13, 700, &["/g13/run"]).env(&[&tag_me]));
        // Steam's reaper and its wrapper for AppId 570.
        fake::add(
            root,
            P::new(
                FAKE + 20,
                800,
                &["/s/steam-launch-wrapper", "--", "/s/reaper", "SteamLaunch", "AppId=570", "--", "/g/dota"],
            ),
        );
        fake::add(root, P::new(FAKE + 21, 801, &["/s/reaper", "SteamLaunch", "AppId=570", "--", "/g/dota"]));
        // Unrelated: no tag, a garbage tag, a game whose arguments look like Steam's.
        fake::add(root, P::new(FAKE + 30, 900, &["/usr/bin/firefox"]).env(&["HOME=/home/u"]));
        let garbage = format!("{TAG_VAR}=nonsense");
        fake::add(root, P::new(FAKE + 31, 901, &["/x"]).env(&[&garbage]));
        fake::add(root, P::new(FAKE + 32, 902, &["/g/run", "AppId=10", "SteamLaunch"]));
        let procs = read_procs_in(root, true);
        let found = find_running(&procs, root, me);
        assert_eq!(found.len(), 3, "{found:#?}");
        assert_eq!(
            found[0],
            Found {
                what: FoundKind::Steam { appid: 570 },
                procs: vec![(FAKE + 20, 800), (FAKE + 21, 801)],
                since: 800
            }
        );
        assert_eq!(
            found[1],
            Found {
                what: FoundKind::Tagged { id: 11, tag: tag_dead[TAG_VAR.len() + 1..].to_string(), foreign: false },
                procs: vec![(FAKE + 10, 520), (FAKE + 11, 500)],
                since: 500,
            }
        );
        assert!(
            matches!(&found[2].what, FoundKind::Tagged { id: 12, foreign: true, .. }),
            "a live Lodestar's launch is marked as followed by it: {:?}",
            found[2]
        );
        // Nothing running: nothing found.
        let empty = tempfile::tempdir().unwrap();
        fake::add(empty.path(), P::new(FAKE + 30, 900, &["/usr/bin/firefox"]));
        assert!(find_running(&read_procs_in(empty.path(), true), empty.path(), me).is_empty());
    }

    #[test]
    fn tags_and_start_times() {
        assert_eq!(parse_tag("1234-56-1700000000123456789"), Some((1234, 56)));
        assert_eq!(parse_tag("1234-56"), None);
        assert_eq!(parse_tag("x-56-1"), None);
        assert_eq!(parse_tag(""), None);
        let dir = tempfile::tempdir().unwrap();
        fake::boot(dir.path(), 1_700_000_000);
        assert_eq!(boot_time(dir.path()), Some(1_700_000_000));
        assert_eq!(boot_time(&dir.path().join("nope")), None);
        // Started 500 ticks (5 s at 100 Hz) after boot; it is now 65 s after boot.
        let ago = started_ago(500, 1_700_000_000, 100, 1_700_000_065.0);
        assert_eq!(ago, Duration::from_secs(60));
        // A clock that went backwards never gives a negative age.
        assert_eq!(started_ago(500, 1_700_000_000, 100, 1_600_000_000.0), Duration::ZERO);
        assert!(clock_ticks() > 0);
    }

    fn adopt_msg(mode: Mode, known: &[(i32, u64)], age: u64) -> Adopt {
        Adopt {
            id: 11,
            mode,
            tag: format!("{}-11-1", FAKE + 1),
            folder: None,
            known: known.iter().copied().collect(),
            since: known.iter().map(|k| k.1).min().unwrap_or(0),
            age: Duration::from_secs(age),
        }
    }

    #[test]
    fn adopted_session_ends_with_its_processes_timed_from_the_real_start() {
        let now = Instant::now();
        let mut e = adopted_entry(1, adopt_msg(Mode::Session, &[(FAKE + 10, 500)], 3600), now);
        // Followed by its processes, never by a session id (0 would be the kernel's).
        assert_eq!((e.mode, e.session(), e.child_done), (Mode::Session, None, Some(None)));
        let mut game = p(FAKE + 10, 1, FAKE + 10, FAKE + 10);
        game.start = 500;
        let mut child = p(FAKE + 11, FAKE + 10, FAKE + 10, FAKE + 10);
        child.start = 510;
        let kthread = p(2, 0, 0, 0);
        let mut ev = vec![];
        assert!(!update_session(&mut e, &[kthread.clone(), game, child.clone()], &mut ev));
        // The launcher is gone, its child (the game) still runs.
        assert!(!update_session(&mut e, &[kthread.clone(), child], &mut ev));
        assert!(ev.is_empty());
        assert!(update_session(&mut e, &[kthread], &mut ev));
        let [LaunchEvent::Exited { id: 11, secs }] = ev.as_slice() else { panic!("{ev:?}") };
        assert!((3600..3610).contains(secs), "timed from the game's start: {secs}");
    }

    #[test]
    fn adopted_process_tracking_ends_with_the_first_process_not_its_leftovers() {
        let now = Instant::now();
        let mut e = adopted_entry(1, adopt_msg(Mode::Process, &[(FAKE + 10, 500)], 120), now);
        assert_eq!((e.mode, e.session()), (Mode::Process, None));
        let mut game = p(FAKE + 10, 1, FAKE + 10, FAKE + 10);
        game.start = 500;
        // Something it started that stays behind (an updater, say), reparented away later.
        let mut helper = p(FAKE + 11, FAKE + 10, FAKE + 10, FAKE + 10);
        helper.start = 510;
        let mut ev = vec![];
        assert!(!update_process(&mut e, &[game.clone(), helper.clone()], &mut ev));
        // Stop: the game and what it runs, never by a session id.
        let my = Myself { pid: FAKE + 99, pgrp: FAKE + 99, sid: FAKE + 98 };
        let kthread = p(2, 0, 0, 0);
        let procs = [kthread.clone(), game, helper.clone()];
        assert_eq!(stop_targets(&e, &procs, my), (vec![FAKE + 10], vec![FAKE + 10, FAKE + 11]));
        helper.ppid = 1;
        assert!(update_process(&mut e, &[kthread, helper], &mut ev), "ended with its first process");
        let [LaunchEvent::Exited { id: 11, secs }] = ev.as_slice() else { panic!("{ev:?}") };
        assert!((120..125).contains(secs), "{secs}");
    }

    #[test]
    fn adopted_steam_game_ends_when_its_reaper_does() {
        let t0 = Instant::now();
        let mut e = adopted_entry(1, adopt_msg(Mode::Steam { appid: 570 }, &[(FAKE + 20, 800)], 600), t0);
        assert_eq!(e.reapers, vec![FAKE + 20]);
        let mut reaper = p(FAKE + 20, 1, FAKE + 20, FAKE + 20);
        reaper.cmdline = Some(vec!["reaper".into(), "SteamLaunch".into(), "AppId=570".into()]);
        let mut ev = vec![];
        let to = Duration::from_secs(180);
        assert!(!update_steam(&mut e, 570, std::slice::from_ref(&reaper), &mut ev, t0, to));
        assert!(ev.is_empty(), "already started: no second Started");
        assert!(!update_steam(&mut e, 570, &[], &mut ev, t0 + Duration::from_secs(10), to));
        assert!(update_steam(&mut e, 570, &[], &mut ev, t0 + Duration::from_secs(11) + STEAM_GRACE, to));
        assert_eq!(ev, vec![LaunchEvent::Exited { id: 11, secs: 610 }]);
    }

    #[test]
    fn stop_of_an_adopted_game_signals_only_its_processes() {
        let my = Myself { pid: FAKE + 99, pgrp: FAKE + 99, sid: FAKE + 98 };
        let e = adopted_entry(1, adopt_msg(Mode::Session, &[(FAKE + 10, 1000)], 60), Instant::now());
        let mut game = p(FAKE + 10, 1, FAKE + 10, FAKE + 10);
        game.start = 1000;
        let child = p(FAKE + 11, FAKE + 10, FAKE + 10, FAKE + 10);
        // Shares the game's old session id but isn't the game's (nor a child of it).
        let same_session = p(FAKE + 12, 1, FAKE + 12, FAKE + 10);
        // The pid of a known process, reused by something else (another start time).
        let mut reused = p(FAKE + 13, 1, FAKE + 13, FAKE + 13);
        reused.start = 7;
        let procs = vec![
            p(1, 0, 1, 1),
            p(2, 0, 0, 0), // kernel thread: session / group 0
            game,
            child,
            same_session,
            reused,
            p(FAKE + 99, 1, FAKE + 99, FAKE + 98), // Lodestar itself
        ];
        assert_eq!(stop_targets(&e, &procs, my), (vec![FAKE + 10], vec![FAKE + 10, FAKE + 11]));
        // Steam: the reaper and what it runs; its group (shared with the Steam client) isn't.
        let e = adopted_entry(1, adopt_msg(Mode::Steam { appid: 570 }, &[(FAKE + 20, 5)], 60), Instant::now());
        let procs = vec![
            p(FAKE + 30, 1, FAKE + 30, FAKE + 30), // Steam client
            p(FAKE + 20, FAKE + 30, FAKE + 30, FAKE + 30),
            p(FAKE + 21, FAKE + 20, FAKE + 21, FAKE + 21),
            p(2, 0, 0, 0),
        ];
        assert_eq!(stop_targets(&e, &procs, my), (vec![], vec![FAKE + 20, FAKE + 21]));
    }

    #[test]
    fn a_game_is_adopted_once() {
        let (ev_tx, _ev_rx) = unbounded();
        let (tx, _rx) = unbounded();
        let mut m = Monitor {
            ctx: egui::Context::default(),
            ev_tx,
            tx,
            every: Duration::from_secs(1),
            steam_timeout: Duration::from_secs(1),
            entries: vec![],
            next_token: 1,
        };
        assert!(m.handle(Msg::Adopt(adopt_msg(Mode::Session, &[(FAKE + 10, 5)], 1))));
        assert!(!m.handle(Msg::Adopt(adopt_msg(Mode::Session, &[(FAKE + 10, 5)], 1))));
        assert_eq!(m.entries.len(), 1);
    }

    fn p(pid: i32, ppid: i32, pgrp: i32, sid: i32) -> ProcInfo {
        ProcInfo { pid, ppid, pgrp, sid, start: pid as u64 * 10, zombie: false, cmdline: None }
    }

    #[test]
    fn stat_parsing() {
        let s = "1234 (my (weird) game) S 1 1234 1234 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 3 0 98765 1000 10 18446744073709551615";
        let p = parse_stat(1234, s).unwrap();
        assert_eq!((p.ppid, p.pgrp, p.sid, p.start, p.zombie), (1, 1234, 1234, 98765, false));
        assert!(parse_stat(1, "garbage").is_none());
        assert!(parse_stat(1, "1 (x) Z 1 2").is_none());
        let z = "7 (x) Z 1 7 7 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 55 0 0";
        assert!(parse_stat(7, z).unwrap().zombie);
    }

    #[test]
    fn members_follow_session_group_and_descendants() {
        let procs = vec![
            p(100, 1, 100, 100),   // launcher (session leader)
            p(101, 1, 101, 100),   // forked, own group, same session
            p(200, 1, 200, 200),   // escaped via setsid, but we saw it before…
            p(201, 200, 200, 200), // …and its child
            p(300, 1, 300, 300),   // unrelated
            p(400, 101, 400, 400), // child of a member that called setsid
        ];
        let known: HashSet<(i32, u64)> = [(200, 2000)].into_iter().collect();
        let mut m: Vec<i32> = session_members(Some(100), &known, &procs).iter().map(|p| p.pid).collect();
        m.sort();
        assert_eq!(m, vec![100, 101, 200, 201, 400]);
        // Reused pid (different start time) is not "known".
        let known: HashSet<(i32, u64)> = [(200, 1)].into_iter().collect();
        let mut m: Vec<i32> = session_members(Some(100), &known, &procs).iter().map(|p| p.pid).collect();
        m.sort();
        assert_eq!(m, vec![100, 101, 400]);
    }

    #[test]
    fn reused_session_id_is_not_followed() {
        let procs = vec![p(100, 1, 100, 100), p(101, 100, 100, 100), p(7, 1, 7, 7)];
        let known: HashSet<(i32, u64)> = [(7, 70)].into_iter().collect();
        let m: Vec<i32> = session_members(None, &known, &procs).iter().map(|p| p.pid).collect();
        assert_eq!(m, vec![7]);
    }

    #[test]
    fn reaper_detection() {
        let c: Vec<String> =
            ["/x/reaper", "SteamLaunch", "AppId=570", "--", "/x/game"].iter().map(|s| s.to_string()).collect();
        assert!(is_reaper_for(&c, "AppId=570"));
        assert!(!is_reaper_for(&c, "AppId=57"));
        let no: Vec<String> = ["steam", "AppId=570"].iter().map(|s| s.to_string()).collect();
        assert!(!is_reaper_for(&no, "AppId=570"));
        let v = |a: &[&str]| -> Vec<String> { a.iter().map(|s| s.to_string()).collect() };
        // Reaper started through steam-launch-wrapper (newer clients).
        let wrapped = v(&[
            "/s/ubuntu12_32/steam-launch-wrapper",
            "--",
            "/s/ubuntu12_32/reaper",
            "SteamLaunch",
            "AppId=570",
            "--",
            "/s/common/SteamLinuxRuntime_sniper/_v2-entry-point",
            "--verb=waitforexitandrun",
            "--",
            "/s/common/Proton/proton",
            "waitforexitandrun",
            "/g/game.exe",
        ]);
        assert!(is_reaper_for(&wrapped, "AppId=570"));
        // Non-Steam shortcut: 32-bit appid.
        let sc = v(&["reaper", "SteamLaunch", "AppId=3123456789", "--", "/opt/x/run"]);
        assert!(is_reaper_for(&sc, "AppId=3123456789"));
        // `AppId=` only counts before the game command, right after `SteamLaunch`.
        let game_args = v(&["reaper", "SteamLaunch", "AppId=10", "--", "/g/run", "AppId=570"]);
        assert!(!is_reaper_for(&game_args, "AppId=570"));
        assert!(is_reaper_for(&game_args, "AppId=10"));
        let not_reaper = v(&["/g/run", "AppId=570", "SteamLaunch"]);
        assert!(!is_reaper_for(&not_reaper, "AppId=570"));
    }

    #[test]
    fn steam_state_machine() {
        let t0 = Instant::now();
        let mut e = Entry {
            token: 1,
            id: 9,
            mode: Mode::Steam { appid: 570 },
            sid: 1,
            child_start: None,
            sid_reused: false,
            child_done: None,
            created: t0,
            started: None,
            known: HashSet::new(),
            reapers: vec![],
            gone_since: None,
            tag: String::new(),
            untagged: HashSet::new(),
            log: PathBuf::new(),
            stopping: false,
            term_sent: None,
            killed: false,
            folder: None,
            in_folder: HashSet::new(),
            not_in_folder: HashSet::new(),
            folder_seen: false,
            adopted: false,
        };
        let mut reaper = p(50, 1, 50, 50);
        reaper.cmdline = Some(vec!["reaper".into(), "SteamLaunch".into(), "AppId=570".into()]);
        let mut ev = vec![];
        let to = Duration::from_secs(180);
        assert!(!update_steam(&mut e, 570, &[], &mut ev, t0 + Duration::from_secs(5), to));
        assert!(ev.is_empty());
        assert!(!update_steam(&mut e, 570, std::slice::from_ref(&reaper), &mut ev, t0 + Duration::from_secs(10), to));
        assert_eq!(ev, vec![LaunchEvent::Started { id: 9 }]);
        // Reaper gone: nothing yet (grace period) …
        assert!(!update_steam(&mut e, 570, &[], &mut ev, t0 + Duration::from_secs(70), to));
        assert_eq!(ev.len(), 1);
        // … it comes back (Steam's pre-launch step ended, the real game started): still running.
        assert!(!update_steam(&mut e, 570, std::slice::from_ref(&reaper), &mut ev, t0 + Duration::from_secs(80), to));
        assert_eq!(ev.len(), 1);
        assert!(!update_steam(&mut e, 570, &[], &mut ev, t0 + Duration::from_secs(100), to));
        // Gone for the whole grace period: exited, timed to when it disappeared.
        assert!(update_steam(&mut e, 570, &[], &mut ev, t0 + Duration::from_secs(101) + STEAM_GRACE, to));
        assert_eq!(ev[1], LaunchEvent::Exited { id: 9, secs: 90 });
        e.gone_since = None;
        // Never seen -> silent stop after the timeout.
        e.started = None;
        ev.clear();
        assert!(!update_steam(&mut e, 570, &[], &mut ev, t0 + Duration::from_secs(179), to));
        assert!(update_steam(&mut e, 570, &[], &mut ev, t0 + Duration::from_secs(181), to));
        assert_eq!(ev, vec![LaunchEvent::Exited { id: 9, secs: 0 }]);
    }

    #[test]
    fn reads_real_proc() {
        let me = std::process::id() as i32;
        let procs = read_procs(true);
        let mine = procs.iter().find(|p| p.pid == me).expect("own process listed");
        assert!(mine.cmdline.as_ref().is_some_and(|c| !c.is_empty()));
    }

    #[test]
    fn log_paths_are_shown_relative_to_home() {
        let home = Path::new("/home/u");
        assert_eq!(
            tilde(Path::new("/home/u/.cache/lodestar/logs/1-x.log"), Some(home)),
            "~/.cache/lodestar/logs/1-x.log"
        );
        assert_eq!(tilde(Path::new("/tmp/l/1.log"), Some(home)), "/tmp/l/1.log");
        assert_eq!(tilde(Path::new("/home/user2/a.log"), Some(home)), "/home/user2/a.log");
        assert_eq!(tilde(Path::new("/tmp/a.log"), None), "/tmp/a.log");
    }
}
