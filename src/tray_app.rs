//! The app side of the system tray, hiding the window and the single instance:
//! * starts / stops the tray icon (tray.rs) with Settings → General → "Show Lodestar in the
//!   system tray", and keeps what it shows (game, Show / Hide) up to date;
//! * acts on tray clicks: toggle / show / hide the window, stop the game, quit;
//! * hides the window to the tray (winctl.rs) when closing says so (running_app.rs), and tells
//!   the user once where it went;
//! * a second start of the GUI (instance.rs) shows this window instead.
//!
//! Everything here runs in `App::logic`, which eframe keeps calling while the window is hidden.

use crate::app::AppState;
use crate::tray::{TrayEvent, TrayHandle, TrayState};
use crate::winctl::Platform;
use crossbeam_channel::{Receiver, Sender};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Per-session state (`AppState::shell`).
pub(crate) struct ShellState {
    /// Events from the tray that runs now: a new channel for each start, so a tray thread that
    /// was turned off and is still winding down can never undo what its successor said.
    events_rx: Receiver<TrayEvent>,
    /// Messages from later starts (`activate [mode]`).
    activate_tx: Sender<String>,
    activate_rx: Receiver<String>,
    /// The window is closing for good: later starts are told so and run themselves once this
    /// process is gone, instead of handing over to a window that is going away.
    closing: Arc<AtomicBool>,
    tray: Option<TrayHandle>,
    /// A tray host shows the icon right now.
    tray_host: bool,
    /// The window is hidden to the tray.
    pub hidden: bool,
    /// The tray may be used this session: the GUI, never screenshot runs or tests (they must not
    /// put an icon on the user's desktop).
    allowed: bool,
    platform: Platform,
    /// What the tray was last told.
    sent: Option<TrayState>,
    /// When the window last had the keyboard focus (a tray click hides a window only when it
    /// was in front just now; one behind others or minimised comes to the front instead).
    focused_at: Option<Instant>,
}

impl Default for ShellState {
    fn default() -> Self {
        let (activate_tx, activate_rx) = crossbeam_channel::unbounded();
        ShellState {
            events_rx: crossbeam_channel::never(),
            activate_tx,
            activate_rx,
            closing: Arc::new(AtomicBool::new(false)),
            tray: None,
            tray_host: false,
            hidden: false,
            allowed: false,
            platform: Platform::X11,
            sent: None,
            focused_at: None,
        }
    }
}

/// How long after losing the focus the window still counts as in front for a tray click.
const IN_FRONT_GRACE: Duration = Duration::from_millis(1500);

pub(crate) const TRAY_HINT: &str = "Lodestar is in the system tray — click its icon to bring it back.";

impl AppState {
    /// At startup (GUI): the tray when allowed and wanted, and answering later starts.
    pub(crate) fn init_shell(&mut self, allowed: bool, instance: Option<crate::instance::Instance>) {
        self.shell.allowed = allowed;
        self.shell.platform = Platform::detect();
        self.apply_tray_setting();
        if let Some(inst) = instance {
            let (tx, ctx, closing) = (self.shell.activate_tx.clone(), self.ctx().clone(), self.shell.closing.clone());
            inst.serve(move |m| {
                if closing.load(Ordering::SeqCst) {
                    log::info!("another start of Lodestar ({m}) while quitting: it runs itself");
                    return false;
                }
                log::info!("another start of Lodestar: {m}");
                let _ = tx.send(m);
                ctx.request_repaint();
                true
            });
        }
    }

    /// The tray icon on or off per the setting.
    pub(crate) fn apply_tray_setting(&mut self) {
        if !self.shell.allowed {
            return;
        }
        let want = self.config.tray_icon;
        if want && self.shell.tray.is_none() {
            let state = self.tray_state();
            let (tx, rx) = crossbeam_channel::unbounded();
            self.shell.events_rx = rx;
            self.shell.tray = Some(crate::tray::start(self.ctx(), tx, state.clone()));
            self.shell.sent = Some(state);
        } else if !want && self.shell.tray.is_some() {
            self.shell.tray = None;
            self.shell.events_rx = crossbeam_channel::never();
            self.shell.sent = None;
            self.set_tray_host(false);
        }
    }

