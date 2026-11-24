//! Games that keep running while Lodestar comes and goes:
//! * picking up games that already run when Lodestar starts (or its window gets the focus
//!   back): ones an earlier Lodestar launched, found by their launch tag, and Steam games
//!   (`Launcher::start_running_scan`, on a thread of its own);
//! * what closing the window does (`config.when_closing`): hide to the system tray while a game
//!   runs (default when there is a tray) or always, minimise while a game runs (default without
//!   a tray), quit (a game keeps running and is picked up again on the next start) or ask. The
//!   menu's Quit, Ctrl+Q and the tray's Quit always quit.

use crate::app::AppState;
use crate::config::WhenClosing;
use crate::model::GameId;
use std::time::{Duration, Instant};

/// Per-session state (`AppState::running`).
#[derive(Debug, Default)]
pub(crate) struct RunningState {
    /// "Quit while a game runs?" is waiting for an answer.
    pub close_prompt: bool,
    /// The window was minimised (or hidden to the tray) instead of closed: don't pop it back up
    /// when the game exits.
    pub minimised_for_close: bool,
    /// When running games were last looked for.
    last_adopt: Option<Instant>,
    /// The window had the focus last frame.
    focused: bool,
}

/// Looking for running games again when the window gets the focus back waits this long after the
/// last look (a few ms each, but focus can flap).
const ADOPT_AGAIN_AFTER: Duration = Duration::from_secs(5);

/// While the controller drives an unfocused Lodestar (`[controller] background_input`): how often
/// to look for games started elsewhere (Steam, a desktop shortcut, Heroic …), so a game's button
/// presses never act on Lodestar (input/gate.rs).
pub(crate) const BACKGROUND_LOOK: Duration = Duration::from_secs(2);
/// A look older than this no longer counts: the background controller waits (only Guide gets
/// through) until a new one says no game runs.
pub(crate) const LOOK_FRESH: Duration = Duration::from_millis(4500);

/// What a request to close the window does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAction {
    Close,
    /// Cancel the close and hide the window to the system tray.
    HideToTray,
    /// Cancel the close, minimise the window and keep following the game.
    Minimise,
    /// Cancel the close and ask.
    Ask,
}

/// Answers to the "quit while a game runs?" prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAnswer {
    /// Keep Lodestar running: hidden to the tray when there is one, else minimised.
    KeepRunning,
    Quit,
    Cancel,
}

/// What closing the window does: Quit (menu, Ctrl+Q, tray: `explicit_quit`) and scripted
/// screenshot runs always close. Otherwise the setting (`WhenClosing::effective`) decides; the
/// tray settings act as "minimise while a game runs" while Lodestar has no icon in a tray
/// (`tray`), and every setting but "always hide to the tray" closes when no game runs.
pub fn close_action(
    explicit_quit: bool,
    scripted: bool,
    game_running: bool,
    setting: WhenClosing,
    tray: bool,
) -> CloseAction {
    if explicit_quit || scripted {
        return CloseAction::Close;
    }
    let setting = if setting.uses_tray() && !tray { WhenClosing::MinimiseWhilePlaying } else { setting };
    match setting {
        WhenClosing::TrayAlways => CloseAction::HideToTray,
        _ if !game_running => CloseAction::Close,
        WhenClosing::TrayWhilePlaying => CloseAction::HideToTray,
        WhenClosing::MinimiseWhilePlaying => CloseAction::Minimise,
        WhenClosing::Quit => CloseAction::Close,
        WhenClosing::Ask => CloseAction::Ask,
    }
}

impl AppState {
    /// Look for games that already run (see the module docs); `poll_running_scan` follows them.
    pub(crate) fn adopt_running_games(&mut self) {
        self.running.last_adopt = Some(Instant::now());
        // Screenshot runs show what they were asked to, not whatever the user plays right now
        // (`LODESTAR_SCREENSHOT_ADOPT=1` to check this feature; Stop does nothing there).
        if self.no_launch && std::env::var_os("LODESTAR_SCREENSHOT_ADOPT").is_none_or(|v| v.is_empty()) {
            return;
        }
        let dirs: Vec<std::path::PathBuf> = self
            .library
            .games
            .iter()
            .filter(|g| g.installed)
            .filter_map(|g| g.install_dir.clone())
            .filter(|d| crate::launch::is_game_folder(d))
            .collect();
        self.launcher.start_running_scan(dirs);
    }

