//! Achievement counts for the whole library ("31 / 54" on covers, the List column, controller
//! themes' selected-game line), kept apart from the game pages' full answers:
//!
//! * a background pass over the library (`worker`) — Steam from the client's own files (one
//!   folder listing, parsing only what changed; repeated every half minute and half a minute
//!   after a Steam game exits, when Steam has rewritten its files), RetroAchievements from the
//!   player's completion list (one request, cached an hour) and the cheap matches;
//! * every game page answer (`Hub::poll`), except title-only matches;
//! * `<cache>/achievements/summary.json`, so counts show at once on the next start.
//!
//! The UI reads [`Hub::counts`]: a map of only what may be shown under the current settings,
//! rebuilt when something changes, so a frame does one hash lookup per visible game.

use super::{Progress, Service};
use crate::config::AchievementsConfig;
use crate::model::{Game, GameId};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A game's count in the library.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub unlocked: u32,
    pub total: u32,
    pub service: Service,
}

impl Summary {
    /// The library count of a game page's answer: only when the player's own progress is known
    /// and the match is certain (a title-only match may be another version of the game).
    pub fn of(p: &Progress) -> Option<Summary> {
        (p.synced && p.total > 0 && !p.probable).then_some(Summary {
            unlocked: p.unlocked.min(p.total),
            total: p.total,
            service: p.service,
        })
    }
}

/// What `summary.json` holds.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    /// The RetroAchievements account the RetroAchievements counts belong to (`ra_user_key`).
    pub ra_user: String,
    pub games: HashMap<GameId, Summary>,
}

pub fn file(cache_dir: &Path) -> PathBuf {
    cache_dir.join("summary.json")
}

