//! The `Launcher`: starting resolved commands with their scripts and tracking mode, following
//! sessions through `track`, launcher hygiene, and the events the UI polls.

use super::*;

/// What else a launch needs besides the resolved command: how to track it and the scripts that
/// run around the session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchPlan {
    pub tracking: TrackingMode,
    /// For `TrackingMode::Folder`.
    pub install_dir: Option<PathBuf>,
    /// Started through a store client that stays open (Battle.net, EA, Ubisoft …): follow only the
    /// game's own folder, not the client.
    pub via_client: bool,
    pub scripts: SessionScripts,
}

impl LaunchPlan {
    /// The plan for starting `game`: with `action` (the Play action or another one) its tracking
    /// mode, scripts (global from Settings + the game's) for a play session only.
    pub fn for_game(game: &Game, action: Option<&GameAction>, global_scripts: &GameScripts, play: bool) -> LaunchPlan {
        let mut tracking = action.map(|a| a.tracking).unwrap_or_default();
        let mut install_dir = game.install_dir.clone();
        // An emulated game whose emulator profile says how to follow it (its own program, a
        // fork that hands off …): the profile's tracking, unless the action sets its own.
        if tracking == TrackingMode::Default
            && game.emu.is_some()
            && let Some(reg) = crate::emulation::registry::current()
            && let Some((t, folder)) = crate::emulation::registry::profile_tracking(&reg, game, action)
        {
            tracking = t;
            if t == TrackingMode::Folder && folder.is_some() {
                install_dir = folder;
            }
        }
        LaunchPlan {
            tracking,
            install_dir,
            // Only for the source's own launch: a user action may run something else entirely.
            via_client: action.is_none_or(|a| a.play && a.path.trim().is_empty()) && launched_via_client(game.source),
            scripts: if play { SessionScripts::for_game(game, global_scripts) } else { SessionScripts::default() },
        }
    }
}

/// How the monitor follows a launch, from how it was resolved and the action's tracking mode.
pub(super) fn track_mode(kind: ResolvedKind, tracking: TrackingMode, folder: Option<&Path>) -> track::Mode {
    let default = match kind {
        ResolvedKind::Process => track::Mode::Session,
        ResolvedKind::Steam { appid } | ResolvedKind::SteamShortcut { appid } => track::Mode::Steam { appid },
        // A link handed to a store client (Heroic's heroic://launch, itch …): the client starts
        // the game later from its install folder, so follow that folder when it is known
        // (found testing a real GOG game through Heroic: it appeared ~40 s after the link).
        ResolvedKind::OpenUrl if folder.is_some_and(is_game_folder) => track::Mode::Folder,
        ResolvedKind::OpenUrl => track::Mode::Untracked,
    };
    match tracking {
        TrackingMode::Default => default,
        TrackingMode::None => track::Mode::Untracked,
        TrackingMode::Process if kind == ResolvedKind::Process => track::Mode::Process,
        // `steam -applaunch` / xdg-open exit at once: the started process says nothing there.
        TrackingMode::Process => default,
        TrackingMode::Folder => match folder {
            Some(d) if is_game_folder(d) => track::Mode::Folder,
            _ => {
                log::warn!("folder tracking without an install folder; tracking as usual");
                default
            }
        },
    }
}

/// How the monitor follows the launch of `r` under `plan` (`track_mode` plus the store-client
/// and Heroic cases).
pub(super) fn launch_mode(r: &Resolved, plan: &LaunchPlan) -> track::Mode {
    let mut mode = track_mode(r.kind, plan.tracking, plan.install_dir.as_deref());
    if plan.via_client
        && plan.tracking == TrackingMode::Default
        && plan.install_dir.as_deref().is_some_and(is_game_folder)
        && matches!(mode, track::Mode::Session | track::Mode::Folder)
    {
        mode = track::Mode::GameFolder;
    }
    // Heroic stays open after the game too (and, started by the link, carries the launch
    // tag): follow only the game's folder.
    if is_heroic_link(r) && mode == track::Mode::Folder {
        mode = track::Mode::GameFolder;
    }
    mode
}

