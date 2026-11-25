//! Desktop (Playnite-style) UI: top bar, explorer panel, grid / list / details views, details
//! panel, dialogs and settings.
//!
//! Structure: every frame the panels draw from `st` and push [`Action`]s; the actions are applied
//! after drawing (so drawing code only needs shared borrows of the library). The filtered view
//! (`self.view`) is recomputed only when the library generation or the filter changes.

mod actions;
mod add;
mod add_cloud;
mod addons;
mod ambient;
mod artwork;
mod cleanup;
mod credits;
mod debug_input;
mod details;
mod dialogs;
mod edit;
mod edit_emulation;
mod emulation_import;
mod emulation_ui;
mod explorer;
mod facets;
mod game_options_ui;
mod grid;
mod grouping;
mod hidden_ui;
mod html;
mod keys;
mod libraries;
mod list;
mod menus;
mod missing_ui;
mod own_window;
mod pad;
mod persist;
mod random;
mod romm_ui;
mod score;
mod scroll;
mod settings;
mod sources_ui;
mod stats;
mod statusbar;
mod style;
mod theme_creator;
mod topbar;
mod undo;
pub(crate) mod update_ui;
mod view;
mod viewer;
mod welcome_ui;
mod whats_new_ui;

pub(crate) use own_window::{capture_x11 as capture_own_window, window_rects as own_window_rects};
pub use style::{Pal, apply_style};

// Sibling modules (dialogs, tests) reach these as `super::X`.
pub(crate) use actions::Action;
pub(crate) use debug_input::*;
pub(crate) use facets::*;
pub(crate) use html::strip_html;
pub(crate) use menus::*;
pub(crate) use persist::ExplorerState;
pub(crate) use scroll::{ScrollKeep, offset_to_show};
use undo::*;

use crate::app::AppState;
use crate::config::{DesktopView, QuickPreset, SortBy};
use crate::library::{Collection, CollectionKind, Filter, ViewCache};
use crate::model::*;
use egui::{Key, Modifiers};
use std::time::{Duration, Instant};

/// Explorer "Library" section entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum LibSel {
    #[default]
    All,
    Installed,
    /// Games that aren't installed.
    NotInstalled,
    /// Not-installed Steam games that are no longer in the account's Steam library (an ended
    /// subscription, a family member's game; steam_owned.rs). Listed under Not installed.
    NotInSteamLibrary,
    Favorites,
    Recent,
    Hidden,
}

/// What the store / add-on behind a game can do for it (cached for the selected game: the
/// details page's Install button, the More and right-click menus).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct StoreCaps {
    pub id: GameId,
    pub can_install: bool,
    /// Install opens the game in Steam: it is no longer in the account's Steam library.
    pub not_in_steam_library: bool,
    pub can_uninstall: bool,
    pub store_page: Option<String>,
    /// Its install size is being added up ("Calculate install size" waits).
    pub sizing: bool,
}

impl StoreCaps {
    fn compute(st: &AppState, id: GameId) -> StoreCaps {
        StoreCaps {
            id,
            can_install: st.can_install(id),
            not_in_steam_library: st.library.get(id).is_some_and(|g| st.library.not_in_steam_library(g)),
            can_uninstall: st.can_uninstall(id),
            store_page: st.store_page(id),
            sizing: st.size_calculating(id),
        }
    }
}

/// What the shown games were worked out for: (library generation, filter, field selection, quick
/// preset, (Match all, Library entry, Sources selection)).
type ViewKey = (u64, Filter, FacetSel, QuickPreset, (bool, LibSel, Vec<SrcKey>));

