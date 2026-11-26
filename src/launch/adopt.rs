//! Adopting games that already run (Lodestar was closed and opened again while they played):
//! scanning /proc for them, deciding which library games they are, and following them.

use super::*;

/// What a scan for running games found (`Launcher::start_running_scan`).
pub(crate) struct RunningScan {
    pub procs: Vec<track::ProcInfo>,
    pub found: Vec<track::Found>,
    /// Boot time (unix seconds) to date the processes found.
    pub btime: Option<u64>,
    /// The /proc it looked at (a fake one in tests).
    pub root: PathBuf,
    /// Something that looks like a game runs (`track::other_game`), and which.
    pub others: Option<track::OtherGame>,
    /// When the scan started reading /proc: a slow scan's "nothing runs" is only as fresh as
    /// that (a game started while it read is not in it).
    pub started: std::time::Instant,
}

/// A running game to follow again (`Launcher::start_running_scan`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adoption {
    pub id: GameId,
    /// How it runs (decides who counts its playtime, like a launch's kind).
    pub kind: ResolvedKind,
    /// Another Lodestar that still runs (a `--play` shortcut) follows it and records its
    /// session; this one only shows it.
    pub foreign: bool,
    pub msg: track::Adopt,
}

/// The game Steam's reaper for `appid` belongs to: the Steam game with that AppId first, else a
/// game launched through it (a non-Steam shortcut, a `steam://rungameid/…` entry).
pub(super) fn steam_game(games: &[Game], appid: u32) -> Option<(&Game, ResolvedKind)> {
    let kind_of = |g: &Game| -> Option<ResolvedKind> {
        if let Some(a) = g.steam_appid() {
            return Some(ResolvedKind::Steam { appid: a });
        }
        match base_of(g.launch.as_ref()?) {
            Base::Steam { appid } => Some(ResolvedKind::Steam { appid }),
            Base::Shortcut { appid, .. } => Some(ResolvedKind::SteamShortcut { appid }),
            _ => None,
        }
    };
    let mut hits = games.iter().filter_map(|g| {
        let k = kind_of(g)?;
        matches!(k, ResolvedKind::Steam { appid: a } | ResolvedKind::SteamShortcut { appid: a } if a == appid)
            .then_some((g, k))
    });
    let first = hits.next()?;
    if first.0.source == Source::Steam {
        return Some(first);
    }
    Some(hits.find(|(g, _)| g.source == Source::Steam).unwrap_or(first))
}

/// How a game Lodestar launched (its Play launch) is followed when it is adopted from its tag:
/// None for launches the tag says nothing about (Steam's own launches are found by their reaper:
/// the Steam client a launch started carries that launch's tag for as long as it runs) and for
/// untracked ones.
pub(super) fn adopt_mode(g: &Game) -> Option<(ResolvedKind, track::Mode, Option<PathBuf>)> {
    let r = match resolve(g) {
        Ok(r) => r,
        // It did run: follow its processes.
        Err(_) => return Some((ResolvedKind::Process, track::Mode::Session, None)),
    };
    if matches!(r.kind, ResolvedKind::Steam { .. } | ResolvedKind::SteamShortcut { .. }) {
        return None;
    }
    let plan = LaunchPlan::for_game(g, g.play_action(), &GameScripts::default(), false);
    match launch_mode(&r, &plan) {
        track::Mode::Untracked | track::Mode::Steam { .. } => None,
        m @ (track::Mode::Process | track::Mode::Session) => Some((r.kind, m, None)),
        m @ (track::Mode::Folder | track::Mode::GameFolder) => Some((r.kind, m, plan.install_dir)),
    }
}

