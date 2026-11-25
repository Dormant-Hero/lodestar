//! Keyboard navigation of the explorer (left sidebar), Playnite / TV-app style: two keyboard
//! zones, the games (default) and the sidebar. ← (Details / List view, or the grid's leftmost
//! column) moves focus to the sidebar; there ↑/↓/Home/End/PageUp/PageDown move a focus ring
//! between the entries of all open sections and apply the entry live, like a click; →, Enter
//! or Esc (or any mouse click) return to the games.

use super::super::{Counts, DesktopUi, Facet, LibSel, SrcKey};
use crate::app::AppState;
use crate::config::DesktopView;
use egui::{Context, Key, Modifiers};

/// One clickable explorer row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Entry {
    Lib(LibSel),
    /// One of the user's collections (by id).
    Coll(u64),
    Source(SrcKey),
    Tag(String),
    /// (`Facet::index`, lowercase key)
    Facet(usize, String),
}

/// Keyboard focus in the sidebar.
#[derive(Default)]
pub(crate) struct SideNav {
    /// The sidebar has the keyboard (else the games do).
    pub(crate) active: bool,
    focus: Option<Entry>,
    /// Position of `focus` when it was set (fallback when that entry disappears).
    idx: usize,
    /// Scroll the focused row into view when it is next drawn.
    scroll: bool,
}

impl SideNav {
    /// The sidebar has the keyboard and its focused entry satisfies `f`.
    pub(crate) fn is(&self, f: impl Fn(&Entry) -> bool) -> bool {
        self.active && self.focus.as_ref().is_some_and(f)
    }

    /// Called with the focused row's response: scrolls it into view once after a move.
    pub(crate) fn reveal(&mut self, r: &egui::Response) {
        if std::mem::take(&mut self.scroll) {
            r.scroll_to_me(None);
        }
    }
}

/// The explorer's clickable rows in drawing order: open sections only (headers, hints and
/// collapsed sections are skipped).
pub(crate) fn entries(c: &Counts, collapsed: &[String]) -> Vec<Entry> {
    let open = |k: &str| !collapsed.iter().any(|c| c == k);
    let mut out = vec![];
    if open("library") {
        out.extend([LibSel::All, LibSel::Installed].map(Entry::Lib));
        if c.not_installed > 0 {
            out.push(Entry::Lib(LibSel::NotInstalled));
        }
        if c.not_in_steam_library > 0 {
            out.push(Entry::Lib(LibSel::NotInSteamLibrary));
        }
        out.extend([LibSel::Favorites, LibSel::Recent].map(Entry::Lib));
        if c.hidden > 0 {
            out.push(Entry::Lib(LibSel::Hidden));
        }
    }
    if open("collections") {
        out.extend(c.colls.iter().map(|x| Entry::Coll(x.0)));
    }
    if open("sources") {
        out.extend(c.sources.iter().map(|&(s, _)| Entry::Source(s)));
    }
    let plat = Facet::Platform.index();
    if open(Facet::Platform.key()) {
        out.extend(c.facets[plat].iter().map(|(k, _, _)| Entry::Facet(plat, k.clone())));
    }
    if open("tags") {
        out.extend(c.tags.iter().map(|(t, _)| Entry::Tag(t.clone())));
    }
    for f in Facet::ALL {
        if c.shows(f) && f != Facet::Platform && open(f.key()) {
            let fi = f.index();
            out.extend(c.facets[fi].iter().map(|(k, _, _)| Entry::Facet(fi, k.clone())));
        }
    }
    out
}

/// A navigation key inside the sidebar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Move {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

/// New position after `m` in a list of `len` entries (clamped, no wrap-around).
pub(crate) fn step(len: usize, idx: usize, m: Move, page: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let last = len - 1;
    let idx = idx.min(last);
    match m {
        Move::Up => idx.saturating_sub(1),
        Move::Down => (idx + 1).min(last),
        Move::PageUp => idx.saturating_sub(page.max(1)),
        Move::PageDown => (idx + page.max(1)).min(last),
        Move::Home => 0,
        Move::End => last,
    }
}

/// Where the focus is now: `focus` if it is still listed, else the old position (clamped).
pub(crate) fn locate(list: &[Entry], focus: Option<&Entry>, old_idx: usize) -> usize {
    focus.and_then(|f| list.iter().position(|e| e == f)).unwrap_or_else(|| old_idx.min(list.len().saturating_sub(1)))
}

/// Does ← in the games move the keyboard to the sidebar? Always in Details / List view; in
/// the grid only from the leftmost column (or with no games).
pub(crate) fn left_enters_sidebar(view: DesktopView, show_sidebar: bool, cur: usize, cols: usize, n: usize) -> bool {
    show_sidebar && (view != DesktopView::Grid || n == 0 || cur.is_multiple_of(cols.max(1)))
}

const PAGE: usize = 10;