pub struct DesktopUi {
    initialized: bool,
    style_gen: u64,
    pub(crate) pal: Pal,
    pub(crate) fonts: style::Fonts,
    cache: ViewCache,
    /// Library filter (search, sort, installed …). Its `sources` stays empty: the Sources entries
    /// are `src_sel` (they tell cloud services apart, which `Source` can't).
    pub(crate) filter: Filter,
    /// Explorer Sources selection (any of them).
    pub(crate) src_sel: Vec<SrcKey>,
    pub(crate) lib_sel: LibSel,
    /// Explorer Genre / Platform / Developer / Publisher selection (lowercase values), indexed
    /// by `Facet::index`. Any value within a field, all fields (like sources + tags).
    pub(crate) facet_sel: FacetSel,
    /// Several tags / values of one field picked: games must have all of them (else any).
    pub(crate) match_all: bool,
    /// The Statistics page, shown instead of the games while open (stats.rs).
    pub(crate) stats: Option<stats::StatsPage>,
    /// Collapsed explorer sections (by key: "library", "sources", "tags", "genre", …).
    pub(crate) collapsed: Vec<String>,
    /// The user's collection shown instead of a Library entry (by id).
    pub(crate) coll_sel: Option<u64>,
    /// The collection name prompt (New / Rename).
    pub(crate) coll_dlg: Option<explorer::colls::CollDlg>,
    /// Hash of the explorer state last written to the DB / when it started differing.
    saved_explorer: Option<u64>,
    explorer_changed: Option<Instant>,
    /// Keyboard focus in the explorer (← from the games; see explorer/nav.rs).
    pub(crate) side_nav: explorer::nav::SideNav,
    /// Indices into `st.library.games`, the current filtered + sorted view.
    view: Vec<usize>,
    view_key: Option<ViewKey>,
    /// Quick preset (copied from the config each frame).
    pub(crate) preset: QuickPreset,
    view_installed: usize,
    /// Group by …: the shown list (positions = `sel_pos`) and its group headers.
    pub(crate) groups: grouping::GroupState,
    /// The "Picked …  Pick another" toast of the random game button.
    pub(crate) random: Option<random::RandomPick>,
    pub(crate) selected: Option<GameId>,
    sel_pos: Option<usize>,
    /// Multi-selection (Ctrl / Shift + click); empty or ≥ 2 games. `selected` stays the focused
    /// game shown in the details.
    pub(crate) multi: std::collections::HashSet<GameId>,
    /// Where a Shift+click range starts.
    multi_anchor: Option<GameId>,
    /// When the last bulk artwork download's result started showing in the status bar.
    bulk_done_at: Option<Instant>,
    scroll_to_sel: bool,
    grid_cols: usize,
    /// Grid scroll area: content width × viewport height and scroll offset, as of last frame.
    grid_view_size: Option<egui::Vec2>,
    grid_offset: f32,
    /// Grid content height as of last frame (scroll offsets are kept within it).
    grid_content_h: f32,
    /// Frames spent asking again for an offset that fell short (capped).
    grid_retries: u8,
    list_keep: ScrollKeep,
    compact_keep: ScrollKeep,
    page_rows: usize,
    pub(crate) counts: Counts,
    /// Store actions of the selected game.
    pub(crate) caps: Option<StoreCaps>,
    config_dirty: Option<Instant>,
    /// Draw the game name under grid covers (stored in the DB kv table).
    pub(crate) show_names: bool,
    focus_search: bool,
    actions: Vec<Action>,
    dlg: dialogs::Dialogs,
    /// Dialogs in their own windows (own_window.rs).
    own: own_window::OwnWindows,
    /// The controller in desktop mode (pad.rs).
    pad: pad::PadState,
    undo: Option<Undo>,
    /// (game id, library generation) → plain-text description.
    desc_cache: Option<(GameId, u64, String)>,
    /// `LODESTAR_UI_TIMING=1`: log the average time spent in `show()` every 120 frames.
    timing: Option<(Duration, u32, Duration)>,
    debug_open: Option<String>,
    /// Testing hook: open the game's "•••" menu ("menu") or the app menu ("appmenu").
    pub(crate) debug_menu: Option<String>,
    /// Testing hook (`LODESTAR_DESKTOP_OPEN=pencil`): show the artwork pencils without hover.
    pub(crate) debug_pencil: bool,
    /// Arrange mode on a game's page (details/arrange.rs).
    pub(crate) arrange: Option<details::Arrange>,
    /// Insert: open the app menu with its "Add game" submenu.
    pub(crate) open_add_menu: bool,
    /// The app menu was opened by Insert: open its Add game submenu once it is drawn.
    pub(crate) add_submenu_open: bool,
    /// Testing hook (`LODESTAR_DESKTOP_BULK_CANCEL_AT=n`): cancel a bulk artwork download once
    /// n games are done (as the status bar's Cancel button would).
    debug_bulk_cancel: Option<usize>,
    /// Testing hook: `LODESTAR_DESKTOP_KEYS=right,down,enter,text:abc` injected one by one
    /// ("text:" types, "%2C" is a comma; "open:<what>" opens a `LODESTAR_DESKTOP_OPEN` target).
    debug_keys: Vec<String>,
    /// Testing hook: `debug_open` was set by an "open:<what>" key (open it before the rest).
    debug_open_now: bool,
    frame: u64,
    /// Screen size (points) and zoom of the previous frame: when they change (window resized,
    /// UI scale applied after the first frame), the selection is scrolled back into view.
    last_screen: Option<(egui::Vec2, f32)>,
    /// Selection last written to the DB, and when the selection started differing from it.
    saved_selected: Option<GameId>,
    sel_changed: Option<Instant>,
    ambient: ambient::Ambient,
    /// Panel fills for the current frame (translucent when the ambient background is on).
    fills: ambient::Fills,
}