/// Which of `found` (track::find_running) to follow, as what: library games only, not the ones
/// `skip(id, tag)` rules out (already followed, a session that ended). A store-client launch
/// (`GameFolder`) counts only while something runs from the game's folder (the client it opened
/// stays open after the game). One adoption per game; one this Lodestar can own wins over a
/// foreign one. `age(since)` is how long ago a process that started at `since` did.
pub(crate) fn plan_adoptions(
    found: &[track::Found],
    games: &[Game],
    procs: &[track::ProcInfo],
    proc_root: &Path,
    skip: impl Fn(GameId, &str) -> bool,
    age: impl Fn(u64) -> Duration,
) -> Vec<Adoption> {
    let mut out: Vec<Adoption> = vec![];
    // The Steam client a launch started is no game, whatever game the tag names.
    let roots = if found.is_empty() { vec![] } else { track::steam_roots() };
    for f in found.iter().filter_map(|f| track::without_steam(f, procs, proc_root, &roots)) {
        let mut own = f.procs.clone();
        let (id, kind, mode, tag, folder, foreign) = match &f.what {
            track::FoundKind::Steam { appid } => {
                let Some((g, kind)) = steam_game(games, *appid) else {
                    log::debug!("Steam AppId {appid} runs; not in the library");
                    continue;
                };
                (g.id, kind, track::Mode::Steam { appid: *appid }, String::new(), None, false)
            }
            track::FoundKind::Tagged { id, tag, foreign } => {
                let Some(g) = games.iter().find(|g| g.id == *id) else { continue };
                let Some((kind, mode, folder)) = adopt_mode(g) else { continue };
                if mode == track::Mode::GameFolder {
                    let Some(dir) = folder.as_deref() else { continue };
                    let in_folder = procs.iter().any(|p| {
                        !p.zombie
                            && p.start >= f.since
                            && track::runs_from(proc_root, p.pid, Some(p.cmdline.as_deref().unwrap_or_default()), dir)
                    });
                    if !in_folder {
                        continue;
                    }
                }
                if mode == track::Mode::Process {
                    // Only the launch's own process counts, not what it leaves behind: the
                    // launch's first process, a session leader (setsid() in SpawnJob::spawn).
                    // Gone: that session ended, whatever still carries its tag.
                    let first = f.procs.iter().min_by_key(|k| k.1).copied();
                    let leader = |&(pid, start): &(i32, u64)| {
                        procs.iter().any(|p| p.pid == pid && p.start == start && p.sid == pid && !p.zombie)
                    };
                    let Some(first) = first.filter(leader) else { continue };
                    own = vec![first];
                }
                (g.id, kind, mode, tag.clone(), folder, *foreign)
            }
        };
        if skip(id, &tag) {
            continue;
        }
        let since = own.iter().map(|k| k.1).min().unwrap_or(f.since);
        let a = Adoption {
            id,
            kind,
            foreign,
            msg: track::Adopt { id, mode, tag, folder, known: own.iter().copied().collect(), since, age: age(since) },
        };
        match out.iter_mut().find(|o| o.id == id) {
            Some(prev) if prev.foreign && !a.foreign => *prev = a,
            Some(_) => {}
            None => out.push(a),
        }
    }
    out
}

/// Sources whose games start through a store client that stays open after the game exits.
pub(crate) fn launched_via_client(src: Source) -> bool {
    matches!(src, Source::BattleNet | Source::Ea | Source::Ubisoft | Source::Rockstar | Source::Riot | Source::HoYoPlay)
}

/// A folder specific enough to stand for one game: not empty, not `/`, not the home folder or a
/// shallow folder above games (`/home/x/Games`, `/mnt/drive`) — anything running from those would
/// keep a "game" running forever.
pub(crate) fn is_game_folder(d: &Path) -> bool {
    if d.as_os_str().is_empty() || !d.is_absolute() {
        return false;
    }
    if dirs::home_dir().is_some_and(|h| d == h || h.starts_with(d)) {
        return false;
    }
    let depth = d.components().filter(|c| matches!(c, std::path::Component::Normal(_))).count();
    let generic = d.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
        matches!(n.to_ascii_lowercase().as_str(), "games" | "heroic" | "steamlibrary" | "common" | "steamapps")
    });
    depth >= 3 && !generic
}

impl Launcher {
    /// Look for games that already run but that this Lodestar didn't start: ones an earlier
    /// Lodestar launched (closed and opened again while they played, found by their launch tag)
    /// and Steam games (Steam's reaper). The /proc scan (every process' command line and
    /// environment: a few ms, but a process stuck in the kernel can make such a read hang) runs
    /// on a thread of its own; `finish_running_scan` follows what it found. One scan at a time.
    /// `game_dirs`: installed games' folders (a program running from one is a game this run may
    /// not follow, see `track::other_game`).
    pub fn start_running_scan(&mut self, game_dirs: Vec<PathBuf>) {
        if self.running_scan.is_some() {
            return;
        }
        let (tx, rx) = unbounded();
        let ctx = self.ctx.clone();
        let me = std::process::id() as i32;
        let r = std::thread::Builder::new().name("running-scan".into()).spawn(move || {
            let t = std::time::Instant::now();
            let root = Path::new("/proc");
            let procs = track::read_procs(true);
            let found = track::find_running(&procs, root, me);
            let btime = track::boot_time(root);
            let others = track::other_game(&found, &procs, root, me, &game_dirs);
            let what = others.as_ref().map_or("nothing".into(), |o| o.describe(&[]));
            log::debug!("looked for running games in {:?}; looks like a game: {what}", t.elapsed());
            let _ = tx.send(RunningScan { procs, found, btime, root: root.to_path_buf(), others, started: t });
            ctx.request_repaint();
        });
        match r {
            Ok(_) => self.running_scan = Some(rx),
            Err(e) => log::warn!("couldn't look for running games: {e}"),
        }
    }

