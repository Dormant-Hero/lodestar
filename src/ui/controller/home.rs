//! Playnite Fullscreen-style navigation, for themes with a home page (`[home]` view) and/or a
//! navigation rail (`rail` element): the home page's cover rows, the rail's sections (Home,
//! All games, Platforms, Sources, Collections, Filters) and the collections page of one
//! section. Themes without these elements never reach this code.

use super::*;
use crate::theme::model::Element;
use crate::theme::render::{RailScene, RowScene, SceneExtra, pick_rows, rail_entries};

/// The rail section a collection belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Group {
    All,
    Platforms,
    Sources,
    Collections,
    Filters,
}

impl Group {
    pub(super) fn of(c: &Collection) -> Group {
        match &c.kind {
            CollectionKind::All => Group::All,
            CollectionKind::Platform(_) | CollectionKind::Pc => Group::Platforms,
            CollectionKind::Source(_) => Group::Sources,
            CollectionKind::User(_) => Group::Collections,
            CollectionKind::Tag(_) if is_hidden_collection(c) => Group::Filters,
            CollectionKind::Tag(_) => Group::Collections,
            CollectionKind::Recent
            | CollectionKind::Favorites
            | CollectionKind::Installed
            | CollectionKind::Controller => Group::Filters,
        }
    }

    pub(super) fn key(self) -> &'static str {
        match self {
            Group::All => "all",
            Group::Platforms => "platforms",
            Group::Sources => "sources",
            Group::Collections => "collections",
            Group::Filters => "filters",
        }
    }

    fn from_key(k: &str) -> Option<Group> {
        [Group::All, Group::Platforms, Group::Sources, Group::Collections, Group::Filters]
            .into_iter()
            .find(|g| g.key() == k)
    }

    /// Heading of the section's collections page.
    pub(super) fn label(self) -> &'static str {
        match self {
            Group::All => "All games",
            Group::Platforms => "Platforms",
            Group::Sources => "Sources",
            Group::Collections => "Collections",
            Group::Filters => "Filters",
        }
    }
}

/// The rail of the view on screen: which of its entries are shown, and their keys.
pub(super) struct RailInfo {
    pub shown: Vec<usize>,
    pub keys: Vec<String>,
}

#[derive(Default)]
pub(super) struct HomeState {
    /// The theme's home page was opened (later frames don't jump back to it).
    started: bool,
    /// The controller theme (by key) whose opening screen was settled: a `start = "games"`
    /// theme opens its All games list once, when it is put on (not while it is only previewed
    /// in the theme menu, nor when a preview is cancelled back to it).
    theme_seen: Option<String>,
    /// (theme, collections generation) the rows were built for.
    built: Option<(usize, Option<CollsSig>)>,
    /// Home rows: index into `colls` + the row's games (indices into `library.games`).
    pub(super) rows: Vec<(usize, Vec<usize>)>,
    pub(super) row: usize,
    /// Selected item of each row.
    pub(super) cols: Vec<usize>,
    /// Kept across rebuilds (a rescan, a favourite toggled): the focused row's collection
    /// and the selected game of each row, by collection key.
    row_key: Option<String>,
    picked: HashMap<String, GameId>,
    pub(super) rows_id: u64,
    /// The rail has the focus; its highlighted section (index into the shown entries).
    pub(super) rail_focus: bool,
    pub(super) rail_sel: usize,
    /// The collections page of one section: what it was built for, its cards and their
    /// indices in `colls`.
    group_built: Option<(Option<CollsSig>, Group)>,
    pub(super) group_items: Vec<CollectionItem>,
    pub(super) group_map: Vec<usize>,
    pub(super) group: Option<Group>,
    /// The details page was opened from the home page: B goes back there.
    pub(super) details_from_home: bool,
}

impl ControllerUi {
    pub(super) fn has_home(st: &AppState) -> bool {
        st.controller_theme.file.home.is_some()
    }

    /// The screen controller mode starts on: the theme's home page, the game list of a theme
    /// that starts there (once it is open), else the collections.
    pub(super) fn first_screen(&self, st: &AppState) -> Screen {
        if Self::has_home(st) {
            Screen::Home
        } else if Self::starts_on_games(st) && self.cur.is_some() {
            Screen::Games
        } else {
            Screen::Collections
        }
    }

    /// The theme opens on the All games list (Playnite Fullscreen's main screen).
    fn starts_on_games(st: &AppState) -> bool {
        st.controller_theme.file.theme.start.eq_ignore_ascii_case("games")
    }

    /// The game menu of a home-page cover → "Show details": that cover's details.
    pub(super) fn open_home_game(&mut self, st: &AppState, id: GameId, now: f64) {
        let row = self.home.row.min(self.home.rows.len().saturating_sub(1));
        let col =
            self.home.rows.get(row).and_then(|(_, games)| {
                games.iter().position(|&gi| st.library.games.get(gi).is_some_and(|g| g.id == id))
            });
        if let Some(col) = col {
            self.open_from_home(st, row, col, now);
        }
    }

