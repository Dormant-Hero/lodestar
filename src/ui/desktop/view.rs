//! The games shown: the filter made from the sidebar, the top bar and the search; the
//! filtered, sorted and grouped view; the selection in it.

use super::*;

impl DesktopUi {
    pub(super) fn sync_filter(&mut self, st: &AppState) {
        let d = &st.config.desktop;
        let f = &mut self.filter;
        // "Not installed" in the sidebar wins over the Installed preset.
        f.installed_only = d.preset == QuickPreset::Installed && self.lib_sel != LibSel::NotInstalled;
        f.favorites_only = d.preset == QuickPreset::Favorites;
        if self.groups.by != d.group_by || self.groups.collapsed != d.collapsed_groups {
            self.groups.by = d.group_by;
            self.groups.collapsed.clone_from(&d.collapsed_groups);
        }
        self.preset = d.preset;
        let f = &mut self.filter;
        f.show_hidden = d.show_hidden || self.lib_sel == LibSel::Hidden;
        let (sort, descending, _) = self.effective_sort(d);
        let f = &mut self.filter;
        f.sort = sort;
        f.descending = descending;
        // One of the user's collections (gone: back to All games).
        if let Some(id) = self.coll_sel {
            match st.library.user_collection(id) {
                Some(u) => {
                    let same =
                        matches!(&f.collection, Some(Collection { kind: CollectionKind::User(x), .. }) if x == u);
                    if !same {
                        f.collection = Some(Collection { kind: CollectionKind::User(u.clone()), name: String::new() });
                    }
                    return;
                }
                None => self.coll_sel = None,
            }
        }
        let f = &mut self.filter;
        let want = match self.lib_sel {
            LibSel::All | LibSel::Hidden | LibSel::NotInstalled => None,
            LibSel::Installed => Some(CollectionKind::Installed),
            LibSel::Favorites => Some(CollectionKind::Favorites),
            LibSel::Recent => Some(CollectionKind::Recent),
        };
        if f.collection.as_ref().map(|c| &c.kind) != want.as_ref() {
            f.collection = want.map(|kind| Collection { kind, name: String::new() });
        }
    }

    pub(crate) fn set_lib_sel(&mut self, sel: LibSel) {
        self.lib_sel = sel;
        self.coll_sel = None;
        self.src_sel.clear();
        self.filter.tags.clear();
        self.facet_sel.iter_mut().for_each(Vec::clear);
    }

    /// No source / tag / facet narrowing on top of the Library entry.
    pub(crate) fn no_extra_filters(&self) -> bool {
        self.coll_sel.is_none()
            && self.src_sel.is_empty()
            && self.filter.tags.is_empty()
            && self.facet_sel.iter().all(Vec::is_empty)
    }

    /// Sort actually applied to the view: "Recently played" is always newest first, whatever
    /// the configured sort. Returns (sort, reversed, locked).
    pub(crate) fn effective_sort(&self, d: &crate::config::DesktopConfig) -> (SortBy, bool, bool) {
        if self.lib_sel == LibSel::Recent || d.preset == QuickPreset::Recent {
            (SortBy::LastPlayed, false, true)
        } else if d.preset == QuickPreset::MostPlayed {
            (SortBy::Playtime, false, true)
        } else {
            (d.sort, d.sort_descending, false)
        }
    }

    pub(super) fn refresh_view_for(&mut self, lib: &crate::library::Library) {
        let extra = (self.match_all, self.lib_sel, self.src_sel.clone());
        let generation = lib.view_generation(self.filter.sort);
        let fresh = matches!(&self.view_key, Some((g, f, fs, p, x))
            if *g == generation && *f == self.filter && *fs == self.facet_sel && *p == self.preset && *x == extra);
        if fresh {
            self.regroup(lib);
            return;
        }
        // Tags are matched with the fields (Match any / all, explorer.rs `extra_match`).
        let mut lib_filter = self.filter.clone();
        lib_filter.tags.clear();
        let mut indices: Vec<usize> = self.cache.get(lib, &lib_filter).to_vec();
        indices.retain(|&i| self.extra_match(&lib.games[i]));
        random::preset_retain(self.preset, lib, &mut indices);
        self.view = indices;
        self.groups.stamp += 1;
        let same_filter = matches!(&self.view_key, Some((_, f, fs, p, x))
            if *f == self.filter && *fs == self.facet_sel && *p == self.preset && *x == extra);
        if !same_filter {
            self.multi.clear();
        } else if !self.multi.is_empty() {
            let in_view: std::collections::HashSet<GameId> = self.view.iter().map(|&i| lib.games[i].id).collect();
            self.multi.retain(|id| in_view.contains(id));
            if self.multi.len() < 2 {
                self.multi.clear();
            }
        }
        self.view_key = Some((generation, self.filter.clone(), self.facet_sel.clone(), self.preset, extra));
        self.view_installed = self.view.iter().filter(|&&i| lib.games[i].installed).count();
        // `sel_pos` (a place in the shown, grouped list) is set by `regroup`.
        self.sel_pos = None;
        self.regroup(lib);
        // Keep the selection on screen when the user changed what the list shows (filter, search,
        // sidebar entry). A library update alone (metadata / art arriving, a rescan) must not move
        // a list the user scrolled away from the selection.
        if !same_filter {
            self.scroll_to_sel = true;
        }
    }

    pub(crate) fn select_pos(&mut self, st: &AppState, pos: usize) {
        if let Some(&i) = self.groups.items.get(pos) {
            self.sel_pos = Some(pos);
            self.selected = Some(st.library.games[i].id);
            self.scroll_to_sel = true;
        }
    }
}