impl Launcher {
    pub fn new(ctx: &egui::Context) -> Launcher {
        let log_dir = crate::config::Paths::detect().cache_dir.join("logs");
        Launcher::with_log_dir(ctx, log_dir)
    }

    pub fn with_log_dir(ctx: &egui::Context, log_dir: PathBuf) -> Launcher {
        let (ev_tx, ev_rx) = unbounded();
        let (notice_tx, notices) = unbounded();
        Launcher {
            ctx: ctx.clone(),
            log_dir,
            monitor: None,
            ev_tx,
            ev_rx,
            pending: vec![],
            active: vec![],
            started: vec![],
            logs: HashMap::new(),
            kinds: HashMap::new(),
            poll_every: Duration::from_millis(1500),
            steam_timeout: Duration::from_secs(180),
            scripts: ScriptRunner::new(ctx, notice_tx),
            notices,
            session_scripts: HashMap::new(),
            script_started: Default::default(),
            prelaunch: HashMap::new(),
            hygiene_cfg: Default::default(),
            hygiene: None,
            hygiene_ids: Default::default(),
            via: HashMap::new(),
            foreign: Default::default(),
            adopted_tags: HashMap::new(),
            ended_tags: Default::default(),
            adopted_at: HashMap::new(),
            running_scan: None,
            others_look: None,
        }
    }

    /// Turn launcher hygiene on / off (Settings → General → Launchers). Off by default, so tests, tools and
    /// screenshot runs never minimise or close anything.
    pub fn set_hygiene(&mut self, cfg: hygiene::HygieneConfig) {
        if cfg == self.hygiene_cfg {
            return;
        }
        if let Some(tx) = &self.hygiene {
            let _ = tx.send(hygiene::Msg::Config(cfg.clone()));
        }
        self.hygiene_cfg = cfg;
    }

    /// Before quitting (`--play`): let the hygiene thread close what it still has to close (the
    /// clients of a game that just exited), waiting at most `timeout`.
    pub fn finish_hygiene(&mut self, timeout: Duration) {
        let Some(tx) = self.hygiene.take() else { return };
        let (done_tx, done_rx) = unbounded();
        if tx.send(hygiene::Msg::Finish(done_tx)).is_ok() {
            let _ = done_rx.recv_timeout(timeout);
        }
    }

    pub(super) fn hygiene_tx(&mut self) -> Sender<hygiene::Msg> {
        if let Some(tx) = &self.hygiene {
            return tx.clone();
        }
        let tx = hygiene::spawn(self.hygiene_cfg.clone());
        self.hygiene = Some(tx.clone());
        tx
    }

    /// Hand a launch to the hygiene thread when it goes through a store client and is followed
    /// by the game's own folder (so `Started` means the game itself runs).
    pub(super) fn register_hygiene(
        &mut self,
        id: GameId,
        r: &Resolved,
        plan: &LaunchPlan,
        mode: track::Mode,
        tag: &str,
    ) {
        self.hygiene_ids.remove(&id);
        if !self.hygiene_cfg.active() {
            return;
        }
        // Steam games: Steam's own windows are minimised once the game runs; a Steam Lodestar
        // started may be closed after the game (launch/steam.rs).
        if matches!(mode, track::Mode::Steam { .. }) {
            if !self.hygiene_cfg.hide_steam && !self.hygiene_cfg.closes_steam() {
                return;
            }
            let info = hygiene::LaunchInfo {
                id,
                tag: tag.to_string(),
                folder: plan.install_dir.clone(),
                prefix: None,
                client: hygiene::Client::Steam,
                heroic_was_running: false,
            };
            let _ = self.hygiene_tx().send(hygiene::Msg::Launch(info));
            self.hygiene_ids.insert(id);
            return;
        }
        if mode != track::Mode::GameFolder {
            return;
        }
        let client = if is_heroic_link(r) {
            hygiene::Client::Heroic
        } else if plan.via_client {
            hygiene::Client::Wine
        } else {
            return;
        };
        // Whether Heroic already runs decides whether it is closed after the game: look before
        // the link is opened (one /proc scan, only for Heroic links).
        let heroic_was_running = client == hygiene::Client::Heroic && hygiene::heroic_running(&track::read_procs(true));
        let info = hygiene::LaunchInfo {
            id,
            tag: tag.to_string(),
            folder: plan.install_dir.clone(),
            prefix: launch_prefix(r),
            client,
            heroic_was_running,
        };
        let _ = self.hygiene_tx().send(hygiene::Msg::Launch(info));
        self.hygiene_ids.insert(id);
    }

