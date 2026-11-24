//! Controller (fullscreen, themed) UI: Collections → Games → Details, driven entirely by
//! `input::Nav` events (gamepad + keyboard), with quick menu, game options, on-screen keyboard
//! search, launch-option editing and a "Playing" overlay.
//!
//! Rendering of the Collections and Games screens (and Details, when the theme has one) goes
//! through `theme::render::Renderer`; menus, keyboard and the fallback details page are drawn
//! here with the theme's palette and fonts.

mod act;
pub(crate) mod combo;
mod credits;
mod details;
mod draw;
mod emu;
mod home;
mod ingame;
mod ingame_sys;
mod items;
mod lists;
mod look;
mod menu;
mod osk;
mod power;
mod prompt;
pub(crate) mod settings;
mod sounds;
mod viewer;

use crate::app::{AppState, Mode};
use crate::config::SortBy;
use crate::input::Nav;
use crate::library::{Collection, CollectionKind, Library};
use crate::model::*;
use crate::theme::render::draw::{self as rd, Button, Hint, fade};
use crate::theme::render::{CollectionItem, FocusKind, Items, Prepared, Renderer, Scene, SceneExtra};
use crate::theme::{Theme, ViewKind};
use details::DetailsState;
use egui::{Color32, Rect, pos2, vec2};
use look::Look;
use menu::{Act, Item, Menu, MenuKind};
use osk::{Osk, OskResult, Purpose};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use lists::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    /// The theme's home page (themes with a `[home]` view; Playnite Fullscreen style).
    Home,
    Collections,
    Games,
    Details,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ListKey {
    generation: u64,
    coll: String,
    sort: Option<SortBy>,
    descending: bool,
    search: String,
}

struct ThemePreview {
    original: Arc<Theme>,
    cache: HashMap<String, Arc<Theme>>,
}

/// What the collections were built for: library generation, `show_hidden`.
type CollsSig = (u64, bool);

pub struct ControllerUi {
    renderer: Renderer,
    screen: Screen,
    /// Collections + their (sorted) game indices, memoised per library generation.
    colls: Vec<CollectionItem>,
    colls_games: Vec<Vec<usize>>,
    /// Library generation + `show_hidden` the collections were built for.
    colls_gen: Option<CollsSig>,
    coll_sel: usize,
    /// Key of the selected collection (survives regeneration).
    coll_key: Option<String>,
    /// Collection shown in the Games screen (index into `colls`).
    cur: Option<usize>,
    /// The Games screen's list: indices into `library.games` as of the last `refresh_list`.
    /// Only valid for that library; `list_ids` (same order) is the source of truth across a
    /// rescan, which can shrink or reorder `library.games` under an open list.
    list: Vec<usize>,
    list_ids: Vec<GameId>,
    list_key: Option<ListKey>,
    list_id: u64,
    game_sel: usize,
    /// Selected game per collection key.
    remembered: HashMap<String, GameId>,
    sort: Option<SortBy>,
    descending: bool,
    search: String,
    details: Option<DetailsState>,
    menus: Vec<Menu>,
    osk: Option<Osk>,
    theme_preview: Option<ThemePreview>,
    /// Screen transition start.
    transition: f64,
    letter: Option<(String, f64)>,
    /// Running game, its start (unix secs) and when this UI first saw it (ui time).
    running: Option<(GameId, i64, f64)>,
    stop_armed: Option<f64>,
    /// "Show hidden games" (quick menu, this session): adds a Hidden Games collection.
    show_hidden: bool,
    /// "Hide game" was pressed once for this game at this time; a second press confirms.
    hide_armed: Option<(GameId, f64)>,
    /// Screenshot runs: `LODESTAR_CONTROLLER_OPEN=starting|switch|close` fakes that launch state for
    /// the game whose details page opens first (see ui/launching.rs).
    debug_launch: Option<String>,
    /// B put the Playing overlay away for the current running game (browse the library).
    overlay_hidden: bool,
    /// Button prompt style drawn last frame: the quick menu's "Automatic (…)" follows a change.
    prompt_style: Option<crate::input::ButtonStyle>,
    /// The artwork viewer (details page → Artwork, or the game menu's "View artwork").
    viewer: Option<viewer::CtlViewer>,
    /// Home page, navigation rail and section pages of rail themes (home.rs).
    home: home::HomeState,
    /// The menu button's in-game menu and screens, clock / battery, the mouse pointer
    /// (ingame.rs, settings.rs).
    sys: ingame::SysState,
}

/// Presses of A right after a launch (a double-tap on Play) must not arm "Quit game".
const STOP_GRACE_SECS: f64 = 1.0;
/// A second A press within this time confirms "Quit game".
const STOP_CONFIRM_SECS: f64 = 3.0;
/// A second press of "Hide game" within this time confirms it.
const HIDE_CONFIRM_SECS: f64 = 3.0;
/// Tag of the virtual "Hidden Games" collection. Real tags are user text, never a control
/// character, so its key (`tag:\u{1}hidden`) can't collide with one.
const HIDDEN_TAG: &str = "\u{1}hidden";

impl Default for ControllerUi {
    fn default() -> Self {
        ControllerUi::new()
    }
}

fn hash_of(v: impl Hash) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