    /// Follow what the scan `start_running_scan` started found, once it is done (None while it
    /// runs or when none was started). They show as running with a working Stop, and their
    /// session ends (and is counted) like a launched one's, timed from the game's real start.
    /// Games already followed aren't adopted twice. Returns what was adopted.
    pub fn finish_running_scan(&mut self, games: &[Game]) -> Option<Vec<Adoption>> {
        let scan = match self.running_scan.as_ref()?.try_recv() {
            Ok(s) => s,
            Err(crossbeam_channel::TryRecvError::Empty) => return None,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                self.running_scan = None;
                return None;
            }
        };
        self.running_scan = None;
        self.others_look = Some((scan.started, scan.others.is_some()));
        Some(self.adopt_found(&scan, games))
    }

    /// The last scan for running games finished less than `fresh` ago and saw nothing that
    /// looks like a game this run may not follow (the background controller may act then).
    pub fn others_clear(&self, fresh: Duration) -> bool {
        self.others_look.is_some_and(|(at, others)| !others && at.elapsed() < fresh)
    }

    /// When the last finished scan for running games started (None before the first one).
    pub fn others_looked_at(&self) -> Option<std::time::Instant> {
        self.others_look.map(|(at, _)| at)
    }

    /// A scan for running games is under way.
    pub fn looking(&self) -> bool {
        self.running_scan.is_some()
    }

    /// The last scan for running games saw something that looks like a game (see
    /// `others_clear`).
    pub fn others_seen(&self) -> bool {
        self.others_look.is_some_and(|(_, others)| others)
    }

    /// `finish_running_scan` for one scan's results.
    pub(crate) fn adopt_found(&mut self, scan: &RunningScan, games: &[Game]) -> Vec<Adoption> {
        // A Steam AppId one of this run's launches waits for or follows (also one started from
        // another entry than the Steam game's own, e.g. a `steam://rungameid` one) is that
        // launch's.
        let followed: std::collections::HashSet<u32> = self
            .active
            .iter()
            .filter_map(|id| match self.kinds.get(id)? {
                ResolvedKind::Steam { appid } | ResolvedKind::SteamShortcut { appid } => Some(*appid),
                _ => None,
            })
            .collect();
        let found: Vec<track::Found> = scan
            .found
            .iter()
            .filter(|f| !matches!(f.what, track::FoundKind::Steam { appid } if followed.contains(&appid)))
            .cloned()
            .collect();
        if found.is_empty() {
            return vec![];
        }
        let hz = track::clock_ticks();
        let now =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
        let btime = scan.btime;
        let plans = plan_adoptions(
            &found,
            games,
            &scan.procs,
            &scan.root,
            |id, tag| self.active.contains(&id) || (!tag.is_empty() && self.ended_tags.contains(tag)),
            |since| btime.map_or(Duration::ZERO, |b| track::started_ago(since, b, hz, now)),
        );
        for a in &plans {
            self.adopt(a);
        }
        plans
    }

    /// Follow one adopted game (see `start_running_scan`).
    pub(crate) fn adopt(&mut self, a: &Adoption) {
        let id = a.id;
        if self.active.contains(&id) {
            return;
        }
        log::info!(
            "adopted game {id}: already running ({:?}, {} process(es), started {}s ago{}{})",
            a.msg.mode,
            a.msg.known.len(),
            a.msg.age.as_secs(),
            if a.msg.tag.is_empty() { String::new() } else { format!(", tag {}", a.msg.tag) },
            if a.foreign { ", followed by another Lodestar too" } else { "" }
        );
        let _ = self.monitor_tx().send(track::Msg::Adopt(a.msg.clone()));
        self.mark_active(id);
        self.started.push(id);
        self.kinds.insert(id, a.kind);
        self.via.insert(
            id,
            if matches!(a.kind, ResolvedKind::Steam { .. } | ResolvedKind::SteamShortcut { .. }) {
                Via::Steam
            } else {
                Via::Direct
            },
        );
        self.session_scripts.remove(&id);
        self.script_started.remove(&id);
        self.prelaunch.remove(&id);
        self.hygiene_ids.remove(&id);
        if a.foreign {
            self.foreign.insert(id);
        } else {
            self.foreign.remove(&id);
        }
        self.adopted_at.insert(id, now_unix() - a.msg.age.as_secs() as i64);
        if a.msg.tag.is_empty() {
            self.adopted_tags.remove(&id);
        } else {
            self.adopted_tags.insert(id, a.msg.tag.clone());
        }
    }

    /// When an adopted game really started (unix seconds; None: launched by this Lodestar).
    pub fn adopted_start(&self, id: GameId) -> Option<i64> {
        self.adopted_at.get(&id).copied()
    }

    /// `id` was adopted from another Lodestar that still runs and records its session.
    pub fn is_foreign(&self, id: GameId) -> bool {
        self.foreign.contains(&id)
    }
}
