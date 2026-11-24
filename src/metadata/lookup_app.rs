//! The app side of lookups the user asks for (Edit Game Details → Download metadata…).
//!
//! The dialog starts a lookup with the name and platform typed in it (`lookup_metadata`), gets
//! the answer back with `take_lookup` (nothing is written yet: the dialog shows what changed,
//! and an ambiguous name comes back as candidates to pick from), and stores it on Save
//! (`store_lookup`): the answer *replaces* what was fetched before (`Db::replace_fetched`), so
//! nothing from an earlier, wrong match is left. Cancel discards it.

use super::{FetchResult, LookupReq, Pick};
use crate::app::AppState;
use crate::model::*;
use std::collections::BTreeMap;

/// Answers kept for a dialog that hasn't picked them up (a closed dialog's are dropped).
const KEEP: usize = 16;

#[derive(Debug, Default)]
pub struct Lookups {
    next: u64,
    done: BTreeMap<u64, FetchResult>,
}

impl Lookups {
    /// An answer arrived from the metadata worker.
    pub(crate) fn answered(&mut self, r: FetchResult) {
        let Some(token) = r.lookup else { return };
        self.done.insert(token, r);
        while self.done.len() > KEEP {
            self.done.pop_first();
        }
    }
}

impl AppState {
    /// Start a lookup the user asked for. `g` is the game as typed in the dialog (id 0 or an
    /// unknown id: a game not added yet). The token comes back with the answer
    /// (`take_lookup`); Err in plain words when no source can look it up.
    pub fn lookup_metadata(&mut self, g: &Game, pick: Option<Pick>) -> Result<u64, String> {
        let saved = self.library.get(g.id).is_some();
        let scan_description =
            saved && self.db.scan_meta(g.id).is_ok_and(|m| m.description.is_some_and(|d| !d.trim().is_empty()));
        self.lookups.next += 1;
        let token = self.lookups.next;
        let req = LookupReq { token, pick, with_art: saved, scan_description };
        self.metadata.lookup(g, req)?;
        Ok(token)
    }

    /// The answer to lookup `token`, once it is there (taken: asked once).
    pub fn take_lookup(&mut self, token: u64) -> Option<FetchResult> {
        self.lookups.done.remove(&token)
    }

    /// The game as it will be once `r` is stored (for the dialog; nothing is written).
    pub fn preview_lookup(&self, id: GameId, r: &FetchResult) -> Option<Game> {
        self.db
            .preview_fetched(id, &r.art, &r.meta)
            .inspect_err(|e| log::warn!("metadata preview: {e:#}"))
            .ok()
            .flatten()
    }

    /// Store a lookup's answer for game `id` (Save in Edit Game Details): it replaces the
    /// metadata fetched before. The game is reloaded.
    pub fn store_lookup(&mut self, id: GameId, r: &FetchResult) {
        if let Err(e) = self.db.replace_fetched(id, &r.art, &r.meta) {
            self.notify_error(format!("Couldn't save the metadata: {e:#}"));
            return;
        }
        if r.wiki_backfill {
            self.wikipedia_backfill_done(id);
        }
        if let Ok(Some(g)) = self.db.get(id) {
            self.library.update(id, |old| *old = g);
        }
    }
}