    /// Per logic pass: while the controller drives Lodestar in the background (`wanted`: not
    /// focused, background input on, no game followed), look for games started elsewhere every
    /// [`BACKGROUND_LOOK`] and keep frames coming that often (the gate is decided per frame).
    pub(crate) fn background_look(&mut self, wanted: bool) {
        if !wanted {
            return;
        }
        if self.running.last_adopt.is_none_or(|t| t.elapsed() >= BACKGROUND_LOOK) {
            self.adopt_running_games();
        }
        self.ctx().request_repaint_after(BACKGROUND_LOOK);
    }

    /// Per frame: follow what a scan for running games found, once it is done; says which in a
    /// toast.
    pub(crate) fn poll_running_scan(&mut self) {
        let Some(adopted) = self.launcher.finish_running_scan(&self.library.games) else { return };
        let names: Vec<String> = adopted.iter().filter(|a| !a.foreign).map(|a| self.game_name(a.id)).collect();
        match names.as_slice() {
            [] => {}
            [one] => self.notify(format!("{one} is running")),
            many => self.notify(format!("Running: {}", many.join(", "))),
        }
    }

    /// Once per frame with the window's focus: getting it back looks for running games again
    /// (a game started from Steam or a desktop shortcut while Lodestar was in the background).
    pub(crate) fn on_focus(&mut self, focused: bool) {
        let gained = focused && !self.running.focused;
        self.running.focused = focused;
        self.note_focus(focused);
        if !gained {
            return;
        }
        self.running.minimised_for_close = false;
        self.on_shown();
        if self.running.last_adopt.is_none_or(|t| t.elapsed() >= ADOPT_AGAIN_AFTER) {
            self.adopt_running_games();
        }
    }

    /// The game that keeps Lodestar from closing: the latest one it launched or picked up and
    /// follows itself (not one another Lodestar follows).
    pub(crate) fn close_guard_game(&self) -> Option<GameId> {
        self.launcher.active_games().iter().rev().copied().find(|id| !self.launcher.is_foreign(*id))
    }

    /// The window was asked to close (window manager, Alt+F4, the menu's Quit). Returns true when
    /// the close is to be cancelled.
    pub(crate) fn on_close_request(&mut self, scripted: bool) -> bool {
        let game = self.close_guard_game();
        let tray = self.tray_shown();
        let setting = WhenClosing::effective(self.config.when_closing, tray);
        match close_action(self.quitting(), scripted, game.is_some(), setting, tray) {
            CloseAction::Close => {
                if let Some(id) = game {
                    log::info!(
                        "quitting while {} runs; it keeps running and is picked up on the next start",
                        self.game_name(id)
                    );
                }
                false
            }
            CloseAction::HideToTray => {
                self.hide_to_tray();
                true
            }
            CloseAction::Minimise => {
                self.minimise_for_close();
                true
            }
            CloseAction::Ask => {
                self.running.close_prompt = true;
                self.ctx().request_repaint();
                true
            }
        }
    }

    /// The game the "quit while a game runs?" prompt names, while it is open.
    pub(crate) fn close_prompt_game(&self) -> Option<String> {
        if !self.running.close_prompt {
            return None;
        }
        Some(self.close_guard_game().map(|id| self.game_name(id)).unwrap_or_else(|| "The game".into()))
    }

    pub(crate) fn answer_close_prompt(&mut self, answer: CloseAnswer) {
        self.running.close_prompt = false;
        match answer {
            // The game ended meanwhile: closing was what the user asked for.
            CloseAnswer::KeepRunning if self.close_guard_game().is_none() => self.quit(),
            CloseAnswer::KeepRunning if self.tray_shown() => self.hide_to_tray(),
            CloseAnswer::KeepRunning => self.minimise_for_close(),
            CloseAnswer::Quit => self.quit(),
            CloseAnswer::Cancel => {}
        }
    }

