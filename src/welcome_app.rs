//! The first start's welcome step: before the first artwork lookups, it recommends adding a free
//! SteamGridDB API key, the source that has art for emulated games, shortcuts and launchers
//! without art of their own (Steam and GOG games bring theirs).
//!
//! Shown once, on a fresh install only: `Config::load` sets `welcome_pending` when there is no
//! config file, and an existing library whose config.toml went missing doesn't count. While the
//! step is open the scan runs, but the online artwork and metadata lookups wait for it
//! (`lookups_wait_for_welcome`), so a key added in the step is used by the very first lookups.
//! Its answer starts them for every game never looked up; games already looked up get the key's
//! art through the automatic fill, which asks each source once per game (art_app.rs). The
//! desktop (ui/desktop/welcome_ui.rs) and controller mode (ui/controller/welcome.rs) draw the step.

use crate::app::{AppState, Mode};
use crate::art::KeyCheck;

/// Where SteamGridDB hands out API keys (after signing in).
pub const SGDB_KEY_PAGE: &str = "https://www.steamgriddb.com/profile/preferences/api";

/// Where the key can be added later.
pub const KEY_LATER: &str = "Settings → Metadata → SteamGridDB";

/// Where the key can be added later, seen from `mode` (controller mode has no such page).
pub fn key_later(mode: Mode) -> String {
    match mode {
        Mode::Desktop => KEY_LATER.into(),
        Mode::Controller => format!("desktop mode: {KEY_LATER}"),
    }
}

/// A key check's answer in plain words (the key fields of the step and of Settings).
pub fn key_check_text(c: &KeyCheck) -> String {
    match c {
        KeyCheck::Checking => "Checking the key…".into(),
        KeyCheck::Works => "The key works".into(),
        KeyCheck::Rejected => "SteamGridDB didn't accept this key. Copy it again from the API page.".into(),
        KeyCheck::Unreachable(_) => {
            "Couldn't check it right now; Lodestar will use it once SteamGridDB can be reached.".into()
        }
    }
}

#[derive(Default)]
pub struct WelcomeState {
    /// The step is showing.
    pub open: bool,
    /// The key field's text (both modes edit the same one).
    pub key: String,
}

impl AppState {
    /// At start-up, after the library was loaded: show the step when it is still pending.
    /// `had_games`: the library had games before this start's scan.
    pub(crate) fn init_welcome(&mut self, had_games: bool) {
        if !self.config.welcome_pending {
            return;
        }
        let file = self.paths.config_file();
        let saved = file.exists();
        if !saved && had_games {
            // config.toml is gone but the library isn't: not a new install.
            self.config.welcome_pending = false;
            return;
        }
        self.welcome.open = true;
        if !saved {
            // Written now, so closing Lodestar before answering shows the step again next time.
            if let Err(e) = self.config.save(&file) {
                log::warn!("couldn't save the settings: {e:#}");
            }
        }
    }

    /// True while the step is open: online lookups wait for its answer (`finish_welcome`).
    pub(crate) fn lookups_wait_for_welcome(&self) -> bool {
        self.welcome.open
    }

