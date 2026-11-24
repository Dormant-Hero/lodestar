//! Achievements on a game's page: Steam games from the Steam client's own files (with an optional
//! Steam Web API key), emulated games from RetroAchievements' public Web API (the user's username
//! and Web API key). Lodestar only shows progress; unlocking is the game's / emulator's business.
//!
//! One background worker (`Hub`) answers requests; the UI asks for the game it shows
//! (`AppState::achievements.request`) and reads `state(id)` every frame. Results are kept in
//! memory for the session and on disk by each service's module (`steam`, `ra`), so pages open
//! instantly the next time. Nothing here ever blocks the UI thread.
//!
//! RetroAchievements' rules: third-party apps use the public Web API with the user's own key,
//! never the emulators' login (`dorequest.php`), and send a descriptive User-Agent. Lodestar never
//! asks for the RetroAchievements password and never reads or writes emulators' login tokens.

pub mod ra;
pub mod steam;
pub mod summary;

use crate::config::AchievementsConfig;
use crate::model::{Game, GameId};
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A shown result is asked again after this long (a page left open, then revisited).
const STALE_AFTER: Duration = Duration::from_secs(10 * 60);
/// Changed library counts are written to `summary.json` this long after the first change.
const SAVE_AFTER: Duration = Duration::from_secs(5);

/// Where a game's achievements come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Service {
    Steam,
    RetroAchievements,
}

impl Service {
    pub fn label(self) -> &'static str {
        match self {
            Service::Steam => "Steam",
            Service::RetroAchievements => "RetroAchievements",
        }
    }
}

/// One achievement, in the service's display order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Achievement {
    /// The service's id (Steam API name, RetroAchievements achievement id).
    pub id: String,
    pub name: String,
    /// Empty for a hidden achievement that is still locked.
    pub description: String,
    /// Picture URL (unlocked look) and the locked (grey) one, when the service has them.
    pub icon: Option<String>,
    pub icon_locked: Option<String>,
    /// Unix seconds; None = locked.
    pub unlocked_at: Option<i64>,
    /// RetroAchievements: earned in hardcore mode.
    pub hardcore: bool,
    /// Hidden until unlocked (its description is withheld while locked).
    pub hidden: bool,
    /// Share of players who have it, 0..=100, when known.
    pub rarity: Option<f32>,
}

/// A game's achievements and the player's progress.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub service: Service,
    pub unlocked: u32,
    pub total: u32,
    /// Every achievement, in display order.
    pub list: Vec<Achievement>,
    /// The full list on the service's site.
    pub page_url: Option<String>,
    /// Shown under the count, e.g. "Matched by title — your ROM version may not be supported".
    pub note: Option<String>,
    /// True when the player's own progress is known (false: only the list, all shown locked).
    pub synced: bool,
    /// Matched by title only (RetroAchievements): shown on the game's page with a note, never
    /// as a count in the library (it may be another version of the game).
    pub probable: bool,
}

impl Progress {
    /// Unlocked achievements, newest first.
    pub fn recent(&self) -> Vec<&Achievement> {
        let mut v: Vec<&Achievement> = self.list.iter().filter(|a| a.unlocked_at.is_some()).collect();
        v.sort_by_key(|a| std::cmp::Reverse(a.unlocked_at));
        v
    }
}

/// What the game page shows for a game.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// Asked; nothing known yet.
    Loading,
    Ready(Arc<Progress>),
    /// The game has no achievements here (or isn't a kind Lodestar can look up): no section.
    None,
    /// One plain line with what to do, e.g. "Add your RetroAchievements username and Web API key
    /// in Settings → Achievements to see your progress".
    Hint(String),
}

/// Everything a lookup needs, copied so the worker never touches app state.
#[derive(Clone, Debug)]
pub struct Request {
    pub game: Game,
    pub config: AchievementsConfig,
    /// Ask the service again even when a recent answer is cached (after the game was played).
    pub force: bool,
}

/// Paths and transport the providers use.
pub struct Env {
    /// `<cache>/achievements`: per-service caches (lists, progress, hashes, pictures).
    pub cache_dir: PathBuf,
    pub net: Arc<dyn crate::art::net::Net>,
}

