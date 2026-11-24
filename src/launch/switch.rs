//! One game at a time (`config.launchers.one_game_at_a_time`): starting a game while
//! another game Lodestar launched is still launching or running
//! * AllowSeveral: just starts it;
//! * CloseRunning: stops the running game(s) (the usual polite stop; the monitor kills what
//!   ignores it after a grace period, and a second stop here forces it), waits for their `Exited`,
//!   then starts the new game by itself;
//! * Ask: sets `AppState::switch_prompt = Some((running, new))`, which the UIs show as
//!   "<A> is running. Close it and start <B>?" — `confirm_switch()` (Close & start) /
//!   `cancel_switch()`.
//!
//! Also `AppState::launch_status(id)`: the line UIs show while a launch is on its way
//! ("Starting Diablo IV via Battle.net… sign in there if asked", "Closing A to start B…").

use crate::app::AppState;
use crate::config::OneGameAtATime;
use crate::model::{GameId, Source};
use std::time::{Duration, Instant};

/// A second stop request after this long (escalates to SIGKILL).
const SWITCH_FORCE_AFTER: Duration = Duration::from_secs(8);
/// Give up waiting for the running game to close.
const SWITCH_GIVE_UP: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
pub struct SwitchState {
    /// The action of the game in `AppState::switch_prompt` (None = Play).
    prompt_action: Option<usize>,
    pending: Option<Pending>,
    /// Play ▾ → Play in a window waiting here: the deferred Play starts windowed too.
    pub(crate) windowed: Option<GameId>,
    /// Play ▾ → Play with ▸ <emulator> (game, emulator, profile): that Play uses it (emu_app.rs).
    pub(crate) play_with: Option<(GameId, String, String)>,
    /// "Choose what to play" → the line picked (emu_app.rs): that Play starts it.
    pub(crate) play_choice: Option<(GameId, crate::model::PlayChoice)>,
}

#[derive(Debug)]
struct Pending {
    next: GameId,
    action: Option<usize>,
    /// Games asked to close that haven't exited yet.
    closing: Vec<GameId>,
    since: Instant,
    forced: bool,
}

/// The store client's name for status lines.
fn client_name(src: Source) -> &'static str {
    match src {
        Source::Rockstar => "Rockstar Games Launcher",
        Source::Riot => "Riot Client",
        s => s.display(),
    }
}

impl AppState {
    /// Before a launch of `id`: true = go ahead. Otherwise another game Lodestar launched runs and
    /// the setting says to ask first or to close it first (the launch then happens later).
    pub(crate) fn one_game_gate(&mut self, id: GameId, action: Option<usize>) -> bool {
        let others = self.other_games(id);
        if others.is_empty() {
            return true;
        }
        match self.config.launchers.one_game_at_a_time {
            OneGameAtATime::AllowSeveral => true,
            OneGameAtATime::CloseRunning => {
                self.begin_switch(others, id, action);
                false
            }
            OneGameAtATime::Ask => {
                let running = *others.last().unwrap_or(&id);
                self.switch_prompt = Some((running, id));
                self.switch.prompt_action = action;
                self.ctx().request_repaint();
                false
            }
        }
    }

    fn other_games(&self, id: GameId) -> Vec<GameId> {
        self.launcher.active_games().iter().copied().filter(|g| *g != id).collect()
    }

    pub(crate) fn game_name(&self, id: GameId) -> String {
        self.library.get(id).map(|g| g.display_name().to_string()).unwrap_or_else(|| "the game".into())
    }

    /// "Close & start" in the prompt: close the running game, then start the new one.
    pub fn confirm_switch(&mut self) {
        let Some((_, next)) = self.switch_prompt.take() else { return };
        let action = self.switch.prompt_action.take();
        let others = self.other_games(next);
        if others.is_empty() {
            self.start_game(next, action);
        } else {
            self.begin_switch(others, next, action);
        }
    }

    /// "Cancel" in the prompt (also drops a launch still waiting for a game to close).
    pub fn cancel_switch(&mut self) {
        self.switch_prompt = None;
        self.switch.prompt_action = None;
        self.switch.pending = None;
    }

