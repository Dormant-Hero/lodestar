//! Launcher hygiene: keep store clients out of sight while the game they started
//! runs, and close the clients Lodestar itself started once that game exits.
//!
//! One background thread, started with the first launch through a store client; it blocks on its
//! channel whenever nothing is scheduled (no polling loop while idle).
//!
//! * A launch through a Wine store client (Battle.net, EA app, Ubisoft Connect …) or a
//!   `heroic://` link is registered with its launch tag, prefix and game folder. Only launches
//!   tracked by the game's own folder take part: their `Started` means the game process was seen,
//!   so any login / update screen of the client is done by then.
//! * `Started`: the client's windows are minimised (winhide.rs) right away and again a few times
//!   over ~10 s (clients re-show their window). Client processes = the prefix's processes and
//!   the launch's tagged ones, minus the game (inside its folder, and its children) and Wine's own
//!   services; for Heroic, Heroic's processes.
//! * `Exited` (after a game that was seen running): a short while later, close what Lodestar
//!   started for this launch — the clients the store-client helper had to start for it
//!   (`client_launch::started_clients`, records whose `game_tag` is this launch's tag), through
//!   `client_launch::close_client` (polite, SIGTERM, SIGKILL). A client the user had open has no
//!   record and stays. Kept open too while a Windows program outside the client's folders runs in
//!   its prefix (the user started another game from it; EA / Ubisoft games need their client).
//!   Processes still carrying this launch's own tag outside the game (a client started without
//!   the helper) get SIGTERM, SIGKILL after a grace period. Heroic: closed (SIGTERM to its main
//!   process) when it wasn't running before the launch.
//! * A new launch through the same client while a close is pending (one game at a time switching
//!   between two Battle.net games) cancels the close; the client then belongs to the new launch
//!   and closes after that game instead.
//! * Steam: its windows are minimised like a client's; it is closed only when the user asked for
//!   it, Lodestar started it (`launch/steam.rs` record) and it is idle after the game (cloud sync
//!   done), through `steam -shutdown` — never with a signal.

use super::steam::{self as steam_ctl, IdleWatch, SteamStarted, SteamTiming};
use super::track::{self, TAG_VAR};
use crate::client_launch::StartedClient;
use crate::client_launch::procs::{dir_forms, image_under, same_prefix};
use crate::model::GameId;
use crate::procfs;
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// What the hygiene thread does (Settings → General → Launchers; off unless the app turns it on, so tests
/// and screenshot runs never touch anything).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HygieneConfig {
    /// Minimise store clients once their game runs.
    pub hide: bool,
    /// Close store clients Lodestar started once their game exits.
    pub close: bool,
    /// Minimise the Steam client's own windows while a Steam game runs.
    pub hide_steam: bool,
    /// Close a Steam Lodestar started once its game ended and Steam is idle.
    pub close_steam: bool,
    /// Keep Steam running (Steam Input controller layouts): never closed, only minimised.
    pub keep_steam: bool,
}

impl HygieneConfig {
    pub fn from_settings(s: &crate::config::LaunchersConfig) -> HygieneConfig {
        HygieneConfig {
            hide: s.hide_launchers,
            close: s.close_launchers_after,
            hide_steam: s.hide_steam,
            close_steam: s.close_steam_after,
            keep_steam: s.keep_steam_for_controller,
        }
    }

    pub fn active(&self) -> bool {
        self.hide || self.close || self.hide_steam || self.closes_steam()
    }

    /// Steam may be closed after its game (when Lodestar started it).
    pub fn closes_steam(&self) -> bool {
        self.close_steam && !self.keep_steam
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Client {
    /// A Windows store client in a Wine prefix.
    Wine,
    /// Heroic (a `heroic://` link).
    Heroic,
    /// The Steam client (a Steam game): minimised; closed only when Lodestar started it and the
    /// user wants that.
    Steam,
}

#[derive(Clone, Debug)]
pub struct LaunchInfo {
    pub id: GameId,
    /// `TAG_VAR` value of this launch.
    pub tag: String,
    /// The game's install folder (tells the game's processes from the client's).
    pub folder: Option<PathBuf>,
    /// Wine prefix of the client.
    pub prefix: Option<PathBuf>,
    pub client: Client,
    /// Heroic was already running when the link was opened (then it stays open).
    pub heroic_was_running: bool,
}

pub enum Msg {
    Config(HygieneConfig),
    Launch(LaunchInfo),
    Started(GameId),
    Exited(GameId),
    /// Lodestar is about to quit (`--play`): do pending closes now, wait for them, then answer.
    Finish(Sender<()>),
}

/// What hygiene needs to know about one process.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Proc {
    pub pid: i32,
    pub ppid: i32,
    pub start: u64,
    pub zombie: bool,
    pub cmdline: Vec<String>,
    pub exe: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    /// `WINEPREFIX` from its environment.
    pub prefix: Option<PathBuf>,
    /// `TAG_VAR` from its environment.
    pub tag: Option<String>,
    /// Its pids in nested pid namespaces (Flatpak, pressure-vessel): `_NET_WM_PID` holds those.
    pub nspids: Vec<i32>,
}

/// The system as hygiene sees it (a fake one in tests).
pub trait Sys: Send + 'static {
    fn procs(&mut self) -> Vec<Proc>;
    /// Signal `pid` if it is still the process that started at `start`.
    fn signal(&mut self, pid: i32, start: u64, sig: i32);
    /// Minimise the windows of `pids` (real and pid-namespace pids): X11 first, then the
    /// compositor (native Wayland windows). Returns how many X11 windows were asked to.
    fn iconify(&mut self, pids: &[u32]) -> usize;
    /// Clients the store-client helper started that still run (`client_launch` records).
    fn started_clients(&mut self) -> Vec<StartedClient>;
    /// Close one of them (in the background: it waits for the client to go).
    fn close_client(&mut self, c: StartedClient);
    /// Wait for the closes started by `close_client`.
    fn wait_closing(&mut self) {}
    /// The record of a Steam Lodestar started (`launch/steam.rs`).
    fn steam_record(&mut self) -> Option<SteamStarted>;
    fn save_steam_record(&mut self, r: &SteamStarted);
    fn remove_steam_record(&mut self);
    /// Steam's logs now: their sizes / change times, and the cloud log since the record's start.
    fn steam_logs(&mut self, r: &SteamStarted) -> (steam_ctl::LogSig, String);
    /// `steam -shutdown` (Flatpak or not).
    fn steam_shutdown(&mut self, flatpak: bool);
}

/// The real system: /proc, kill(2), X11.
#[derive(Default)]
pub struct RealSys {
    closing: Vec<std::thread::JoinHandle<()>>,
}

impl Sys for RealSys {
    fn procs(&mut self) -> Vec<Proc> {
        read_procs(Path::new("/proc"))
    }

    fn signal(&mut self, pid: i32, start: u64, sig: i32) {
        let me = std::process::id() as i32;
        let same = procfs::is_same_live(Path::new("/proc"), pid, start);
        if same && pid > 1 && pid != me {
            // SAFETY: kill(2) on a pid checked just above.
            unsafe { libc::kill(pid, sig) };
        }
    }