    /// Lodestar's icon is in the tray (so hiding there can be undone).
    pub(crate) fn tray_shown(&self) -> bool {
        self.shell.tray.is_some() && self.shell.tray_host
    }

    /// Hiding takes the window out of the taskbar here (else it only minimises).
    pub(crate) fn can_hide_from_taskbar(&self) -> bool {
        self.shell.platform.hides_from_taskbar()
    }

    fn set_tray_host(&mut self, up: bool) {
        self.shell.tray_host = up;
        // The tray went away while the window was hidden there: never leave it unreachable.
        if !up && self.shell.hidden {
            self.show_window();
        }
    }

    fn tray_state(&self) -> TrayState {
        TrayState {
            shown: !self.shell.hidden,
            game: self.running_game().map(|id| self.game_name(id)),
            can_stop: !self.no_launch,
        }
    }

    /// Per frame: tray clicks, later starts, and the tray's state.
    pub(crate) fn poll_shell(&mut self) {
        while let Ok(ev) = self.shell.events_rx.try_recv() {
            self.on_tray_event(ev);
        }
        while let Ok(m) = self.shell.activate_rx.try_recv() {
            self.show_window();
            if let Some(mode) = crate::instance::requested_mode(&m) {
                self.switch_mode(mode);
            }
        }
        if let Some(t) = &self.shell.tray {
            let s = self.tray_state();
            if self.shell.sent.as_ref() != Some(&s) {
                t.set_state(s.clone());
                self.shell.sent = Some(s);
            }
        }
    }

    pub(crate) fn on_tray_event(&mut self, ev: TrayEvent) {
        match ev {
            TrayEvent::Toggle if !self.in_front() => self.show_window(),
            TrayEvent::Toggle | TrayEvent::Hide => self.hide_to_tray(),
            TrayEvent::Show => self.show_window(),
            TrayEvent::StopGame => {
                if let Some(id) = self.running_game() {
                    crate::ui::launching::stop(self, id);
                }
            }
            TrayEvent::Quit => self.quit(),
            TrayEvent::Host(up) => self.set_tray_host(up),
        }
    }

    /// Hide the window to the tray (out of the taskbar where the desktop allows it, else
    /// minimised). The first time ever, says where it went.
    pub(crate) fn hide_to_tray(&mut self) {
        if self.shell.hidden {
            return;
        }
        log::info!("window hidden to the tray");
        crate::winctl::hide(self.ctx(), self.shell.platform);
        self.shell.hidden = true;
        // Don't pop back up when the game ends.
        self.running.minimised_for_close = true;
        if !self.config.tray_hint_shown {
            self.config.tray_hint_shown = true;
            self.save_config();
            self.notify_long(TRAY_HINT);
            if self.shell.allowed {
                desktop_notice(TRAY_HINT);
            }
        }
    }

    /// Show the window (from the tray, a later start) and bring it to the front.
    pub(crate) fn show_window(&mut self) {
        crate::winctl::show(self.ctx(), self.shell.platform);
        self.shell.hidden = false;
    }

    /// The window closes for good (quit, or a close that wasn't turned into hiding).
    pub(crate) fn on_closing(&mut self) {
        self.shell.closing.store(true, Ordering::SeqCst);
    }

    /// The window got the focus: whatever hid it, it is shown now.
    pub(crate) fn on_shown(&mut self) {
        self.shell.hidden = false;
    }

    /// Per frame: the window has the keyboard focus.
    pub(crate) fn note_focus(&mut self, focused: bool) {
        if focused {
            self.shell.focused_at = Some(Instant::now());
        }
    }

    /// The window is shown and was in front just now (clicking the tray may have taken the
    /// focus a moment ago). A minimised window, or one behind others, is not.
    fn in_front(&self) -> bool {
        !self.shell.hidden && self.shell.focused_at.is_some_and(|t| t.elapsed() < IN_FRONT_GRACE)
    }

    #[cfg(test)]
    pub(crate) fn fake_tray(&mut self, host: bool) {
        self.shell.tray = Some(crate::tray::TrayHandle::detached());
        self.shell.tray_host = host;
    }
}

