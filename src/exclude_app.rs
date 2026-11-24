//! The app side of "Not a game" cleanup: which games are excluded from the
//! library, and the actions that change it. Everything applies at once, in memory (no rescan,
//! no DB reload): excluded games keep their rows and wait in `ExclState::games`.

use crate::app::AppState;
use crate::config::ExcludeRule;
use crate::exclude::{self, Exclusion, Key, Mark, Matcher, Reason};
use crate::game_options_bulk::GameOptionsPatch;
use crate::model::{Game, GameId, Source};

/// What Undo of "Not a game" restores: the keys marked, and the marks they had before.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarkUndo {
    pub keys: Vec<Key>,
    pub prev: Vec<Exclusion>,
}

impl MarkUndo {
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// Games out of the library, and what put them there.
#[derive(Default)]
pub struct ExclState {
    /// Excluded games (marked "Not a game" or caught by a rule), in load order.
    pub games: Vec<Game>,
    /// The `exclusions` table as last read.
    pub marks: Vec<Exclusion>,
    matcher: Matcher,
    /// Bumped on every change (the settings page caches its list on it).
    pub generation: u64,
}

impl ExclState {
    pub fn reason(&self, g: &Game) -> Option<Reason> {
        self.matcher.reason(g)
    }
}

impl AppState {
    /// Put freshly loaded games into the library; excluded ones go to `excl.games`.
    pub(crate) fn set_games(&mut self, games: Vec<Game>) {
        self.refresh_matcher();
        let (shown, out) = exclude::split(games, &self.excl.matcher);
        self.library.replace(shown);
        // The user's collections as stored (a delete / merge may have changed them).
        self.library.user = crate::library::user::load(&self.db);
        self.excl.games = out;
        self.excl.generation += 1;
    }

    /// Re-read the marks and recompile the rules (cheap: a few rows, a few patterns).
    fn refresh_matcher(&mut self) {
        match self.db.exclusions() {
            Ok(m) => self.excl.marks = m,
            Err(e) => log::warn!("reading exclusions: {e:#}"),
        }
        self.excl.matcher = Matcher::new(&self.config.exclude, &self.excl.marks);
    }

    /// Rules or marks changed: re-split the games in memory (no DB reload).
    pub fn reapply_exclusions(&mut self) {
        let mut all = std::mem::take(&mut self.library.games);
        all.append(&mut self.excl.games);
        // Back in the order the DB gives (sort key), as a reload would.
        all.sort_by(|a, b| a.sort_name.cmp(&b.sort_name));
        self.set_games(all);
    }

    /// "Not a game": out of every view, and a rescan never brings these back. Returns what
    /// Undo needs (the keys marked and the marks they had before).
    pub fn exclude_games(&mut self, ids: &[GameId]) -> MarkUndo {
        let items: Vec<(Source, String, String)> = ids
            .iter()
            .filter_map(|&id| self.library.get(id).or_else(|| self.excl.games.iter().find(|g| g.id == id)))
            .map(|g| (g.source, g.source_id.clone(), g.display_name().to_string()))
            .collect();
        if items.is_empty() {
            return MarkUndo::default();
        }
        let keys: Vec<Key> = items.iter().map(|(s, id, _)| (*s, id.clone())).collect();
        let prev: Vec<Exclusion> = self
            .excl
            .marks
            .iter()
            .filter(|m| keys.iter().any(|k| k.0 == m.source && k.1 == m.source_id))
            .cloned()
            .collect();
        if let Err(e) = self.db.set_exclusions(&items, Mark::NotGame) {
            self.notify_error(format!("Couldn't save: {e:#}"));
            return MarkUndo::default();
        }
        self.reapply_exclusions();
        MarkUndo { keys, prev }
    }

    /// Undo of [`Self::exclude_games`]: the marks go back to what they were (a game brought
    /// back from a rule earlier stays brought back).
    pub fn undo_marks(&mut self, u: &MarkUndo) {
        if u.keys.is_empty() {
            return;
        }
        let mut res = self.db.clear_exclusions(&u.keys);
        for mark in [Mark::NotGame, Mark::Keep] {
            let items: Vec<(Source, String, String)> = u
                .prev
                .iter()
                .filter(|m| m.mark == mark)
                .map(|m| (m.source, m.source_id.clone(), m.name.clone()))
                .collect();
            if res.is_ok() && !items.is_empty() {
                res = self.db.set_exclusions(&items, mark);
            }
        }
        if let Err(e) = res {
            self.notify_error(format!("Couldn't save: {e:#}"));
        }
        self.reapply_exclusions();
    }

    /// Forget marks ("Show again" of a marked entry whose game is gone).
    #[cfg(test)]
    pub fn clear_marks(&mut self, keys: &[Key]) {
        if keys.is_empty() {
            return;
        }
        if let Err(e) = self.db.clear_exclusions(keys) {
            self.notify_error(format!("Couldn't save: {e:#}"));
            return;
        }
        self.reapply_exclusions();
    }