impl DesktopUi {
    /// Sidebar keyboard handling, called from `handle_keys` (no dialog / popup / text field has
    /// the keyboard). Returns true when the sidebar has the keyboard: the game keys are skipped.
    pub(crate) fn sidebar_keys(&mut self, ctx: &Context, st: &AppState) -> bool {
        let show = st.config.desktop.show_sidebar;
        if self.side_nav.active && (!show || self.focus_search || ctx.input(|i| i.pointer.any_pressed())) {
            // Hidden sidebar, Ctrl+F or a mouse click: back to the games.
            self.side_nav.active = false;
        }
        if !self.side_nav.active {
            let grid = st.config.desktop.view == DesktopView::Grid;
            let cols = if grid { self.grid_cols.max(1) } else { 1 };
            // Within its group (each group starts a new grid row).
            let n = self.groups.items.len();
            let cur = self.sel_pos.unwrap_or(0);
            let cur = cur - crate::ui::desktop::grouping::run_of(&self.groups.spans, n, cur).0;
            let enter = left_enters_sidebar(st.config.desktop.view, show, cur, cols, n)
                && !entries(&self.counts, &self.collapsed).is_empty()
                && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowLeft));
            if !enter {
                return false;
            }
            let list = entries(&self.counts, &self.collapsed);
            // Start on the entry the explorer shows as selected (else the first one).
            let idx = list.iter().position(|e| self.entry_selected(e)).unwrap_or(0);
            self.side_nav.active = true;
            self.side_nav.focus = list.get(idx).cloned();
            self.side_nav.idx = idx;
            self.side_nav.scroll = true;
            surrender_focus(ctx);
            ctx.request_repaint();
            return true;
        }
        let mut mv = None;
        let mut leave = false;
        ctx.input_mut(|i| {
            let m = Modifiers::NONE;
            for (k, v) in [
                (Key::ArrowUp, Move::Up),
                (Key::ArrowDown, Move::Down),
                (Key::PageUp, Move::PageUp),
                (Key::PageDown, Move::PageDown),
                (Key::Home, Move::Home),
                (Key::End, Move::End),
            ] {
                if i.consume_key(m, k) {
                    mv = Some(v);
                }
            }
            for k in [Key::ArrowRight, Key::Enter, Key::Escape] {
                leave |= i.consume_key(m, k);
            }
            // Already in the leftmost zone: keep egui from moving widget focus with it.
            i.consume_key(m, Key::ArrowLeft);
        });
        if leave {
            self.side_nav.active = false;
            // Back on the games: the kept selection, or the first game of the filtered view
            // (`refresh_view_for` picks it when the old one is filtered out).
            self.scroll_to_sel = true;
            ctx.request_repaint();
            return true;
        }
        if let Some(m) = mv {
            let list = entries(&self.counts, &self.collapsed);
            let cur = locate(&list, self.side_nav.focus.as_ref(), self.side_nav.idx);
            let idx = step(list.len(), cur, m, PAGE);
            if let Some(e) = list.get(idx).cloned() {
                self.apply_entry(&e);
                self.side_nav.focus = Some(e);
                self.side_nav.idx = idx;
                self.side_nav.scroll = true;
            }
            surrender_focus(ctx);
            ctx.request_repaint();
        }
        true
    }

    /// Is `e` highlighted as selected in the explorer?
    fn entry_selected(&self, e: &Entry) -> bool {
        match e {
            Entry::Lib(sel) => self.lib_sel == *sel && self.no_extra_filters(),
            Entry::Coll(id) => self.coll_sel == Some(*id),
            Entry::Source(s) => self.src_sel.contains(s),
            Entry::Tag(t) => self.filter.tags.contains(t),
            Entry::Facet(fi, k) => self.facet_sel[*fi].contains(k),
        }
    }

    /// Select exactly `e`, like a plain click on a row that isn't selected yet.
    fn apply_entry(&mut self, e: &Entry) {
        match e {
            Entry::Lib(sel) => self.set_lib_sel(*sel),
            Entry::Coll(id) => {
                self.set_lib_sel(LibSel::All);
                self.coll_sel = Some(*id);
            }
            Entry::Source(s) => {
                self.clear_extras();
                self.src_sel.push(*s);
                self.lib_sel = LibSel::All;
            }
            Entry::Tag(t) => {
                self.clear_extras();
                self.filter.tags.push(t.clone());
                self.lib_sel = LibSel::All;
            }
            Entry::Facet(fi, k) => {
                self.clear_extras();
                self.facet_sel[*fi].push(k.clone());
                self.lib_sel = LibSel::All;
            }
        }
    }
}