/// A desktop notification (the window it would show in is hidden), best effort.
fn desktop_notice(text: &str) {
    let Some(ns) = crate::launch::find_in_path("notify-send") else { return };
    let child = std::process::Command::new(ns)
        .args(["--app-name=Lodestar", "--icon=lodestar", "--expire-time=6000", "Lodestar", text])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Ok(mut c) = child {
        let _ = std::thread::Builder::new().name("notify-send".into()).spawn(move || c.wait());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WhenClosing;
    use crate::model::{Art, LaunchSpec};

    fn sleeper(st: &mut AppState) -> crate::model::GameId {
        st.add_manual_game(
            "Sleeper",
            LaunchSpec::Command { program: "sleep".into(), args: vec!["3".into()], cwd: None, env: vec![] },
            Art::default(),
        );
        st.library.games.iter().find(|g| g.name == "Sleeper").unwrap().id
    }

    #[test]
    fn closing_hides_to_the_tray_and_the_tray_brings_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        st.fake_tray(true);
        let id = sleeper(&mut st);
        st.launch(id);
        assert!(st.on_close_request(false), "close not cancelled");
        assert!(st.shell.hidden && st.running.minimised_for_close);
        assert!(st.config.tray_hint_shown);
        assert!(st.toasts.iter().any(|t| t.text == TRAY_HINT));
        assert_eq!(st.tray_state(), TrayState { shown: false, game: Some("Sleeper".into()), can_stop: true });
        // The hint only once.
        st.toasts.clear();
        st.on_tray_event(TrayEvent::Show);
        assert!(!st.shell.hidden);
        // Shown but never in front (minimised, behind other windows): a click brings it forward.
        st.on_tray_event(TrayEvent::Toggle);
        assert!(!st.shell.hidden);
        // In front: a click hides it.
        st.on_focus(true);
        st.on_tray_event(TrayEvent::Toggle);
        assert!(st.shell.hidden && st.toasts.is_empty());
        st.on_tray_event(TrayEvent::Toggle);
        assert!(!st.shell.hidden);
        // The tray going away while hidden shows the window.
        st.on_tray_event(TrayEvent::Hide);
        st.on_tray_event(TrayEvent::Host(false));
        assert!(!st.shell.hidden && !st.tray_shown());
        // No tray: closing minimises (the automatic setting falls back).
        assert!(st.on_close_request(false));
        assert!(!st.shell.hidden && st.running.minimised_for_close);
        // Stop from the tray stops the game; Quit quits.
        st.on_tray_event(TrayEvent::StopGame);
        assert!(st.poll_until(Duration::from_secs(15), |s| s.launcher.running().is_none()), "not stopped");
        st.on_tray_event(TrayEvent::Quit);
        assert!(st.quitting());
    }

    #[test]
    fn always_to_the_tray_also_without_a_game() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        st.config.when_closing = Some(WhenClosing::TrayAlways);
        // No tray: nothing to hide to, so closing quits.
        assert!(!st.on_close_request(false));
        st.fake_tray(true);
        assert!(st.on_close_request(false));
        assert!(st.shell.hidden);
        st.quit();
        assert!(!st.on_close_request(false), "Quit always quits");
    }

    #[test]
    fn a_click_hides_only_a_window_that_was_in_front() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        st.fake_tray(true);
        st.on_focus(true);
        // Clicking the tray icon may take the focus away a moment before the click arrives.
        st.on_focus(false);
        st.on_tray_event(TrayEvent::Toggle);
        assert!(st.shell.hidden, "in front a moment ago: hidden");
        st.on_tray_event(TrayEvent::Toggle);
        assert!(!st.shell.hidden);
        // Long out of focus (behind other windows, minimised while a game runs): shown instead.
        st.shell.focused_at = Some(Instant::now() - Duration::from_secs(10));
        st.on_tray_event(TrayEvent::Toggle);
        assert!(!st.shell.hidden);
        // Hide in the menu always hides.
        st.on_tray_event(TrayEvent::Hide);
        assert!(st.shell.hidden);
    }

    #[test]
    fn a_later_start_shows_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        st.fake_tray(true);
        st.hide_to_tray();
        st.shell.activate_tx.send("activate controller".into()).unwrap();
        st.poll_shell();
        assert!(!st.shell.hidden);
    }
}