/// Move a selection in a focused element. Returns the new index (clamped).
pub(crate) fn step(kind: FocusKind, nav: Nav, sel: usize, n: usize, page: usize, wrap: bool) -> Option<usize> {
    if n == 0 {
        return None;
    }
    let sel = sel.min(n - 1);
    let (back, fwd) = match kind {
        FocusKind::Carousel { vertical: false } => (nav == Nav::Left, nav == Nav::Right),
        FocusKind::Carousel { vertical: true } | FocusKind::List => (nav == Nav::Up, nav == Nav::Down),
        FocusKind::Grid { .. } => (nav == Nav::Left, nav == Nav::Right),
        FocusKind::None => (matches!(nav, Nav::Left | Nav::Up), matches!(nav, Nav::Right | Nav::Down)),
    };
    if back {
        return Some(if sel == 0 { if wrap { n - 1 } else { 0 } } else { sel - 1 });
    }
    if fwd {
        return Some(if sel + 1 >= n { if wrap { 0 } else { n - 1 } } else { sel + 1 });
    }
    if let FocusKind::Grid { columns } = kind {
        let c = columns.max(1);
        match nav {
            Nav::Up => return Some(if sel >= c { sel - c } else { sel }),
            Nav::Down => {
                if sel + c < n {
                    return Some(sel + c);
                }
                // Partial last row: go to the last item if it is on a lower row.
                return Some(if sel / c < (n - 1) / c { n - 1 } else { sel });
            }
            _ => {}
        }
    }
    match nav {
        Nav::PageUp => Some(sel.saturating_sub(page.max(1))),
        Nav::PageDown => Some((sel + page.max(1)).min(n - 1)),
        _ => None,
    }
}

/// What the current screen is drawn with this frame.
#[derive(Clone, Copy)]
struct ScreenPass<'a> {
    screen: Rect,
    hints: &'a [Hint],
    /// The screen transition's fade-in.
    opacity: f32,
    /// False while a menu, the keyboard or an overlay covers the screen: no clicks then.
    interactive: bool,
    now: f64,
    dt: f32,
}

impl ControllerUi {
    pub fn new() -> ControllerUi {
        ControllerUi {
            renderer: Renderer::new(),
            screen: Screen::Collections,
            colls: vec![],
            colls_games: vec![],
            colls_gen: None,
            coll_sel: 0,
            coll_key: None,
            cur: None,
            list: vec![],
            list_ids: vec![],
            list_key: None,
            list_id: 0,
            game_sel: 0,
            remembered: HashMap::new(),
            sort: None,
            descending: false,
            search: String::new(),
            details: None,
            menus: vec![],
            osk: None,
            theme_preview: None,
            transition: -10.0,
            letter: None,
            running: None,
            stop_armed: None,
            show_hidden: false,
            hide_armed: None,
            debug_launch: std::env::var("LODESTAR_CONTROLLER_OPEN").ok().filter(|v| !v.is_empty()),
            overlay_hidden: false,
            prompt_style: None,
            viewer: None,
            home: Default::default(),
            sys: Default::default(),
        }
    }

    // ------------------------------------------------------------------ data

    /// Id of the selected game in the Games list.
    fn sel_id(&self) -> Option<GameId> {
        self.list_ids.get(self.game_sel).copied()
    }