    fn begin_switch(&mut self, closing: Vec<GameId>, next: GameId, action: Option<usize>) {
        for g in &closing {
            self.launcher.stop_game(*g);
        }
        let names: Vec<String> = closing.iter().map(|g| self.game_name(*g)).collect();
        self.notify(format!("Closing {} to start {}", names.join(", "), self.game_name(next)));
        self.switch_prompt = None;
        self.switch.prompt_action = None;
        self.switch.pending = Some(Pending { next, action, closing, since: Instant::now(), forced: false });
        self.ctx().request_repaint_after(Duration::from_millis(500));
    }

    /// Per frame, after the launcher's events: start the waiting game once the running one is
    /// gone; drop a prompt whose running game ended meanwhile.
    pub(crate) fn advance_switch(&mut self) {
        if let Some((running, next)) = self.switch_prompt
            && !self.launcher.active_games().contains(&running)
        {
            match self.other_games(next).last() {
                Some(&other) => self.switch_prompt = Some((other, next)),
                None => self.cancel_switch(),
            }
        }
        let Some(p) = self.switch.pending.as_mut() else { return };
        let active = self.launcher.active_games();
        p.closing.retain(|g| active.contains(g));
        if p.closing.is_empty() {
            let Some(p) = self.switch.pending.take() else { return };
            self.start_game(p.next, p.action);
            return;
        }
        let waited = p.since.elapsed();
        if waited >= SWITCH_GIVE_UP {
            let Some(p) = self.switch.pending.take() else { return };
            let names: Vec<String> = p.closing.iter().map(|g| self.game_name(*g)).collect();
            self.notify_error(format!("{} didn't close; {} wasn't started", names.join(", "), self.game_name(p.next)));
            return;
        }
        if !p.forced && waited >= SWITCH_FORCE_AFTER {
            p.forced = true;
            for g in p.closing.clone() {
                log::info!("game {g} is still closing; forcing it");
                self.launcher.stop_game(g);
            }
        }
        self.ctx().request_repaint_after(Duration::from_millis(500));
    }

    /// The game waiting for another one to close (CloseRunning / confirmed prompt).
    pub fn switch_pending(&self) -> Option<GameId> {
        self.switch.pending.as_ref().map(|p| p.next)
    }