    fn iconify(&mut self, pids: &[u32]) -> usize {
        let x11 = self.iconify_x11(pids);
        if x11 > 0 || cfg!(test) {
            return x11;
        }
        // Nothing on X11: the client may draw native Wayland windows (Heroic, winewayland).
        if let Some(route) = crate::launch::winhide_wayland::minimise(pids) {
            log::info!("asked the compositor ({route}) to minimise the windows of {pids:?}");
        }
        0
    }
    fn started_clients(&mut self) -> Vec<StartedClient> {
        // Unit tests never look at (or close) the user's real clients.
        if cfg!(test) {
            return vec![];
        }
        crate::client_launch::started_clients(&crate::config::Paths::detect())
    }

    fn close_client(&mut self, c: StartedClient) {
        if cfg!(test) {
            return;
        }
        let r = std::thread::Builder::new().name("close-client".into()).spawn(move || {
            match crate::client_launch::close_client(&c) {
                Ok(rep) => log::info!(
                    "closed {} ({} processes, politely: {}, terminated {:?}, killed {:?})",
                    c.client,
                    rep.found,
                    rep.closed_politely,
                    rep.terminated,
                    rep.killed
                ),
                Err(e) => log::warn!("closing {}: {e:#}", c.client),
            }
        });
        match r {
            Ok(h) => self.closing.push(h),
            Err(e) => log::warn!("couldn't start a thread to close a store client: {e}"),
        }
        self.closing.retain(|h| !h.is_finished());
    }

    fn wait_closing(&mut self) {
        for h in self.closing.drain(..) {
            let _ = h.join();
        }
    }

    fn steam_record(&mut self) -> Option<SteamStarted> {
        // Unit tests never look at (or close) the user's real Steam.
        if cfg!(test) {
            return None;
        }
        steam_ctl::read_record(&Self::steam_record_path())
    }

    fn save_steam_record(&mut self, r: &SteamStarted) {
        if cfg!(test) {
            return;
        }
        if let Err(e) = steam_ctl::write_record(&Self::steam_record_path(), r) {
            log::warn!("couldn't update the Steam record: {e:#}");
        }
    }

    fn remove_steam_record(&mut self) {
        if !cfg!(test) {
            steam_ctl::remove_record(&Self::steam_record_path());
        }
    }

    fn steam_logs(&mut self, r: &SteamStarted) -> (steam_ctl::LogSig, String) {
        match &r.logs {
            Some(d) => (steam_ctl::LogSig::read(d), steam_ctl::cloud_text_since(d, r.cloud_offset)),
            None => Default::default(),
        }
    }

    fn steam_shutdown(&mut self, flatpak: bool) {
        steam_ctl::shutdown(flatpak);
    }
}

impl RealSys {
    fn steam_record_path() -> PathBuf {
        steam_ctl::record_path(&crate::config::Paths::detect().cache_dir)
    }

    /// Minimise on `$DISPLAY`.
    fn iconify_x11(&mut self, pids: &[u32]) -> usize {
        // Unit tests never reach for the user's real desktop.
        if cfg!(test) {
            return 0;
        }
        // x11rb can panic on a malformed $DISPLAY; that must not end the hygiene thread.
        match std::panic::catch_unwind(|| crate::launch::winhide::iconify_windows_of(None, pids)) {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => {
                log::info!("minimising store client windows: {e:#}");
                0
            }
            Err(_) => {
                log::warn!("minimising store client windows failed ($DISPLAY)");
                0
            }
        }
    }
}

/// When the client's windows are minimised after the game showed up (clients re-show them).
const HIDE_AT: [Duration; 5] =
    [Duration::ZERO, Duration::from_secs(2), Duration::from_secs(4), Duration::from_secs(7), Duration::from_secs(10)];
/// How long after the game exited its client is closed (a following launch through the same
/// client, e.g. one game at a time switching games, takes it over meanwhile).
const CLOSE_DELAY: Duration = Duration::from_secs(4);
/// SIGKILL for what of a client is still there this long after SIGTERM.
const KILL_AFTER: Duration = Duration::from_secs(8);

struct Launch {
    info: LaunchInfo,
    /// Launch tags whose processes belong to this launch's client (its own, plus ones taken over).
    tags: Vec<String>,
    /// Lodestar started Heroic for this launch (or one it took over).
    heroic_ours: bool,
    started: bool,
    /// Folders the client's own Windows programs run from (seen while the game ran).
    client_dirs: HashSet<String>,
}

impl Launch {
    fn new(info: LaunchInfo) -> Launch {
        Launch {
            tags: vec![info.tag.clone()],
            heroic_ours: info.client == Client::Heroic && !info.heroic_was_running,
            started: false,
            client_dirs: HashSet::new(),
            info,
        }
    }

    /// Take over the client of `old` (it closes after this launch's game instead).
    fn take_over(&mut self, old: Launch) {
        for t in old.tags {
            if !self.tags.contains(&t) {
                self.tags.push(t);
            }
        }
        self.heroic_ours |= old.heroic_ours;
        self.client_dirs.extend(old.client_dirs);
    }

    fn shares_client(&self, o: &Launch) -> bool {
        self.info.client == o.info.client
            && match self.info.client {
                Client::Heroic | Client::Steam => true,
                Client::Wine => match (&self.info.prefix, &o.info.prefix) {
                    (Some(a), Some(b)) => same_prefix(a, b),
                    _ => false,
                },
            }
    }

    fn tagged(&self, p: &Proc) -> bool {
        p.tag.as_ref().is_some_and(|t| self.tags.contains(t))
    }
}

enum Job {
    Hide(GameId),
    Close(Launch),
    Kill(Vec<(i32, u64)>),
    /// Waiting for a Steam Lodestar started to be idle before `steam -shutdown`.
    SteamWait(Box<SteamWait>),
}

struct SteamWait {
    launch: Launch,
    rec: SteamStarted,
    watch: IdleWatch,
}

impl Job {
    /// The launch whose client this job is about to close.
    fn closing(&self) -> Option<&Launch> {
        match self {
            Job::Close(l) => Some(l),
            Job::SteamWait(w) => Some(&w.launch),
            _ => None,
        }
    }

    fn into_closing(self) -> Option<Launch> {
        match self {
            Job::Close(l) => Some(l),
            Job::SteamWait(w) => Some(w.launch),
            _ => None,
        }
    }
}

pub struct Hygiene<S: Sys> {
    sys: S,
    cfg: HygieneConfig,
    launches: HashMap<GameId, Launch>,
    jobs: Vec<(Instant, Job)>,
    hide_at: Vec<Duration>,
    close_delay: Duration,
    kill_after: Duration,
    steam_timing: SteamTiming,
}

impl<S: Sys> Hygiene<S> {
    pub fn new(sys: S, cfg: HygieneConfig) -> Hygiene<S> {
        Hygiene {
            sys,
            cfg,
            launches: HashMap::new(),
            jobs: vec![],
            hide_at: HIDE_AT.to_vec(),
            close_delay: CLOSE_DELAY,
            kill_after: KILL_AFTER,
            steam_timing: SteamTiming::default(),
        }
    }

    /// Other waits for Steam (tests).
    #[cfg(test)]
    pub fn with_steam_timing(mut self, t: SteamTiming) -> Self {
        self.steam_timing = t;
        self
    }

