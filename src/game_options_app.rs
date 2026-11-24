//! App-side plumbing for game options: keeps the process-wide
//! `game_options::LaunchEnv` in step with the settings, the detected capabilities and the monitor
//! the window is on, so `launch::resolve` (and every "Will run" preview) sees them.
//!
//! * Startup: `init_game_options()` publishes the saved defaults and starts one background
//!   capability probe (file checks plus one cached `gamescope --help`; never on the UI thread).
//! * Every frame: `poll_game_options(ctx)` picks up a finished probe and the monitor size
//!   (cheap: a comparison unless something changed). A probe thread that died without an answer
//!   ends the "Checking…" state; one that hangs stops holding the editors back after a while.
//! * A real launch before the first probe finished: `ensure_game_options_probed()` waits for it
//!   (bounded), so Play right after startup doesn't silently lose its wrappers.
//! * Settings → Games "Check again", showing Settings → Games, opening a game's Launch options:
//!   `reprobe_game_options()` (tools installed while Lodestar runs show up).
//! * `save_config` calls `publish_game_option_defaults()`.

use crate::app::AppState;
use crate::game_options::{self, Caps};
use crossbeam_channel::{Receiver, RecvTimeoutError, TryRecvError, bounded};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// After this long the running probe stops holding the editors back ("Checking…" ends; nothing
/// is greyed from unprobed caps). The thread is left alone: a second one would only block on the
/// same slow drive.
pub const PROBE_STALL: Duration = Duration::from_secs(20);

/// How long a launch waits for the probe still running (its slowest step is `gamescope --help`).
pub const LAUNCH_PROBE_WAIT: Duration = Duration::from_secs(game_options::caps::GAMESCOPE_HELP_TIMEOUT.as_secs() + 1);

/// Probe state (AppState::game_opts).
#[derive(Default)]
pub struct GameOptsState {
    /// A probe is running; its result arrives here.
    rx: Option<Receiver<Caps>>,
    /// When the running probe started.
    started: Option<Instant>,
    /// The app manages probing (`init_game_options` ran). Off in test states: a launch there
    /// never probes the real machine or touches the process-wide LaunchEnv.
    pub(crate) active: bool,
    /// At least one probe finished, died or stalled (before that, "not installed" would be a
    /// guess). `caps.probed` says whether real results arrived.
    pub probed: bool,
    /// The capabilities last published (for the UI: availability, layer list, lsfg profiles).
    pub caps: Arc<Caps>,
}

impl GameOptsState {
    pub fn probing(&self) -> bool {
        self.rx.is_some()
    }

    /// The running probe takes much longer than it should (a slow or unreachable drive).
    pub fn stalled(&self) -> bool {
        self.rx.is_some() && self.started.is_some_and(|t| t.elapsed() >= PROBE_STALL)
    }

    fn start(&mut self, rx: Receiver<Caps>) {
        self.rx = Some(rx);
        self.started = Some(Instant::now());
    }

    /// One non-blocking look at the running probe: Some = new caps to publish.
    fn poll(&mut self) -> Option<Arc<Caps>> {
        let r = self.rx.as_ref()?.try_recv();
        match r {
            Ok(caps) => Some(self.finished(caps)),
            Err(TryRecvError::Disconnected) => {
                self.died();
                None
            }
            Err(TryRecvError::Empty) => {
                if !self.probed && self.stalled() {
                    log::warn!("game options: the capability probe is taking over {PROBE_STALL:?}");
                    self.probed = true;
                }
                None
            }
        }
    }

    /// Block up to `timeout` for the running probe: Some = new caps to publish.
    fn wait(&mut self, timeout: Duration) -> Option<Arc<Caps>> {
        let r = self.rx.as_ref()?.recv_timeout(timeout);
        match r {
            Ok(caps) => Some(self.finished(caps)),
            Err(RecvTimeoutError::Disconnected) => {
                self.died();
                None
            }
            Err(RecvTimeoutError::Timeout) => {
                log::warn!("game options: launching before the capability probe finished (waited {timeout:?})");
                None
            }
        }
    }

