//! AppState side of artwork editing.
//!
//! CONTRACT (the desktop UI calls these; keep signatures):
//!   st.art                      ArtService (field on AppState, created in App::new)
//!   st.art_state                ArtState: search results / thumbnails / bulk progress for the UI
//!   st.set_art_slot(id, slot, Option<PathBuf>)   set a slot's user art (None = back to default)
//!   st.set_art_from_file(id, slot, &Path)         import_local + set (errors → toast)
//!   st.set_art_from_url(id, slot, &str)           download in background, set when done
//!   st.search_art(id, slot, Option<&str>)          start a search (results in art_state.searches)
//!   st.bulk_art(Vec<GameId>, BulkOptions)          start a bulk download
//!   st.cancel_bulk_art()
//!   AppState::poll_art()                           called from poll_background each frame
//!
//! Automatic artwork (`config.art.auto_fill`, Settings → Metadata): after a scan
//! (`auto_art_after_scan`) and once the metadata fetcher is idle (`run_auto_art`, end of
//! poll_background), games missing a cover or background that were never looked up get one
//! quiet confident-match lookup per available source, in the order of Settings → Metadata
//! (Steam store name search, GOG, Lutris.net, PCGamingWiki, libretro-thumbnails for console
//! platforms, and the keyed ones with the user's credentials). Steam store games are left to the
//! metadata fetcher. Found images are stored as *fetched*
//! art (`<art_dir>/auto/<game>/`), so scanner and user art always win; slots that got art in
//! the meantime are left alone. Sources asked per game are remembered in the DB (`kv`
//! "art_auto_tried"), so each game is looked up once per source, ever; network failures are
//! retried after a later scan. Off in screenshot runs and tests (`AutoArt::allowed`).
//!
//! Everything slow (decoding, downloads) happens on the ArtService threads; these methods only
//! queue work and, when results arrive, update the DB (one small UPDATE) and the library.

use crate::app::AppState;
use crate::art::*;
use crate::model::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default)]
pub struct SearchState {
    pub items: Vec<Candidate>,
    pub done: bool,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct BulkStatus {
    pub done: usize,
    pub total: usize,
    pub current: String,
    pub running: bool,
    /// Set when finished: (changed, failed, cancelled).
    pub result: Option<(usize, usize, bool)>,
    /// Games with no confident match (or no images) — set when finished.
    pub not_found: Vec<String>,
    /// Why the run stopped early, if it did.
    pub error: Option<String>,
}

#[derive(Default)]
pub struct ArtState {
    pub searches: HashMap<(GameId, ArtSlot), SearchState>,
    /// Candidate thumbnail URL → local file.
    pub thumbs: HashMap<String, PathBuf>,
    /// Thumbnail URLs that couldn't be downloaded.
    pub thumbs_failed: HashSet<String>,
    /// (game, slot) downloads in flight from `set_art_from_url`.
    pub applying: Vec<(GameId, ArtSlot)>,
    pub bulk: Option<BulkStatus>,
    /// The SteamGridDB key being typed into a key field and whether it works.
    pub(crate) key_check: KeyCheckState,
}

/// `AppState::sgdb_key_check`'s state.
#[derive(Default)]
pub(crate) struct KeyCheckState {
    /// The field's key and since when it reads so: asked about once it stays the same a moment.
    typed: Option<(String, Instant)>,
    /// The key asked about and its answer (`Checking` until it arrives).
    answer: Option<(String, KeyCheck)>,
}

impl KeyCheckState {
    /// The answer for `key` once it was asked about (`Checking` until it arrives).
    pub(crate) fn answer_for(&self, key: &str) -> Option<KeyCheck> {
        self.answer.as_ref().filter(|(k, _)| k == key.trim()).map(|(_, a)| a.clone())
    }