    /// The theme view on screen, for the clock / battery corner (None: the empty library or
    /// the built-in details page, which the theme doesn't draw).
    pub(super) fn status_view(&self, st: &AppState) -> Option<ViewKind> {
        if self.colls.is_empty() {
            return None;
        }
        let kind = self.view_kind();
        st.controller_theme.file.view(kind).map(|_| kind)
    }

    pub(super) fn view_kind(&self) -> ViewKind {
        match self.screen {
            Screen::Home => ViewKind::Home,
            Screen::Collections => ViewKind::Collections,
            Screen::Games => ViewKind::Games,
            Screen::Details => ViewKind::Details,
        }
    }

    /// The rail of `kind`'s view, with the sections that have something to show.
    pub(super) fn rail_info(&mut self, st: &AppState, kind: ViewKind) -> Option<RailInfo> {
        let theme = st.controller_theme.clone();
        let coll = match kind {
            ViewKind::Collections => self.colls.get(self.coll_sel),
            ViewKind::Home => None,
            _ => self.cur.and_then(|i| self.colls.get(i)),
        }
        .map(|c| c.collection.clone());
        let p = self.renderer.prepare(&theme, kind, coll.as_ref(), st.config.controller.variant.as_deref())?;
        let Element::Rail { entries, .. } = &p.view.elements[p.view.rail_index()?] else {
            return None;
        };
        let has_home = Self::has_home(st);
        let mut info = RailInfo { shown: vec![], keys: vec![] };
        for (i, e) in rail_entries(entries).iter().enumerate() {
            let ok = match e.key.as_str() {
                "home" => has_home,
                "search" | "all" => self.colls.iter().any(|c| c.collection.kind == CollectionKind::All),
                "menu" => true,
                k => Group::from_key(k).is_some_and(|g| self.colls.iter().any(|c| Group::of(&c.collection) == g)),
            };
            if ok {
                info.shown.push(i);
                info.keys.push(e.key.clone());
            }
        }
        Some(info)
    }