pub fn load(cache_dir: &Path) -> Saved {
    std::fs::read(file(cache_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save(cache_dir: &Path, s: &Saved) {
    let _ = std::fs::create_dir_all(cache_dir);
    match serde_json::to_vec(s) {
        Ok(b) => {
            if let Err(e) = crate::fsutil::write_atomic(&file(cache_dir), &b) {
                log::warn!("achievements: couldn't save the library counts: {e}");
            }
        }
        Err(e) => log::warn!("achievements: couldn't save the library counts: {e}"),
    }
}

/// The RetroAchievements account counts belong to (lower case; "" = none set up).
pub fn ra_user_key(cfg: &AchievementsConfig) -> String {
    if cfg.ra_username.trim().is_empty() { String::new() } else { cfg.ra_user().to_lowercase() }
}

/// The counts the library may show under `cfg`: none with achievements or library counts
/// switched off, and none of a service that is switched off.
pub fn visible(all: &HashMap<GameId, Summary>, cfg: &AchievementsConfig) -> HashMap<GameId, (u32, u32)> {
    if !cfg.show || !cfg.library_counts {
        return HashMap::new();
    }
    all.iter()
        .filter(|(_, s)| match s.service {
            Service::Steam => cfg.steam,
            Service::RetroAchievements => cfg.retroachievements,
        })
        .filter(|(_, s)| s.total > 0)
        .map(|(id, s)| (*id, (s.unlocked.min(s.total), s.total)))
        .collect()
}

/// Share unlocked, 0..=1.
pub fn fraction((unlocked, total): (u32, u32)) -> f32 {
    if total == 0 { 0.0 } else { (unlocked.min(total) as f32 / total as f32).clamp(0.0, 1.0) }
}

/// The games a library pass looks at.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Steam games: (id, appid, install folder).
    pub steam: Vec<(GameId, u32, Option<PathBuf>)>,
    /// Emulated games (what matching needs).
    pub emu: Vec<Game>,
}

impl Snapshot {
    pub fn of(games: &[Game]) -> Snapshot {
        let mut s = Snapshot::default();
        for g in games {
            if super::steam::applies(g) {
                if let Ok(appid) = g.source_id.parse() {
                    s.steam.push((g.id, appid, g.install_dir.clone()));
                }
            } else if super::ra::applies(g) {
                s.emu.push(g.clone());
            }
        }
        s
    }

    /// What a pass depends on, hashed (a library change that doesn't touch it starts nothing).
    pub fn fingerprint(games: &[Game]) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for g in games {
            if super::steam::applies(g) {
                (g.id, &g.source_id, &g.install_dir).hash(&mut h);
            } else if let Some(e) = &g.emu {
                (g.id, &e.platform, &e.rom, &e.dat_name, g.meta.ra_game_id, &g.name, &g.user_name).hash(&mut h);
            }
        }
        h.finish()
    }
}

/// Work for the library pass's thread.
pub enum Cmd {
    Library(Arc<Snapshot>),
    Config(AchievementsConfig),
    /// A Steam game exited: look at Steam's files again after this long.
    SteamSoon(Duration),
    /// An emulated game exited: ask RetroAchievements again (not the hour-old list).
    RaRefresh,
    Save(Saved),
}

/// Answers from the library pass's thread.
#[derive(Debug, PartialEq)]
pub enum Out {
    /// What `summary.json` held at start.
    Loaded(Saved),
    /// Steam games whose count changed (None: nothing to show any more).
    Steam(Vec<(GameId, Option<Summary>)>),
    /// RetroAchievements counts of the games that could be matched, for account `user`.
    Ra { user: String, games: Vec<(GameId, Summary)> },
}

/// Steam's files are looked at again this often (only changed games are parsed again).
pub const STEAM_EVERY: Duration = Duration::from_secs(30);
/// After a Steam game exits, its files are read again after this long (Steam writes them a
/// little after the game has closed).
pub const STEAM_AFTER_EXIT: Duration = Duration::from_secs(30);

/// RetroAchievements settings must rest this long before a pass uses them.
const RA_SETTLE: Duration = Duration::from_secs(3);

/// Start the library pass's thread (None: no thread could be started).
pub fn spawn(
    ctx: &egui::Context,
    cache_dir: PathBuf,
    net: Arc<dyn crate::art::net::Net>,
) -> Option<(Sender<Cmd>, Receiver<Out>)> {
    let (tx, cmd_rx) = unbounded::<Cmd>();
    let (out_tx, rx) = unbounded::<Out>();
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new().name("achievements-library".into()).spawn(move || {
        // SAFETY: gettid / setpriority take no pointers; this lowers only this thread (it reads
        // and hashes files in the background and must never compete with the UI or a game).
        unsafe {
            libc::setpriority(libc::PRIO_PROCESS, libc::gettid() as libc::id_t, 15);
        }
        let send = |o: Out| {
            let ok = out_tx.send(o).is_ok();
            ctx.request_repaint();
            ok
        };
        Worker::new(cache_dir, net).run(&cmd_rx, &send);
    });
    match spawned {
        Ok(_) => Some((tx, rx)),
        Err(e) => {
            log::warn!("achievements: no thread for the library counts: {e}");
            None
        }
    }
}

struct Worker {
    cache_dir: PathBuf,
    net: Arc<dyn crate::art::net::Net>,
    snapshot: Arc<Snapshot>,
    config: Option<AchievementsConfig>,
    steam_memo: super::steam::bulk::Memo,
    /// What was last sent per Steam game.
    steam_sent: HashMap<GameId, Option<Summary>>,
    /// Steam's files are read again then.
    steam_at: Option<Instant>,
    ra_due: bool,
    ra_force: bool,
    /// RetroAchievements settings changed (being typed?): the pass waits until then.
    ra_after: Option<Instant>,
}

impl Worker {
    fn new(cache_dir: PathBuf, net: Arc<dyn crate::art::net::Net>) -> Worker {
        Worker {
            cache_dir,
            net,
            snapshot: Arc::new(Snapshot::default()),
            config: None,
            steam_memo: Default::default(),
            steam_sent: HashMap::new(),
            steam_at: None,
            ra_due: false,
            ra_force: false,
            ra_after: None,
        }
    }

    fn steam_on(&self) -> bool {
        self.config.as_ref().is_some_and(|c| c.show && c.steam) && !self.snapshot.steam.is_empty()
    }

    fn ra_on(&self) -> bool {
        self.config.as_ref().is_some_and(AchievementsConfig::ra_ready) && !self.snapshot.emu.is_empty()
    }

    fn run(mut self, rx: &Receiver<Cmd>, send: &dyn Fn(Out) -> bool) {
        if !send(Out::Loaded(load(&self.cache_dir))) {
            return;
        }
        loop {
            let next = match (self.steam_at, self.ra_after.filter(|_| self.ra_due)) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            let wait = next.map(|t| t.saturating_duration_since(Instant::now()));
            let first = match wait {
                Some(w) => match rx.recv_timeout(w) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                },
                None => match rx.recv() {
                    Ok(c) => Some(c),
                    Err(_) => return,
                },
            };
            for cmd in first.into_iter().chain(rx.try_iter().collect::<Vec<_>>()) {
                self.take(cmd);
            }
            if self.steam_at.is_some_and(|t| t <= Instant::now()) {
                self.steam_at = None;
                if self.steam_on() {
                    let changed = self.steam_pass();
                    if !changed.is_empty() && !send(Out::Steam(changed)) {
                        return;
                    }
                    self.steam_at = Some(Instant::now() + STEAM_EVERY);
                }
            }
            if self.ra_after.is_some_and(|t| t > Instant::now()) {
                continue;
            }
            self.ra_after = None;
            if std::mem::take(&mut self.ra_due) && self.ra_on() {
                let force = std::mem::take(&mut self.ra_force);
                if !self.ra_pass(rx, send, force) {
                    return;
                }
            }
        }
    }

    fn take(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Library(s) => {
                self.snapshot = s;
                self.steam_at = Some(Instant::now());
                self.ra_due = true;
            }
            Cmd::Config(c) => {
                let ra_changed = self.config.as_ref().is_none_or(|o| {
                    (o.ra_ready(), ra_user_key(o), &o.ra_api_key) != (c.ra_ready(), ra_user_key(&c), &c.ra_api_key)
                });
                let steam_changed = self.config.as_ref().is_none_or(|o| (o.show, o.steam) != (c.show, c.steam));
                self.config = Some(c);
                if steam_changed {
                    self.steam_at = Some(Instant::now());
                }
                if ra_changed {
                    self.ra_due = true;
                    // Settings being typed: one pass once they rest, not one per key press.
                    self.ra_after = Some(Instant::now() + RA_SETTLE);
                }
            }
            Cmd::SteamSoon(d) => {
                let at = Instant::now() + d;
                self.steam_at = Some(self.steam_at.map_or(at, |t| t.min(at)));
            }
            Cmd::RaRefresh => {
                self.ra_due = true;
                self.ra_force = true;
            }
            Cmd::Save(s) => save(&self.cache_dir, &s),
        }
    }

    /// Read Steam's files; the games whose count changed since the last pass.
    fn steam_pass(&mut self) -> Vec<(GameId, Option<Summary>)> {
        let dirs: Vec<&Path> = self.snapshot.steam.iter().filter_map(|(_, _, d)| d.as_deref()).collect();
        let roots = super::steam::bulk::roots(&dirs);
        let appids: Vec<u32> = self.snapshot.steam.iter().map(|(_, a, _)| *a).collect();
        let counts = super::steam::bulk::counts(&roots, &appids, &mut self.steam_memo);
        let mut changed = vec![];
        for (id, appid, _) in &self.snapshot.steam {
            let Some(c) = counts.get(appid) else { continue };
            let s = c.map(|(unlocked, total)| Summary { unlocked, total, service: Service::Steam });
            if self.steam_sent.get(id) != Some(&s) {
                self.steam_sent.insert(*id, s);
                changed.push((*id, s));
            }
        }
        changed
    }

    /// Match the emulated games and count them: first with the ROM hashes already known, then
    /// (when the library has ROMs not hashed yet) again after hashing them. False: the UI is gone.
    fn ra_pass(&mut self, rx: &Receiver<Cmd>, send: &dyn Fn(Out) -> bool, force: bool) -> bool {
        let Some(cfg) = self.config.clone() else { return true };
        let user = cfg.ra_user().to_string();
        let key = cfg.ra_api_key.as_deref().unwrap_or("").trim().to_string();
        let dir = self.cache_dir.join("ra");
        let client = super::ra::api::Client::new(self.net.as_ref(), &key, cfg.secrets());
        let done = match super::ra::bulk::completion(&client, &dir, &user, force) {
            Ok(d) => d,
            Err(e) => {
                log::info!("achievements: no RetroAchievements counts for the library ({e:?})");
                return true;
            }
        };
        let mut lists = super::ra::bulk::Lists::new(&client, &dir);
        let mut hashes = super::ra::hash::HashCache::load(&dir);
        let games = self.snapshot.clone();
        let user_key = ra_user_key(&cfg);
        let wrap = |m: HashMap<GameId, (u32, u32)>| Out::Ra {
            user: user_key.clone(),
            games: m
                .into_iter()
                .map(|(id, (unlocked, total))| (id, Summary { unlocked, total, service: Service::RetroAchievements }))
                .collect(),
        };
        let first = super::ra::bulk::library_counts(&games.emu, &dir, &mut lists, &mut hashes, &done, false, &|| true);
        if !send(wrap(first)) {
            return false;
        }
        // Hash what isn't yet, stopping when new work arrives (the pass starts again with it).
        let unhashed = games.emu.iter().any(|g| {
            g.emu.as_ref().is_some_and(|e| {
                super::ra::hash::console(&e.platform).is_some() && hashes.cached(&e.platform, &e.rom).is_none()
            })
        });
        if unhashed {
            let quiet = || rx.is_empty();
            let second =
                super::ra::bulk::library_counts(&games.emu, &dir, &mut lists, &mut hashes, &done, true, &quiet);
            if !rx.is_empty() {
                self.ra_due = true;
            }
            return send(wrap(second));
        }
        true
    }
}