/// Look one game up (worker thread; may block on disk or network).
pub fn fetch(req: &Request, env: &Env) -> State {
    if !req.config.show {
        return State::None;
    }
    if steam::applies(&req.game) {
        if !req.config.steam {
            return State::None;
        }
        return steam::load(req, env);
    }
    if ra::applies(&req.game) {
        if !req.config.retroachievements {
            return State::None;
        }
        return ra::load(req, env);
    }
    State::None
}

/// One account check's answer.
type RaCheck = Result<RaAccount, String>;

/// A request still unanswered after this long is taken as lost and may be sent again.
const LOST_AFTER: Duration = Duration::from_secs(5 * 60);

/// What the worker gets: the request and the `Hub::clear` generation it was sent in.
type Job = (u64, Request);

/// The background worker and the answers so far.
pub struct Hub {
    tx: Sender<Job>,
    rx: Receiver<(u64, GameId, State)>,
    states: HashMap<GameId, (State, Instant)>,
    /// Games with a request on its way, and since when (an earlier answer may be shown meanwhile).
    pending: HashMap<GameId, Instant>,
    /// Bumped by `clear`: answers to requests sent before it (old keys / switches) are dropped.
    generation: u64,
    ctx: egui::Context,
    net: Arc<dyn crate::art::net::Net>,
    cache_dir: PathBuf,
    ra_check: Option<(Receiver<RaCheck>, Option<RaCheck>)>,
    lib: Library,
}

/// The library's counts (`summary`): what is known, what may be shown, the pass in the background.
#[derive(Default)]
struct Library {
    all: HashMap<GameId, summary::Summary>,
    /// The RetroAchievements account `all`'s RetroAchievements counts belong to.
    ra_user: String,
    /// What the UI shows (`all` filtered by the settings), rebuilt when either changes.
    shown: Arc<HashMap<GameId, (u32, u32)>>,
    /// Bumped whenever `shown` changes (views sorted by achievements compare it).
    generation: u64,
    dirty: bool,
    /// The settings `shown` was made for.
    shown_for: Option<(bool, bool, bool, bool)>,
    /// The library pass's thread (started by the first `sync`).
    worker: Option<(Sender<summary::Cmd>, Receiver<summary::Out>)>,
    /// Screenshot runs: made-up counts, no pass.
    fake: bool,
    /// Library generation and fingerprint last sent to the pass.
    sent_generation: Option<u64>,
    sent_fingerprint: Option<u64>,
    sent_config: Option<AchievementsConfig>,
    /// Game pages to ask again later (a Steam game exited: Steam writes its files a little after).
    later: Vec<(Instant, GameId)>,
    /// Unsaved changes are written then.
    save_at: Option<Instant>,
    loaded: bool,
}

/// The next job: the newest one, with every older job for the same game folded into it (only the
/// newest settings matter; a forced refresh stays forced).
fn take_newest(queue: &mut Vec<Job>) -> Option<Job> {
    let (generation, mut req) = queue.pop()?;
    let id = req.game.id;
    queue.retain(|(_, r)| {
        if r.game.id != id {
            return true;
        }
        req.force |= r.force;
        false
    });
    Some((generation, req))
}

/// How the worker looks a game up ([`fetch`]; tests put a fake in).
type Lookup = dyn Fn(&Request, &Env) -> State + Send;

/// Look one game up; a provider that panics shows no section instead of taking the worker down.
fn fetch_guarded(lookup: &Lookup, req: &Request, env: &Env) -> State {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lookup(req, env))).unwrap_or_else(|_| {
        log::warn!("achievements: looking up game {} failed", req.game.id);
        State::None
    })
}

impl Hub {
    pub fn new(ctx: &egui::Context, cache_dir: PathBuf) -> Hub {
        Hub::with_net(ctx, cache_dir, Arc::new(crate::art::net::UreqNet::new()))
    }

    pub fn with_net(ctx: &egui::Context, cache_dir: PathBuf, net: Arc<dyn crate::art::net::Net>) -> Hub {
        Hub::with_lookup(ctx, cache_dir, net, Box::new(fetch))
    }