    /// No key was checked, nor is one waiting to be.
    #[cfg(test)]
    pub(crate) fn untouched(&self) -> bool {
        self.typed.is_none() && self.answer.is_none()
    }
}

/// How long a key field must stay unchanged before its key is checked (one request per paste,
/// not one per typed character).
const KEY_CHECK_DELAY: Duration = Duration::from_millis(600);

/// State of the automatic artwork fill (see the module docs).
#[derive(Default)]
pub struct AutoArt {
    /// May run this session. False in screenshot runs and tests (a test sets it to opt in).
    pub allowed: bool,
    /// A scan finished since the last run: look for games once the metadata fetcher is idle.
    pub(crate) wanted: bool,
    /// A run is queued or in progress (one at a time).
    pub(crate) running: bool,
    /// Sources asked per game (`SourceKind::bit`), loaded from the DB on first use.
    tried: Option<HashMap<GameId, u16>>,
}

impl AutoArt {
    pub fn new(allowed: bool) -> AutoArt {
        AutoArt { allowed, ..Default::default() }
    }
}

const AUTO_TRIED_KEY: &str = "art_auto_tried";

/// What the automatic fill would ask for `g` (None = leave it alone). `tried` = bits of the
/// sources already asked; `sources` = the available ones, in priority order.
pub(crate) fn auto_request(g: &Game, tried: u16, sources: &[SourceKind]) -> Option<AutoRequest> {
    // Hidden = removed by the user; Steam store games are the metadata fetcher's (by appid).
    if g.hidden || steam_appid(g).is_some() {
        return None;
    }
    if g.art.cover.is_some() && g.art.hero.is_some() {
        return None;
    }
    let platform = crate::art::sources::art_platform(g);
    let known = crate::art::sources::console_platform(platform.as_deref());
    let console = known.is_some();
    let libretro = known.is_some_and(|p| p.libretro.is_some());
    let gametdb = crate::art::sources::gametdb_key_of(g).is_some();
    let sources: Vec<SourceKind> = sources
        .iter()
        .copied()
        .filter(|k| tried & k.bit() == 0)
        // Same rule as the bulk run: PC stores for PC games, libretro for consoles.
        .filter(|k| match k {
            SourceKind::Libretro => libretro,
            SourceKind::GameTdb => gametdb,
            SourceKind::Steam | SourceKind::Gog | SourceKind::PcGamingWiki => !console,
            _ => true,
        })
        .collect();
    if sources.is_empty() || g.display_name().trim().is_empty() {
        return None;
    }
    // Fetched art only shows in slots nothing else fills (effective = user > scanner > fetched).
    let need: Vec<ArtSlot> = ArtSlot::ALL.into_iter().filter(|s| s.get(&g.art).is_none()).collect();
    Some(AutoRequest {
        id: g.id,
        name: g.display_name().to_string(),
        appid: None,
        gog_id: crate::art::gog_product_id(g),
        platform,
        rom_names: crate::art::sources::rom_names(g),
        serial: crate::art::sources::rom_serial(g),
        need,
        sources,
    })
}

impl ArtState {
    #[cfg(test)]
    pub fn is_applying(&self, id: GameId, slot: ArtSlot) -> bool {
        self.applying.contains(&(id, slot))
    }
    pub fn bulk_running(&self) -> bool {
        self.bulk.as_ref().is_some_and(|b| b.running)
    }
}

impl AppState {
    /// Set a slot's user art (`None` = back to what the scanner / online lookup found).
    /// Files Lodestar stored for the slot that are no longer used are deleted.
    pub fn set_art_slot(&mut self, id: GameId, slot: ArtSlot, path: Option<PathBuf>) {
        let before_eff = self.library.get(id).and_then(|g| slot.get(&g.art).cloned());
        let before_bg = self.library.get(id).and_then(|g| g.art.background().cloned());
        let mut old_user = None;
        let res = self.db.set_user_art_slot(id, |a| {
            old_user = slot.get(a).cloned();
            slot.set(a, path.clone());
        });
        if let Err(e) = res {
            self.notify_error(format!("Couldn't save the {}: {e:#}", slot.label().to_lowercase()));
            return;
        }
        let art_dir = self.art.art_dir().to_path_buf();
        // Old file of ours no longer referenced (other extension, or reset to default).
        if let Some(old) = &old_user
            && Some(old) != path.as_ref()
            && is_user_art_file(&art_dir, id, old)
        {
            let _ = std::fs::remove_file(old);
        }
        if path.is_none() {
            remove_slot_files(&art_dir, id, slot);
        }
        if let Ok(Some(g)) = self.db.get(id) {
            let after = slot.get(&g.art).cloned();
            // Another background picture: the hand-placed crop was for the old one.
            let reset_crop = g.edits.background_placement.is_some() && g.art.background() != before_bg.as_ref();
            let edits = g.edits.clone();
            self.library.update(id, |old| *old = g);
            for p in [before_eff, old_user, path, after].into_iter().flatten() {
                self.images.invalidate(&p);
            }
            if reset_crop {
                self.set_edits(id, crate::model::UserEdits { background_placement: None, ..edits });
            }
        }
    }