    /// Resolve and start `game` (its Play action when set). Returns once the process is spawned
    /// (never waits for it). No scripts; see `launch_planned`.
    pub fn launch(&mut self, game: &Game) -> anyhow::Result<()> {
        let r = resolve(game)?;
        let plan = LaunchPlan {
            tracking: game.play_action().map(|a| a.tracking).unwrap_or_default(),
            install_dir: game.install_dir.clone(),
            via_client: game.play_action().is_none() && launched_via_client(game.source),
            scripts: Default::default(),
        };
        self.launch_planned(game.id, game.display_name(), &r, &plan)
    }

    /// Start an already resolved command (also used by tests with harmless commands).
    #[cfg(test)]
    pub fn launch_resolved(&mut self, id: GameId, name: &str, r: &Resolved) -> anyhow::Result<()> {
        self.launch_planned(id, name, r, &LaunchPlan::default())
    }

    /// Start a resolved command tracked per `plan.tracking`, with its scripts: before-start
    /// scripts run first on a background thread (each waited for up to 30 s; a failure is
    /// reported and the game still starts), after-start ones once the game runs, after-exit ones
    /// when it ended. Errors that are known at once (missing program / folder) are returned;
    /// a spawn that fails after the before-start scripts arrives as `Failed` + `Exited`.
    pub fn launch_planned(&mut self, id: GameId, name: &str, r: &Resolved, plan: &LaunchPlan) -> anyhow::Result<()> {
        let direct = direct_url(r);
        let r = direct.as_ref().unwrap_or(r);
        // Steam isn't running: start it quietly (no library window), and remember that Lodestar did.
        let quiet = steam::cold_start(r);
        let r = quiet.as_ref().unwrap_or(r);
        let program = check_runnable(r)?;
        let mode = launch_mode(r, plan);
        let folder =
            matches!(mode, track::Mode::Folder | track::Mode::GameFolder).then(|| plan.install_dir.clone()).flatten();
        let tracked = mode != track::Mode::Untracked;
        // The game runs from the moment it is spawned (the monitor announces the others once
        // it sees them).
        let announced = matches!(mode, track::Mode::Session | track::Mode::Process)
            || (mode == track::Mode::Folder && r.kind == ResolvedKind::Process);
        let log_path = self.log_dir.join(log_file_name(id, name));
        // Every process of this launch inherits the tag (see SpawnJob::spawn).
        let tag = format!("{}-{id}-{}", std::process::id(), now_unix_nanos());
        if quiet.is_some() {
            steam::note_cold_start(&r.program, &tag);
        }
        let job = SpawnJob {
            id,
            name: name.to_string(),
            r: r.clone(),
            program,
            log_dir: self.log_dir.clone(),
            log_path: log_path.clone(),
            mode,
            folder,
            announced,
            tag: tag.clone(),
        };
        let tx = self.monitor_tx();
        self.via.insert(
            id,
            if is_heroic_link(r) {
                Via::Heroic
            } else if plan.via_client {
                Via::StoreClient
            } else if matches!(r.kind, ResolvedKind::Steam { .. } | ResolvedKind::SteamShortcut { .. }) {
                Via::Steam
            } else {
                Via::Direct
            },
        );
        self.register_hygiene(id, r, plan, mode, &tag);
        self.foreign.remove(&id);
        self.adopted_tags.remove(&id);
        self.adopted_at.remove(&id);
        self.script_started.remove(&id);
        self.session_scripts.remove(&id);
        self.prelaunch.remove(&id);
        let scripts = &plan.scripts;
        if scripts.before_start.is_empty() {
            job.spawn(&tx)?;
            self.logs.insert(id, log_path);
            self.kinds.insert(id, r.kind);
            if tracked {
                self.mark_active(id);
                if !scripts.after_start.is_empty() || !scripts.after_exit.is_empty() {
                    self.session_scripts.insert(id, scripts.clone());
                }
            } else {
                // Not followed: after-start scripts run right away, after-exit ones never.
                self.scripts.spawn(scripts.after_start.clone(), scripts.env.clone(), scripts.cwd.clone(), false);
            }
            if announced {
                self.pending.push(LaunchEvent::Started { id });
                // Running from now on: a second Play before the next poll() must not start a
                // second instance.
                if tracked && !self.started.contains(&id) {
                    self.started.push(id);
                }
            }
            return Ok(());
        }
        // Before-start scripts first, off the UI thread; the game counts as launching meanwhile.
        self.logs.insert(id, log_path);
        self.kinds.insert(id, r.kind);
        if tracked {
            self.mark_active(id);
            self.session_scripts.insert(id, scripts.clone());
        }
        let state = Arc::new(AtomicU8::new(PRE_PENDING));
        self.prelaunch.insert(id, state.clone());
        let runner = self.scripts.clone();
        let ev_tx = self.ev_tx.clone();
        let ctx = self.ctx.clone();
        let scripts = scripts.clone();
        let spawned = std::thread::Builder::new().name("pre-launch".into()).spawn(move || {
            let guard = runner.hold();
            for s in &scripts.before_start {
                if state.load(Ordering::SeqCst) == PRE_CANCELLED {
                    break;
                }
                runner.run_one(s, &scripts.env, scripts.cwd.as_deref(), false);
            }
            let go = state.compare_exchange(PRE_PENDING, PRE_SPAWNED, Ordering::SeqCst, Ordering::SeqCst).is_ok();
            let send = |ev: LaunchEvent| {
                let _ = ev_tx.send(ev);
                ctx.request_repaint();
            };
            if !go {
                log::info!("launch of game {id} cancelled before it started");
                send(LaunchEvent::Exited { id, secs: 0 });
                return;
            }
            match job.spawn(&tx) {
                Ok(()) => {
                    if announced {
                        send(LaunchEvent::Started { id });
                    }
                    if !tracked {
                        for s in &scripts.after_start {
                            runner.run_one(s, &scripts.env, scripts.cwd.as_deref(), false);
                        }
                    }
                }
                Err(e) => {
                    send(LaunchEvent::Failed { id, error: format!("{e:#}") });
                    if tracked {
                        send(LaunchEvent::Exited { id, secs: 0 });
                    }
                }
            }
            drop(guard);
        });
        if let Err(e) = spawned {
            self.active.retain(|a| *a != id);
            self.session_scripts.remove(&id);
            self.prelaunch.remove(&id);
            bail!("couldn't start the pre-launch thread: {e}");
        }
        Ok(())
    }