    /// Shorter waits (tests).
    #[cfg(test)]
    pub fn with_timing(mut self, hide_at: Vec<Duration>, close_delay: Duration, kill_after: Duration) -> Self {
        self.hide_at = hide_at;
        self.close_delay = close_delay;
        self.kill_after = kill_after;
        self
    }

    /// When the next scheduled job is due.
    pub fn next_due(&self) -> Option<Instant> {
        self.jobs.iter().map(|(t, _)| *t).min()
    }

    pub fn handle(&mut self, m: Msg, now: Instant) {
        match m {
            Msg::Config(c) => self.cfg = c,
            Msg::Launch(info) => {
                let mut l = Launch::new(info);
                if let Some(old) = self.launches.remove(&l.info.id) {
                    l.take_over(old);
                }
                // A client about to be closed that this launch uses too stays open.
                let mut i = 0;
                while i < self.jobs.len() {
                    if self.jobs[i].1.closing().is_some_and(|old| old.shares_client(&l)) {
                        if let Some(old) = self.jobs.swap_remove(i).1.into_closing() {
                            log::info!("store client of game {} stays open for game {}", old.info.id, l.info.id);
                            l.take_over(old);
                        }
                    } else {
                        i += 1;
                    }
                }
                self.launches.insert(l.info.id, l);
            }
            Msg::Started(id) => {
                let Some(l) = self.launches.get_mut(&id) else { return };
                if l.started {
                    return;
                }
                l.started = true;
                let steam = l.info.client == Client::Steam;
                for d in &self.hide_at {
                    self.jobs.push((now + *d, Job::Hide(id)));
                }
                if steam {
                    self.claim_steam(id);
                }
            }
            Msg::Finish(done) => {
                self.finish();
                let _ = done.send(());
            }
            Msg::Exited(id) => {
                self.jobs.retain(|(_, j)| !matches!(j, Job::Hide(h) if *h == id));
                let Some(l) = self.launches.remove(&id) else { return };
                // A game that never showed up: the user may still be busy in the client (login,
                // update, a stopped wait), so it stays.
                let close = if l.info.client == Client::Steam { self.cfg.closes_steam() } else { self.cfg.close };
                if l.started && close {
                    self.jobs.push((now + self.close_delay, Job::Close(l)));
                }
            }
        }
    }

    /// Before Lodestar quits: pending closes run now (no later launch can take the client over),
    /// their SIGKILL follow-ups when due, and the record closes are waited for.
    fn finish(&mut self) {
        let now = Instant::now();
        self.jobs.retain(|(_, j)| !matches!(j, Job::Hide(_)));
        for (t, j) in &mut self.jobs {
            if matches!(j, Job::Close(_)) {
                *t = now;
            }
        }
        self.run_due(now);
        while let Some(t) = self.next_due() {
            std::thread::sleep(t.saturating_duration_since(Instant::now()));
            self.run_due(Instant::now());
        }
        self.sys.wait_closing();
    }

    /// Run every job due at `now`.
    pub fn run_due(&mut self, now: Instant) {
        loop {
            let Some(i) = self.jobs.iter().position(|(t, _)| *t <= now) else { return };
            let (_, job) = self.jobs.swap_remove(i);
            match job {
                Job::Hide(id) => self.hide(id),
                Job::Close(l) => self.close(l, now),
                Job::SteamWait(w) => self.steam_wait(*w, now),
                Job::Kill(v) => {
                    let procs = self.sys.procs();
                    for p in procs.iter().filter(|p| !p.zombie && v.contains(&(p.pid, p.start))) {
                        log::info!("store client process {} ignored SIGTERM; killing it", p.pid);
                        self.sys.signal(p.pid, p.start, libc::SIGKILL);
                    }
                }
            }
        }
    }

    fn hide(&mut self, id: GameId) {
        let Some(l) = self.launches.get_mut(&id) else { return };
        let procs = self.sys.procs();
        let clients = client_procs(l, &procs);
        for p in &clients {
            if let Some(d) = exe_dir(&p.cmdline) {
                l.client_dirs.insert(d);
            }
        }
        let enabled = if l.info.client == Client::Steam { self.cfg.hide_steam } else { self.cfg.hide };
        if !enabled || clients.is_empty() {
            return;
        }
        let mut pids: Vec<u32> = vec![];
        for p in &clients {
            for x in std::iter::once(p.pid).chain(p.nspids.iter().copied()) {
                if x > 0 && !pids.contains(&(x as u32)) {
                    pids.push(x as u32);
                }
            }
        }
        let n = self.sys.iconify(&pids);
        if n > 0 {
            log::info!("minimised {n} store client window(s) for game {id}");
        } else {
            log::debug!("no store client windows to minimise for game {id} (pids {pids:?})");
        }
    }

    fn close(&mut self, l: Launch, now: Instant) {
        // Another game runs through the same client (several games at once): it stays for that one.
        if let Some(other) = self.launches.values_mut().find(|o| o.shares_client(&l)) {
            other.take_over(l);
            return;
        }
        let procs = self.sys.procs();
        match l.info.client {
            // Only a Steam Lodestar started for this launch (or one it took over) is closed, once idle.
            Client::Steam => {
                if !self.cfg.closes_steam() {
                    return;
                }
                match self.sys.steam_record() {
                    Some(rec) if l.tags.contains(&rec.game_tag) => {
                        log::info!("game {} ended: closing Steam (Lodestar started it) once it is idle", l.info.id);
                        self.jobs.push((
                            now,
                            Job::SteamWait(Box::new(SteamWait { launch: l, rec, watch: IdleWatch::new(now) })),
                        ));
                    }
                    _ => log::info!("Steam stays open after game {}: Lodestar didn't start it", l.info.id),
                }
            }
            Client::Heroic => {
                if !l.heroic_ours {
                    return;
                }
                for p in procs.iter().filter(|p| !p.zombie && is_heroic_main(&p.cmdline)) {
                    log::info!("closing Heroic (pid {}): Lodestar started it for game {}", p.pid, l.info.id);
                    self.sys.signal(p.pid, p.start, libc::SIGTERM);
                }
            }
            Client::Wine => {
                let mut kept = false;
                for c in self.sys.started_clients() {
                    if !c.game_tag.as_ref().is_some_and(|t| l.tags.contains(t)) {
                        continue;
                    }
                    match client_in_use(&l, &c, &procs) {
                        Some(why) => {
                            log::info!("{} stays open after game {}: {why}", c.client, l.info.id);
                            kept = true;
                        }
                        None => {
                            log::info!("closing {}: Lodestar started it for game {}", c.client, l.info.id);
                            self.sys.close_client(c);
                        }
                    }
                }
                if kept {
                    return;
                }
                match leftovers(&l, &procs) {
                    Ok(v) if v.is_empty() => {}
                    Ok(v) => {
                        log::info!("ending what is left of game {}'s launch ({} processes)", l.info.id, v.len());
                        for (pid, start) in &v {
                            self.sys.signal(*pid, *start, libc::SIGTERM);
                        }
                        self.jobs.push((now + self.kill_after, Job::Kill(v)));
                    }
                    Err(why) => log::info!("store client of game {} stays open: {why}", l.info.id),
                }
            }
        }
    }
}