/// A keyboard-focused widget (after Tab) would also follow the arrow keys.
fn surrender_focus(ctx: &Context) {
    ctx.memory_mut(|m| {
        if let Some(f) = m.focused() {
            m.surrender_focus(f);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;

    fn counts() -> Counts {
        let mut c = Counts {
            hidden: 0,
            sources: vec![(SrcKey::source(Source::Steam), 3), (SrcKey::source(Source::Gog), 2)],
            tags: vec![("Co-op".into(), 1)],
            ..Default::default()
        };
        c.facets[Facet::Genre.index()] = vec![("rpg".into(), "RPG".into(), 2)];
        c.facets[Facet::Year.index()] = vec![("2020".into(), "2020".into(), 1)];
        c
    }

    #[test]
    fn collections_and_platforms_come_before_tags() {
        let mut c = counts();
        c.colls = vec![(7, "Co-op night".into(), 3, false), (9, "RPGs".into(), 5, true)];
        c.facets[Facet::Platform.index()] = vec![
            ("windows".into(), "Windows".into(), 4),
            ("sony playstation 2".into(), "Sony PlayStation 2".into(), 1),
        ];
        let l = entries(&c, &[]);
        assert_eq!(l[4], Entry::Coll(7));
        assert_eq!(l[5], Entry::Coll(9));
        assert_eq!(l[6], Entry::Source(SrcKey::source(Source::Steam)));
        assert_eq!(l[8], Entry::Facet(Facet::Platform.index(), "windows".into()));
        assert_eq!(l[9], Entry::Facet(Facet::Platform.index(), "sony playstation 2".into()));
        assert_eq!(l[10], Entry::Tag("Co-op".into()));
        assert_eq!(l[11], Entry::Facet(Facet::Genre.index(), "rpg".into()));
        assert_eq!(l.len(), 12);
        let l = entries(&c, &["collections".into(), "platform".into()]);
        assert_eq!(l[4], Entry::Source(SrcKey::source(Source::Steam)));
        assert_eq!(l.len(), 8);
    }

    #[test]
    fn entries_follow_drawing_order_and_skip_collapsed() {
        let c = counts();
        let all = entries(&c, &[]);
        assert_eq!(all[0], Entry::Lib(LibSel::All));
        assert_eq!(all[3], Entry::Lib(LibSel::Recent));
        assert_eq!(all[4], Entry::Source(SrcKey::source(Source::Steam)));
        assert_eq!(all[5], Entry::Source(SrcKey::source(Source::Gog)));
        assert_eq!(all[6], Entry::Tag("Co-op".into()));
        assert_eq!(all[7], Entry::Facet(Facet::Genre.index(), "rpg".into()));
        // Year has no explorer section; no Hidden entry without hidden games.
        assert_eq!(all.len(), 8);

        let some = entries(&c, &["sources".into(), "genre".into()]);
        assert_eq!(some.len(), 5);
        assert_eq!(some[4], Entry::Tag("Co-op".into()));
        assert!(entries(&Counts::default(), &["library".into()]).is_empty());
    }

    #[test]
    fn hidden_entry_only_with_hidden_games() {
        let mut c = counts();
        c.hidden = 2;
        let l = entries(&c, &[]);
        assert_eq!(l[4], Entry::Lib(LibSel::Hidden));
        assert_eq!(l[5], Entry::Source(SrcKey::source(Source::Steam)));
    }

    #[test]
    fn step_clamps() {
        assert_eq!(step(5, 0, Move::Up, 10), 0);
        assert_eq!(step(5, 0, Move::Down, 10), 1);
        assert_eq!(step(5, 4, Move::Down, 10), 4);
        assert_eq!(step(5, 2, Move::Home, 10), 0);
        assert_eq!(step(5, 2, Move::End, 10), 4);
        assert_eq!(step(5, 1, Move::PageDown, 10), 4);
        assert_eq!(step(30, 25, Move::PageUp, 10), 15);
        assert_eq!(step(0, 3, Move::Down, 10), 0);
        // A stale position past the end moves from the last entry.
        assert_eq!(step(3, 9, Move::Up, 10), 1);
    }

    #[test]
    fn locate_keeps_entry_or_falls_back() {
        let l = entries(&counts(), &[]);
        let gog = Entry::Source(SrcKey::source(Source::Gog));
        assert_eq!(locate(&l, Some(&gog), 0), 5);
        let gone = Entry::Tag("nope".into());
        assert_eq!(locate(&l, Some(&gone), 3), 3);
        assert_eq!(locate(&l, Some(&gone), 99), l.len() - 1);
        assert_eq!(locate(&[], None, 4), 0);
    }

    #[test]
    fn left_zone_rules() {
        use DesktopView::*;
        assert!(left_enters_sidebar(Details, true, 7, 1, 20));
        assert!(left_enters_sidebar(List, true, 7, 1, 20));
        assert!(!left_enters_sidebar(Details, false, 7, 1, 20));
        // Grid: only from the leftmost column (or an empty view).
        assert!(left_enters_sidebar(Grid, true, 0, 5, 20));
        assert!(left_enters_sidebar(Grid, true, 10, 5, 20));
        assert!(!left_enters_sidebar(Grid, true, 7, 5, 20));
        assert!(left_enters_sidebar(Grid, true, 3, 5, 0));
        assert!(!left_enters_sidebar(Grid, false, 0, 5, 20));
    }
}