    /// Copy a local image into Lodestar's art folder (in the background) and use it.
    pub fn set_art_from_file(&mut self, id: GameId, slot: ArtSlot, path: &Path) {
        self.art.import_file(id, slot, path);
        self.mark_applying(id, slot);
    }

    /// Download `url` (or import a pasted local path) in the background and use it.
    pub fn set_art_from_url(&mut self, id: GameId, slot: ArtSlot, url: &str) {
        let url = url.trim();
        if url.is_empty() {
            return;
        }
        self.art.apply_url(id, slot, url);
        self.mark_applying(id, slot);
    }

    fn mark_applying(&mut self, id: GameId, slot: ArtSlot) {
        if !self.art_state.applying.contains(&(id, slot)) {
            self.art_state.applying.push((id, slot));
        }
    }

    /// Start a search; results stream into `art_state.searches[(id, slot)]`.
    pub fn search_art(&mut self, id: GameId, slot: ArtSlot, query: Option<&str>) {
        let Some(g) = self.library.get(id) else {
            return;
        };
        self.art.search(g, slot, query);
        self.art_state.searches.insert((id, slot), SearchState::default());
    }

    /// Download art for many games in the background (progress in `art_state.bulk`).
    pub fn bulk_art(&mut self, ids: Vec<GameId>, opts: BulkOptions) {
        if self.art_state.bulk_running() {
            self.notify("An artwork download is already running");
            return;
        }
        let games: Vec<Game> = ids.iter().filter_map(|id| self.library.get(*id).cloned()).collect();
        if games.is_empty() || opts.slots.is_empty() {
            return;
        }
        let total = games.len();
        self.art.bulk(games, opts);
        self.art_state.bulk = Some(BulkStatus { total, running: true, ..Default::default() });
    }

    pub fn cancel_bulk_art(&mut self) {
        self.art.cancel_bulk();
    }

    /// Whether SteamGridDB accepts `key`, for a key field drawn every frame: the key is checked
    /// in the background once it stays the same for a moment. None for an empty field.
    pub fn sgdb_key_check(&mut self, key: &str) -> Option<KeyCheck> {
        let key = key.trim();
        if key.is_empty() {
            return None;
        }
        let kc = &mut self.art_state.key_check;
        if let Some(answer) = kc.answer_for(key) {
            return Some(answer);
        }
        match &kc.typed {
            Some((k, since)) if k == key => {
                let left = KEY_CHECK_DELAY.saturating_sub(since.elapsed());
                if left.is_zero() {
                    kc.typed = None;
                    kc.answer = Some((key.to_string(), KeyCheck::Checking));
                    self.art.check_sgdb_key(key);
                } else {
                    self.ctx().request_repaint_after(left);
                }
            }
            _ => {
                kc.typed = Some((key.to_string(), Instant::now()));
                self.ctx().request_repaint_after(KEY_CHECK_DELAY);
            }
        }
        Some(KeyCheck::Checking)
    }

    // ----- automatic fill -----

    /// A scan finished: fill missing art once the metadata fetcher is idle (`run_auto_art`).
    pub(crate) fn auto_art_after_scan(&mut self) {
        if self.lookups_wait_for_welcome() {
            return;
        }
        if self.auto_art.allowed && self.config.art.auto_fill {
            self.auto_art.wanted = true;
        }
    }