    pub(super) fn mark_active(&mut self, id: GameId) {
        self.active.retain(|a| *a != id);
        self.active.push(id);
        self.started.retain(|a| *a != id);
    }

    pub(super) fn monitor_tx(&mut self) -> Sender<track::Msg> {
        if let Some(tx) = &self.monitor {
            return tx.clone();
        }
        let tx = track::spawn(self.ctx.clone(), self.ev_tx.clone(), self.poll_every, self.steam_timeout);
        self.monitor = Some(tx.clone());
        tx
    }

    /// Events since the last call (non-blocking). Also starts the session's after-start /
    /// after-exit scripts (in the background).
    pub fn poll(&mut self) -> Vec<LaunchEvent> {
        let mut out = std::mem::take(&mut self.pending);
        out.extend(self.ev_rx.try_iter());
        for ev in &out {
            if let Some(tx) = &self.hygiene {
                match ev {
                    LaunchEvent::Started { id } if self.hygiene_ids.contains(id) => {
                        let _ = tx.send(hygiene::Msg::Started(*id));
                    }
                    LaunchEvent::Exited { id, .. } if self.hygiene_ids.remove(id) => {
                        let _ = tx.send(hygiene::Msg::Exited(*id));
                    }
                    _ => {}
                }
            }
            match ev {
                LaunchEvent::Exited { id, .. } => {
                    if let Some(tag) = self.adopted_tags.remove(id) {
                        self.ended_tags.insert(tag);
                    }
                    self.active.retain(|a| a != id);
                    self.started.retain(|a| a != id);
                    self.prelaunch.remove(id);
                    let ran = self.script_started.remove(id);
                    if let Some(s) = self.session_scripts.remove(id)
                        && ran
                    {
                        self.scripts.spawn(s.after_exit, s.env, s.cwd, false);
                    }
                }
                LaunchEvent::Started { id } => {
                    if !self.started.contains(id) {
                        self.started.push(*id);
                    }
                    if self.script_started.insert(*id)
                        && let Some(s) = self.session_scripts.get(id)
                    {
                        self.scripts.spawn(s.after_start.clone(), s.env.clone(), s.cwd.clone(), false);
                    }
                }
                LaunchEvent::Failed { .. } => {}
            }
        }
        out
    }

