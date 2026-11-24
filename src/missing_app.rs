//! The app side of missing games (a game no longer in the library it came from, e.g. a
//! non-Steam shortcut deleted in Steam; see `Db::merge_scan`): removing them for good with
//! Undo, and keeping them.

use crate::app::AppState;
use crate::db::TakenRows;
use crate::model::{GameId, Source};

/// What Undo of "Remove from library…" puts back: the rows, and their places in the user's
/// collections (collection id, position, game).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RemovedGames {
    pub rows: TakenRows,
    pub places: Vec<(u64, usize, GameId)>,
}

impl RemovedGames {
    pub fn len(&self) -> usize {
        self.rows.len()
    }
}

/// "Steam" when every game is from one library, else None.
pub fn one_source(st: &AppState, ids: &[GameId]) -> Option<Source> {
    let mut it = ids.iter().filter_map(|id| st.library.get(*id)).map(|g| g.source);
    let first = it.next()?;
    it.all(|s| s == first).then_some(first)
}

impl AppState {
    /// Remove missing games from the library for good (only games that are missing; others
    /// in `ids` are left alone). None when nothing was removed.
    pub fn remove_missing(&mut self, ids: &[GameId]) -> Option<RemovedGames> {
        let ids: Vec<GameId> =
            ids.iter().copied().filter(|id| self.library.get(*id).is_some_and(|g| g.missing.is_some())).collect();
        if ids.is_empty() {
            return None;
        }
        // Out of the database first: when that fails, the collections stay as they were.
        let rows = match self.db.take_games(&ids) {
            Ok(rows) => rows,
            Err(e) => {
                self.notify_error(format!("Couldn't remove: {e:#}"));
                return None;
            }
        };
        // Then out of the manual collections (SQLite can reuse a deleted game's id).
        let mut colls = crate::library::user::load(&self.db);
        let mut places = vec![];
        for c in colls.iter_mut() {
            for (i, g) in c.games.iter().enumerate() {
                if ids.contains(g) {
                    places.push((c.id, i, *g));
                }
            }
            c.games.retain(|g| !ids.contains(g));
        }
        if !places.is_empty() {
            crate::library::user::save(&self.db, &colls);
        }
        let out = Some(RemovedGames { rows, places });
        self.reload_library();
        out
    }

    /// Undo `remove_missing`.
    pub fn restore_removed(&mut self, r: &RemovedGames) {
        match self.db.restore_games(&r.rows) {
            Ok(n) if n < r.len() => self.notify(format!(
                "{} of {} games came back (the others are in the library again already)",
                n,
                r.len()
            )),
            Ok(_) => {}
            Err(e) => self.notify_error(format!("Couldn't put them back: {e:#}")),
        }
        if !r.places.is_empty() {
            let mut colls = crate::library::user::load(&self.db);
            // In their old order, so earlier positions are taken first.
            for &(cid, at, g) in &r.places {
                if let Some(c) = colls.iter_mut().find(|c| c.id == cid && !c.is_smart())
                    && !c.games.contains(&g)
                {
                    c.games.insert(at.min(c.games.len()), g);
                }
            }
            crate::library::user::save(&self.db, &colls);
        }
        self.reload_library();
    }

    /// Keep missing games in the library (no longer listed in Missing games), or list them again.
    pub fn keep_missing(&mut self, ids: &[GameId], kept: bool) {
        match self.db.set_missing_kept(ids, kept) {
            Ok(()) => {
                for &id in ids {
                    self.library.update(id, |g| {
                        if let Some(m) = &mut g.missing {
                            m.kept = kept;
                        }
                    });
                }
            }
            Err(e) => self.notify_error(format!("Couldn't save: {e:#}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::model::ScannedGame;

    #[test]
    fn remove_and_undo_keep_collections_and_data() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let scan = [
            ScannedGame::new(Source::Steam, "shortcut:1", "Gone"),
            ScannedGame::new(Source::Steam, "shortcut:2", "Stays"),
        ];
        let mut db = Db::open(&st.paths.db_file()).unwrap();
        db.merge_scan(&scan, &[Source::Steam]).unwrap();
        st.reload_library();
        let gone = st.library.games.iter().find(|g| g.name == "Gone").unwrap().id;
        let stays = st.library.games.iter().find(|g| g.name == "Stays").unwrap().id;
        let colls =
            vec![crate::library::UserCollection { id: 7, name: "Couch".into(), games: vec![stays, gone], rule: None }];
        crate::library::user::save(&st.db, &colls);
        st.db.set_favorite(gone, true).unwrap();
        db.merge_scan(&scan[1..], &[Source::Steam]).unwrap();
        st.reload_library();
        assert_eq!(st.library.missing().map(|g| g.id).collect::<Vec<_>>(), [gone]);
        assert_eq!(one_source(&st, &[gone, stays]), Some(Source::Steam));

        // Not missing: left alone.
        assert!(st.remove_missing(&[stays]).is_none());
        let r = st.remove_missing(&[gone, stays]).unwrap();
        assert_eq!(r.len(), 1);
        assert!(st.library.get(gone).is_none());
        assert_eq!(st.library.user[0].games, [stays]);

        st.restore_removed(&r);
        let g = st.library.get(gone).unwrap();
        assert!(g.favorite && g.missing.is_some());
        assert_eq!(st.library.user[0].games, [stays, gone]);

        // Keep: out of the list, still in the library; a scan doesn't change that.
        st.keep_missing(&[gone], true);
        assert_eq!(st.library.missing().count(), 0);
        db.merge_scan(&scan[1..], &[Source::Steam]).unwrap();
        st.reload_library();
        assert!(st.library.get(gone).unwrap().missing.unwrap().kept);
        st.keep_missing(&[gone], false);
        assert_eq!(st.library.missing().count(), 1);
    }
}