    /// Start the automatic fill when one is wanted and nothing else is looking games up.
    /// Called at the end of every poll (after the metadata results were applied).
    pub(crate) fn run_auto_art(&mut self) {
        let a = &self.auto_art;
        if !a.wanted || a.running {
            return;
        }
        if !a.allowed || !self.config.art.auto_fill {
            self.auto_art.wanted = false;
            return;
        }
        if self.scan.running {
            return; // its ScanDone asks again
        }
        if self.metadata.pending() > 0 {
            // A lookup that ends without a result requests no frame: look again in a while.
            self.ctx().request_repaint_after(std::time::Duration::from_secs(1));
            return;
        }
        self.auto_art.wanted = false;
        let reqs = self.auto_art_requests();
        if reqs.is_empty() {
            return;
        }
        log::info!("art auto: looking up art for {} games", reqs.len());
        self.auto_art.running = true;
        self.art.auto_fill(reqs);
    }

    /// Games to look up, newest first.
    fn auto_art_requests(&mut self) -> Vec<AutoRequest> {
        let steam = self.config.metadata.steam_store;
        // The store name search follows the "Steam store" switch, as before. Each game uses
        // the source list that matches it (PC games / emulated games).
        let list = |l: ArtList| -> Vec<SourceKind> {
            self.art
                .sources_for(l)
                .into_iter()
                .filter(|s| s.available && (steam || s.kind != SourceKind::Steam))
                .map(|s| s.kind)
                .collect()
        };
        let (pc, emu) = (list(ArtList::Pc), list(ArtList::Emulated));
        if pc.is_empty() && emu.is_empty() {
            return vec![];
        }
        self.load_auto_tried();
        let tried = self.auto_art.tried.as_ref();
        let mut games: Vec<&Game> = self.library.games.iter().collect();
        games.sort_by_key(|g| std::cmp::Reverse(g.added));
        games
            .into_iter()
            .filter_map(|g| {
                let t = tried.and_then(|m| m.get(&g.id)).copied().unwrap_or(0);
                let sources = match crate::art::sources::art_list(g) {
                    ArtList::Pc => &pc,
                    ArtList::Emulated => &emu,
                };
                auto_request(g, t, sources)
            })
            .collect()
    }

    fn load_auto_tried(&mut self) {
        if self.auto_art.tried.is_none() {
            let m = self.db.kv_get(AUTO_TRIED_KEY).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            self.auto_art.tried = Some(m);
        }
    }

    /// One game's automatic lookup is over: remember it, store what it found.
    fn apply_auto_art(&mut self, id: GameId, mut art: Art, tried: &[SourceKind]) {
        if !tried.is_empty() {
            self.load_auto_tried();
            let m = self.auto_art.tried.get_or_insert_with(Default::default);
            let e = m.entry(id).or_insert(0);
            for k in tried {
                *e |= k.bit();
            }
            match serde_json::to_string(&*m) {
                Ok(s) => {
                    if let Err(e) = self.db.kv_set(AUTO_TRIED_KEY, &s) {
                        log::warn!("art auto: saving lookups: {e:#}");
                    }
                }
                Err(e) => log::warn!("art auto: {e}"),
            }
        }
        // Slots that got art in the meantime (the user, a scan, the metadata fetcher) win.
        let current = self.library.get(id).map(|g| g.art.clone());
        for slot in ArtSlot::ALL {
            let Some(p) = slot.get(&art).cloned() else {
                continue;
            };
            if current.as_ref().is_none_or(|c| slot.get(c).is_some()) {
                let _ = std::fs::remove_file(&p);
                slot.set(&mut art, None);
            }
        }
        if ArtSlot::ALL.iter().all(|s| s.get(&art).is_none()) {
            return;
        }
        if let Err(e) = self.db.set_fetched(id, &art, &Metadata::default()) {
            log::warn!("art auto: saving art of game {id}: {e:#}");
            return;
        }
        if let Ok(Some(g)) = self.db.get(id) {
            self.library.update(id, |old| *old = g);
        }
    }

    /// Setting switched off: stop a running automatic fill.
    pub(crate) fn stop_auto_art(&mut self) {
        self.auto_art.wanted = false;
        self.art.cancel_auto();
    }

