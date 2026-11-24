//! What the window keeps: the explorer selection and the selected game (database key /
//! value store), and config changes (saved debounced).

use super::*;

/// Explorer selection, kept across restarts (DB kv, JSON).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct ExplorerState {
    pub lib: String,
    pub sources: Vec<String>,
    pub tags: Vec<String>,
    pub genre: Vec<String>,
    pub platform: Vec<String>,
    pub developer: Vec<String>,
    pub publisher: Vec<String>,
    pub feature: Vec<String>,
    pub completion: Vec<String>,
    pub year: Vec<String>,
    pub playtime: Vec<String>,
    pub size: Vec<String>,
    pub score: Vec<String>,
    /// Match all (else any) of several values picked in one field.
    pub match_all: bool,
    /// Collapsed section keys.
    pub collapsed: Vec<String>,
    /// The user's collection shown (by id).
    pub coll: Option<u64>,
}

impl DesktopUi {
    pub(crate) fn explorer_state(&self) -> ExplorerState {
        let f = &self.facet_sel;
        ExplorerState {
            lib: self.lib_sel.key().into(),
            sources: self.src_sel.iter().map(|s| s.0.to_string()).collect(),
            tags: self.filter.tags.clone(),
            genre: f[Facet::Genre.index()].clone(),
            platform: f[Facet::Platform.index()].clone(),
            developer: f[Facet::Developer.index()].clone(),
            publisher: f[Facet::Publisher.index()].clone(),
            feature: f[Facet::Feature.index()].clone(),
            completion: f[Facet::Completion.index()].clone(),
            year: f[Facet::Year.index()].clone(),
            playtime: f[Facet::Playtime.index()].clone(),
            size: f[Facet::Size.index()].clone(),
            score: f[Facet::Score.index()].clone(),
            match_all: self.match_all,
            collapsed: self.collapsed.clone(),
            coll: self.coll_sel,
        }
    }

    pub(crate) fn apply_explorer_state(&mut self, es: &ExplorerState) {
        self.lib_sel = LibSel::from_key(&es.lib);
        self.filter.sources.clear();
        self.src_sel = es.sources.iter().filter_map(|k| SrcKey::from_key(k)).collect();
        self.filter.tags = es.tags.clone();
        let lower = |v: &Vec<String>| v.iter().map(|s| s.to_lowercase()).collect::<Vec<_>>();
        self.facet_sel[Facet::Genre.index()] = lower(&es.genre);
        self.facet_sel[Facet::Platform.index()] = lower(&es.platform);
        self.facet_sel[Facet::Developer.index()] = lower(&es.developer);
        self.facet_sel[Facet::Publisher.index()] = lower(&es.publisher);
        self.facet_sel[Facet::Feature.index()] = lower(&es.feature);
        self.facet_sel[Facet::Completion.index()] = lower(&es.completion);
        self.facet_sel[Facet::Year.index()] = lower(&es.year);
        self.facet_sel[Facet::Playtime.index()] = lower(&es.playtime);
        self.facet_sel[Facet::Size.index()] = lower(&es.size);
        self.facet_sel[Facet::Score.index()] = lower(&es.score);
        self.match_all = es.match_all;
        self.collapsed = es.collapsed.clone();
        self.coll_sel = es.coll;
    }

    /// Cheap per-frame fingerprint of the explorer state (no allocation).
    pub(super) fn explorer_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.lib_sel.key().hash(&mut h);
        self.src_sel.hash(&mut h);
        self.filter.tags.hash(&mut h);
        self.facet_sel.hash(&mut h);
        self.match_all.hash(&mut h);
        self.collapsed.hash(&mut h);
        self.coll_sel.hash(&mut h);
        h.finish()
    }

    pub(crate) fn mark_config_dirty(&mut self) {
        if self.config_dirty.is_none() {
            self.config_dirty = Some(Instant::now());
        }
    }

    pub(super) fn flush_config(&mut self, ctx: &egui::Context, st: &mut AppState) {
        const DEBOUNCE: Duration = Duration::from_millis(700);
        if let Some(t) = self.config_dirty {
            let el = t.elapsed();
            if el >= DEBOUNCE {
                self.config_dirty = None;
                st.save_config();
            } else {
                ctx.request_repaint_after(DEBOUNCE - el);
            }
        }
        // Remember the explorer selection (DB kv), debounced the same way.
        let eh = self.explorer_hash();
        if Some(eh) != self.saved_explorer {
            let now = Instant::now();
            let since = *self.explorer_changed.get_or_insert(now);
            if now - since >= DEBOUNCE {
                if let Ok(json) = serde_json::to_string(&self.explorer_state())
                    && let Err(e) = st.db.kv_set(KV_EXPLORER, &json)
                {
                    log::warn!("saving the explorer selection: {e:#}");
                }
                self.saved_explorer = Some(eh);
                self.explorer_changed = None;
            } else {
                ctx.request_repaint_after(DEBOUNCE - (now - since));
            }
        }
        // Remember the selected game (DB kv, not the config file), debounced the same way.
        if self.selected != self.saved_selected {
            let now = Instant::now();
            let since = *self.sel_changed.get_or_insert(now);
            if now - since >= DEBOUNCE {
                if let Some(id) = self.selected
                    && let Err(e) = st.db.kv_set(KV_SELECTED, &id.to_string())
                {
                    log::warn!("saving the selected game: {e:#}");
                }
                self.saved_selected = self.selected;
                self.sel_changed = None;
            } else {
                ctx.request_repaint_after(DEBOUNCE - (now - since));
            }
        }
    }
}