    fn with_lookup(
        ctx: &egui::Context,
        cache_dir: PathBuf,
        net: Arc<dyn crate::art::net::Net>,
        lookup: Box<Lookup>,
    ) -> Hub {
        let (tx, req_rx) = unbounded::<Job>();
        let (res_tx, rx) = unbounded();
        let hub_ctx = ctx.clone();
        let ctx = ctx.clone();
        let env = Env { cache_dir: cache_dir.clone(), net: net.clone() };
        let spawned = std::thread::Builder::new().name("achievements".into()).spawn(move || {
            // One lookup at a time, always the newest request first: the game on screen waits at
            // most for the lookup already running, never for a backlog of pages scrolled past.
            let mut queue: Vec<Job> = Vec::new();
            loop {
                if queue.is_empty() {
                    match req_rx.recv() {
                        Ok(j) => queue.push(j),
                        Err(_) => return,
                    }
                }
                queue.extend(req_rx.try_iter());
                let Some((generation, req)) = take_newest(&mut queue) else { continue };
                let state = fetch_guarded(&*lookup, &req, &env);
                if res_tx.send((generation, req.game.id, state)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        });
        if let Err(e) = spawned {
            log::warn!("achievements: no worker thread: {e}");
        }
        Hub {
            tx,
            rx,
            states: HashMap::new(),
            pending: HashMap::new(),
            generation: 0,
            ctx: hub_ctx,
            net,
            cache_dir,
            ra_check: None,
            lib: Library::default(),
        }
    }

    /// What is known about a game (None: never asked).
    pub fn state(&self, id: GameId) -> Option<&State> {
        self.states.get(&id).map(|(s, _)| s)
    }

    /// Ask for a game unless a fresh answer is there (or one is on its way). Cheap: call it every
    /// frame for the game on screen.
    pub fn request(&mut self, game: &Game, config: &AchievementsConfig) {
        if self.pending.get(&game.id).is_some_and(|at| at.elapsed() < LOST_AFTER) {
            return;
        }
        let fresh = self.states.get(&game.id).is_some_and(|(s, at)| *s != State::Loading && at.elapsed() < STALE_AFTER);
        if !fresh {
            self.send(game, config, false);
        }
    }

    /// Ask again now (after the game was played, or the settings changed).
    pub fn refresh(&mut self, game: &Game, config: &AchievementsConfig) {
        self.send(game, config, true);
    }

    /// Forget every answer (settings changed: keys, switches); answers still on their way are
    /// dropped when they arrive.
    pub fn clear(&mut self) {
        self.states.clear();
        self.pending.clear();
        self.generation += 1;
    }

    fn send(&mut self, game: &Game, config: &AchievementsConfig, force: bool) {
        let req = Request { game: game.clone(), config: config.clone(), force };
        if self.tx.send((self.generation, req)).is_err() {
            // No worker: show nothing rather than "loading" for ever.
            self.states.insert(game.id, (State::None, Instant::now()));
            return;
        }
        self.pending.insert(game.id, Instant::now());
        // Keep showing an earlier answer while the new one loads.
        let keep = self.states.get(&game.id).is_some_and(|(s, _)| matches!(s, State::Ready(_)));
        if !keep {
            self.states.insert(game.id, (State::Loading, Instant::now()));
        }
    }

    /// Check a RetroAchievements username + Web API key in the background; the answer shows in
    /// `ra_account_check`.
    pub fn check_ra_account(&mut self, username: &str, api_key: &str) {
        let (tx, rx) = unbounded();
        let (u, k, net, dir) =
            (username.trim().to_string(), api_key.trim().to_string(), self.net.clone(), self.cache_dir.clone());
        let ctx = self.ctx.clone();
        let spawned = std::thread::Builder::new().name("ra-check".into()).spawn(move || {
            let env = Env { cache_dir: dir, net };
            let _ = tx.send(ra::check_account(&u, &k, &env));
            ctx.request_repaint();
        });
        // No thread: the receiver reports the check as stopped (see `ra_account_check`).
        if let Err(e) = spawned {
            log::warn!("achievements: no thread for the account check: {e}");
        }
        self.ra_check = Some((rx, None));
    }

    /// None: never checked this session; Some(None): checking; Some(Some(answer)).
    pub fn ra_account_check(&mut self) -> Option<Option<&RaCheck>> {
        let (rx, answer) = self.ra_check.as_mut()?;
        if answer.is_none() {
            *answer = match rx.try_recv() {
                Ok(a) => Some(a),
                Err(crossbeam_channel::TryRecvError::Empty) => None,
                // The check's thread ended without an answer: never "checking…" for ever.
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    Some(Err("The check stopped before RetroAchievements answered. Try again.".into()))
                }
            };
        }
        Some(answer.as_ref())
    }

    /// Take finished answers; true when something changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        let answers: Vec<(u64, GameId, State)> = self.rx.try_iter().collect();
        for (generation, id, state) in answers {
            if generation != self.generation {
                continue;
            }
            self.pending.remove(&id);
            match &state {
                // Known but not countable (not synced, a title match): the library's count stays.
                State::Ready(p) => {
                    if let Some(s) = summary::Summary::of(p) {
                        self.set_summary(id, Some(s));
                    }
                }
                State::None if self.lib.all.contains_key(&id) => self.set_summary(id, None),
                _ => {}
            }
            self.states.insert(id, (state, Instant::now()));
            changed = true;
        }
        changed
    }