impl Default for DesktopUi {
    fn default() -> Self {
        DesktopUi {
            initialized: false,
            style_gen: 0,
            pal: Pal::default(),
            fonts: style::Fonts::default(),
            cache: ViewCache::default(),
            filter: Filter::default(),
            src_sel: vec![],
            lib_sel: LibSel::All,
            facet_sel: Default::default(),
            match_all: false,
            stats: None,
            collapsed: vec!["developer".into(), "publisher".into()],
            coll_sel: None,
            coll_dlg: None,
            saved_explorer: None,
            explorer_changed: None,
            side_nav: Default::default(),
            view: vec![],
            view_key: None,
            preset: QuickPreset::All,
            view_installed: 0,
            groups: Default::default(),
            random: None,
            selected: None,
            sel_pos: None,
            multi: Default::default(),
            multi_anchor: None,
            bulk_done_at: None,
            scroll_to_sel: false,
            grid_cols: 1,
            grid_view_size: None,
            grid_offset: 0.0,
            grid_content_h: 0.0,
            grid_retries: 0,
            list_keep: ScrollKeep::default(),
            compact_keep: ScrollKeep::default(),
            page_rows: 10,
            counts: Counts::default(),
            caps: None,
            config_dirty: None,
            show_names: true,
            focus_search: false,
            actions: vec![],
            dlg: dialogs::Dialogs::default(),
            own: Default::default(),
            pad: Default::default(),
            undo: None,
            desc_cache: None,
            timing: None,
            debug_open: None,
            debug_menu: None,
            debug_pencil: false,
            arrange: None,
            open_add_menu: false,
            add_submenu_open: false,
            debug_bulk_cancel: None,
            last_screen: None,
            debug_keys: vec![],
            debug_open_now: false,
            frame: 0,
            saved_selected: None,
            sel_changed: None,
            ambient: ambient::Ambient::default(),
            fills: ambient::Fills::BLACK,
        }
    }
}

const KV_SHOW_NAMES: &str = "desktop.show_names";
const KV_SELECTED: &str = "desktop.selected";
const KV_EXPLORER: &str = "desktop.explorer";

pub(crate) type FacetSel = [Vec<String>; N_FACETS];

impl LibSel {
    fn key(self) -> &'static str {
        match self {
            LibSel::All => "all",
            LibSel::Installed => "installed",
            LibSel::NotInstalled => "notinstalled",
            LibSel::NotInSteamLibrary => "notinsteamlibrary",
            LibSel::Favorites => "favorites",
            LibSel::Recent => "recent",
            LibSel::Hidden => "hidden",
        }
    }

    /// An entry that shows only games that aren't installed.
    pub(crate) fn not_installed(self) -> bool {
        matches!(self, LibSel::NotInstalled | LibSel::NotInSteamLibrary)
    }

    fn from_key(k: &str) -> LibSel {
        match k {
            "installed" => LibSel::Installed,
            "notinstalled" => LibSel::NotInstalled,
            "notinsteamlibrary" => LibSel::NotInSteamLibrary,
            "favorites" => LibSel::Favorites,
            "recent" => LibSel::Recent,
            "hidden" => LibSel::Hidden,
            _ => LibSel::All,
        }
    }
}