    /// Answer the step: Continue with the key in the field (`use_key`; an empty field adds
    /// nothing) or Skip for now. Either way it is never shown again.
    pub fn finish_welcome(&mut self, use_key: bool) {
        let key = self.welcome.key.trim().to_string();
        let added = use_key && !key.is_empty();
        if added {
            self.config.metadata.steamgriddb_key = Some(key);
        }
        self.config.welcome_pending = false;
        self.welcome.open = false;
        self.welcome.key.clear();
        self.save_config();
        // The lookups that waited, and those of games found in an earlier session that closed
        // before answering (only games never looked up are asked). A running scan starts them
        // when it ends.
        if !self.scan.running {
            self.queue_metadata();
            self.auto_art_after_scan();
        }
        if added {
            self.notify("SteamGridDB key saved: Lodestar is looking up artwork for your games");
        } else {
            self.notify_long(format!("You can add a SteamGridDB key any time in {}", key_later(self.mode)));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::AppState;
    use crate::config::Config;

    /// A start of Lodestar on `root`: the config as `main` loads it, the library from the DB.
    /// Lookups the answer queues get "not found" (nothing goes online).
    fn start(root: &std::path::Path) -> AppState {
        let mut st = AppState::for_test(root);
        st.config = Config::load(&st.paths.config_file());
        st.metadata = crate::metadata::MetadataFetcher::offline(st.ctx(), st.paths.art_dir(), &st.config.metadata);
        let had_games = !st.library.games.is_empty();
        st.init_welcome(had_games);
        st
    }

    fn add_game(st: &mut AppState) {
        let launch = crate::model::LaunchSpec::Url("x://".into());
        st.add_manual_game("Celeste", launch, Default::default());
    }

    #[test]
    fn shown_on_an_empty_home_until_answered() {
        let tmp = tempfile::tempdir().unwrap();
        let st = start(tmp.path());
        assert!(st.welcome.open);
        assert!(st.paths.config_file().exists(), "pending flag kept for the next start");
        drop(st);
        // Closed without an answer (the first scan found games meanwhile): shown again.
        let mut st = start(tmp.path());
        add_game(&mut st);
        drop(st);
        let mut st = start(tmp.path());
        assert!(st.welcome.open);
        st.finish_welcome(false);
        drop(st);
        assert!(!start(tmp.path()).welcome.open, "never twice");
    }

    #[test]
    fn existing_config_or_library_never_sees_it() {
        let tmp = tempfile::tempdir().unwrap();
        let st = AppState::for_test(tmp.path());
        std::fs::create_dir_all(&st.paths.config_dir).unwrap();
        std::fs::write(st.paths.config_file(), "start_mode = \"desktop\"\n").unwrap();
        drop(st);
        assert!(!start(tmp.path()).welcome.open, "a config from before");

        let tmp = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(tmp.path());
        add_game(&mut st);
        drop(st);
        let st = start(tmp.path());
        assert!(!st.welcome.open, "a library whose config.toml went missing");
        assert!(!st.config.welcome_pending);
    }

    #[test]
    fn skip_persists_and_says_where_to_add_the_key() {
        let tmp = tempfile::tempdir().unwrap();
        let mut st = start(tmp.path());
        st.welcome.key = "typed but skipped".into();
        st.finish_welcome(false);
        assert!(!st.welcome.open);
        assert_eq!(st.config.metadata.steamgriddb_key, None);
        assert!(st.toasts.iter().any(|t| t.text.contains(super::KEY_LATER)));
        let saved = Config::load(&st.paths.config_file());
        assert!(!saved.welcome_pending && saved.metadata.steamgriddb_key.is_none());
    }

    #[test]
    fn controller_mode_says_the_key_goes_in_desktop_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let mut st = start(tmp.path());
        st.mode = crate::app::Mode::Controller;
        st.finish_welcome(false);
        let toast = &st.toasts.last().unwrap().text;
        assert!(toast.contains("in desktop mode: Settings → Metadata → SteamGridDB"), "{toast}");
    }

    /// The first session closed before the step was answered, and this start doesn't scan:
    /// the games found the first time still get their lookups once it is answered.
    #[test]
    fn answering_looks_up_games_found_in_an_unanswered_session() {
        let tmp = tempfile::tempdir().unwrap();
        let mut st = start(tmp.path());
        add_game(&mut st);
        drop(st);
        let mut st = start(tmp.path());
        assert!(st.welcome.open && !st.scan.running);
        st.auto_art.allowed = true;
        st.finish_welcome(false);
        assert!(st.auto_art.wanted, "the answer starts the artwork lookups");
    }

    #[test]
    fn continue_saves_the_key() {
        let tmp = tempfile::tempdir().unwrap();
        let mut st = start(tmp.path());
        st.welcome.key = "  0123abcd  ".into();
        st.finish_welcome(true);
        assert_eq!(st.config.metadata.steamgriddb_key.as_deref(), Some("0123abcd"));
        let saved = Config::load(&st.paths.config_file());
        assert_eq!(saved.metadata.steamgriddb_key.as_deref(), Some("0123abcd"));
        assert!(!saved.welcome_pending);
    }
}