impl<S: Sys> Hygiene<S> {
    /// The game of a Steam launch runs: if Lodestar started that Steam, note its main processes in
    /// the record (the Steam that runs now is that one).
    fn claim_steam(&mut self, id: GameId) {
        let Some(l) = self.launches.get(&id) else { return };
        let Some(mut rec) = self.sys.steam_record() else { return };
        if !l.tags.contains(&rec.game_tag) || !rec.pids.is_empty() {
            return;
        }
        let pids = steam_ctl::claim_pids(&rec, &self.sys.procs());
        if !pids.is_empty() {
            rec.pids = pids;
            self.sys.save_steam_record(&rec);
        }
    }

    /// One look at Steam after its game: still ours, nothing else using it, idle → `steam -shutdown`.
    fn steam_wait(&mut self, mut w: SteamWait, now: Instant) {
        if !self.cfg.closes_steam() {
            log::info!("Steam stays open: closing it was turned off");
            return;
        }
        // Another Steam game Lodestar follows: it belongs to that one now.
        if let Some(other) = self.launches.values_mut().find(|o| o.shares_client(&w.launch)) {
            other.take_over(w.launch);
            return;
        }
        let procs = self.sys.procs();
        if !steam_ctl::is_ours(&w.rec, &procs) {
            log::info!("Steam stays as it is: the Steam Lodestar started is gone");
            self.sys.remove_steam_record();
            return;
        }
        if let Some(app) = steam_ctl::other_game(&procs) {
            log::info!("Steam stays open: another Steam game runs (AppId {app})");
            return;
        }
        let (sig, cloud) = self.sys.steam_logs(&w.rec);
        let pending = steam_ctl::cloud_pending(&cloud);
        match w.watch.poll(now, &self.steam_timing, sig, &pending) {
            steam_ctl::Idle::Wait => self.jobs.push((now + self.steam_timing.poll, Job::SteamWait(Box::new(w)))),
            v => {
                if v == steam_ctl::Idle::TimedOut {
                    log::info!(
                        "Steam still busy after the game (cloud sync of {pending:?}?); asking it to quit anyway"
                    );
                } else {
                    log::info!("Steam is idle after game {}: asking it to quit", w.launch.info.id);
                }
                self.sys.steam_shutdown(w.rec.flatpak);
                self.sys.remove_steam_record();
            }
        }
    }
}

/// The client's processes for a launch: Wine — in its prefix or carrying its tags, minus the game
/// (inside its folder, and everything it started) and Wine's own services; Heroic — Heroic's.
fn client_procs<'a>(l: &Launch, procs: &'a [Proc]) -> Vec<&'a Proc> {
    let live = procs.iter().filter(|p| !p.zombie);
    match l.info.client {
        Client::Heroic => live.filter(|p| is_heroic_process(&p.cmdline)).collect(),
        Client::Steam => live.filter(|p| is_steam_client_process(&p.cmdline)).collect(),
        Client::Wine => {
            let game: HashSet<i32> = descendants_incl(procs, |p| in_game(p, l.info.folder.as_deref()));
            live.filter(|p| {
                let ours =
                    l.tagged(p) || matches!((&p.prefix, &l.info.prefix), (Some(a), Some(b)) if same_prefix(a, b));
                ours && !game.contains(&p.pid) && !p.cmdline.first().is_some_and(|a| is_wine_system(a))
            })
            .collect()
        }
    }
}

/// Why a client the helper started must stay open after the game: the game still runs, or a
/// Windows program from outside the client's folders runs in its prefix (another game started
/// from the client, which may need it). Wine's own services don't count.
fn client_in_use(l: &Launch, c: &StartedClient, procs: &[Proc]) -> Option<String> {
    let mut dirs: Vec<String> = c.client_dirs.iter().flat_map(|d| dir_forms(&c.prefix, d)).collect();
    dirs.extend(l.client_dirs.iter().cloned());
    for p in procs.iter().filter(|p| !p.zombie && p.prefix.as_deref().is_some_and(|x| same_prefix(x, &c.prefix))) {
        if in_game(p, l.info.folder.as_deref()) {
            return Some(format!("the game still runs (pid {})", p.pid));
        }
        let Some(img) = image(&p.cmdline) else { continue };
        if img.ends_with(".exe") && !is_wine_system(&img) && !image_under(&img, &dirs) {
            return Some(format!("{img} runs in its prefix"));
        }
    }
    None
}

/// What to close for a Wine client after its game: this launch's tagged processes (but not Wine's
/// own services and wineserver, which end by themselves once the client is gone). Err when
/// something says the client is still in use.
fn leftovers(l: &Launch, procs: &[Proc]) -> Result<Vec<(i32, u64)>, String> {
    let me = std::process::id() as i32;
    let tagged: Vec<&Proc> = procs.iter().filter(|p| !p.zombie && p.pid != me && l.tagged(p)).collect();
    if let Some(p) = tagged.iter().find(|p| in_game(p, l.info.folder.as_deref())) {
        return Err(format!("the game still runs (pid {})", p.pid));
    }
    let mut out = vec![];
    for p in tagged {
        let argv0 = p.cmdline.first().map(String::as_str).unwrap_or("");
        if is_wine_system(argv0) {
            continue;
        }
        if let Some(img) = image(&p.cmdline).filter(|i| i.ends_with(".exe"))
            && !image_under(&img, &l.client_dirs.iter().cloned().collect::<Vec<_>>())
        {
            return Err(format!("{img} runs from it (another game?)"));
        }
        out.push((p.pid, p.start));
    }
    Ok(out)
}

/// Pids of processes matching `root` and all their descendants.
fn descendants_incl(procs: &[Proc], root: impl Fn(&Proc) -> bool) -> HashSet<i32> {
    let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
    for p in procs {
        children.entry(p.ppid).or_default().push(p.pid);
    }
    let mut out = HashSet::new();
    let mut stack: Vec<i32> = procs.iter().filter(|p| root(p)).map(|p| p.pid).collect();
    while let Some(pid) = stack.pop() {
        if out.insert(pid)
            && let Some(c) = children.get(&pid)
        {
            stack.extend(c.iter().copied());
        }
    }
    out
}

/// The process runs from the game's folder (executable, working directory or program path; Wine
/// `C:\…` paths through its prefix's `drive_c`).
fn in_game(p: &Proc, folder: Option<&Path>) -> bool {
    let Some(dir) = folder else { return false };
    let inside = |x: &Path| track::is_inside(x, dir) || lower(x).starts_with(&lower(dir));
    if p.exe.as_deref().is_some_and(inside) || p.cwd.as_deref().is_some_and(inside) {
        return true;
    }
    let Some(a0) = p.cmdline.first() else { return false };
    if let Some(u) = track::unix_path_of(a0) {
        return inside(&u);
    }
    // `C:\Program Files\Game\game.exe` → `<prefix>/drive_c/Program Files/Game/game.exe`.
    if let (Some(rest), Some(pfx)) = (a0.strip_prefix("C:").or_else(|| a0.strip_prefix("c:")), &p.prefix) {
        let pfx = if pfx.join("drive_c").is_dir() || !pfx.join("pfx").is_dir() { pfx.clone() } else { pfx.join("pfx") };
        let u = pfx.join("drive_c").join(rest.replace('\\', "/").trim_start_matches('/'));
        return inside(&u);
    }
    false
}