impl DesktopUi {
    pub fn new() -> DesktopUi {
        DesktopUi::default()
    }

    pub fn show(&mut self, ui: &mut egui::Ui, st: &mut AppState) {
        let t0 = Instant::now();
        self.show_inner(ui, st);
        if let Some((sum, n, max)) = &mut self.timing {
            let el = t0.elapsed();
            *sum += el;
            *n += 1;
            *max = (*max).max(el);
            if *n >= 120 {
                log::info!(
                    "desktop ui: avg {:?} max {:?} per frame over {n} frames ({} games in view)",
                    *sum / *n,
                    *max,
                    self.view.len()
                );
                *sum = Duration::ZERO;
                *n = 0;
                *max = Duration::ZERO;
            }
        }
    }

    fn show_inner(&mut self, ui: &mut egui::Ui, st: &mut AppState) {
        let ctx = ui.ctx().clone();
        if !self.initialized {
            self.first_frame(st);
        }
        self.refresh_look(&ctx, st);
        self.refresh_counts(st);
        st.fonts.request_coverage(&self.filter.search);
        self.sync_filter(st);
        self.refresh_view_for(&st.library);
        self.refresh_caps(st);
        let screen = (ctx.content_rect().size(), ctx.zoom_factor());
        // Also during the first frames, while egui settles the side panels' sizes.
        if self.last_screen != Some(screen) || self.frame < 3 {
            self.last_screen = Some(screen);
            self.scroll_to_sel = true;
        }
        self.frame += 1;
        // While a dialog has its own window, that window gets the pad (own_window.rs).
        let pad_here = !self.own.any();
        if pad_here {
            self.pad_pass(&ctx, st);
        }
        self.debug_next_key(&ctx, st);
        self.handle_keys(&ctx, st);
        self.debug_tick(&ctx, st);
        self.draw_panels(ui, st);

        self.dialogs(&ctx, st);
        emulation_ui::allow_prompt(&ctx, st, &self.pal);
        romm_ui::folder_prompt(&ctx, st, &self.pal);
        self.collection_dialog(&ctx, st);
        self.arrange_tick(&ctx, st);
        self.undo_toast(&ctx, st);
        self.random_toast(&ctx, st);
        self.merge_banner(&ctx, st);
        welcome_ui::show(&ctx, st, &self.pal);
        self.apply_actions(&ctx, st);
        self.flush_config(&ctx, st);
        if pad_here {
            self.pad_end(&ctx);
        }
    }

    /// Once: testing hooks from the environment and the state kept from the last run.
    fn first_frame(&mut self, st: &AppState) {
        self.initialized = true;
        if std::env::var_os("LODESTAR_UI_TIMING").is_some() {
            self.timing = Some((Duration::ZERO, 0, Duration::ZERO));
        }
        self.read_debug_env();
        self.show_names = st.db.kv_get(KV_SHOW_NAMES).is_none_or(|v| v != "0");
        self.selected = st.db.kv_get(KV_SELECTED).and_then(|v| v.parse().ok());
        self.saved_selected = self.selected;
        if let Some(es) = st.db.kv_get(KV_EXPLORER).and_then(|v| serde_json::from_str::<ExplorerState>(&v).ok()) {
            self.apply_explorer_state(&es);
        }
        self.saved_explorer = Some(self.explorer_hash());
        self.scroll_to_sel = true;
    }

    /// The desktop theme's style, palette and fonts.
    fn refresh_look(&mut self, ctx: &egui::Context, st: &AppState) {
        if self.style_gen != st.style_generation {
            self.style_gen = st.style_generation;
            apply_style(ctx, &st.desktop_theme);
            self.pal = Pal::of_theme(&st.desktop_theme);
        }
        // Theme fonts load in the background and become usable a few frames in: look them up
        // every frame (two map lookups), not only when the theme changes.
        self.fonts = style::Fonts {
            bold: st.fonts.family(&st.desktop_theme, Some("bold")),
            title: st.fonts.family(&st.desktop_theme, Some("title")),
        };
    }