    /// "Show again" for excluded games: a mark is cleared; a game a rule caught is kept from now
    /// on (the rule stays for everything else). Several at once.
    pub fn show_again(&mut self, keys: &[Key]) {
        let mut unmark = vec![];
        let mut keep = vec![];
        let rules_only = Matcher::new(&self.config.exclude, &[]);
        for k in keys {
            let Some(g) = self.excl.games.iter().find(|g| g.source == k.0 && g.source_id == k.1) else {
                // A mark without a game row (not seen since): just forget it.
                unmark.push(k.clone());
                continue;
            };
            let marked =
                self.excl.marks.iter().any(|m| m.source == k.0 && m.source_id == k.1 && m.mark == Mark::NotGame);
            if marked {
                unmark.push(k.clone());
            }
            // Would a rule still catch it once unmarked?
            if rules_only.reason(g).is_some() {
                keep.push((g.source, g.source_id.clone(), g.display_name().to_string()));
            }
        }
        let res = self.db.clear_exclusions(&unmark).and_then(|()| self.db.set_exclusions(&keep, Mark::Keep));
        if let Err(e) = res {
            self.notify_error(format!("Couldn't save: {e:#}"));
        }
        self.reapply_exclusions();
    }

    /// Hide / unhide several games (one transaction).
    pub fn set_hidden_many(&mut self, ids: &[GameId], hidden: bool) {
        match self.db.set_hidden_many(ids, hidden) {
            Ok(()) => {
                for &id in ids {
                    self.library.update(id, |g| g.hidden = hidden);
                }
            }
            Err(e) => self.notify_error(format!("Couldn't save: {e:#}")),
        }
    }

    /// A user rule; applies at once. Returns how many games it took out of the library.
    pub fn add_exclude_rule(&mut self, rule: ExcludeRule) -> usize {
        if !exclude::pattern_usable(&rule.pattern) {
            return 0;
        }
        let before = self.library.games.len();
        if !self.config.exclude.rules.contains(&rule) {
            self.config.exclude.rules.push(rule);
        }
        self.exclude_config_changed();
        before.saturating_sub(self.library.games.len())
    }

    /// Remove the user rule equal to `rule` (Undo of "…and everything named like this").
    pub fn remove_exclude_rule(&mut self, rule: &ExcludeRule) {
        if let Some(i) = self.config.exclude.rules.iter().rposition(|r| r == rule) {
            self.config.exclude.rules.remove(i);
            self.exclude_config_changed();
        }
    }

    /// `config.exclude` was edited (rule toggled, removed, built-in switched): save and apply.
    pub fn exclude_config_changed(&mut self) {
        self.save_config();
        self.reapply_exclusions();
    }