    // ------------------------------------------------------------------ library counts

    /// A game's count for the library ("31 / 54"), as the settings allow (None: nothing to
    /// show). One hash lookup: fine for every visible cover every frame.
    #[cfg(test)]
    pub fn count(&self, id: GameId) -> Option<(u32, u32)> {
        self.lib.shown.get(&id).copied()
    }

    /// Every count the library may show (cheap to clone: hold it while drawing a frame).
    pub fn counts(&self) -> &Arc<HashMap<GameId, (u32, u32)>> {
        &self.lib.shown
    }

    /// Changes whenever `counts` does.
    pub fn counts_generation(&self) -> u64 {
        self.lib.generation
    }

    /// Everything known (also what the settings hide), for tests and diagnostics.
    pub fn summary(&self, id: GameId) -> Option<summary::Summary> {
        self.lib.all.get(&id).copied()
    }

    fn set_summary(&mut self, id: GameId, s: Option<summary::Summary>) {
        let old = match s {
            Some(s) => self.lib.all.insert(id, s),
            None => self.lib.all.remove(&id),
        };
        if old != s {
            self.lib.dirty = true;
            self.lib.save_at.get_or_insert_with(|| Instant::now() + SAVE_AFTER);
        }
    }

    /// Screenshot runs: show these counts and never start the library pass.
    pub fn set_fake_counts(&mut self, counts: HashMap<GameId, summary::Summary>) {
        self.lib.fake = true;
        self.lib.all = counts;
        self.lib.dirty = true;
    }

    /// A game ended: ask its page again now and, for a Steam game, once more when Steam has
    /// written its files; the library's counts are looked at again too.
    pub fn game_exited(&mut self, game: &Game, config: &AchievementsConfig) {
        self.refresh(game, config);
        let cmd = if steam::applies(game) {
            self.lib.later.retain(|(_, id)| *id != game.id);
            self.lib.later.push((Instant::now() + summary::STEAM_AFTER_EXIT, game.id));
            self.ctx.request_repaint_after(summary::STEAM_AFTER_EXIT);
            summary::Cmd::SteamSoon(summary::STEAM_AFTER_EXIT)
        } else if ra::applies(game) {
            summary::Cmd::RaRefresh
        } else {
            return;
        };
        if let Some((tx, _)) = &self.lib.worker {
            let _ = tx.send(cmd);
        }
    }