    /// The section that is open on this screen (a rail key).
    fn current_section(&self) -> Option<&'static str> {
        let group_of = |i: Option<usize>| i.and_then(|i| self.colls.get(i)).map(|c| Group::of(&c.collection).key());
        match self.screen {
            Screen::Home => Some("home"),
            Screen::Collections => group_of(Some(self.coll_sel)),
            Screen::Details if self.home.details_from_home => Some("home"),
            Screen::Games | Screen::Details => group_of(self.cur),
        }
    }

    pub(super) fn rail_scene<'a>(&self, info: &'a RailInfo) -> RailScene<'a> {
        let current = self.current_section().and_then(|k| info.keys.iter().position(|x| x == k));
        RailScene {
            shown: &info.shown,
            current,
            selected: self.home.rail_sel.min(info.keys.len().saturating_sub(1)),
            focused: self.home.rail_focus,
        }
    }

    /// Keep the home page, its rows and the section page in step with the theme and library.
    /// Cheap when nothing changed.
    pub(super) fn home_sync(&mut self, st: &AppState, now: f64) {
        let has_home = Self::has_home(st);
        if !has_home {
            self.home.started = false;
            if self.screen == Screen::Home {
                self.screen = Screen::Collections;
                self.home.rail_focus = false;
            }
            let key = st.controller_theme.key.as_str();
            if self.theme_preview.is_none() && self.home.theme_seen.as_deref() != Some(key) && !self.colls.is_empty() {
                self.home.theme_seen = Some(key.to_string());
                if Self::starts_on_games(st) && self.screen == Screen::Collections {
                    // All games, else (no such collection) the first one.
                    let i = self.colls.iter().position(|c| c.collection.key() == "all").unwrap_or(0);
                    self.coll_sel = i;
                    self.enter_collection(&st.library, i, now);
                }
            }
        } else if !self.home.started {
            self.home.started = true;
            if self.screen == Screen::Collections {
                self.go(Screen::Home, now);
            }
        }
        if has_home {
            let sig = (Arc::as_ptr(&st.controller_theme) as usize, self.colls_gen);
            if self.home.built != Some(sig) {
                self.home.built = Some(sig);
                self.build_rows(st);
            }
        }
        let kind = self.view_kind();
        let rail = if kind == ViewKind::Details { None } else { self.rail_info(st, kind) };
        if rail.is_none() {
            self.home.rail_focus = false;
        }
        // The collections page of a rail theme shows one section.
        let group = (self.screen == Screen::Collections && rail.is_some())
            .then(|| self.colls.get(self.coll_sel).map(|c| Group::of(&c.collection)))
            .flatten();
        self.home.group = group;
        if let Some(g) = group
            && self.home.group_built != Some((self.colls_gen, g))
        {
            self.home.group_built = Some((self.colls_gen, g));
            self.home.group_map =
                (0..self.colls.len()).filter(|&i| Group::of(&self.colls[i].collection) == g).collect();
            self.home.group_items = self.home.group_map.iter().map(|&i| self.colls[i].clone()).collect();
            self.renderer.reset_view(ViewKind::Collections);
        }
    }

    fn build_rows(&mut self, st: &AppState) {
        let theme = st.controller_theme.clone();
        let spec = self
            .renderer
            .prepare(&theme, ViewKind::Home, None, st.config.controller.variant.as_deref())
            .and_then(|p| {
                p.view.elements.iter().find_map(|e| match e {
                    Element::Rows { rows, max_rows, row_items, .. } => {
                        Some((rows.clone(), *max_rows as usize, *row_items as usize))
                    }
                    _ => None,
                })
            });
        let (specs, max, per_row) = spec.unwrap_or_else(|| (vec!["all".into()], 1, 30));
        let mut picked = pick_rows(&specs, &self.colls, max.max(1));
        if picked.is_empty() {
            // Nothing the theme asked for has games: at least show the whole library.
            picked = pick_rows(&["all".to_string()], &self.colls, 1);
        }
        self.home.rows = picked
            .into_iter()
            .map(|ci| {
                let games =
                    self.colls_games.get(ci).map(|g| g[..g.len().min(per_row.max(1))].to_vec()).unwrap_or_default();
                (ci, games)
            })
            .collect();
        let key_of = |ci: usize| self.colls.get(ci).map(|c| c.collection.key()).unwrap_or_default();
        let keys: Vec<String> = self.home.rows.iter().map(|(ci, _)| key_of(*ci)).collect();
        self.home.row = self
            .home
            .row_key
            .as_ref()
            .and_then(|k| keys.iter().position(|x| x == k))
            .unwrap_or(self.home.row)
            .min(keys.len().saturating_sub(1));
        self.home.cols = self
            .home
            .rows
            .iter()
            .zip(&keys)
            .map(|((_, games), k)| {
                self.home
                    .picked
                    .get(k)
                    .and_then(|id| games.iter().position(|&gi| st.library.games.get(gi).is_some_and(|g| g.id == *id)))
                    .unwrap_or(0)
            })
            .collect();
        self.home.rows_id = hash_of(&keys);
    }

    /// The game under the home page's focus.
    pub(super) fn home_game<'a>(&self, st: &'a AppState) -> Option<&'a Game> {
        let (_, games) = self.home.rows.get(self.home.row)?;
        let col = self.home.cols.get(self.home.row).copied().unwrap_or(0);
        games.get(col).and_then(|&gi| st.library.games.get(gi))
    }

    fn remember_home(&mut self, st: &AppState) {
        let Some(&(ci, _)) = self.home.rows.get(self.home.row) else { return };
        let Some(key) = self.colls.get(ci).map(|c| c.collection.key()) else { return };
        if let Some(id) = self.home_game(st).map(|g| g.id) {
            self.home.picked.insert(key.clone(), id);
        }
        self.home.row_key = Some(key);
    }

    fn focus_rail(&mut self, st: &AppState) {
        let kind = self.view_kind();
        if let Some(info) = self.rail_info(st, kind) {
            let cur = self.current_section();
            self.home.rail_sel = cur.and_then(|k| info.keys.iter().position(|x| x == k)).unwrap_or(0);
            self.home.rail_focus = true;
        }
    }

    /// Open a rail section.
    pub(super) fn rail_activate(&mut self, st: &mut AppState, key: &str, now: f64) {
        self.home.rail_focus = false;
        match key {
            "home" if Self::has_home(st) => self.go(Screen::Home, now),
            "all" => {
                if let Some(i) = self.all_index() {
                    self.coll_sel = i;
                    self.coll_key = Some(self.colls[i].collection.key());
                    self.enter_collection(&st.library, i, now);
                }
            }
            "search" => self.search_all(now, &st.library),
            "menu" => self.open_quick_menu(st, now),
            k => {
                let Some(g) = Group::from_key(k) else { return };
                let keep = self.colls.get(self.coll_sel).is_some_and(|c| Group::of(&c.collection) == g);
                if !keep && let Some(i) = self.colls.iter().position(|c| Group::of(&c.collection) == g) {
                    self.coll_sel = i;
                }
                if let Some(c) = self.colls.get(self.coll_sel) {
                    self.coll_key = Some(c.collection.key());
                }
                self.go(Screen::Collections, now);
                self.home_sync(st, now);
            }
        }
    }

    fn all_index(&self) -> Option<usize> {
        self.colls.iter().position(|c| c.collection.kind == CollectionKind::All)
    }

    /// Y: search the whole library.
    pub(super) fn search_all(&mut self, now: f64, lib: &Library) {
        if let Some(i) = self.all_index() {
            self.coll_sel = i;
            self.coll_key = Some(self.colls[i].collection.key());
            self.enter_collection(lib, i, now);
            self.open_search(now);
        }
    }

    /// A on a home-page cover: its details, with its row's collection as the list (so
    /// LB / RB step through it); B comes back to the home page.
    fn open_from_home(&mut self, st: &AppState, row: usize, col: usize, now: f64) {
        let Some((ci, games)) = self.home.rows.get(row) else { return };
        let ci = *ci;
        let Some(id) = games.get(col).and_then(|&gi| st.library.games.get(gi)).map(|g| g.id) else {
            return;
        };
        let Some(key) = self.colls.get(ci).map(|c| c.collection.key()) else { return };
        self.cur = Some(ci);
        self.coll_sel = ci;
        self.coll_key = Some(key.clone());
        self.search.clear();
        self.list_key = None;
        self.remembered.insert(key, id);
        self.refresh_list(&st.library);
        if let Some(p) = self.list_ids.iter().position(|&g| g == id) {
            self.game_sel = p;
        }
        self.renderer.reset_view(ViewKind::Games);
        self.open_details(st, now);
        self.home.details_from_home = true;
    }

    /// Navigation for rail themes. Returns true when the event was handled here.
    pub(super) fn home_nav(&mut self, st: &mut AppState, nav: Nav, screen: Rect, now: f64) -> bool {
        self.home_sync(st, now);
        let kind = self.view_kind();
        if kind == ViewKind::Details {
            return false;
        }
        let rail = self.rail_info(st, kind);
        if self.home.rail_focus
            && let Some(info) = &rail
        {
            let n = info.keys.len();
            let sel = self.home.rail_sel.min(n.saturating_sub(1));
            match nav {
                Nav::Up => self.home.rail_sel = sel.saturating_sub(1),
                Nav::Down => self.home.rail_sel = (sel + 1).min(n.saturating_sub(1)),
                Nav::JumpUp | Nav::PageUp => self.home.rail_sel = 0,
                Nav::JumpDown | Nav::PageDown => self.home.rail_sel = n.saturating_sub(1),
                Nav::Accept => {
                    if let Some(k) = info.keys.get(sel).cloned() {
                        self.rail_activate(st, &k, now);
                    }
                }
                Nav::Right | Nav::Back => self.home.rail_focus = false,
                Nav::Search => {
                    self.home.rail_focus = false;
                    self.search_all(now, &st.library);
                }
                Nav::Select => self.open_quick_menu(st, now),
                _ => {}
            }
            return true;
        }
        match self.screen {
            Screen::Home => {
                self.rows_nav(st, nav, now, rail.is_some());
                true
            }
            Screen::Collections if rail.is_some() => {
                self.section_nav(st, nav, screen, now);
                true
            }
            Screen::Games if rail.is_some() => {
                let (fk, _) = self.focus_of(st, ViewKind::Games, screen);
                if nav == Nav::Left && at_left_edge(fk, self.game_sel) {
                    self.remember();
                    self.focus_rail(st);
                    return true;
                }
                let in_all =
                    self.cur.and_then(|i| self.colls.get(i)).is_some_and(|c| Group::of(&c.collection) == Group::All);
                if nav == Nav::Back && self.search.is_empty() && in_all {
                    // "All games" is opened from the rail: back to the home page (or the rail).
                    self.remember();
                    if Self::has_home(st) {
                        self.go(Screen::Home, now);
                    } else {
                        self.focus_rail(st);
                    }
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn rows_nav(&mut self, st: &mut AppState, nav: Nav, now: f64, has_rail: bool) {
        let n = self.home.rows.len();
        if n == 0 {
            match nav {
                Nav::Left | Nav::Back if has_rail => self.focus_rail(st),
                Nav::Search => self.search_all(now, &st.library),
                Nav::Select => self.open_quick_menu(st, now),
                _ => {}
            }
            return;
        }
        let row = self.home.row.min(n - 1);
        let m = self.home.rows[row].1.len();
        let col = self.home.cols.get(row).copied().unwrap_or(0).min(m.saturating_sub(1));
        let set_col = |s: &mut Self, c: usize| {
            if let Some(slot) = s.home.cols.get_mut(row) {
                *slot = c;
            }
        };
        match nav {
            Nav::Up | Nav::PageUp => self.home.row = row.saturating_sub(1),
            Nav::Down | Nav::PageDown => self.home.row = (row + 1).min(n - 1),
            Nav::Left if col == 0 => {
                if has_rail {
                    self.focus_rail(st);
                }
            }
            Nav::Left => set_col(self, col - 1),
            Nav::Right => set_col(self, (col + 1).min(m.saturating_sub(1))),
            Nav::JumpUp => set_col(self, 0),
            Nav::JumpDown => set_col(self, m.saturating_sub(1)),
            Nav::Accept => self.open_from_home(st, row, col, now),
            Nav::Back if has_rail => self.focus_rail(st),
            Nav::Favorite => {
                if let Some(id) = self.home_game(st).map(|g| g.id) {
                    st.toggle_favorite(id);
                }
            }
            Nav::Search => self.search_all(now, &st.library),
            Nav::Select => {
                if let Some(id) = self.home_game(st).map(|g| g.id) {
                    self.open_game_options(st, id, now);
                }
            }
            _ => {}
        }
        self.remember_home(st);
    }

    /// The collections page of one rail section (Platforms, Sources …).
    fn section_nav(&mut self, st: &mut AppState, nav: Nav, screen: Rect, now: f64) {
        let map = self.home.group_map.clone();
        let n = map.len();
        let gsel = map.iter().position(|&i| i == self.coll_sel).unwrap_or(0);
        let (fk, page) = self.focus_of(st, ViewKind::Collections, screen);
        match nav {
            Nav::Left if at_left_edge(fk, gsel) => self.focus_rail(st),
            Nav::Back => {
                if Self::has_home(st) {
                    self.go(Screen::Home, now);
                } else {
                    self.focus_rail(st);
                }
            }
            Nav::Accept => {
                if n > 0 {
                    self.enter_collection(&st.library, self.coll_sel, now);
                }
            }
            Nav::Search => self.search_all(now, &st.library),
            Nav::Select => self.open_quick_menu(st, now),
            Nav::JumpUp if n > 0 => self.coll_sel = map[0],
            Nav::JumpDown if n > 0 => self.coll_sel = map[n - 1],
            _ => {
                if let Some(i) = step(fk, nav, gsel, n, page, false) {
                    self.coll_sel = map[i];
                }
            }
        }
        if let Some(c) = self.colls.get(self.coll_sel) {
            self.coll_key = Some(c.collection.key());
        }
    }

    pub(super) fn home_hints(&self, st: &AppState) -> Option<Vec<Hint>> {
        if self.home.rail_focus {
            return Some(vec![
                Hint::new(Button::A, "Open"),
                Hint::new(Button::B, "Back"),
                Hint::new(Button::Y, "Search"),
                Hint::new(Button::Start, "Menu"),
            ]);
        }
        match self.screen {
            Screen::Home => {
                let fav = self.home_game(st).is_some_and(|g| g.favorite);
                Some(vec![
                    Hint::new(Button::A, "Details"),
                    Hint::new(Button::X, if fav { "Unfavourite" } else { "Favourite" }),
                    Hint::new(Button::Y, "Search"),
                    Hint::new(Button::Select, "Options"),
                    Hint::new(Button::Start, "Menu"),
                ])
            }
            Screen::Collections if self.home.group.is_some() => Some(vec![
                Hint::new(Button::A, "Open"),
                Hint::new(Button::B, "Back"),
                Hint::new(Button::Y, "Search"),
                Hint::new(Button::Start, "Menu"),
            ]),
            _ => None,
        }
    }

    /// Draw the home page (rail, hero, cover rows) and handle its mouse input.
    pub(super) fn draw_home(&mut self, ui: &mut egui::Ui, st: &mut AppState, pass: &ScreenPass) {
        let ScreenPass { screen, hints, opacity, interactive, now, .. } = *pass;
        self.home_sync(st, now);
        let theme = st.controller_theme.clone();
        let Some(p) = self.renderer.prepare(&theme, ViewKind::Home, None, st.config.controller.variant.as_deref())
        else {
            return;
        };
        let rail = self.rail_info(st, ViewKind::Home);
        let row = self.home.row.min(self.home.rows.len().saturating_sub(1));
        let rows: Vec<RowScene> = self
            .home
            .rows
            .iter()
            .zip(&self.home.cols)
            .filter_map(|((ci, games), &col)| {
                Some(RowScene { collection: self.colls.get(*ci)?, indices: games, selected: col })
            })
            .collect();
        let (coll, indices, col) =
            rows.get(row).map(|r| (Some(r.collection), r.indices, r.selected)).unwrap_or((None, &[], 0));
        let game = indices.get(col).and_then(|&gi| st.library.games.get(gi));
        let scene = Scene {
            kind: ViewKind::Home,
            game,
            collection: coll,
            items: Items::Games { games: &st.library.games, indices },
            selected: col,
            list_id: self.home.rows_id,
            hints,
            prompts: st.prompts(),
            opacity,
            interactive,
            extra: SceneExtra {
                games: &st.library.games,
                rows: &rows,
                row,
                rail: rail.as_ref().map(|i| self.rail_scene(i)),
                section: Some("Home"),
            },
        };
        let out = self.renderer.draw(ui, screen, &theme, &p, &st.fonts, &mut st.images, &scene);
        drop(rows);
        if let (Some(j), Some(info)) = (out.rail_clicked, &rail) {
            if let Some(k) = info.keys.get(j).cloned() {
                self.rail_activate(st, &k, now);
            }
            return;
        }
        if let Some((r, c)) = out.row_clicked {
            self.home.rail_focus = false;
            if r == self.home.row && self.home.cols.get(r) == Some(&c) {
                self.open_from_home(st, r, c, now);
            } else {
                self.home.row = r;
                if let Some(slot) = self.home.cols.get_mut(r) {
                    *slot = c;
                }
            }
            self.remember_home(st);
        } else if out.scroll_steps != 0 && !self.home.rows.is_empty() {
            let n = self.home.rows.len() as i64;
            self.home.row = (self.home.row as i64 + out.scroll_steps as i64).clamp(0, n - 1) as usize;
            self.remember_home(st);
        }
    }
}

/// Left from here leaves the list (onto the rail): the first column of a grid, the first item
/// of a horizontal strip; vertical lists don't use Left at all.
pub(super) fn at_left_edge(kind: FocusKind, sel: usize) -> bool {
    match kind {
        FocusKind::Grid { columns } => sel.is_multiple_of(columns.max(1)),
        FocusKind::Carousel { vertical: false } => sel == 0,
        FocusKind::Carousel { vertical: true } | FocusKind::List | FocusKind::None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(id: i64, name: &str, platform: Option<&str>, played: Option<i64>) -> Game {
        Game {
            id,
            source: if platform.is_some() { Source::Emulator } else { Source::Steam },
            source_id: id.to_string(),
            emu: platform.map(|p| crate::model::EmuRef { platform: p.into(), ..Default::default() }),
            last_played: played,
            ..Game::blank(name)
        }
    }

    fn setup() -> (tempfile::TempDir, AppState, ControllerUi) {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        st.set_controller_theme("Fullscreen");
        st.library.replace(vec![
            game(1, "Alpha", None, Some(1_700_000_300)),
            game(2, "Bravo", None, Some(1_700_000_200)),
            game(3, "Charlie", Some("sony_playstation2"), Some(1_700_000_100)),
            game(4, "Delta", Some("sony_playstation2"), None),
            game(5, "Echo", None, None),
        ]);
        let mut ui = ControllerUi::new();
        ui.refresh_collections(&st.library);
        ui.home_sync(&st, 0.0);
        (dir, st, ui)
    }

    fn nav(ui: &mut ControllerUi, st: &mut AppState, navs: &[Nav]) {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0));
        for &n in navs {
            ui.handle_nav(st, n, screen, 1.0, (false, false));
            ui.refresh_list(&st.library);
        }
    }

    fn row_names(ui: &ControllerUi) -> Vec<String> {
        ui.home.rows.iter().map(|(ci, _)| ui.colls[*ci].name.clone()).collect()
    }

    #[test]
    fn home_rows_follow_the_theme() {
        let (_d, st, ui) = setup();
        assert_eq!(ui.screen, Screen::Home, "a theme with a home page starts there");
        // Recent, (no favourites), the console, the store, (no collections), all games.
        assert_eq!(row_names(&ui), ["Continue Playing", "PlayStation 2", "Steam", "Emulators", "All Games"]);
        assert_eq!(ui.home_game(&st).map(|g| g.name.as_str()), Some("Alpha"));
        // Without a home page the theme starts on the collections screen as before.
        let (_d2, mut st2, mut ui2) = setup();
        st2.set_controller_theme("Lodestar");
        ui2.home_sync(&st2, 0.0);
        assert_eq!(ui2.screen, Screen::Collections);
        assert!(ui2.rail_info(&st2, ViewKind::Collections).is_none());
    }

    #[test]
    fn games_first_themes_open_all_games_once() {
        let (_d, mut st, mut ui) = setup();
        let mut t = (*st.themes.load("Lodestar")).clone();
        t.key = "games-first".into();
        t.file.theme.start = "games".into();
        let games_first = Arc::new(t);
        st.controller_theme = games_first.clone();
        ui.home_sync(&st, 0.0);
        assert_eq!(ui.screen, Screen::Games, "opens on the game list");
        assert_eq!(ui.cur.map(|i| ui.colls[i].collection.key()), Some("all".to_string()));
        assert_eq!(ui.first_screen(&st), Screen::Games);
        // B goes to the collections, and later frames leave the user there.
        ui.screen = Screen::Collections;
        ui.home_sync(&st, 0.0);
        assert_eq!(ui.screen, Screen::Collections);
        // Previewing other themes in the theme menu and cancelling back doesn't reopen it.
        ui.theme_preview = Some(ThemePreview { original: games_first.clone(), cache: HashMap::new() });
        st.controller_theme = st.themes.load("Lodestar");
        ui.home_sync(&st, 0.0);
        st.controller_theme = games_first;
        ui.end_theme_preview(&mut st, false);
        ui.home_sync(&st, 0.0);
        assert_eq!(ui.screen, Screen::Collections);
    }

    #[test]
    fn rows_move_and_remember_their_place() {
        let (_d, mut st, mut ui) = setup();
        nav(&mut ui, &mut st, &[Nav::Right, Nav::Right]);
        assert_eq!(ui.home_game(&st).map(|g| g.name.as_str()), Some("Charlie"));
        nav(&mut ui, &mut st, &[Nav::Down]);
        assert_eq!(ui.home.row, 1);
        assert_eq!(ui.home_game(&st).map(|g| g.name.as_str()), Some("Charlie"), "first cover of the console row");
        nav(&mut ui, &mut st, &[Nav::Up]);
        assert_eq!(ui.home_game(&st).map(|g| g.name.as_str()), Some("Charlie"), "back where it was");
        // A rebuild (rescan, theme change) keeps the row and the cover.
        ui.home.built = None;
        ui.home_sync(&st, 0.0);
        assert_eq!((ui.home.row, ui.home_game(&st).map(|g| g.id)), (0, Some(3)));
        // Right stops at the end of the row.
        nav(&mut ui, &mut st, &[Nav::Right, Nav::Right, Nav::Right]);
        assert_eq!(ui.home_game(&st).map(|g| g.name.as_str()), Some("Charlie"));
    }

    #[test]
    fn details_from_home_and_back() {
        let (_d, mut st, mut ui) = setup();
        nav(&mut ui, &mut st, &[Nav::Right, Nav::Accept]);
        assert_eq!(ui.screen, Screen::Details);
        assert_eq!(ui.details.as_ref().map(|d| d.game), Some(2));
        // LB / RB walk the row's collection.
        assert_eq!(ui.cur.map(|i| ui.colls[i].collection.kind.clone()), Some(CollectionKind::Recent));
        nav(&mut ui, &mut st, &[Nav::Back]);
        assert_eq!(ui.screen, Screen::Home);
        assert_eq!(ui.home_game(&st).map(|g| g.id), Some(2));
    }

    #[test]
    fn rail_opens_sections() {
        let (_d, mut st, mut ui) = setup();
        // Left at the first cover (or B) moves onto the rail, on the open section.
        nav(&mut ui, &mut st, &[Nav::Left]);
        assert!(ui.home.rail_focus);
        let info = ui.rail_info(&st, ViewKind::Home).unwrap();
        // No user collections in this library: that section is left out.
        assert_eq!(info.keys, ["home", "all", "platforms", "sources", "filters"]);
        assert_eq!(ui.rail_scene(&info).current, Some(0));
        // Right goes back to the covers without changing anything.
        nav(&mut ui, &mut st, &[Nav::Right]);
        assert!(!ui.home.rail_focus && ui.screen == Screen::Home);
        // All games: the grid; B returns home.
        nav(&mut ui, &mut st, &[Nav::Back, Nav::Down, Nav::Accept]);
        assert_eq!(ui.screen, Screen::Games);
        assert_eq!(ui.list.len(), 5);
        nav(&mut ui, &mut st, &[Nav::Back]);
        assert_eq!(ui.screen, Screen::Home);
        // Platforms: the section page lists PC Games and the consoles; A opens one, B goes back up.
        nav(&mut ui, &mut st, &[Nav::Back, Nav::Down, Nav::Down, Nav::Accept]);
        assert_eq!(ui.screen, Screen::Collections);
        assert_eq!(ui.home.group, Some(Group::Platforms));
        let names: Vec<&str> = ui.home.group_items.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["PC Games", "PlayStation 2"]);
        nav(&mut ui, &mut st, &[Nav::Right, Nav::Accept]);
        assert_eq!((ui.screen, ui.list.len()), (Screen::Games, 2));
        // Left on the first column of the grid reaches the rail from the grid page too.
        nav(&mut ui, &mut st, &[Nav::Left]);
        assert!(ui.home.rail_focus);
        nav(&mut ui, &mut st, &[Nav::Right, Nav::Back]);
        assert_eq!(ui.screen, Screen::Collections);
        nav(&mut ui, &mut st, &[Nav::Back]);
        assert_eq!(ui.screen, Screen::Home);
        // Filters: Continue Playing and All-style smart lists, not consoles or stores.
        nav(&mut ui, &mut st, &[Nav::Left, Nav::JumpDown, Nav::Accept]);
        assert_eq!(ui.home.group, Some(Group::Filters));
        assert!(ui.home.group_items.iter().all(|c| Group::of(&c.collection) == Group::Filters));
    }

    /// The game menu of the cover under the home page's focus.
    fn options_of_home_game(ui: &mut ControllerUi, st: &mut AppState) -> Vec<String> {
        nav(ui, st, &[Nav::Select]);
        ui.menus.last().map(|m| m.items.iter().map(|i| i.label.clone()).collect()).unwrap_or_default()
    }

    #[test]
    fn hiding_from_a_details_page_opened_from_home_goes_home() {
        let (_d, mut st, mut ui) = setup();
        // Bravo's details from the Continue Playing row; its game menu → Hide game (twice).
        nav(&mut ui, &mut st, &[Nav::Right, Nav::Accept]);
        assert_eq!((ui.screen, ui.home.details_from_home), (Screen::Details, true));
        ui.open_game_options(&st, 2, 1.0);
        ui.menu_act(&mut st, Act::Hide(2), 1.0);
        ui.menu_act(&mut st, Act::Hide(2), 1.5);
        assert!(st.library.get(2).is_some_and(|g| g.hidden));
        assert_eq!(ui.screen, Screen::Home, "back where the page was opened, not the row's grid");
        // The page itself also steps back home when its game leaves the list some other way.
        nav(&mut ui, &mut st, &[Nav::Accept]);
        assert_eq!(ui.screen, Screen::Details);
        let id = ui.details.as_ref().unwrap().game;
        st.set_hidden(id, true);
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(Default::default(), |ui_| {
            let look = Look::new(&st, ui_.max_rect());
            let pass = ScreenPass {
                screen: ui_.max_rect(),
                hints: &[],
                opacity: 1.0,
                interactive: false,
                now: 2.0,
                dt: 0.016,
            };
            let _ = ui.draw_details(ui_, &mut st, &look, &pass);
        });
        assert_eq!(ui.screen, Screen::Home);
    }

    #[test]
    fn home_game_menu_opens_the_right_details_and_has_no_sort() {
        let (_d, mut st, mut ui) = setup();
        // An earlier grid left a list behind (All games, Alpha selected).
        nav(&mut ui, &mut st, &[Nav::Left, Nav::Down, Nav::Accept, Nav::Back]);
        assert_eq!(ui.screen, Screen::Home);
        // Home: the console row's second game (Delta).
        nav(&mut ui, &mut st, &[Nav::Down, Nav::Right]);
        assert_eq!(ui.home_game(&st).map(|g| g.id), Some(4));
        let labels = options_of_home_game(&mut ui, &mut st);
        assert!(labels.iter().any(|l| l == "Show details"));
        assert!(!labels.iter().any(|l| l == "Sort by"), "sorting changes nothing on the home page: {labels:?}");
        ui.menu_act(&mut st, Act::ShowDetails(4), 2.0);
        assert_eq!(ui.details.as_ref().map(|d| d.game), Some(4), "the cover's game, not the old grid's");
        assert!(ui.home.details_from_home);
        // Y there searches the whole library, as on the home page.
        nav(&mut ui, &mut st, &[Nav::Search]);
        assert_eq!(ui.cur.map(|i| ui.colls[i].collection.kind.clone()), Some(CollectionKind::All));
        assert!(ui.osk.is_some());
    }

    #[test]
    fn random_game_from_home_comes_back_home() {
        let (_d, mut st, mut ui) = setup();
        // A one-game grid left behind: Random must not be stuck on it.
        nav(&mut ui, &mut st, &[Nav::Left, Nav::Down, Nav::Down, Nav::Accept, Nav::Accept, Nav::Back, Nav::Back]);
        assert_eq!(ui.screen, Screen::Home);
        ui.random_game(&mut st, 3.0);
        assert_eq!(ui.screen, Screen::Details);
        assert_eq!(ui.list_ids.len(), 5, "picked from all games");
        nav(&mut ui, &mut st, &[Nav::Back]);
        assert_eq!(ui.screen, Screen::Home);
    }

    #[test]
    fn in_game_library_starts_at_home_and_b_returns_to_the_menu() {
        let (_d, mut st, mut ui) = setup();
        st.no_launch = true;
        // The menu button question was answered already (else Home asks first).
        st.config.controller.menu_button_asked = true;
        st.mode = Mode::Controller;
        nav(&mut ui, &mut st, &[Nav::Right, Nav::Accept]);
        let _fake = crate::ui::launching::FAKE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(crate::ui::launching::debug_fake(&st, 2, "starting"));
        ui.on_guide(&mut st, true, 2.0);
        assert_eq!(ui.sys.menus.last().map(|m| m.kind.clone()), Some(MenuKind::InGame));
        // Library.
        ui.sys_nav(&mut st, Nav::Down, 2.0);
        ui.sys_nav(&mut st, Nav::Accept, 2.0);
        assert!(ui.sys.library && ui.sys.menus.is_empty());
        assert_eq!(ui.screen, Screen::Home, "the theme's first page");
        assert!(ui.hints(&st).iter().any(|h| h.button == Button::B && h.label == "In-game menu"));
        nav(&mut ui, &mut st, &[Nav::Back]);
        assert_eq!(ui.sys.menus.last().map(|m| m.kind.clone()), Some(MenuKind::InGame));
        crate::ui::launching::stop(&mut st, 2);
    }

    #[test]
    fn edges_of_lists() {
        assert!(at_left_edge(FocusKind::Grid { columns: 7 }, 14));
        assert!(!at_left_edge(FocusKind::Grid { columns: 7 }, 15));
        assert!(at_left_edge(FocusKind::Carousel { vertical: false }, 0));
        assert!(!at_left_edge(FocusKind::Carousel { vertical: false }, 1));
        assert!(at_left_edge(FocusKind::List, 5));
    }
}