    fn selected_game<'a>(&self, st: &'a AppState) -> Option<&'a Game> {
        match self.screen {
            Screen::Details => self.details.as_ref().and_then(|d| st.library.get(d.game)),
            Screen::Games => self.sel_id().and_then(|id| st.library.get(id)),
            Screen::Home => self.home_game(st),
            Screen::Collections => None,
        }
    }

    fn remember(&mut self) {
        if let (Some(k), Some(id)) = (self.cur_key(), self.sel_id()) {
            self.remembered.insert(k, id);
        }
    }

    /// What is on screen, for the latency probe ("Games", "Games/Quick menu" …).
    pub fn probe_where(&self) -> String {
        let base = format!("{:?}", self.screen);
        let top = if self.osk.is_some() {
            Some("Keyboard".to_string())
        } else if self.viewer.is_some() {
            Some("Viewer".to_string())
        } else {
            self.sys.menus.last().or(self.menus.last()).map(|m| match &m.kind {
                MenuKind::Settings => format!("Settings:{}", m.title),
                k => format!("{k:?}").split('(').next().unwrap_or("Menu").to_string(),
            })
        };
        match top {
            Some(t) => format!("{base}/{t}"),
            None => base,
        }
    }

    // ------------------------------------------------------------------ navigation

    fn go(&mut self, screen: Screen, now: f64) {
        if self.screen != screen {
            self.screen = screen;
            self.transition = now;
        }
    }

    fn enter_collection(&mut self, lib: &Library, i: usize, now: f64) {
        if i >= self.colls.len() {
            return;
        }
        self.cur = Some(i);
        self.search.clear();
        self.list_key = None;
        self.refresh_list(lib);
        self.renderer.reset_view(ViewKind::Games);
        self.go(Screen::Games, now);
    }

    fn open_details(&mut self, st: &AppState, now: f64) {
        if let Some(id) = self.sel_id().filter(|&id| st.library.get(id).is_some()) {
            self.home.details_from_home = false;
            self.details = Some(DetailsState::new(id));
            self.renderer.reset_view(ViewKind::Details);
            self.go(Screen::Details, now);
        }
    }

    /// Where B (or a game leaving the list, e.g. hidden) goes from the details page: the home
    /// page it was opened from, else the game list.
    fn details_back(&self) -> Screen {
        if self.home.details_from_home { Screen::Home } else { Screen::Games }
    }

    fn focus_of(&mut self, st: &AppState, kind: ViewKind, screen: Rect) -> (FocusKind, usize) {
        let theme = st.controller_theme.clone();
        let coll = match kind {
            ViewKind::Collections => self.colls.get(self.coll_sel).map(|c| c.collection.clone()),
            _ => self.cur.and_then(|i| self.colls.get(i)).map(|c| c.collection.clone()),
        };
        match self.renderer.prepare(&theme, kind, coll.as_ref(), st.config.controller.variant.as_deref()) {
            Some(p) => {
                let fi = p.focus_info(screen);
                (fi.kind, fi.page)
            }
            None => (FocusKind::None, 10),
        }
    }

    fn handle_nav(&mut self, st: &mut AppState, nav: Nav, screen: Rect, now: f64, kb: (bool, bool)) {
        if self.overlay_nav(st, nav, now, kb) {
            return;
        }
        match nav {
            Nav::Menu => {
                self.open_quick_menu(st, now);
                return;
            }
            // Menu button presses normally go through `on_guide` (which also closes the menu);
            // one that reaches the library here opens it, as Start does.
            Nav::Guide => {
                self.open_quick_menu(st, now);
                return;
            }
            _ => {}
        }
        if self.colls.is_empty() {
            // Empty library: A rescans right here (the screen says how).
            if nav == Nav::Accept && !st.scan.running {
                st.rescan();
                st.notify("Rescanning library…");
            }
            return;
        }
        if self.home_nav(st, nav, screen, now) {
            return;
        }
        match self.screen {
            Screen::Home => {}
            Screen::Collections => self.collections_nav(st, nav, screen, now),
            Screen::Games => self.games_nav(st, nav, screen, now),
            Screen::Details => self.details_nav(st, nav, screen, now),
        }
    }

    /// What covers the library gets the press: the in-game menu and the menu button screens,
    /// the "another game is running" prompt, the artwork viewer, the keyboard, menus, the
    /// Playing overlay. False when none is shown.
    fn overlay_nav(&mut self, st: &mut AppState, nav: Nav, now: f64, kb: (bool, bool)) -> bool {
        if self.sys_nav(st, nav, now) {
            return true;
        }
        // The "another game is running" prompt sits on top of all the others.
        if let Some(p) = crate::ui::launching::prompt(st) {
            use crate::ui::launching::{PromptAnswer, answer_prompt};
            match nav {
                Nav::Accept => answer_prompt(st, p.kind, PromptAnswer::Primary),
                Nav::Favorite if p.alt.is_some() => answer_prompt(st, p.kind, PromptAnswer::Alt),
                Nav::Back => answer_prompt(st, p.kind, PromptAnswer::Cancel),
                _ => {}
            }
            return true;
        }
        if let Some(v) = &mut self.viewer {
            if !v.nav(nav) {
                self.viewer = None;
            }
            return true;
        }
        if self.osk.is_some() {
            let r = self.osk.as_mut().map(|o| o.nav(nav, kb.0, kb.1)).unwrap_or(OskResult::None);
            self.osk_result(st, r, now);
            return true;
        }
        if !self.menus.is_empty() {
            self.menu_nav(st, nav, now);
            return true;
        }
        if self.overlay_shown() {
            self.playing_nav(st, nav, now);
            return true;
        }
        false
    }

    fn collections_nav(&mut self, st: &mut AppState, nav: Nav, screen: Rect, now: f64) {
        let n = self.colls.len();
        let (kind, page) = self.focus_of(st, ViewKind::Collections, screen);
        match nav {
            Nav::Accept => self.enter_collection(&st.library, self.coll_sel, now),
            Nav::Search => {
                let all = self.colls.iter().position(|c| c.collection.kind == CollectionKind::All);
                if let Some(i) = all {
                    self.coll_sel = i;
                    self.coll_key = Some(self.colls[i].collection.key());
                    self.enter_collection(&st.library, i, now);
                    self.open_search(now);
                }
            }
            Nav::Select => self.open_quick_menu(st, now),
            Nav::JumpUp => self.coll_sel = 0,
            Nav::JumpDown => self.coll_sel = n.saturating_sub(1),
            _ => {
                if let Some(i) = step(kind, nav, self.coll_sel, n, page, true) {
                    self.coll_sel = i;
                }
            }
        }
        if let Some(c) = self.colls.get(self.coll_sel) {
            self.coll_key = Some(c.collection.key());
        }
    }

    fn games_nav(&mut self, st: &mut AppState, nav: Nav, screen: Rect, now: f64) {
        let n = self.list.len();
        let (kind, page) = self.focus_of(st, ViewKind::Games, screen);
        match nav {
            Nav::Accept => self.open_details(st, now),
            Nav::Back => {
                if !self.search.is_empty() {
                    self.search.clear();
                } else {
                    self.remember();
                    self.go(Screen::Collections, now);
                }
            }
            Nav::Favorite => {
                if let Some(g) = self.selected_game(st).map(|g| g.id) {
                    st.toggle_favorite(g);
                }
            }
            Nav::Search => self.open_search(now),
            Nav::Select => {
                if let Some(id) = self.selected_game(st).map(|g| g.id) {
                    self.open_game_options(st, id, now);
                }
            }
            // A theme that draws filter tabs (All Games, Favourites …): LB / RB switch
            // them, as in Playnite, instead of paging.
            Nav::PageUp | Nav::PageDown if !lists::theme_tab_keys(&st.controller_theme.file).is_empty() => {
                let next = self.tab_step(&st.controller_theme.file, nav == Nav::PageDown);
                if let Some(i) = next.filter(|&i| Some(i) != self.cur) {
                    self.remember();
                    self.coll_sel = i;
                    self.coll_key = Some(self.colls[i].collection.key());
                    self.enter_collection(&st.library, i, now);
                }
            }
            Nav::JumpUp | Nav::JumpDown => self.jump_in_games(st, nav == Nav::JumpDown, now),
            _ => {
                if let Some(i) = step(kind, nav, self.game_sel, n, page, false) {
                    self.game_sel = i;
                }
            }
        }
        self.remember();
    }

    /// LT / RT in the games: the next / previous letter in a list sorted by name (showing the
    /// letter), else the first / last game.
    fn jump_in_games(&mut self, st: &AppState, down: bool, now: f64) {
        let n = self.list.len();
        let name_sorted = self.sort.is_none_or(|s| s == SortBy::Name)
            && self.search.is_empty()
            && !matches!(
                self.cur.and_then(|i| self.colls.get(i)).map(|c| &c.collection.kind),
                Some(CollectionKind::Recent)
            );
        if n == 0 {
            return;
        }
        self.game_sel = if name_sorted {
            jump_letter(&st.library.games, &self.list, self.game_sel, down)
        } else if down {
            n - 1
        } else {
            0
        };
        if name_sorted && let Some(g) = self.sel_id().and_then(|id| st.library.get(id)) {
            self.letter = Some((letter_of(g).to_string(), now));
        }
    }

    fn details_nav(&mut self, st: &mut AppState, nav: Nav, screen: Rect, now: f64) {
        let Some(id) = self.details.as_ref().map(|d| d.game) else {
            self.go(self.details_back(), now);
            return;
        };
        let themed = st.controller_theme.file.details.is_some();
        match nav {
            Nav::Back => self.go(self.details_back(), now),
            Nav::Left if !themed => {
                if let Some(d) = &mut self.details {
                    d.focus = d.focus.saturating_sub(1);
                }
            }
            Nav::Right if !themed => {
                if let Some(d) = &mut self.details {
                    d.focus = (d.focus + 1).min(details::BUTTONS - 1);
                }
            }
            Nav::Up | Nav::Down => {
                if let Some(d) = &mut self.details {
                    let step_px = screen.height() * 0.021 * 1.35 * 3.0;
                    d.scroll_by(if nav == Nav::Up { -step_px } else { step_px });
                }
            }
            Nav::Accept => {
                let focus = if themed { 0 } else { self.details.as_ref().map(|d| d.focus).unwrap_or(0) };
                self.details_button(st, id, focus, now);
            }
            Nav::Favorite => st.toggle_favorite(id),
            Nav::Select => self.open_game_options(st, id, now),
            // Opened from the home page: Y searches the whole library, as it does there.
            Nav::Search if self.home.details_from_home => self.search_all(now, &st.library),
            Nav::Search => {
                self.go(Screen::Games, now);
                self.open_search(now);
            }
            Nav::PageUp | Nav::PageDown | Nav::JumpUp | Nav::JumpDown => {
                // Previous / next game without leaving the details page.
                let n = self.list.len();
                if n > 0 {
                    let fwd = matches!(nav, Nav::PageDown | Nav::JumpDown);
                    self.game_sel = if fwd { (self.game_sel + 1).min(n - 1) } else { self.game_sel.saturating_sub(1) };
                    if let Some(gid) = self.sel_id()
                        && gid != id
                    {
                        let focus = self.details.as_ref().map(|d| d.focus).unwrap_or(0);
                        let mut d = DetailsState::new(gid);
                        d.focus = focus;
                        self.details = Some(d);
                        self.remember();
                    }
                }
            }
            _ => {}
        }
    }

    fn details_button(&mut self, st: &mut AppState, id: GameId, button: usize, now: f64) {
        match button {
            0 => match Self::primary(st, id) {
                // The game the Playing overlay is for: bring the overlay back (A, A stops it
                // there); another one of several running games: stop it.
                details::Primary::Stop | details::Primary::Cancel => {
                    if self.running.map(|r| r.0) == Some(id) {
                        self.overlay_hidden = false;
                        self.stop_armed = None;
                    } else {
                        crate::ui::launching::stop(st, id);
                    }
                }
                details::Primary::Install => Self::install(st, id),
                details::Primary::Play => emu::play_or_choose(self, st, id, now),
            },
            1 => st.toggle_favorite(id),
            2 => self.open_viewer(st, id),
            _ => self.open_game_options(st, id, now),
        }
    }

    /// The artwork viewer for `id` (a note when the game has no artwork).
    fn open_viewer(&mut self, st: &mut AppState, id: GameId) {
        let v = st.library.get(id).and_then(|g| viewer::CtlViewer::open(st.ctx(), g));
        if v.is_none() {
            st.notify("This game has no artwork yet");
        }
        self.viewer = v;
    }

    /// What A does on the details page: Stop (running), Install (not installed and the store
    /// can install it) or Play.
    fn primary(st: &AppState, id: GameId) -> details::Primary {
        let running = crate::ui::launching::is_active(st, id);
        match st.library.get(id) {
            Some(g) => details::Primary::for_game(
                g,
                running,
                crate::ui::launching::starting(st, id),
                !g.installed && st.can_install(id),
            ),
            None => details::Primary::Play,
        }
    }

    /// Hand the game to its store's client. Scripted screenshot runs never open anything
    /// (`install_game` checks too; this keeps the controller path safe on its own).
    fn install(st: &mut AppState, id: GameId) {
        if st.no_launch {
            log::info!("no-launch run: not installing game {id}");
            return;
        }
        st.install_game(id);
    }

    fn uninstall(st: &mut AppState, id: GameId) {
        if st.no_launch {
            log::info!("no-launch run: not uninstalling game {id}");
            return;
        }
        st.uninstall_game(id);
    }

    /// The Playing / Starting overlay shows (a game runs and B hasn't put it away).
    /// While a game runs, the Playing overlay owns the controller unless the user chose to
    /// browse (Home → Library) or one of Lodestar's own menus is open: stray presses meant for
    /// the game can then never reach the library or the quick menu (a focused Lodestar only gets
    /// the overlay's buttons; see input/gate.rs).
    fn overlay_shown(&self) -> bool {
        self.running.is_some() && (!self.overlay_hidden || !self.wants_full_input())
    }

    /// The "Playing" overlay: A arms "Quit game", a second A within 3 s stops it; B / Start
    /// cancel an armed stop. Browsing the library is Home → Library (two deliberate presses, so
    /// presses meant for the game can't get there); the game's Stop button on its details page
    /// brings the overlay back. B is "back" everywhere else, so it must never be the button that
    /// kills a game.
    fn playing_nav(&mut self, st: &mut AppState, nav: Nav, now: f64) {
        match nav {
            Nav::Accept => {
                let seen = self.running.map(|r| r.2).unwrap_or(now);
                if now - seen < STOP_GRACE_SECS {
                    return;
                }
                if self.stop_armed.is_some_and(|t| now - t < STOP_CONFIRM_SECS) {
                    if let Some((id, _, _)) = self.running {
                        crate::ui::launching::stop(st, id);
                    }
                    self.stop_armed = None;
                } else {
                    self.stop_armed = Some(now);
                }
            }
            Nav::Back | Nav::Menu | Nav::Guide => self.stop_armed = None,
            _ => {}
        }
    }

    fn open_search(&mut self, now: f64) {
        let name = self.cur.and_then(|i| self.colls.get(i)).map(|c| c.name.clone()).unwrap_or_default();
        self.osk = Some(Osk::new(Purpose::Search, format!("Search {name}"), self.search.clone(), now));
    }

    fn osk_result(&mut self, st: &mut AppState, r: OskResult, now: f64) {
        let Some(o) = &self.osk else { return };
        let purpose = o.purpose;
        match (purpose, r) {
            (_, OskResult::None) => {}
            (Purpose::Search, OskResult::Changed) => {
                self.search = o.text.clone();
            }
            (Purpose::Search, OskResult::Done | OskResult::Cancel) => {
                self.search = o.text.trim().to_string();
                self.close_osk(st);
            }
            (Purpose::LaunchOptions(_) | Purpose::Command(_), OskResult::Changed) => {}
            (Purpose::LaunchOptions(id) | Purpose::Command(id), OskResult::Done) => {
                let text = o.text.trim().to_string();
                let mut ov = st.library.get(id).and_then(|g| g.launch_override.clone()).unwrap_or_default();
                match purpose {
                    Purpose::LaunchOptions(_) => ov.options = text,
                    _ => ov.command = if text.is_empty() { None } else { Some(text) },
                }
                st.set_launch_override(id, Some(ov));
                self.close_osk(st);
                self.refresh_launch_menu(st, id, now);
            }
            (Purpose::LaunchOptions(_) | Purpose::Command(_), OskResult::Cancel) => self.close_osk(st),
        }
    }

    fn close_osk(&mut self, st: &AppState) {
        if let Some(mut o) = self.osk.take() {
            o.close(st.ctx());
        }
    }

    // ------------------------------------------------------------------ menus

    fn open_quick_menu(&mut self, st: &AppState, now: f64) {
        let m = Menu::new(MenuKind::Quick, "Menu", self.quick_items(st), now).with_subtitle(format!(
            "{} games · {}",
            st.library.games.iter().filter(|g| !g.hidden).count(),
            if st.scan.running { "scanning…".to_string() } else { "Lodestar".to_string() }
        ));
        self.menus.push(m);
    }

    fn open_game_options(&mut self, st: &AppState, id: GameId, now: f64) {
        let Some(g) = st.library.get(id) else { return };
        let m = Menu::new(MenuKind::GameOptions(id), g.display_name().to_string(), self.game_option_items(st, id), now)
            .with_subtitle(format!("{} · {}", emu::source_label(g), format_playtime(g.playtime_secs)));
        self.menus.push(m);
    }

    fn pop_menu(&mut self, st: &mut AppState) {
        if let Some(m) = self.menus.pop()
            && m.kind == MenuKind::Themes
        {
            self.end_theme_preview(st, false);
        }
    }

    fn close_menus(&mut self, st: &mut AppState) {
        while !self.menus.is_empty() {
            self.pop_menu(st);
        }
    }

    fn menu_nav(&mut self, st: &mut AppState, nav: Nav, now: f64) {
        if nav != Nav::Accept {
            // Moving away from (or closing) an armed "Hide game" cancels it.
            self.disarm_hide();
        }
        match nav {
            Nav::Up => {
                if let Some(m) = self.menus.last_mut() {
                    m.move_sel(-1);
                }
            }
            Nav::Down => {
                if let Some(m) = self.menus.last_mut() {
                    m.move_sel(1);
                }
            }
            Nav::PageUp | Nav::PageDown if self.menus.last().is_some_and(|m| !m.tabs.is_empty()) => {
                let (tab, n) = self.menus.last().map(|m| (m.tab, m.tabs.len())).unwrap_or((0, 1));
                let next = if nav == Nav::PageUp { tab + n - 1 } else { tab + 1 };
                self.settings_tab(st, next);
            }
            Nav::PageUp | Nav::PageDown => {
                if let Some(m) = self.menus.last_mut() {
                    for _ in 0..5 {
                        m.move_sel(if nav == Nav::PageUp { -1 } else { 1 });
                    }
                }
            }
            Nav::Back => self.pop_menu(st),
            Nav::Menu => self.close_menus(st),
            Nav::Accept | Nav::Right | Nav::Left => {
                let act = self.menus.last().and_then(|m| m.selected_act()).cloned();
                let is_value = matches!(
                    act,
                    Some(
                        Act::ToggleSwapAb
                            | Act::CycleButtonPrompts
                            | Act::ToggleFullscreen
                            | Act::ToggleHideLaunchers
                            | Act::ToggleCloseLaunchers
                            | Act::ToggleHideSteam
                            | Act::ToggleCloseSteam
                            | Act::ToggleKeepSteam
                            | Act::CycleOneGame
                            | Act::CycleCloseWhilePlaying
                            | Act::ToggleTrayIcon
                            | Act::ToggleDescending
                            | Act::ToggleShowHidden
                            | Act::Setting(_)
                            | Act::ThemeOption(..)
                    )
                );
                // ← turns the volume down.
                let act = match act {
                    Some(Act::Setting(menu::Setting::Volume(_))) if nav == Nav::Left => {
                        Some(Act::Setting(menu::Setting::Volume(-1)))
                    }
                    Some(Act::ThemeOption(i, _)) if nav == Nav::Left => Some(Act::ThemeOption(i, -1)),
                    a => a,
                };
                if (nav == Nav::Accept || is_value)
                    && let Some(a) = act
                {
                    self.menu_act(st, a, now);
                }
            }
            _ => {}
        }
        if self.menus.last().is_some_and(|m| m.kind == MenuKind::Themes) {
            self.preview_theme(st);
        }
    }

    fn refresh_quick(&mut self, st: &AppState) {
        self.refresh_settings(st);
        let items = self.quick_items(st);
        if let Some(m) = self.menus.iter_mut().rev().find(|m| m.kind == MenuKind::Quick) {
            m.replace_items(items);
        }
    }

    // ------------------------------------------------------------------ hints

    fn hints(&self, st: &AppState) -> Vec<Hint> {
        let mut h = self.screen_hints(st);
        // Browsing from the in-game menu: B on the first screen goes back to that menu.
        if self.sys.library && self.screen == self.first_screen(st) && !self.home.rail_focus {
            h.retain(|x| x.button != Button::B);
            h.insert(1.min(h.len()), Hint::new(Button::B, "In-game menu").always());
        }
        h
    }

    fn screen_hints(&self, st: &AppState) -> Vec<Hint> {
        if let Some(h) = self.home_hints(st) {
            return h;
        }
        let fav = self.selected_game(st).is_some_and(|g| g.favorite);
        match self.screen {
            Screen::Home => vec![],
            Screen::Collections => {
                vec![Hint::new(Button::A, "Open"), Hint::new(Button::Y, "Search"), Hint::new(Button::Start, "Menu")]
            }
            Screen::Games => {
                let mut v = vec![
                    Hint::new(Button::A, "Details"),
                    Hint::new(Button::B, if self.search.is_empty() { "Back" } else { "Clear search" }),
                ];
                v.push(Hint::new(Button::X, if fav { "Unfavourite" } else { "Favourite" }));
                v.push(Hint::new(Button::Y, "Search"));
                v.push(Hint::new(Button::Select, "Options"));
                v
            }
            Screen::Details => {
                let primary =
                    self.details.as_ref().map(|d| Self::primary(st, d.game)).unwrap_or(details::Primary::Play);
                // The built-in page: A presses the focused button, so the hint names it.
                let themed = st.controller_theme.file.details.is_some();
                let focus = if themed { 0 } else { self.details.as_ref().map_or(0, |d| d.focus) };
                let romm = self.selected_game(st).is_some_and(|g| g.source == Source::Romm);
                let a = match focus {
                    0 if primary == details::Primary::Install && romm => "Download",
                    0 => primary.label(),
                    1 if fav => "Unfavourite",
                    1 => "Favourite",
                    2 => "Artwork",
                    _ => "Options",
                };
                vec![
                    Hint::new(Button::A, a),
                    Hint::new(Button::B, "Back"),
                    Hint::new(Button::X, if fav { "Unfavourite" } else { "Favourite" }),
                    // Both buttons, each with its own word: a hint glyph is one button.
                    Hint::new(Button::LB, "Prev"),
                    Hint::new(Button::RB, "Next"),
                    Hint::new(Button::Select, "Options"),
                ]
            }
        }
    }

    // ------------------------------------------------------------------ frame

    /// `navs`: navigation events for this frame (gamepad + keyboard), already filtered for focus.
    pub fn show(&mut self, ui: &mut egui::Ui, st: &mut AppState, navs: &[Nav]) {
        let bg = st.controller_theme.file.palette.background.to_egui();
        egui::CentralPanel::default().frame(egui::Frame::new().fill(bg)).show(ui, |ui| {
            self.frame(ui, st, navs);
        });
    }

    fn frame(&mut self, ui: &mut egui::Ui, st: &mut AppState, navs: &[Nav]) {
        let screen = ui.max_rect();
        let (now, dt) = ui.input(|i| (i.time, i.stable_dt.clamp(1.0 / 240.0, 1.0 / 20.0)));
        let kb = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
        self.frame_start(ui, st, !navs.is_empty(), now);
        for &nav in navs {
            log::debug!("controller nav {nav:?} on {:?}", self.screen);
            let before =
                st.config.controller.sounds_enabled(&st.controller_theme.file).then(|| sounds::Snap::of(self, st));
            self.handle_nav(st, nav, screen, now, kb);
            self.refresh_list(&st.library);
            if let Some(b) = &before {
                sounds::after_nav(self, st, nav, b);
            }
        }
        if self.debug_launch.is_some()
            && let Some(id) = self.details.as_ref().map(|d| d.game)
            && let Some(what) = self.debug_launch.take()
        {
            crate::ui::launching::debug_fake(st, id, &what);
            self.track_running(st, now);
        }
        self.refresh_osk_preview(st);

        // The "Playing" overlay also covers the screen: nothing under it takes clicks.
        let switch_ask = crate::ui::launching::prompt(st);
        let overlay_open = self.osk.is_some()
            || self.viewer.is_some()
            || !self.menus.is_empty()
            || self.overlay_shown()
            || switch_ask.is_some()
            || !self.sys.menus.is_empty();
        let t_in = (((now - self.transition) / 0.22) as f32).clamp(0.0, 1.0);
        let mut animating = t_in < 1.0;
        let hints =
            if self.overlay_shown() || !self.menus.is_empty() || self.osk.is_some() || !self.sys.menus.is_empty() {
                vec![] // the Playing overlay, the menus and the keyboard show their own
            } else {
                self.hints(st)
            };
        let look = Look::new(st, screen);
        let pass = ScreenPass { screen, hints: &hints, opacity: menu::ease(t_in), interactive: !overlay_open, now, dt };
        if self.colls.is_empty() {
            animating |= self.draw_empty(ui, st, screen, &look, now, &hints);
        } else {
            animating |= self.draw_screen(ui, st, &look, &pass);
            self.prefetch_next_screen(st, screen, ui.ctx().pixels_per_point(), now);
        }
        animating |= self.draw_letter_popup(ui, screen, &look, now);
        if st.scan.running {
            animating |= Self::draw_scanning(ui, screen, &look, now);
        }
        if let Some((id, since, _)) = self.running
            && self.overlay_shown()
            && self.menus.is_empty()
            && self.osk.is_none()
            && self.sys.menus.is_empty()
        {
            self.draw_playing(ui, st, screen, &look, (id, since), now);
        }
        if let Some(v) = &mut self.viewer {
            if st.library.get(v.game).is_none() {
                self.viewer = None;
            } else {
                animating |= v.draw(ui, screen, &look, &mut st.images);
            }
        }
        // An armed "Hide game" expires: clear its prompt then (and repaint at that moment).
        if let Some((_, t)) = self.hide_armed {
            let left = HIDE_CONFIRM_SECS - (now - t);
            if left <= 0.0 || self.menus.is_empty() {
                self.disarm_hide();
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(left));
            }
        }
        let pointer = ui.input(|i| i.pointer.hover_pos().map(|p| (p, i.pointer.primary_clicked())));
        let menu_pointer =
            if self.osk.is_some() || switch_ask.is_some() || !self.sys.menus.is_empty() { None } else { pointer };
        animating |= self.draw_top_menu(ui, st, &look, menu_pointer, now, dt);
        if let Some(o) = &mut self.osk {
            let (r, anim) = o.draw(ui, screen, &look, now);
            animating |= anim;
            self.osk_result(st, r, now);
            self.refresh_list(&st.library);
        }
        if let Some(ask) = &switch_ask
            && let Some(answer) = prompt::draw_prompt(ui, screen, &look, ask)
        {
            crate::ui::launching::answer_prompt(st, ask.kind, answer);
        }
        if self.osk.is_none() && self.viewer.is_none() && self.menus.is_empty() && self.sys.menus.is_empty() {
            self.draw_status(ui.painter(), st, screen, &look, now);
        }
        animating |= self.draw_sys(ui.painter(), st, screen, &look, now, pointer);

        if animating {
            ui.ctx().request_repaint();
        }
    }

    /// Bring this frame's state up to date before input is handled: view options, the lists,
    /// the home page, the running game, the pointer.
    fn frame_start(&mut self, ui: &egui::Ui, st: &mut AppState, has_input: bool, now: f64) {
        self.renderer.opts = crate::theme::render::ViewOpts::from_config(&st.config.controller);
        // Theme videos rest while a game runs.
        self.renderer.opts.videos &= self.running.is_none();
        log::trace!("controller frame");
        self.refresh_collections(&st.library);
        self.refresh_list(&st.library);
        self.home_sync(st, now);
        self.track_running(st, now);
        self.sys_tick(st, now);
        self.poll_music_folder(st);
        self.sys.cursor.frame(ui.ctx(), st.config.controller.hide_cursor, has_input, now);
        // Another pad picked up (Automatic): the quick menu names the style it shows now.
        let style = st.prompts().style;
        if self.prompt_style.replace(style).is_some_and(|s| s != style) {
            self.refresh_quick(st);
        }
    }

    /// The keyboard's launch-option preview (recomputed only when the text changes).
    fn refresh_osk_preview(&mut self, st: &AppState) {
        if let Some(o) = &mut self.osk
            && let Purpose::LaunchOptions(id) | Purpose::Command(id) = o.purpose
            && o.preview.as_ref().is_none_or(|(t, _)| t != &o.text)
        {
            let line = st.library.get(id).map(|g| {
                let mut ov = g.launch_override.clone().unwrap_or_default();
                if matches!(o.purpose, Purpose::LaunchOptions(_)) {
                    ov.options = o.text.clone();
                } else {
                    ov.command = Some(o.text.clone()).filter(|t| !t.trim().is_empty());
                }
                crate::launch::preview(g, &ov).map(|r| r.display).map_err(|e| format!("{e:#}"))
            });
            o.preview = line.map(|l| (o.text.clone(), l));
        }
    }

    /// The current screen (home, collections, games, details) and its clicks. True while it
    /// animates.
    fn draw_screen(&mut self, ui: &mut egui::Ui, st: &mut AppState, look: &Look, pass: &ScreenPass) -> bool {
        let ScreenPass { screen, now, .. } = *pass;
        match self.screen {
            Screen::Home => self.draw_home(ui, st, pass),
            Screen::Collections => {
                let out = self.draw_view(ui, st, ViewKind::Collections, pass);
                if let Some(i) = out.clicked {
                    if i == self.coll_sel {
                        self.enter_collection(&st.library, i, now);
                    } else {
                        self.coll_sel = i;
                    }
                }
                if out.scroll_steps != 0 {
                    let n = self.colls.len() as i64;
                    self.coll_sel = (self.coll_sel as i64 + out.scroll_steps as i64).clamp(0, n - 1) as usize;
                }
                if let Some(c) = self.colls.get(self.coll_sel) {
                    self.coll_key = Some(c.collection.key());
                }
            }
            Screen::Games => {
                let out = self.draw_view(ui, st, ViewKind::Games, pass);
                if let Some(i) = out.clicked {
                    if i == self.game_sel {
                        self.open_details(st, now);
                    } else {
                        self.game_sel = i;
                    }
                    self.remember();
                }
                if out.scroll_steps != 0 && !self.list.is_empty() {
                    let (kind, _) = self.focus_of(st, ViewKind::Games, screen);
                    let per = if let FocusKind::Grid { columns } = kind { columns as i64 } else { 1 };
                    let n = self.list.len() as i64;
                    self.game_sel = (self.game_sel as i64 + out.scroll_steps as i64 * per).clamp(0, n - 1) as usize;
                    self.remember();
                }
                if self.list.is_empty() {
                    self.draw_no_results(ui, screen, look);
                }
                if !self.search.is_empty() && self.osk.is_none() {
                    self.draw_search_chip(ui, screen, look);
                }
            }
            Screen::Details => return self.draw_details(ui, st, look, pass),
        }
        false
    }

    /// The big letter shown for a moment after a letter jump. True while it shows.
    fn draw_letter_popup(&mut self, ui: &egui::Ui, screen: Rect, look: &Look, now: f64) -> bool {
        let Some((l, t0)) = &self.letter else {
            return false;
        };
        let age = (now - t0) as f32;
        if age > 0.9 {
            self.letter = None;
            return false;
        }
        let a = if age < 0.6 { 1.0 } else { 1.0 - (age - 0.6) / 0.3 };
        let p = ui.painter();
        let r = Rect::from_center_size(screen.center(), vec2(look.h(0.2), look.h(0.2)));
        p.rect_filled(r, rd::cr(look.h(0.03)), fade(Color32::from_black_alpha(200), a));
        let g = p.layout_no_wrap(l.clone(), look.title(0.12), fade(look.text, a));
        p.galley(r.center() - g.size() / 2.0, g, look.text);
        true
    }

    /// The top menu (only that one is drawn) and its clicks; `pointer` is None while something
    /// covers it. True while it animates.
    fn draw_top_menu(
        &mut self,
        ui: &egui::Ui,
        st: &mut AppState,
        look: &Look,
        pointer: Option<(egui::Pos2, bool)>,
        now: f64,
        dt: f32,
    ) -> bool {
        let Some(m) = self.menus.last_mut() else {
            return false;
        };
        let out = m.draw(ui.painter(), ui.max_rect(), look, now, dt, pointer);
        if let Some(t) = out.tab {
            self.settings_tab(st, t);
        } else if let Some(i) = out.clicked {
            m.sel = i;
            if let Some(a) = m.selected_act().cloned() {
                self.menu_act(st, a, now);
            }
        } else if out.dismissed {
            self.pop_menu(st);
        }
        if self.menus.last().is_some_and(|m| m.kind == MenuKind::Themes) {
            self.preview_theme(st);
        }
        out.animating
    }

    fn track_running(&mut self, st: &AppState, now: f64) {
        match (crate::ui::launching::running(st), self.running) {
            (Some(id), Some((rid, _, _))) if id == rid => {}
            (Some(id), _) => {
                // A game picked up at startup shows when it really started.
                let since = st.launcher.adopted_start(id).unwrap_or_else(now_unix);
                self.running = Some((id, since, now));
                self.stop_armed = None;
                self.overlay_hidden = false;
            }
            (None, Some(_)) => {
                self.running = None;
                self.stop_armed = None;
                self.overlay_hidden = false;
            }
            (None, None) => {}
        }
    }
}

#[cfg(test)]
mod tests;