    fn finished(&mut self, caps: Caps) -> Arc<Caps> {
        self.rx = None;
        self.started = None;
        self.probed = true;
        self.caps = Arc::new(caps);
        self.caps.clone()
    }

    /// The probe thread exited (panicked) without sending: "Check again" works again and the
    /// editors stop waiting; the previous caps stay.
    fn died(&mut self) {
        log::warn!("game options: the capability probe ended without a result");
        self.rx = None;
        self.started = None;
        self.probed = true;
    }
}

impl AppState {
    /// Publish the saved defaults and start the first capability probe.
    pub fn init_game_options(&mut self) {
        self.game_opts.active = true;
        game_options::set_log_dir(self.paths.data_dir.join("proton-logs"));
        self.publish_game_option_defaults();
        self.reprobe_game_options();
    }

    /// The global defaults from `config.game_options` (after Settings changes).
    pub fn publish_game_option_defaults(&mut self) {
        game_options::set_defaults(self.config.game_options.clone());
        game_options::set_source_defaults(self.config.game_options_by_source.clone());
        // Settings → Emulation changes reach Play without a rescan.
        self.refresh_emulation(false);
    }

    /// Detect emulators again and publish them with `config.emulation` (the registry Play builds
    /// emulated games' commands from), on a background thread. `force: false` skips it when the
    /// published registry already has these settings. Test states never probe the machine.
    pub fn refresh_emulation(&self, force: bool) {
        if !self.game_opts.active {
            return;
        }
        // Compared with the last settings asked for, not the published ones: a refresh may still
        // be running (A → B → A while B is detected must still end with A).
        static REQUESTED: parking_lot::Mutex<Option<crate::config::EmulationConfig>> = parking_lot::Mutex::new(None);
        // Refreshes run one at a time, in the order asked for; one that a newer request overtook
        // while it waited is skipped (the newer one runs after it and publishes last).
        static LATEST: AtomicU64 = AtomicU64::new(0);
        static RUN: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
        let ticket = {
            let mut last = REQUESTED.lock();
            if !force && last.as_ref() == Some(&self.config.emulation) {
                return;
            }
            *last = Some(self.config.emulation.clone());
            LATEST.fetch_add(1, Ordering::SeqCst) + 1
        };
        let ctx = crate::scan::ScanContext::from_config(&self.config);
        let egui_ctx = self.ctx().clone();
        // Emulated games: their disk facts are looked up here, never on the UI thread.
        let games: Vec<crate::model::Game> = self.library.games.iter().filter(|g| g.emu.is_some()).cloned().collect();
        let spawned = std::thread::Builder::new().name("emulation-detect".into()).spawn(move || {
            let _one_at_a_time = RUN.lock();
            if LATEST.load(Ordering::SeqCst) != ticket {
                return;
            }
            let t = std::time::Instant::now();
            crate::emulation::registry::refresh(&ctx);
            crate::emulation::registry::warm(&games);
            log::debug!("emulation: emulators detected in {:?}", t.elapsed());
            egui_ctx.request_repaint();
        });
        if let Err(e) = spawned {
            log::warn!("couldn't start emulator detection: {e}");
        }
    }