    pub(crate) fn poll_art(&mut self) {
        for ev in self.art.poll() {
            match ev {
                ArtEvent::SearchStarted { game, slot } => {
                    self.art_state.searches.insert((game, slot), SearchState::default());
                }
                ArtEvent::Candidates { game, slot, items, done, errors } => {
                    let s = self.art_state.searches.entry((game, slot)).or_default();
                    for c in items {
                        if !s.items.iter().any(|o| o.url == c.url) {
                            s.items.push(c);
                        }
                    }
                    s.done |= done;
                    s.errors.extend(errors);
                }
                ArtEvent::ThumbReady { url, path } => {
                    self.art_state.thumbs_failed.remove(&url);
                    self.art_state.thumbs.insert(url, path);
                }
                ArtEvent::ThumbFailed { url } => {
                    self.art_state.thumbs_failed.insert(url);
                }
                ArtEvent::Applied { game, slot, path } => {
                    self.art_state.applying.retain(|k| *k != (game, slot));
                    if self.library.get(game).is_some() {
                        self.set_art_slot(game, slot, Some(path));
                    }
                }
                ArtEvent::Failed { game, slot, error } => {
                    self.art_state.applying.retain(|k| *k != (game, slot));
                    let name = self.library.get(game).map(|g| g.display_name().to_string()).unwrap_or_default();
                    self.notify_error(format!("Couldn't set the {} of {name}: {error}", slot.label().to_lowercase()));
                }
                ArtEvent::BulkProgress { done, total, current } => {
                    let b = self.art_state.bulk.get_or_insert_with(Default::default);
                    b.running = true;
                    b.done = done;
                    b.total = total;
                    b.current = current;
                }
                ArtEvent::BulkNote { not_found, error } => {
                    let b = self.art_state.bulk.get_or_insert_with(Default::default);
                    b.not_found = not_found;
                    b.error = error;
                }
                ArtEvent::AutoGame { game, art, tried } => {
                    self.apply_auto_art(game, art, &tried);
                }
                ArtEvent::KeyChecked { key, check } => {
                    if let Some((k, answer)) = &mut self.art_state.key_check.answer
                        && *k == key
                    {
                        *answer = check;
                    }
                }
                ArtEvent::AutoFinished { changed, error } => {
                    self.auto_art.running = false;
                    if let Some(e) = error {
                        log::info!("art auto: {e}");
                    }
                    if changed > 0 {
                        self.notify(format!("Found artwork for {changed} game{}", if changed == 1 { "" } else { "s" }));
                    }
                }
                ArtEvent::BulkFinished { changed, failed, cancelled } => {
                    let b = self.art_state.bulk.get_or_insert_with(Default::default);
                    b.running = false;
                    b.result = Some((changed, failed, cancelled));
                    b.current.clear();
                    let msg = bulk_summary(b);
                    if b.error.is_some() && changed == 0 {
                        self.notify_error(msg);
                    } else {
                        self.notify(msg);
                    }
                }
            }
        }
    }
}

/// Delete every `<slot>.*` Lodestar stored for the game.
fn remove_slot_files(art_dir: &Path, id: GameId, slot: ArtSlot) {
    let Ok(rd) = std::fs::read_dir(user_art_dir(art_dir, id)) else {
        return;
    };
    let prefix = format!("{}.", slot.key());
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// "Artwork: 12 games updated, 3 not found, 1 failed".
pub fn bulk_summary(b: &BulkStatus) -> String {
    let (changed, failed, cancelled) = b.result.unwrap_or_default();
    let mut parts = vec![];
    let games = |n: usize| {
        if n == 1 { "1 game".to_string() } else { format!("{n} games") }
    };
    if b.total == 0 {
        return "Artwork: every selected game already has that artwork".into();
    }
    parts.push(format!("{} updated", games(changed)));
    if !b.not_found.is_empty() {
        parts.push(format!("{} not found", b.not_found.len()));
    }
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    let mut s = format!("Artwork: {}", parts.join(", "));
    if cancelled {
        s.push_str(" (cancelled)");
    }
    if let Some(e) = &b.error {
        s.push_str(&format!(" — {e}"));
    }
    s
}