    /// The explorer's counts, when the library changed.
    fn refresh_counts(&mut self, st: &mut AppState) {
        if self.counts.generation == st.library.generation {
            return;
        }
        self.counts = Counts::compute(&st.library);
        // The Hidden entry disappears with the last hidden game (or a restored explorer
        // state points at it): don't leave the user on an empty view without a way back.
        if self.lib_sel == LibSel::Hidden && self.counts.hidden == 0 {
            self.set_lib_sel(LibSel::All);
        }
        // Load the CJK fallback font if a tag or field value needs it (no-op once loaded /
        // for ASCII). Game names are checked by App for both modes.
        for (t, _) in &self.counts.tags {
            st.fonts.request_coverage(t);
        }
        for list in &self.counts.facets {
            for (_, d, _) in list {
                st.fonts.request_coverage(d);
            }
        }
    }

    /// The window's panels: top bar, status bar, explorer, then the games in the chosen view
    /// (or the Statistics page).
    fn draw_panels(&mut self, ui: &mut egui::Ui, st: &mut AppState) {
        // Take the view out so drawing code can borrow it next to `&mut self` (put back below).
        let view = std::mem::take(&mut self.view);
        // The shown (grouped) list and its headers, lent out the same way.
        let items = std::mem::take(&mut self.groups.items);
        let spans = std::mem::take(&mut self.groups.spans);
        let shown = grouping::Shown { view: &view, items: &items, spans: &spans };
        let ambient_on = Self::ambient_enabled(st);
        if ambient_on || Self::backdrop_wanted(st) {
            self.paint_ambient(ui, st, ambient_on);
        }
        self.fills = self.fills(st, ambient_on);

        let top_bar = self.draw_bars(ui, st, &view);
        if ambient_on {
            self.paint_art(ui, st, top_bar);
        }
        self.draw_explorer_panel(ui, st);
        let view_mode = st.config.desktop.view;
        match view_mode {
            _ if self.stats.is_some() => {
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(self.fills.center))
                    .show(ui, |ui| self.stats_page(ui, st));
            }
            DesktopView::Grid | DesktopView::List => self.draw_games_with_panel(ui, st, &shown, view_mode),
            DesktopView::Details => self.draw_details_mode(ui, st, &shown),
        }
        self.view = view;
        self.groups.items = items;
        self.groups.spans = spans;
    }

    /// The top bar and the status bar; returns the top bar's rect.
    fn draw_bars(&mut self, ui: &mut egui::Ui, st: &mut AppState, view: &[usize]) -> egui::Rect {
        let fills = self.fills;
        let slot = self.panel_slot(ui);
        let r = egui::Panel::top("ls_topbar")
            .frame(egui::Frame::new().fill(fills.bar).inner_margin(egui::Margin::symmetric(8, 6)))
            .show_separator_line(false)
            .show(ui, |ui| self.top_bar(ui, st));
        let top_bar = r.response.rect;
        self.paint_panel(ui, slot, top_bar, ambient::Panel::Bar);
        let slot = self.panel_slot(ui);
        let r = egui::Panel::bottom("ls_status")
            .frame(egui::Frame::new().fill(fills.bar).inner_margin(egui::Margin::symmetric(10, 4)))
            .show_separator_line(false)
            .show(ui, |ui| self.status_bar(ui, st, view));
        self.paint_panel(ui, slot, r.response.rect, ambient::Panel::Bar);
        top_bar
    }

    /// The explorer sidebar (collapsible; remembered in the config).
    fn draw_explorer_panel(&mut self, ui: &mut egui::Ui, st: &mut AppState) {
        let mut show_sidebar = st.config.desktop.show_sidebar;
        let slot = self.panel_slot(ui);
        let r = egui::Panel::left("ls_explorer")
            .frame(egui::Frame::new().fill(self.fills.sidebar).inner_margin(egui::Margin {
                left: 0,
                right: 0,
                top: 8,
                bottom: 8,
            }))
            .resizable(true)
            .default_size(220.0)
            .size_range(160.0..=380.0)
            .show_separator_line(false)
            .show_collapsible(ui, &mut show_sidebar, |ui| self.explorer(ui, st));
        if let Some(r) = r {
            self.paint_panel(ui, slot, r.response.rect, ambient::Panel::Sidebar);
        }
        if show_sidebar != st.config.desktop.show_sidebar {
            st.config.desktop.show_sidebar = show_sidebar;
            self.mark_config_dirty();
        }
    }

    /// Grid or List view: the games in the middle, the selected game's details panel on the right.
    fn draw_games_with_panel(
        &mut self,
        ui: &mut egui::Ui,
        st: &mut AppState,
        shown: &grouping::Shown,
        view_mode: DesktopView,
    ) {
        let fills = self.fills;
        let mut show_details = st.config.desktop.show_details_panel;
        // Leave the grid / list at least ~240 pt in narrow windows (big UI scale):
        // the panel gives way down to 220 pt instead of squeezing the games out.
        let max_details = (ui.available_width() - 240.0).clamp(220.0, 620.0);
        let slot = self.panel_slot(ui);
        let r = egui::Panel::right("ls_details_panel")
            .frame(egui::Frame::new().fill(fills.surface))
            .resizable(true)
            .default_size(380.0_f32.min(max_details))
            .size_range(300.0_f32.min(max_details)..=max_details)
            .show_separator_line(false)
            .show_collapsible(ui, &mut show_details, |ui| self.details_panel(ui, st));
        if let Some(r) = r {
            self.paint_panel(ui, slot, r.response.rect, ambient::Panel::Details);
        }
        if show_details != st.config.desktop.show_details_panel {
            st.config.desktop.show_details_panel = show_details;
            self.mark_config_dirty();
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(if view_mode == DesktopView::List {
                // Rows of text read better on a denser panel than covers need.
                fills.list
            } else {
                fills.center
            }))
            .show(ui, |ui| {
                self.filter_chips(ui);
                if shown.view.is_empty() {
                    self.empty_view(ui, st);
                } else if view_mode == DesktopView::Grid {
                    self.grid_view(ui, st, shown);
                } else {
                    self.list_view(ui, st, shown);
                }
            });
    }

    /// Details view: a compact list on the left, the selected game's page in the middle.
    fn draw_details_mode(&mut self, ui: &mut egui::Ui, st: &mut AppState, shown: &grouping::Shown) {
        let fills = self.fills;
        let pal = self.pal;
        let slot = self.panel_slot(ui);
        let r = egui::Panel::left("ls_details_list")
            .frame(egui::Frame::new().fill(fills.list))
            .resizable(true)
            .default_size(290.0)
            .size_range(200.0..=520.0)
            .show_separator_line(false)
            .show(ui, |ui| {
                self.filter_chips(ui);
                if shown.view.is_empty() {
                    ui.add_space(16.0);
                    ui.vertical_centered(|ui| ui.label(egui::RichText::new("No games").color(pal.dim)));
                } else {
                    self.compact_list(ui, st, shown);
                }
            });
        self.paint_panel(ui, slot, r.response.rect, ambient::Panel::List);
        let slot = self.panel_slot(ui);
        let r = egui::CentralPanel::default().frame(egui::Frame::new().fill(fills.page)).show(ui, |ui| {
            if shown.view.is_empty() {
                self.empty_view(ui, st);
            } else {
                self.details_view(ui, st);
            }
        });
        self.paint_panel(ui, slot, r.response.rect, ambient::Panel::Page);
    }

    /// The selected game's store actions. Recomputed every frame (a few cheap lookups for one
    /// game): a store page can appear once the store caches have been read in the background.
    fn refresh_caps(&mut self, st: &AppState) {
        self.caps = self.selected.map(|id| StoreCaps::compute(st, id));
    }

    /// The cached store actions of `id` (only the selected game's are kept).
    pub(crate) fn caps_for(&self, id: GameId) -> Option<&StoreCaps> {
        self.caps.as_ref().filter(|c| c.id == id)
    }

    /// Make `id` the focused (details) game without touching the multi-selection.
    fn focus(&mut self, st: &AppState, id: GameId) {
        let items = &self.groups.items;
        let id_at = |p: usize| items.get(p).and_then(|&i| st.library.games.get(i)).map(|g| g.id);
        // A game shown in several groups is selected where it was clicked.
        if let Some(p) = self.groups.clicked_pos.take().filter(|&p| id_at(p) == Some(id)) {
            self.selected = Some(id);
            self.sel_pos = Some(p);
        } else if self.selected != Some(id) {
            self.selected = Some(id);
            self.sel_pos = items.iter().position(|&i| st.library.games[i].id == id);
        }
    }

    /// Is `id` part of a multi-selection (then views highlight it)?
    pub(crate) fn in_multi(&self, id: GameId) -> bool {
        !self.multi.is_empty() && self.multi.contains(&id)
    }

    /// The click on a game in a view, honouring Ctrl / Shift for the multi-selection.
    pub(crate) fn click_action(ui: &egui::Ui, id: GameId) -> Action {
        let m = ui.input(|i| i.modifiers);
        if m.shift {
            Action::SelectRange(id)
        } else if m.command {
            Action::SelectToggle(id)
        } else {
            Action::Select(id)
        }
    }

    /// The games of the multi-selection, in view order.
    pub(crate) fn multi_ids(&self, st: &AppState) -> Vec<GameId> {
        self.multi_ids_in(&st.library, &self.view)
    }

    /// [`Self::multi_ids`] while drawing (the view is lent out to the drawing code then).
    pub(crate) fn multi_ids_in(&self, lib: &crate::library::Library, view: &[usize]) -> Vec<GameId> {
        view.iter().filter_map(|&i| lib.games.get(i).map(|g| g.id)).filter(|id| self.multi.contains(id)).collect()
    }

    fn empty_view(&mut self, ui: &mut egui::Ui, st: &mut AppState) {
        let pal = self.pal;
        ui.add_space(ui.available_height() * 0.3);
        ui.vertical_centered(|ui| {
            if st.library.games.is_empty() {
                if st.scan.running {
                    {
                let c = ui.visuals().strong_text_color();
                crate::ui::widgets::lazy_spinner(ui, 16.0, c);
            }
                    ui.label(egui::RichText::new("Looking for games…").size(18.0).color(pal.text));
                } else {
                    ui.label(egui::RichText::new("No games yet").size(20.0).color(pal.text));
                    ui.label(egui::RichText::new("Lodestar finds games from Steam, Epic, GOG and Amazon (through Heroic), Lutris, Bottles, Faugus, Windows launchers in Wine prefixes, your emulators' ROM folders and more.").color(pal.dim));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let w = 480.0;
                        ui.add_space((ui.available_width() - w).max(0.0) / 2.0);
                        if ui.button("Rescan library").clicked() {
                            st.rescan();
                        }
                        if ui.button("Add game…").on_hover_text("Fill in the details yourself").clicked() {
                            self.push(Action::AddManual);
                        }
                        if ui.button("Scan for games…").on_hover_text("Find games in a folder or installed ones that aren't in the library").clicked() {
                            self.push(Action::AddScan);
                        }
                        if ui
                            .button("Emulated games…")
                            .on_hover_text("Console games from a ROM folder, started with an emulator")
                            .clicked()
                        {
                            self.push(Action::AddRoms);
                        }
                    });
                    ui.add_space(6.0);
                    if ui
                        .add(egui::Button::new(egui::RichText::new("Libraries settings…").color(pal.selection)).frame(false))
                        .clicked()
                    {
                        self.push(Action::Settings(settings::Tab::Library));
                    }
                }
            } else {
                ui.label(egui::RichText::new("No games match").size(20.0).color(pal.text));
                ui.label(egui::RichText::new("Try clearing the search or filters.").color(pal.dim));
                ui.add_space(10.0);
                if ui.button("Clear filters").clicked() {
                    self.filter.search.clear();
                    self.filter.favorites_only = false;
                    st.config.desktop.installed_only = false;
                    st.config.desktop.preset = QuickPreset::All;
                    self.set_lib_sel(LibSel::All);
                    self.mark_config_dirty();
                }
            }
        });
    }
}

#[cfg(test)]
mod tests;