    /// Once a frame: keep the library's counts in step with the library and the settings.
    /// Cheap unless something changed (a library change hashes the fields a pass depends on).
    pub fn sync(&mut self, games: &[Game], generation: u64, config: &AchievementsConfig) {
        let lib = &mut self.lib;
        if !lib.fake && lib.worker.is_none() && !lib.loaded {
            lib.loaded = true;
            lib.worker = summary::spawn(&self.ctx, self.cache_dir.clone(), self.net.clone());
        }
        // Answers of the pass.
        let outs: Vec<summary::Out> = lib.worker.as_ref().map(|(_, rx)| rx.try_iter().collect()).unwrap_or_default();
        for o in outs {
            self.take_library(o, config);
        }
        let lib = &mut self.lib;
        // Another RetroAchievements account: its counts aren't this one's.
        let user = summary::ra_user_key(config);
        if user != lib.ra_user {
            let before = lib.all.len();
            lib.all.retain(|_, s| s.service != Service::RetroAchievements);
            lib.dirty |= lib.all.len() != before;
            lib.ra_user = user;
            lib.save_at.get_or_insert_with(|| Instant::now() + SAVE_AFTER);
        }
        if let Some((tx, _)) = &lib.worker {
            if lib.sent_config.as_ref() != Some(config) {
                lib.sent_config = Some(config.clone());
                let _ = tx.send(summary::Cmd::Config(config.clone()));
            }
            if lib.sent_generation != Some(generation) {
                lib.sent_generation = Some(generation);
                let fp = summary::Snapshot::fingerprint(games);
                if lib.sent_fingerprint != Some(fp) {
                    lib.sent_fingerprint = Some(fp);
                    let _ = tx.send(summary::Cmd::Library(Arc::new(summary::Snapshot::of(games))));
                }
            }
            if lib.save_at.is_some_and(|t| t <= Instant::now()) {
                lib.save_at = None;
                let saved = summary::Saved { ra_user: lib.ra_user.clone(), games: lib.all.clone() };
                let _ = tx.send(summary::Cmd::Save(saved));
            } else if let Some(t) = lib.save_at {
                self.ctx.request_repaint_after(t.saturating_duration_since(Instant::now()));
            }
        }
        // Pages to ask again.
        let now = Instant::now();
        let due: Vec<GameId> = lib.later.iter().filter(|(t, _)| *t <= now).map(|(_, id)| *id).collect();
        lib.later.retain(|(t, _)| *t > now);
        for id in due {
            if let Some(g) = games.iter().find(|g| g.id == id) {
                let g = g.clone();
                self.refresh(&g, config);
            }
        }
        let lib = &mut self.lib;
        let switches = (config.show, config.library_counts, config.steam, config.retroachievements);
        if lib.dirty || lib.shown_for != Some(switches) {
            lib.dirty = false;
            lib.shown_for = Some(switches);
            let shown = summary::visible(&lib.all, config);
            if *lib.shown != shown {
                lib.shown = Arc::new(shown);
                lib.generation += 1;
            }
        }
    }

    fn take_library(&mut self, o: summary::Out, config: &AchievementsConfig) {
        match o {
            summary::Out::Loaded(saved) => {
                let same_user = saved.ra_user == summary::ra_user_key(config);
                for (id, s) in saved.games {
                    if s.service == Service::RetroAchievements && !same_user {
                        continue;
                    }
                    // A page answered already this run: that one is newer.
                    self.lib.all.entry(id).or_insert(s);
                }
                self.lib.dirty = true;
            }
            summary::Out::Steam(changed) => {
                for (id, s) in changed {
                    self.set_summary(id, s);
                    self.page_changed(id, s);
                }
            }
            summary::Out::Ra { user, games } => {
                if user != summary::ra_user_key(config) {
                    return;
                }
                for (id, s) in games {
                    self.set_summary(id, Some(s));
                    self.page_changed(id, Some(s));
                }
            }
        }
    }

    /// The library pass found other numbers than the game's page shows: the page asks again
    /// the next time it is shown (Steam rewrote its files after the page was read).
    fn page_changed(&mut self, id: GameId, s: Option<summary::Summary>) {
        if let Some((State::Ready(p), at)) = self.states.get_mut(&id)
            && summary::Summary::of(p) != s
            && let Some(old) = Instant::now().checked_sub(STALE_AFTER + Duration::from_secs(1))
        {
            *at = old;
        }
    }
}

impl Drop for Hub {
    /// Quitting with counts not saved yet: save them now (a small file), so they show at once
    /// next time.
    fn drop(&mut self) {
        if self.lib.save_at.is_some() && !self.lib.fake && self.lib.worker.is_some() {
            let saved = summary::Saved { ra_user: self.lib.ra_user.clone(), games: self.lib.all.clone() };
            summary::save(&self.cache_dir, &saved);
        }
    }
}

/// A RetroAchievements account check (Settings → Achievements → Check).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RaAccount {
    /// The name RetroAchievements shows (may differ in case from what was typed).
    pub display_name: String,
    /// Permanent account id.
    pub ulid: String,
}

#[cfg(test)]
mod tests;