    /// Minimise instead of closing. On Wayland the window can minimise itself but not come back
    /// by itself; the user restores it from the taskbar, which is all this needs.
    fn minimise_for_close(&mut self) {
        let Some(id) = self.close_guard_game() else { return };
        let name = self.game_name(id);
        log::info!("window closed while {name} runs: minimised instead");
        self.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        self.running.minimised_for_close = true;
        self.notify_long(format!("Lodestar keeps running while {name} plays. Use Quit in the menu to exit."));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use WhenClosing as S;

    #[test]
    fn closing_decision() {
        use CloseAction as A;
        for tray in [false, true] {
            // No game: closing quits, unless the window always goes to the tray.
            for s in S::ALL {
                let want = if s == S::TrayAlways && tray { A::HideToTray } else { A::Close };
                assert_eq!(close_action(false, false, false, s, tray), want, "{s:?} tray={tray}");
            }
            // Quit (menu, Ctrl+Q, tray) and screenshot runs always close.
            for s in S::ALL {
                for game in [false, true] {
                    assert_eq!(close_action(true, false, game, s, tray), A::Close);
                    assert_eq!(close_action(false, true, game, s, tray), A::Close);
                }
            }
        }
        // A game runs: the setting decides.
        assert_eq!(close_action(false, false, true, S::TrayWhilePlaying, true), A::HideToTray);
        assert_eq!(close_action(false, false, true, S::TrayAlways, true), A::HideToTray);
        assert_eq!(close_action(false, false, true, S::MinimiseWhilePlaying, true), A::Minimise);
        assert_eq!(close_action(false, false, true, S::Quit, true), A::Close);
        assert_eq!(close_action(false, false, true, S::Ask, true), A::Ask);
        // No icon in a tray: the tray settings minimise instead.
        assert_eq!(close_action(false, false, true, S::TrayWhilePlaying, false), A::Minimise);
        assert_eq!(close_action(false, false, true, S::TrayAlways, false), A::Minimise);
        assert_eq!(close_action(false, false, true, S::Ask, false), A::Ask);
        // The automatic default follows the tray.
        assert_eq!(close_action(false, false, true, S::effective(None, true), true), A::HideToTray);
        assert_eq!(close_action(false, false, true, S::effective(None, false), false), A::Minimise);
    }

    #[test]
    fn close_request_with_a_game_running_minimises_or_asks() {
        use crate::model::{Art, LaunchSpec};
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        // Nothing running: the window closes.
        assert!(!st.on_close_request(false));
        st.add_manual_game(
            "Sleeper",
            LaunchSpec::Command { program: "sleep".into(), args: vec!["3".into()], cwd: None, env: vec![] },
            Art::default(),
        );
        let id = st.library.games.iter().find(|g| g.name == "Sleeper").unwrap().id;
        st.launch(id);
        assert_eq!(st.close_guard_game(), Some(id));
        assert!(st.on_close_request(false), "close not cancelled");
        assert!(st.running.minimised_for_close);
        assert!(st.toasts.iter().any(|t| t.text.contains("keeps running while Sleeper plays")));
        st.config.when_closing = Some(S::Ask);
        assert!(st.on_close_request(false));
        assert_eq!(st.close_prompt_game().as_deref(), Some("Sleeper"));
        st.answer_close_prompt(CloseAnswer::Cancel);
        assert!(st.close_prompt_game().is_none() && !st.quitting());
        assert!(st.on_close_request(false));
        st.answer_close_prompt(CloseAnswer::Quit);
        assert!(st.quitting());
        // Quit asked for: the close goes through.
        assert!(!st.on_close_request(false));
        // The game ends by itself; then closing quits even without the menu's Quit.
        assert!(st.poll_until(Duration::from_secs(15), |s| s.launcher.running().is_none()), "never exited");
        st.config.when_closing = Some(S::MinimiseWhilePlaying);
        assert_eq!(st.close_guard_game(), None);
    }
}