    /// Finished scripts since the last call (failures, and Test results) for toasts.
    pub fn script_notices(&mut self) -> Vec<ScriptNotice> {
        self.notices.try_iter().collect()
    }

    /// Scripts are still running (`--play` waits for after-exit scripts before it quits).
    pub fn scripts_busy(&self) -> bool {
        self.scripts.busy()
    }

    /// Run scripts now in the background (Test button); every result is reported.
    pub fn run_scripts(&self, scripts: Vec<scripts::Script>, env: Vec<(String, String)>, cwd: Option<PathBuf>) {
        self.scripts.spawn(scripts, env, cwd, true);
    }

    /// The game being launched or running (most recent), if any.
    pub fn running(&self) -> Option<GameId> {
        self.active.last().copied()
    }

    /// The game actually seen running (most recent). Unlike `running()`, this is None while a
    /// Steam game is still being started (Steam may sit on an update or login dialog for minutes),
    /// so use this to decide whether to ignore the controller.
    pub fn started(&self) -> Option<GameId> {
        self.started.last().copied()
    }

    /// Ask the running game to quit (SIGTERM to its processes), best effort. Asking again once
    /// it has ignored that for a few seconds kills it (SIGKILL). While a Steam game hasn't shown
    /// up yet, this just stops waiting for it (Exited { secs: 0 }); while its before-start
    /// scripts run, the game isn't started at all.
    pub fn stop(&mut self) {
        if let Some(id) = self.running() {
            self.stop_game(id);
        }
    }

    /// `stop()` for one game of several (one game at a time closing the running one).
    pub fn stop_game(&mut self, id: GameId) {
        if !self.active.contains(&id) {
            return;
        }
        if let Some(state) = self.prelaunch.get(&id)
            && state.compare_exchange(PRE_PENDING, PRE_CANCELLED, Ordering::SeqCst, Ordering::SeqCst).is_ok()
        {
            return;
        }
        if let Some(tx) = &self.monitor {
            let _ = tx.send(track::Msg::Stop { id });
        }
    }

    /// Games being launched or running, oldest first.
    pub fn active_games(&self) -> &[GameId] {
        &self.active
    }

    /// `id` was seen running (not just being started).
    pub fn is_started(&self, id: GameId) -> bool {
        self.started.contains(&id)
    }

    /// What `id` was last started through this session.
    pub fn launched_via(&self, id: GameId) -> Option<Via> {
        self.via.get(&id).copied()
    }

    /// How `id` was last launched this session (None: not launched). Still answers after its
    /// `Exited` event, so the app can tell a session it timed itself (`Process`) from one the
    /// Steam client ran and counts.
    pub fn launch_kind(&self, id: GameId) -> Option<ResolvedKind> {
        self.kinds.get(&id).copied()
    }

    /// Log file (stdout/stderr) of the last launch of `id` this session.
    #[cfg(test)]
    pub fn last_log(&self, id: GameId) -> Option<&Path> {
        self.logs.get(&id).map(PathBuf::as_path)
    }
}

impl crate::app::AppState {
    /// Ask the running game to quit (best effort).
    #[cfg(test)]
    pub fn stop_running_game(&mut self) {
        self.launcher.stop();
    }

    /// What launching `id` would run right now (None if the game is unknown).
    pub fn launch_preview(&self, id: GameId) -> Option<anyhow::Result<Resolved>> {
        self.library.get(id).map(resolve)
    }
}