    /// Detect what is installed again (no-op while a probe runs).
    pub fn reprobe_game_options(&mut self) {
        // Test states never probe the real machine; one probe at a time.
        if !self.game_opts.active || self.game_opts.rx.is_some() {
            return;
        }
        let (tx, rx) = bounded(1);
        let ctx = self.ctx().clone();
        let spawned = std::thread::Builder::new().name("game-options-probe".into()).spawn(move || {
            let t = std::time::Instant::now();
            let caps = game_options::caps::probe();
            log::debug!("game options: capabilities probed in {:?}", t.elapsed());
            let _ = tx.send(caps);
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => self.game_opts.start(rx),
            Err(e) => log::warn!("couldn't start the game options probe: {e}"),
        }
    }

    /// Before a real launch: make sure the capabilities were detected once, so wrapper options
    /// (Gamescope, MangoHud, GameMode …) aren't dropped as "not installed" right after startup.
    /// Waits for the running probe (bounded); with none running (it couldn't start) probes here.
    pub fn ensure_game_options_probed(&mut self) {
        let s = &mut self.game_opts;
        if !s.active || s.probed {
            return;
        }
        let caps = if s.rx.is_some() {
            s.wait(LAUNCH_PROBE_WAIT)
        } else {
            let t = Instant::now();
            let caps = game_options::caps::probe();
            log::debug!("game options: capabilities probed for a launch in {:?}", t.elapsed());
            Some(s.finished(caps))
        };
        if let Some(caps) = caps {
            game_options::set_caps((*caps).clone());
        }
    }

    /// Per frame: a finished probe, and the size of the monitor the window is on (physical
    /// pixels: gamescope's Auto output resolution).
    pub fn poll_game_options(&mut self, ctx: &egui::Context) {
        if let Some(caps) = self.game_opts.poll() {
            game_options::set_caps((*caps).clone());
        }
        let monitor = ctx.input(|i| i.viewport().monitor_size).map(|s| {
            let ppp = ctx.pixels_per_point();
            ((s.x * ppp).round() as u32, (s.y * ppp).round() as u32)
        });
        if monitor.is_some_and(|(w, h)| w >= 320 && h >= 200) {
            game_options::set_monitor(monitor);
        }
    }
}

/// `lodestar --game-options-report`: what game options can use on this machine (plain text).
pub fn caps_report(c: &Caps) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let path =
        |p: &Option<std::path::PathBuf>| p.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "—".into());
    let b = &c.bins;
    let _ = writeln!(s, "Tools");
    for (n, p) in [
        ("gamescope", &b.gamescope),
        ("mangohud", &b.mangohud),
        ("mangoapp", &b.mangoapp),
        ("gamemoderun", &b.gamemoderun),
        ("game-performance", &b.game_performance),
        ("powerprofilesctl", &b.powerprofilesctl),
        ("tlpctl", &b.tlpctl),
        ("obs-gamecapture", &b.obs_gamecapture),
        ("systemd-inhibit", &b.systemd_inhibit),
    ] {
        let _ = writeln!(s, "  {n:<18} {}", path(p));
    }
    if let Some(g) = &c.gamescope {
        let _ = writeln!(s, "  gamescope version {:?}, filters {:?}, {} flags", g.version, g.filters, g.flags.len());
    }
    let _ = writeln!(s, "System");
    let _ = writeln!(
        s,
        "  distro {:?}, wayland session {}, inside gamescope {}",
        c.distro, c.wayland_session, c.nested_gamescope
    );
    let _ = writeln!(s, "  NTSync {:?}, NVIDIA driver {:?}", c.ntsync, c.nvidia_version);
    let _ = writeln!(
        s,
        "  KDE HDR {:?}, KDE VRR never {:?}, preferred mode {:?}",
        c.kde_hdr_enabled, c.kde_vrr_never, c.preferred_mode
    );
    for g in &c.gpus {
        let _ = writeln!(
            s,
            "  GPU {} {} {:04x}:{:04x} driver {} boot_vga {} vram {:?} gc {:?} discrete-tag {}",
            g.card, g.pci, g.vendor, g.device, g.driver, g.boot_vga, g.vram, g.gc_ip, g.discrete_tag
        );
    }
    let _ = writeln!(s, "Vulkan layers");
    for l in &c.layers {
        let _ = writeln!(s, "  {} (enable {:?}, disable {:?})", l.name, l.enable_env, l.disable_env);
    }
    if let Some(l) = &c.lsfg {
        let _ = writeln!(
            s,
            "  lsfg-vk v{}, profiles {:?}, dll {:?}",
            l.version,
            l.profiles.iter().map(|p| &p.0).collect::<Vec<_>>(),
            l.dll
        );
    }
    if !c.flatpak_ext.is_empty() {
        let _ = writeln!(s, "Flatpak extensions");
        for (k, v) in &c.flatpak_ext {
            let _ = writeln!(s, "  {k} {v:?}");
        }
    }
    let _ = writeln!(s, "Proton builds");
    let mut runners: Vec<_> = c.runners.iter().collect();
    runners.sort_by(|a, b| a.0.cmp(b.0));
    for (dir, r) in runners {
        let mut vars: Vec<&String> = r.vars.iter().collect();
        vars.sort();
        let _ = writeln!(
            s,
            "  {} — {:?} {:?}; wayland driver {}, wine FSR {}, ntsync {}, PROTON_NO_NTSYNC {}, FSR4 contrib {}\n      vars: {}",
            dir.display(),
            r.family,
            r.version,
            r.winewayland,
            r.fshack_fsr,
            r.ntsync,
            r.wineserver_no_ntsync,
            r.contrib_fsr4,
            vars.iter().map(|v| v.as_str()).collect::<Vec<_>>().join(" ")
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn real_caps() -> Caps {
        Caps { probed: true, ..Default::default() }
    }

    #[test]
    fn a_finished_probe_is_published_once() {
        let mut s = GameOptsState::default();
        let (tx, rx) = bounded(1);
        s.start(rx);
        assert!(s.probing() && !s.probed);
        assert_eq!(s.poll(), None, "nothing yet");
        tx.send(real_caps()).unwrap();
        assert_eq!(s.poll().as_deref(), Some(&real_caps()));
        assert!(!s.probing() && s.probed && s.caps.probed);
        assert_eq!(s.poll(), None);
    }

    #[test]
    fn a_probe_that_dies_ends_checking() {
        let mut s = GameOptsState::default();
        let (tx, rx) = bounded::<Caps>(1);
        s.start(rx);
        drop(tx); // the probe thread panicked
        assert_eq!(s.poll(), None);
        assert!(!s.probing(), "Check again works again");
        assert!(s.probed, "the editors stop showing Checking…");
        assert!(!s.caps.probed, "no real caps: nothing gets greyed");
    }

    #[test]
    fn a_hanging_probe_stops_holding_the_editors_back() {
        let mut s = GameOptsState::default();
        let (_tx, rx) = bounded::<Caps>(1);
        s.start(rx);
        assert_eq!(s.poll(), None);
        assert!(!s.probed && !s.stalled());
        s.started = Instant::now().checked_sub(PROBE_STALL + Duration::from_secs(1));
        assert_eq!(s.poll(), None);
        assert!(s.stalled() && s.probed);
        assert!(s.probing(), "no second probe piles up on the same slow drive");
    }

    #[test]
    fn a_launch_waits_for_the_running_probe() {
        let mut s = GameOptsState::default();
        let (tx, rx) = bounded(1);
        s.start(rx);
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            tx.send(real_caps()).unwrap();
        });
        assert_eq!(s.wait(Duration::from_secs(5)).as_deref(), Some(&real_caps()));
        assert!(s.probed && !s.probing());
        t.join().unwrap();
        // Bounded: a probe that never answers doesn't hold the launch for long.
        let (_tx, rx) = bounded::<Caps>(1);
        let mut s = GameOptsState::default();
        s.start(rx);
        let t = Instant::now();
        assert_eq!(s.wait(Duration::from_millis(30)), None);
        assert!(t.elapsed() < Duration::from_secs(2));
        assert!(s.probing() && !s.probed);
    }

    #[test]
    fn test_states_never_probe_on_launch() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        st.ensure_game_options_probed();
        assert!(!st.game_opts.probed && !st.game_opts.probing());
        // Opening Launch options or Settings → Games doesn't start one either.
        st.reprobe_game_options();
        assert!(!st.game_opts.probing());
    }
}