fn lower(p: &Path) -> String {
    let s = p.to_string_lossy().to_lowercase();
    let s = s.trim_end_matches('/');
    format!("{s}/")
}

/// The program a process runs, normalised (`c:/program files/x/x.exe`; after a Wine loader).
fn image(cmdline: &[String]) -> Option<String> {
    crate::client_launch::procs::Proc { cmdline: cmdline.to_vec(), ..Default::default() }.image()
}

/// The folder of a Windows program (normalised, no trailing `/`); None for anything that isn't a
/// `.exe` (umu / pressure-vessel / Lodestar's own helper) or a bare `game.exe`.
fn exe_dir(cmdline: &[String]) -> Option<String> {
    let img = image(cmdline).filter(|i| i.ends_with(".exe"))?;
    let (dir, _) = img.rsplit_once('/')?;
    (!dir.is_empty()).then(|| dir.to_string())
}

fn base_name(argv0: &str) -> String {
    argv0.rsplit(['/', '\\']).next().unwrap_or(argv0).to_lowercase()
}

/// Wine's (and Proton's) own processes: never minimised or signalled.
fn is_wine_system(argv0: &str) -> bool {
    matches!(
        base_name(argv0).as_str(),
        "wineserver"
            | "wine"
            | "wine64"
            | "wine-preloader"
            | "wine64-preloader"
            | "explorer.exe"
            | "services.exe"
            | "winedevice.exe"
            | "plugplay.exe"
            | "svchost.exe"
            | "rpcss.exe"
            | "conhost.exe"
            | "start.exe"
            | "winedbg.exe"
            | "tabtip.exe"
            | "rundll32.exe"
            | "wineboot.exe"
            | "winemenubuilder.exe"
            | "xalia.exe"
            | "steam.exe"
    )
}

/// `/opt/Heroic/heroic`, `/app/bin/heroic/heroic` (Flatpak), `/tmp/.mount_Heroic…/heroic`.
fn is_heroic_process(cmdline: &[String]) -> bool {
    cmdline.first().is_some_and(|a| matches!(base_name(a).as_str(), "heroic" | "heroic.bin"))
}

/// The Steam client's own processes (whose windows are Steam's library, popups, "launching…"):
/// `…/ubuntu12_32/steam` and `steamwebhelper` (native or Flatpak). Never the game: games run
/// through `reaper` / Proton, which have other names.
fn is_steam_client_process(cmdline: &[String]) -> bool {
    cmdline.first().is_some_and(|a| {
        let b = base_name(a);
        b == "steamwebhelper" || (b == "steam" && (a.contains("ubuntu12_32") || a.contains("Steam/")))
    })
}

/// Heroic's main (Electron browser) process: not one of its `--type=…` helpers.
pub fn is_heroic_main(cmdline: &[String]) -> bool {
    is_heroic_process(cmdline) && !cmdline.iter().any(|a| a.starts_with("--type="))
}

/// Heroic runs now (checked when a `heroic://` link is opened).
pub fn heroic_running(procs: &[track::ProcInfo]) -> bool {
    procs.iter().any(|p| !p.zombie && p.cmdline.as_deref().is_some_and(is_heroic_main))
}

/// Processes under `root` (a /proc), with environment facts for those readable (ours); the
/// executable / working directory / pid namespaces only where hygiene can need them.
pub fn read_procs(root: &Path) -> Vec<Proc> {
    let mut out = vec![];
    for p in track::read_procs_in(root, true) {
        let dir = root.join(p.pid.to_string());
        let mut q = Proc {
            pid: p.pid,
            ppid: p.ppid,
            start: p.start,
            zombie: p.zombie,
            cmdline: p.cmdline.unwrap_or_default(),
            ..Default::default()
        };
        if let Some(env) = procfs::read_environ(root, p.pid) {
            for (k, v) in procfs::env_entries(&env) {
                if k == b"WINEPREFIX" {
                    q.prefix = Some(PathBuf::from(String::from_utf8_lossy(v).into_owned()));
                } else if k == TAG_VAR.as_bytes() {
                    q.tag = Some(String::from_utf8_lossy(v).into_owned());
                }
            }
        }
        if q.prefix.is_some() || q.tag.is_some() || is_heroic_process(&q.cmdline) {
            q.exe = std::fs::read_link(dir.join("exe")).ok();
            q.cwd = std::fs::read_link(dir.join("cwd")).ok();
            if let Ok(st) = std::fs::read_to_string(dir.join("status"))
                && let Some(l) = st.lines().find(|l| l.starts_with("NSpid:"))
            {
                q.nspids = l.split_whitespace().skip(2).filter_map(|x| x.parse().ok()).collect();
            }
        }
        out.push(q);
    }
    out
}

/// Start the hygiene thread; it ends when the sender is dropped.
pub fn spawn(cfg: HygieneConfig) -> Sender<Msg> {
    spawn_with(Hygiene::new(RealSys::default(), cfg))
}

pub fn spawn_with<S: Sys>(h: Hygiene<S>) -> Sender<Msg> {
    let (tx, rx) = unbounded();
    let r = std::thread::Builder::new().name("launch-hygiene".into()).spawn(move || run(h, rx));
    if let Err(e) = r {
        log::warn!("couldn't start the launcher hygiene thread: {e}");
    }
    tx
}