    /// What a launch of `id` on its way waits for, one line for the UIs; None when `id` isn't
    /// being started (not launched, or already seen running).
    pub fn launch_status(&self, id: GameId) -> Option<String> {
        if let Some(p) = &self.switch.pending
            && p.next == id
        {
            let names: Vec<String> = p.closing.iter().map(|g| self.game_name(*g)).collect();
            return Some(format!("Closing {} to start {}…", names.join(", "), self.game_name(id)));
        }
        if !self.launcher.active_games().contains(&id) || self.launcher.is_started(id) {
            return None;
        }
        let name = self.game_name(id);
        let client = match self.launcher.launched_via(id) {
            Some(crate::launch::Via::StoreClient) => self.library.get(id).map(|g| client_name(g.source)),
            Some(crate::launch::Via::Heroic) => Some("Heroic"),
            Some(crate::launch::Via::Steam) => return Some(format!("Starting {name} via Steam…")),
            _ => None,
        };
        Some(match client {
            Some(c) => format!("Starting {name} via {c}… sign in there if asked"),
            None => format!("Starting {name}…"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Art, LaunchSpec, ScannedGame};

    fn sleeper(st: &mut AppState, name: &str, secs: &str) -> GameId {
        st.add_manual_game(
            name,
            LaunchSpec::Command { program: "sleep".into(), args: vec![secs.into()], cwd: None, env: vec![] },
            Art::default(),
        );
        st.library.games.iter().find(|g| g.name == name).unwrap().id
    }

    fn wait(st: &mut AppState, what: &str, done: impl FnMut(&AppState) -> bool) {
        assert!(st.poll_until(Duration::from_secs(15), done), "timed out: {what}");
    }

    fn stop_all(st: &mut AppState) {
        for g in st.launcher.active_games().to_vec() {
            st.launcher.stop_game(g);
        }
        wait(st, "everything stopped", |s| s.launcher.active_games().is_empty());
    }

    #[test]
    fn close_running_stops_the_game_then_starts_the_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        st.config.launchers.one_game_at_a_time = OneGameAtATime::CloseRunning;
        let a = sleeper(&mut st, "A", "30");
        let b = sleeper(&mut st, "B", "30");
        st.launch(a);
        wait(&mut st, "A runs", |s| s.launcher.is_started(a));
        st.launch(b);
        assert_eq!(st.switch_pending(), Some(b));
        assert_eq!(st.launch_status(b).as_deref(), Some("Closing A to start B…"));
        assert!(st.switch_prompt.is_none());
        wait(&mut st, "B started after A closed", |s| s.launcher.active_games() == [b]);
        assert_eq!(st.switch_pending(), None);
        assert_eq!(st.library.get(b).unwrap().play_count, 1);
        stop_all(&mut st);
    }

    #[test]
    fn ask_prompts_and_cancel_or_confirm() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        assert_eq!(st.config.launchers.one_game_at_a_time, OneGameAtATime::Ask);
        let a = sleeper(&mut st, "A", "30");
        let b = sleeper(&mut st, "B", "30");
        st.launch(a);
        wait(&mut st, "A runs", |s| s.launcher.is_started(a));
        st.launch(b);
        assert_eq!(st.switch_prompt, Some((a, b)));
        assert_eq!(st.launcher.active_games(), [a], "nothing happens before the answer");
        st.cancel_switch();
        assert!(st.switch_prompt.is_none());
        st.poll_until(Duration::from_millis(300), |_| false);
        assert_eq!(st.launcher.active_games(), [a]);
        assert_eq!(st.library.get(b).unwrap().play_count, 0);
        // Again, and confirm.
        st.launch(b);
        assert_eq!(st.switch_prompt, Some((a, b)));
        st.confirm_switch();
        assert!(st.switch_prompt.is_none());
        wait(&mut st, "B replaced A", |s| s.launcher.active_games() == [b]);
        // Playing the running game again is no switch.
        st.launch(b);
        assert!(st.switch_prompt.is_none());
        // A prompt whose running game ended by itself goes away.
        let c = sleeper(&mut st, "C", "30");
        st.launch(c);
        assert_eq!(st.switch_prompt, Some((b, c)));
        stop_all(&mut st);
        assert!(st.switch_prompt.is_none());
    }

    #[test]
    fn allow_several_runs_both() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        st.config.launchers.one_game_at_a_time = OneGameAtATime::AllowSeveral;
        let a = sleeper(&mut st, "A", "30");
        let b = sleeper(&mut st, "B", "30");
        st.launch(a);
        st.launch(b);
        assert_eq!(st.launcher.active_games(), [a, b]);
        assert!(st.switch_prompt.is_none() && st.switch_pending().is_none());
        // Switching to "close it" with two running closes both.
        st.config.launchers.one_game_at_a_time = OneGameAtATime::CloseRunning;
        let c = sleeper(&mut st, "C", "30");
        st.launch(c);
        assert_eq!(st.launch_status(c).as_deref(), Some("Closing A, B to start C…"));
        wait(&mut st, "C alone", |s| s.launcher.active_games() == [c]);
        stop_all(&mut st);
    }

    #[test]
    fn screenshot_runs_never_switch_or_launch() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        st.config.launchers.one_game_at_a_time = OneGameAtATime::CloseRunning;
        let a = sleeper(&mut st, "A", "30");
        st.no_launch = true;
        st.launch(a);
        assert!(st.launcher.active_games().is_empty());
        assert!(st.switch_pending().is_none());
    }

    #[test]
    fn status_line_names_the_store_client() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let mut db = crate::db::Db::open(&st.paths.db_file()).unwrap();
        let game_dir = dir.path().join("prefix/drive_c/Games/Overwatch");
        std::fs::create_dir_all(&game_dir).unwrap();
        let mut g = ScannedGame::new(Source::BattleNet, "Pro", "Overwatch");
        // A harmless stand-in for the client: never shows the game in its folder.
        g.launch =
            Some(LaunchSpec::Command { program: "sleep".into(), args: vec!["3".into()], cwd: None, env: vec![] });
        g.install_dir = Some(game_dir);
        g.installed = true;
        db.merge_scan(&[g], &[Source::BattleNet]).unwrap();
        st.reload_library();
        let id = st.library.games.iter().find(|g| g.name == "Overwatch").unwrap().id;
        st.launch(id);
        assert_eq!(st.launch_status(id).as_deref(), Some("Starting Overwatch via Battle.net… sign in there if asked"));
        assert_eq!(client_name(Source::Riot), "Riot Client");
        let m = sleeper(&mut st, "Plain", "30");
        assert_eq!(st.launch_status(m), None, "not launched");
        stop_all(&mut st);
        assert_eq!(st.launch_status(id), None);
    }
}