    /// Bulk game options: write `patch` (only the options changed in the dialog) into each
    /// game's own options; every other option keeps the game's value. Returns how many games
    /// changed.
    pub fn apply_game_options_patch(&mut self, ids: &[GameId], patch: &GameOptionsPatch) -> usize {
        let edits: Vec<(GameId, crate::model::UserEdits)> = ids
            .iter()
            .filter_map(|&id| self.library.get(id))
            .filter_map(|g| {
                let opts = patch.apply(&g.edits.game_options);
                (opts != g.edits.game_options).then(|| {
                    let mut e = g.edits.clone();
                    e.game_options = opts;
                    (g.id, e)
                })
            })
            .collect();
        if edits.is_empty() {
            return 0;
        }
        if let Err(e) = self.db.set_edits_many(&edits) {
            self.notify_error(format!("Couldn't save the game options: {e:#}"));
            return 0;
        }
        let n = edits.len();
        for (id, e) in edits {
            self.library.update(id, |g| g.edits.game_options = e.game_options);
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use crate::app::AppState;
    use crate::config::ExcludeRule;
    use crate::db::Db;
    use crate::model::*;

    fn st_with(games: &[(Source, &str, &str)]) -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let scanned: Vec<ScannedGame> = games.iter().map(|(s, id, n)| ScannedGame::new(*s, *id, *n)).collect();
        let mut sources: Vec<Source> = games.iter().map(|g| g.0).collect();
        sources.dedup();
        Db::open(&st.paths.db_file()).unwrap().merge_scan(&scanned, &sources).unwrap();
        st.reload_library();
        (dir, st)
    }

    fn names(st: &AppState) -> Vec<&str> {
        st.library.games.iter().map(|g| g.name.as_str()).collect()
    }

    fn id(st: &AppState, name: &str) -> GameId {
        st.library.games.iter().chain(&st.excl.games).find(|g| g.name == name).unwrap().id
    }

    #[test]
    fn builtin_rules_apply_on_load_and_can_be_switched_off() {
        let (_d, mut st) = st_with(&[
            (Source::Lutris, "1", "Alpha"),
            (Source::Lutris, "2", "Proton 9.0"),
            (Source::Desktop, "u", "Uninstall Foo"),
        ]);
        assert_eq!(names(&st), ["Alpha"]);
        assert_eq!(st.excl.games.len(), 2);
        st.config.exclude.builtin_off.push("proton".into());
        st.exclude_config_changed();
        assert_eq!(names(&st), ["Alpha", "Proton 9.0"]);
    }

    #[test]
    fn not_a_game_is_instant_survives_rescans_and_undoes() {
        let (_d, mut st) = st_with(&[(Source::Desktop, "a.desktop", "Real"), (Source::Desktop, "t.desktop", "Tool")]);
        let tool = id(&st, "Tool");
        let undo = st.exclude_games(&[tool]);
        assert_eq!(names(&st), ["Real"]);
        // A rescan (the entry renamed upstream) doesn't bring it back.
        Db::open(&st.paths.db_file())
            .unwrap()
            .merge_scan(
                &[
                    ScannedGame::new(Source::Desktop, "a.desktop", "Real"),
                    ScannedGame::new(Source::Desktop, "t.desktop", "Tool v2"),
                ],
                &[Source::Desktop],
            )
            .unwrap();
        st.reload_library();
        assert_eq!(names(&st), ["Real"]);
        assert_eq!(st.excl.games[0].name, "Tool v2");
        st.undo_marks(&undo);
        assert_eq!(names(&st), ["Real", "Tool v2"]);
    }

    #[test]
    fn show_again_keeps_a_rule_caught_game_and_unmarks_a_marked_one() {
        let (_d, mut st) = st_with(&[
            (Source::Lutris, "1", "Setup"),
            (Source::Lutris, "2", "Other Setup"),
            (Source::Lutris, "3", "Real"),
        ]);
        assert_eq!(names(&st), ["Real"]);
        let k = (Source::Lutris, "1".to_string());
        st.show_again(std::slice::from_ref(&k));
        assert_eq!(names(&st), ["Real", "Setup"], "kept; the rule still catches the other one");
        // Marked as not a game afterwards, then shown again: the keep comes back with it.
        let setup = id(&st, "Setup");
        st.exclude_games(&[setup]);
        assert_eq!(names(&st), ["Real"]);
        st.show_again(std::slice::from_ref(&k));
        assert_eq!(names(&st), ["Real", "Setup"]);
        // Undo of "Not a game" on a game brought back from a rule: it stays brought back.
        let undo = st.exclude_games(&[setup]);
        assert_eq!(names(&st), ["Real"]);
        st.undo_marks(&undo);
        assert_eq!(names(&st), ["Real", "Setup"], "the keep is restored, not just the mark cleared");
        st.clear_marks(&[k]);
        assert_eq!(names(&st), ["Real"], "without the keep the built-in rule catches it again");
    }

    #[test]
    fn user_rules_scope_and_undo() {
        let (_d, mut st) = st_with(&[
            (Source::Desktop, "a", "Foo Tool"),
            (Source::Lutris, "b", "Foo Game"),
            (Source::Lutris, "c", "Bar"),
        ]);
        let r = ExcludeRule { pattern: "foo *".into(), source: Some("desktop".into()), enabled: true };
        assert_eq!(st.add_exclude_rule(r.clone()), 1);
        assert_eq!(names(&st), ["Bar", "Foo Game"]);
        st.remove_exclude_rule(&r);
        assert_eq!(names(&st), ["Bar", "Foo Game", "Foo Tool"]);
        assert_eq!(st.add_exclude_rule(ExcludeRule { pattern: " * ".into(), ..Default::default() }), 0);
        assert!(st.config.exclude.rules.is_empty(), "an unusable pattern is not saved");
    }

    #[test]
    fn hidden_many_and_bulk_options() {
        use crate::game_options::Tri;
        let (_d, mut st) = st_with(&[(Source::Steam, "1", "A"), (Source::Steam, "2", "B")]);
        let ids = [id(&st, "A"), id(&st, "B")];
        st.set_hidden_many(&ids, true);
        assert!(st.library.games.iter().all(|g| g.hidden));
        st.reload_library();
        assert!(st.library.games.iter().all(|g| g.hidden), "saved");
        // B has its own Gamescope setting; the patch only sets MangoHud.
        let mut e = st.library.get(ids[1]).unwrap().edits.clone();
        e.game_options.gamescope = Tri::Off;
        st.set_edits(ids[1], e);
        let edited = crate::game_options::GameOptions { mangohud: Tri::On, ..Default::default() };
        let patch = crate::game_options_bulk::GameOptionsPatch::between(&Default::default(), &edited);
        assert_eq!(st.apply_game_options_patch(&ids, &patch), 2);
        st.reload_library();
        let b = &st.library.get(ids[1]).unwrap().edits.game_options;
        assert_eq!((b.mangohud, b.gamescope), (Tri::On, Tri::Off));
        assert_eq!(st.library.get(ids[0]).unwrap().edits.game_options.mangohud, Tri::On);
        assert_eq!(st.apply_game_options_patch(&ids, &patch), 0, "nothing left to change");
    }
}