fn run<S: Sys>(mut h: Hygiene<S>, rx: Receiver<Msg>) {
    loop {
        let msg = match h.next_due() {
            // Nothing scheduled: sleep until the next message.
            None => match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => return,
            },
            Some(t) => match rx.recv_timeout(t.saturating_duration_since(Instant::now())) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            },
        };
        if let Some(m) = msg {
            h.handle(m, Instant::now());
        }
        h.run_due(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct World {
        procs: Vec<Proc>,
        signals: Vec<(i32, i32)>,
        iconified: Vec<Vec<u32>>,
        records: Vec<StartedClient>,
        /// Records closed (by their client tag).
        closed: Vec<String>,
    }

    #[derive(Clone, Default)]
    struct Fake(Arc<Mutex<World>>);

    impl Sys for Fake {
        fn procs(&mut self) -> Vec<Proc> {
            self.0.lock().unwrap().procs.clone()
        }
        fn signal(&mut self, pid: i32, start: u64, sig: i32) {
            let mut w = self.0.lock().unwrap();
            if w.procs.iter().any(|p| p.pid == pid && p.start == start) {
                w.signals.push((pid, sig));
            }
        }
        fn iconify(&mut self, pids: &[u32]) -> usize {
            let mut v = pids.to_vec();
            v.sort();
            self.0.lock().unwrap().iconified.push(v);
            1
        }
        fn started_clients(&mut self) -> Vec<StartedClient> {
            self.0.lock().unwrap().records.clone()
        }
        fn close_client(&mut self, c: StartedClient) {
            let mut w = self.0.lock().unwrap();
            w.closed.push(c.tag.clone());
            w.records.retain(|r| r.tag != c.tag);
        }
        fn steam_record(&mut self) -> Option<SteamStarted> {
            None
        }
        fn save_steam_record(&mut self, _: &SteamStarted) {}
        fn remove_steam_record(&mut self) {}
        fn steam_logs(&mut self, _: &SteamStarted) -> (steam_ctl::LogSig, String) {
            Default::default()
        }
        fn steam_shutdown(&mut self, _: bool) {
            panic!("no Steam record: never shut down");
        }
    }

    fn cmd(s: &str) -> Vec<String> {
        s.split('|').map(str::to_string).collect()
    }

    fn proc_(pid: i32, ppid: i32, cmdline: &str) -> Proc {
        Proc { pid, ppid, start: pid as u64 * 10, cmdline: cmd(cmdline), ..Default::default() }
    }

    fn wine(pid: i32, ppid: i32, cmdline: &str, prefix: &str, tag: Option<&str>) -> Proc {
        Proc { prefix: Some(prefix.into()), tag: tag.map(str::to_string), ..proc_(pid, ppid, cmdline) }
    }

    fn info(id: GameId, client: Client, tag: &str, prefix: Option<&str>, heroic_was_running: bool) -> LaunchInfo {
        LaunchInfo {
            id,
            tag: tag.into(),
            folder: Some("/games/bnet/drive_c/Program Files (x86)/Overwatch".into()),
            prefix: prefix.map(PathBuf::from),
            client,
            heroic_was_running,
        }
    }

    fn hygiene(w: &Fake) -> Hygiene<Fake> {
        let cfg = HygieneConfig { hide: true, close: true, hide_steam: true, ..Default::default() };
        Hygiene::new(w.clone(), cfg).with_timing(
            vec![Duration::ZERO, Duration::from_secs(5)],
            Duration::from_secs(2),
            Duration::from_secs(3),
        )
    }

    const PFX: &str = "/games/bnet";
    const BNET: &str = "C:\\Program Files (x86)\\Battle.net\\Battle.net.exe";

    /// Battle.net started by the launch (tagged) with its helper and wineserver, the game run by it.
    fn bnet_world(tag: &str) -> Vec<Proc> {
        let mut game = wine(40, 10, "C:\\Program Files (x86)\\Overwatch\\_retail_\\Overwatch.exe", PFX, Some(tag));
        game.cwd = Some(format!("{PFX}/drive_c/Program Files (x86)/Overwatch/_retail_").into());
        let mut ui = wine(10, 1, BNET, PFX, Some(tag));
        ui.nspids = vec![7];
        vec![
            ui,
            wine(11, 10, "C:\\Program Files (x86)\\Battle.net\\Battle.net Helper.exe|--type=renderer", PFX, Some(tag)),
            wine(12, 1, "/opt/proton/files/bin/wineserver", PFX, Some(tag)),
            wine(13, 1, "C:\\windows\\system32\\explorer.exe|/desktop", PFX, Some(tag)),
            game,
            wine(41, 40, "C:\\Program Files (x86)\\Overwatch\\crash.exe", PFX, Some(tag)),
            proc_(99, 1, "/usr/bin/konsole"),
        ]
    }

    #[test]
    fn hides_the_client_not_the_game_and_closes_what_it_started() {
        let w = Fake::default();
        w.0.lock().unwrap().procs = bnet_world("t1");
        let mut h = hygiene(&w);
        let t0 = Instant::now();
        h.handle(Msg::Launch(info(1, Client::Wine, "t1", Some(PFX), false)), t0);
        h.run_due(t0);
        assert!(w.0.lock().unwrap().iconified.is_empty(), "nothing before the game runs");
        h.handle(Msg::Started(1), t0);
        h.run_due(t0);
        // Battle.net and its helper (also by its namespace pid), not the game, its child, Wine's
        // own processes or anything else.
        assert_eq!(w.0.lock().unwrap().iconified, vec![vec![7, 10, 11]]);
        h.run_due(t0 + Duration::from_secs(5));
        assert_eq!(w.0.lock().unwrap().iconified.len(), 2, "re-checked later");
        // Game exits: the client closes a little later (not wineserver / explorer).
        w.0.lock().unwrap().procs.retain(|p| p.pid != 40 && p.pid != 41);
        let t1 = t0 + Duration::from_secs(60);
        h.handle(Msg::Exited(1), t1);
        h.run_due(t1);
        assert!(w.0.lock().unwrap().signals.is_empty());
        h.run_due(t1 + Duration::from_secs(2));
        assert_eq!(w.0.lock().unwrap().signals, vec![(10, libc::SIGTERM), (11, libc::SIGTERM)]);
        // The helper ignored it.
        w.0.lock().unwrap().procs.retain(|p| p.pid != 10);
        h.run_due(t1 + Duration::from_secs(5));
        assert_eq!(w.0.lock().unwrap().signals.last(), Some(&(11, libc::SIGKILL)));
        assert!(h.next_due().is_none(), "idle again");
    }

    #[test]
    fn a_client_the_user_had_open_stays_open() {
        let w = Fake::default();
        // Nothing carries the launch's tag: the client was running before.
        let mut procs = bnet_world("user");
        for p in &mut procs {
            if p.tag.is_some() {
                p.tag = None;
            }
        }
        w.0.lock().unwrap().procs = procs;
        let mut h = hygiene(&w);
        let t0 = Instant::now();
        h.handle(Msg::Launch(info(1, Client::Wine, "t1", Some("/games/bnet/"), false)), t0);
        h.handle(Msg::Started(1), t0);
        h.run_due(t0);
        assert_eq!(w.0.lock().unwrap().iconified, vec![vec![7, 10, 11]], "still minimised (same prefix)");
        w.0.lock().unwrap().procs.retain(|p| p.pid != 40 && p.pid != 41);
        h.handle(Msg::Exited(1), t0);
        h.run_due(t0 + Duration::from_secs(30));
        assert!(w.0.lock().unwrap().signals.is_empty());
    }

    #[test]
    fn another_game_started_from_the_client_keeps_it_open() {
        let w = Fake::default();
        w.0.lock().unwrap().procs = bnet_world("t1");
        let mut h = hygiene(&w);
        let t0 = Instant::now();
        h.handle(Msg::Launch(info(1, Client::Wine, "t1", Some(PFX), false)), t0);
        h.handle(Msg::Started(1), t0);
        h.run_due(t0 + Duration::from_secs(10));
        {
            let mut w = w.0.lock().unwrap();
            w.procs.retain(|p| p.pid != 40 && p.pid != 41);
            // The user pressed Play on another game in Battle.net.
            w.procs.push(wine(50, 10, "C:\\Program Files (x86)\\Diablo IV\\Diablo IV.exe", PFX, Some("t1")));
        }
        h.handle(Msg::Exited(1), t0 + Duration::from_secs(20));
        h.run_due(t0 + Duration::from_secs(30));
        assert!(w.0.lock().unwrap().signals.is_empty());
    }

    #[test]
    fn nothing_is_closed_when_the_game_never_showed_up_or_closing_is_off() {
        let w = Fake::default();
        w.0.lock().unwrap().procs = bnet_world("t1");
        let mut h = hygiene(&w);
        let t0 = Instant::now();
        h.handle(Msg::Launch(info(1, Client::Wine, "t1", Some(PFX), false)), t0);
        h.handle(Msg::Exited(1), t0);
        h.run_due(t0 + Duration::from_secs(30));
        let cfg = HygieneConfig { hide: false, close: false, hide_steam: false, ..Default::default() };
        h.handle(Msg::Config(cfg), t0);
        h.handle(Msg::Launch(info(2, Client::Wine, "t1", Some(PFX), false)), t0);
        h.handle(Msg::Started(2), t0);
        h.run_due(t0 + Duration::from_secs(1));
        h.handle(Msg::Exited(2), t0 + Duration::from_secs(1));
        h.run_due(t0 + Duration::from_secs(30));
        let w = w.0.lock().unwrap();
        assert!(w.signals.is_empty() && w.iconified.is_empty());
    }

    #[test]
    fn switching_to_a_game_of_the_same_client_keeps_it_open_until_that_one_ends() {
        let w = Fake::default();
        w.0.lock().unwrap().procs = bnet_world("t1");
        let mut h = hygiene(&w);
        let t0 = Instant::now();
        h.handle(Msg::Launch(info(1, Client::Wine, "t1", Some(PFX), false)), t0);
        h.handle(Msg::Started(1), t0);
        h.run_due(t0);
        w.0.lock().unwrap().procs.retain(|p| p.pid != 40 && p.pid != 41);
        h.handle(Msg::Exited(1), t0);
        // The next game goes through the same Battle.net (its own launch, tag t2) before the close.
        h.handle(Msg::Launch(info(2, Client::Wine, "t2", Some("/games/bnet/pfx"), false)), t0 + Duration::from_secs(1));
        h.run_due(t0 + Duration::from_secs(10));
        assert!(w.0.lock().unwrap().signals.is_empty(), "closed under the next game");
        h.handle(Msg::Started(2), t0 + Duration::from_secs(10));
        h.run_due(t0 + Duration::from_secs(10));
        h.handle(Msg::Exited(2), t0 + Duration::from_secs(20));
        h.run_due(t0 + Duration::from_secs(25));
        // Now the client Lodestar started for the first game closes.
        assert_eq!(w.0.lock().unwrap().signals, vec![(10, libc::SIGTERM), (11, libc::SIGTERM)]);
    }

    fn record(client_tag: &str, game_tag: &str) -> StartedClient {
        StartedClient {
            kind: "battlenet".into(),
            client: "battlenet".into(),
            prefix: PFX.into(),
            exe: format!("{PFX}/drive_c/Program Files (x86)/Battle.net/Battle.net.exe").into(),
            client_dirs: vec![format!("{PFX}/drive_c/Program Files (x86)/Battle.net").into()],
            main: vec!["Battle.net.exe".into()],
            tag: client_tag.into(),
            game_tag: Some(game_tag.into()),
            ..Default::default()
        }
    }

    /// The helper started Battle.net for launch t1: the client has its own tag (not the game's).
    fn helper_world() -> Vec<Proc> {
        let mut v = bnet_world("t1");
        for p in &mut v {
            if p.pid != 40 && p.pid != 41 {
                p.tag = None;
            }
        }
        v
    }

    #[test]
    fn clients_the_helper_started_for_this_launch_are_closed_through_their_record() {
        let w = Fake::default();
        {
            let mut w = w.0.lock().unwrap();
            w.procs = helper_world();
            w.records = vec![record("c1", "t1"), record("c-other", "t9")];
        }
        let mut h = hygiene(&w);
        let t0 = Instant::now();
        h.handle(Msg::Launch(info(1, Client::Wine, "t1", Some(PFX), false)), t0);
        h.handle(Msg::Started(1), t0);
        h.run_due(t0);
        assert_eq!(w.0.lock().unwrap().iconified, vec![vec![7, 10, 11]], "found by its prefix");
        // While the game runs nothing is closed.
        h.run_due(t0 + Duration::from_secs(30));
        assert!(w.0.lock().unwrap().closed.is_empty());
        w.0.lock().unwrap().procs.retain(|p| p.pid != 40 && p.pid != 41);
        h.handle(Msg::Exited(1), t0 + Duration::from_secs(40));
        h.run_due(t0 + Duration::from_secs(45));
        let w = w.0.lock().unwrap();
        assert_eq!(w.closed, vec!["c1".to_string()], "only this launch's client");
        assert!(w.signals.is_empty(), "closing is the record's business");
    }

    #[test]
    fn a_recorded_client_stays_while_another_game_runs_in_its_prefix() {
        let w = Fake::default();
        {
            let mut w = w.0.lock().unwrap();
            w.procs = helper_world();
            w.records = vec![record("c1", "t1")];
        }
        let mut h = hygiene(&w);
        let t0 = Instant::now();
        h.handle(Msg::Launch(info(1, Client::Wine, "t1", Some(PFX), false)), t0);
        h.handle(Msg::Started(1), t0);
        h.run_due(t0 + Duration::from_secs(10));
        {
            let mut w = w.0.lock().unwrap();
            w.procs.retain(|p| p.pid != 40 && p.pid != 41);
            w.procs.push(wine(60, 10, "C:\\Program Files (x86)\\Diablo IV\\Diablo IV.exe", PFX, None));
        }
        h.handle(Msg::Exited(1), t0 + Duration::from_secs(20));
        h.run_due(t0 + Duration::from_secs(30));
        let w = w.0.lock().unwrap();
        assert!(w.closed.is_empty() && w.signals.is_empty());
    }

    #[test]
    fn finish_closes_at_once() {
        let w = Fake::default();
        {
            let mut w = w.0.lock().unwrap();
            w.procs = helper_world();
            w.records = vec![record("c1", "t1")];
        }
        let cfg = HygieneConfig { hide: true, close: true, hide_steam: true, ..Default::default() };
        // Real waits: finish must not sit them out (the close delay is far longer than the test).
        let mut h = Hygiene::new(w.clone(), cfg).with_timing(
            vec![Duration::ZERO, Duration::from_secs(600)],
            Duration::from_secs(600),
            Duration::from_millis(50),
        );
        let t0 = Instant::now();
        h.handle(Msg::Launch(info(1, Client::Wine, "t1", Some(PFX), false)), t0);
        h.handle(Msg::Started(1), t0);
        h.run_due(t0);
        w.0.lock().unwrap().procs.retain(|p| p.pid != 40 && p.pid != 41);
        h.handle(Msg::Exited(1), t0);
        let (tx, rx) = unbounded();
        h.handle(Msg::Finish(tx), Instant::now());
        assert!(rx.try_recv().is_ok());
        assert!(t0.elapsed() < Duration::from_secs(5));
        assert_eq!(w.0.lock().unwrap().closed, vec!["c1".to_string()]);
        assert!(h.next_due().is_none());
    }

    fn heroic_world() -> Vec<Proc> {
        vec![
            proc_(20, 1, "/app/bin/heroic/heroic|--no-sandbox"),
            proc_(21, 20, "/app/bin/heroic/heroic|--type=zygote"),
            proc_(22, 20, "/app/bin/heroic/heroic|--type=renderer"),
            proc_(30, 1, "/usr/bin/bwrap|--args|40|--|/app/bin/heroic-run"),
        ]
    }

    #[test]
    fn heroic_is_closed_only_when_lodestar_started_it() {
        for was_running in [false, true] {
            let w = Fake::default();
            w.0.lock().unwrap().procs = heroic_world();
            let mut h = hygiene(&w);
            let t0 = Instant::now();
            h.handle(Msg::Launch(info(3, Client::Heroic, "t3", None, was_running)), t0);
            h.handle(Msg::Started(3), t0);
            h.run_due(t0);
            assert_eq!(w.0.lock().unwrap().iconified, vec![vec![20, 21, 22]]);
            h.handle(Msg::Exited(3), t0);
            h.run_due(t0 + Duration::from_secs(10));
            let want = if was_running { vec![] } else { vec![(20, libc::SIGTERM)] };
            assert_eq!(w.0.lock().unwrap().signals, want, "was running: {was_running}");
        }
    }

    #[test]
    fn steam_windows_are_minimised_but_steam_is_never_closed() {
        let w = Fake::default();
        w.0.lock().unwrap().procs = vec![
            proc_(50, 1, "/home/u/.local/share/Steam/ubuntu12_32/steam|-srt-logger-opened"),
            proc_(51, 50, "/home/u/.local/share/Steam/ubuntu12_64/steamwebhelper|-lang=en_US"),
            proc_(60, 50, "/home/u/.local/share/Steam/ubuntu12_32/reaper|SteamLaunch|AppId=570"),
            proc_(61, 60, "/home/u/.local/share/Steam/steamapps/common/dota 2 beta/game/bin/linuxsteamrt64/dota2"),
            proc_(70, 1, "/usr/bin/steam-not-the-client-helper"),
        ];
        let mut h = hygiene(&w);
        let t0 = Instant::now();
        h.handle(Msg::Launch(info(5, Client::Steam, "t5", None, false)), t0);
        h.handle(Msg::Started(5), t0);
        h.run_due(t0);
        assert_eq!(w.0.lock().unwrap().iconified, vec![vec![50, 51]], "only Steam's own windows");
        h.handle(Msg::Exited(5), t0);
        h.run_due(t0 + Duration::from_secs(30));
        assert!(w.0.lock().unwrap().signals.is_empty(), "Steam is never closed");
        assert!(w.0.lock().unwrap().closed.is_empty());
    }

    #[test]
    fn heroic_detection_from_process_lists() {
        let pi = |pid: i32, c: &str| track::ProcInfo {
            pid,
            ppid: 1,
            pgrp: pid,
            sid: pid,
            start: 1,
            zombie: false,
            cmdline: Some(cmd(c)),
        };
        assert!(heroic_running(&[pi(1, "/usr/bin/bash"), pi(2, "/opt/Heroic/heroic")]));
        assert!(heroic_running(&[pi(2, "/tmp/.mount_HeroicXyz/heroic|--ozone-platform=wayland")]));
        assert!(
            !heroic_running(&[pi(2, "/opt/Heroic/heroic|--type=gpu-process")]),
            "a leftover helper isn't Heroic running"
        );
        assert!(!heroic_running(&[pi(2, "/usr/bin/xdg-open|heroic://launch/gog/1")]));
        assert!(!heroic_running(&[pi(2, "/home/u/.config/heroic/tools/legendary")]));
        assert!(is_heroic_main(&cmd("/app/bin/heroic/heroic")));
    }

    #[test]
    fn helpers() {
        assert!(same_prefix(Path::new("/g/umu/x/"), Path::new("/g/umu/x/pfx")));
        assert!(!same_prefix(Path::new("/g/a"), Path::new("/g/b")));
        assert!(is_wine_system("C:\\windows\\system32\\services.exe"));
        assert!(is_wine_system("/usr/lib/wine/wineserver"));
        assert!(!is_wine_system(BNET));
        assert_eq!(exe_dir(&cmd(BNET)).as_deref(), Some("c:/program files (x86)/battle.net"));
        assert_eq!(exe_dir(&cmd("/usr/bin/wine64|C:\\Games\\x.exe")).as_deref(), Some("c:/games"));
        assert_eq!(exe_dir(&cmd("/usr/bin/umu-run|x")), None);
        assert_eq!(exe_dir(&cmd("game.exe")), None);
        let mut p = proc_(1, 0, "C:\\Program Files\\EA Games\\The Sims 4\\Game\\Bin\\TS4_x64.exe");
        p.prefix = Some("/nonexistent/ea".into());
        assert!(in_game(&p, Some(Path::new("/nonexistent/ea/drive_c/Program Files/EA Games/The Sims 4"))));
        assert!(!in_game(&p, Some(Path::new("/nonexistent/ea/drive_c/Program Files/EA Games/The Sims"))));
        assert!(!in_game(&p, None));
    }

    fn alive(pid: i32) -> bool {
        procfs::read_stat(pid).is_some_and(|p| !p.zombie)
    }

    /// The whole path with harmless processes: a launch through a "store client" (a tagged
    /// `sleep` outside the game folder) whose "game" runs in its folder; once the game exits, the
    /// client the launch started is ended, one the user started stays.
    #[test]
    fn launcher_closes_the_client_its_launch_started_after_the_game() {
        use crate::launch::{LaunchPlan, Launcher, Resolved, ResolvedKind};
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("Game");
        std::fs::create_dir_all(&game).unwrap();
        let pidfile = dir.path().join("client.pid");
        let mut user = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let script = format!("sleep 30 & echo $! > '{}'; cd '{}' && sleep 1.5", pidfile.display(), game.display());
        let r = Resolved {
            program: "sh".into(),
            args: vec!["-c".into(), script.clone()],
            env: vec![("WINEPREFIX".into(), dir.path().join("pfx").display().to_string())],
            cwd: None,
            kind: ResolvedKind::Process,
            display: script,
            notes: vec![],
            options: vec![],
            steam_string: None,
            make_dirs: vec![],
        };
        let plan = LaunchPlan { install_dir: Some(game.clone()), via_client: true, ..Default::default() };
        let mut l = Launcher::with_log_dir(&egui::Context::default(), dir.path().join("logs"));
        l.poll_every = Duration::from_millis(100);
        l.set_hygiene(HygieneConfig { hide: true, close: true, hide_steam: true, ..Default::default() });
        l.launch_planned(70, "Client game", &r, &plan).unwrap();
        let t = Instant::now();
        let mut evs = vec![];
        while t.elapsed() < Duration::from_secs(15)
            && !evs.iter().any(|e| matches!(e, crate::launch::LaunchEvent::Exited { .. }))
        {
            evs.extend(l.poll());
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(evs.contains(&crate::launch::LaunchEvent::Started { id: 70 }), "{evs:?}");
        let client: i32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
        assert!(alive(client), "the client outlives the game");
        let t = Instant::now();
        while alive(client) && t.elapsed() < CLOSE_DELAY + Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(100));
        }
        let closed = !alive(client);
        let user_alive = alive(user.id() as i32);
        let _ = user.kill();
        let _ = user.wait();
        if !closed {
            unsafe { libc::kill(client, libc::SIGKILL) };
        }
        assert!(closed, "the client the launch started wasn't closed");
        assert!(user_alive, "a process the launch didn't start was touched");
    }

    #[test]
    fn reads_this_process_from_proc() {
        let me = std::process::id() as i32;
        let procs = read_procs(Path::new("/proc"));
        let p = procs.iter().find(|p| p.pid == me).expect("own process");
        assert!(!p.cmdline.is_empty());
    }
}
