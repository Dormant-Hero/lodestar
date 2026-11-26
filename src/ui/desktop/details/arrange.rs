//! Arrange mode on a game's page: the four-arrows button (or ••• → Arrange logo
//! and background) turns it on; only then can the logo and the background be moved, so hovering
//! the page never shows outlines or handles. Logo: drag to move, scroll or the corner handle to
//! resize, double-click to reset. Background: drag to pan, scroll to zoom (never below filling
//! the header), double-click to reset. A small floating bar: Done · Reset logo · Reset
//! background; Esc = Done. Saved per game (`logo_placement`, `background_placement`) when a drag
//! ends / a scroll pauses; Done offers Undo for the whole session.

use super::super::{DesktopUi, UndoWhat};
use crate::app::AppState;
use crate::model::{BackgroundPlacement, Game, GameId, LogoPlacement, UserEdits, aligned_uv};
use egui::{Color32, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

/// Seconds after the last scroll step before a zoom / resize is written to the DB.
const SCROLL_SAVE_SECS: f64 = 0.4;

/// Arrange mode on one game's page.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Arrange {
    pub id: GameId,
    /// What the game had when Arrange was turned on (Done's Undo puts it back).
    before: Layout,
    /// Live values: drawn by the page (and the ambient background) while arranging.
    pub live: Layout,
    /// When a scroll last changed something not saved yet.
    unsaved_at: Option<f64>,
    /// Done was asked for (button, Esc, the toggle): finished by `arrange_tick`.
    pub done: bool,
    /// The floating bar sits at the bottom (the logo was in the top half when Arrange started;
    /// fixed for the session so it doesn't jump while the logo is dragged).
    bar_low: Option<bool>,
}

impl Arrange {
    pub fn new(g: &Game) -> Arrange {
        let saved = Layout::of(&g.edits);
        let live =
            Layout { logo: saved.logo.map(LogoPlacement::clamped), bg: saved.bg.map(BackgroundPlacement::clamped) };
        Arrange { id: g.id, before: live, live, unsaved_at: None, done: false, bar_low: None }
    }
}

/// What Arrange changes on a game, kept in its edits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Layout {
    pub logo: Option<LogoPlacement>,
    pub bg: Option<BackgroundPlacement>,
}

impl Layout {
    /// As saved in `e`.
    pub fn of(e: &UserEdits) -> Layout {
        Layout { logo: e.logo_placement, bg: e.background_placement }
    }
}

/// The picture filling a header: its size and the automatic crop's alignment.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HeroPic {
    pub tex: Vec2,
    pub align: Vec2,
}

/// Save a game's layout (one DB write; nothing when unchanged).
pub(crate) fn save_layout(st: &mut AppState, id: GameId, l: Layout) {
    let Some(g) = st.library.get(id) else { return };
    if Layout::of(&g.edits) == l {
        return;
    }
    let mut e = g.edits.clone();
    e.logo_placement = l.logo;
    e.background_placement = l.bg;
    st.set_edits(id, e);
}

/// Logo placement of a drawn logo rect on `hero`.
fn to_placement(hero: Rect, r: Rect) -> LogoPlacement {
    LogoPlacement {
        cx: (r.center().x - hero.left()) / hero.width().max(1.0),
        cy: (r.center().y - hero.top()) / hero.height().max(1.0),
        h: r.height() / hero.height().max(1.0),
    }
    .clamped()
}

impl DesktopUi {
    /// Turn Arrange on for `g` (off again when it already is).
    pub(crate) fn toggle_arrange(&mut self, g: &Game) {
        match &mut self.arrange {
            Some(a) if a.id == g.id => a.done = true,
            // Another game's session (can't normally happen): finish it first.
            Some(a) => a.done = true,
            None => self.arrange = Some(Arrange::new(g)),
        }
    }

    pub(crate) fn arranging(&self, id: GameId) -> bool {
        self.arrange.as_ref().is_some_and(|a| a.id == id && !a.done)
    }

    /// Every frame: save a paused scroll, end the session when Done was asked for or its game is
    /// no longer the one shown (with an Undo toast when something changed).
    pub(crate) fn arrange_tick(&mut self, ctx: &egui::Context, st: &mut AppState) {
        let Some(a) = &mut self.arrange else { return };
        let now = ctx.input(|i| i.time);
        if let Some(t) = a.unsaved_at {
            if now - t >= SCROLL_SAVE_SECS {
                a.unsaved_at = None;
                save_layout(st, a.id, a.live);
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(SCROLL_SAVE_SECS - (now - t)));
            }
        }
        let gone = st.library.get(a.id).is_none();
        // A dialog (Edit game, Edit artwork…) ends it too: its changes mustn't be overwritten.
        if self.dlg.any_open() {
            a.done = true;
        }
        if !(a.done || gone || self.selected != Some(a.id)) {
            return;
        }
        let Some(a) = self.arrange.take() else { return };
        if gone {
            return;
        }
        save_layout(st, a.id, a.live);
        if a.live != a.before {
            let name = st.library.get(a.id).map(|g| g.display_name().to_string()).unwrap_or_default();
            self.show_undo(ctx, UndoWhat::Arrange(a.id, a.before), format!("Saved the layout of “{name}”"), Some(a.id));
        }
    }

    /// Arrange mode's handling of the header `hero` (call after painting it, before the
    /// page's other widgets on it). `pic`: the picture drawn (None = no artwork); `logo`: the
    /// logo's rect when the page places logos (the Details view) and one is drawn.
    pub(crate) fn arrange_hero(
        &mut self,
        ui: &mut Ui,
        st: &mut AppState,
        id: GameId,
        hero: Rect,
        pic: Option<HeroPic>,
        logo: Option<Rect>,
    ) {
        if !self.arranging(id) {
            return;
        }
        let pal = self.pal;
        let now = ui.input(|i| i.time);
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        let mut save = false;

        // ---- background: drag to pan, scroll to zoom, double-click to reset
        let bg_resp = ui.interact(hero, egui::Id::NULL.with(("ls_arrange_bg", id)), Sense::click_and_drag());
        if let Some(pic) = pic
            && let Some(a) = &mut self.arrange
        {
            let size = hero.size();
            let cur = a.live.bg.unwrap_or_else(|| {
                BackgroundPlacement::from_uv(aligned_uv(pic.tex, size, pic.align), pic.tex, size, pic.align)
            });
            let (h, v) = cur.can_pan(pic.tex, size);
            if bg_resp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if bg_resp.hovered() && (h || v) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            if bg_resp.double_clicked() {
                a.live.bg = None;
                save = true;
            } else if bg_resp.dragged() {
                let d = bg_resp.drag_delta();
                // A drag along an axis the picture can't move keeps the automatic crop.
                let next = cur.panned(pic.tex, size, d);
                if d != Vec2::ZERO && (a.live.bg.is_some() || next != cur) {
                    a.live.bg = Some(next);
                }
            }
            if bg_resp.drag_stopped() {
                save = true;
            }
            if bg_resp.hovered() && !bg_resp.dragged() && scroll.abs() > 0.1 {
                ui.ctx().input_mut(|i| i.smooth_scroll_delta.y = 0.0);
                let anchor = ui.input(|i| i.pointer.hover_pos()).map_or(vec2(0.5, 0.5), |p| (p - hero.min) / size);
                let next = cur.zoomed(pic.tex, size, anchor, (1.0 + scroll * 0.002).clamp(0.8, 1.25));
                if a.live.bg.is_some() || next != cur {
                    a.live.bg = Some(next);
                    a.unsaved_at = Some(now);
                }
            }
        }

        // ---- logo: subtle frame + corner handle; drag to move, scroll / handle to resize
        if let Some(r) = logo
            && let Some(a) = &mut self.arrange
        {
            let frame = r.expand(4.0);
            let p = ui.painter();
            p.rect_stroke(frame, 6.0, Stroke::new(1.0, Color32::from_white_alpha(110)), egui::StrokeKind::Outside);
            let handle = Rect::from_center_size(frame.right_bottom(), vec2(12.0, 12.0));
            let resp = ui.interact(frame, egui::Id::NULL.with(("ls_arrange_logo", id)), Sense::click_and_drag());
            let hresp = ui.interact(handle.expand(3.0), egui::Id::NULL.with(("ls_arrange_logo_h", id)), Sense::drag());
            let hot = hresp.hovered() || hresp.dragged();
            let p = ui.painter();
            p.rect_filled(handle, 3.0, if hot { pal.accent } else { Color32::from_white_alpha(230) });
            p.rect_stroke(handle, 3.0, Stroke::new(1.0, Color32::from_black_alpha(120)), egui::StrokeKind::Outside);
            let cur = a.live.logo.unwrap_or_else(|| to_placement(hero, r));
            if hot {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
            } else if resp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            if resp.double_clicked() {
                a.live.logo = None;
                save = true;
            } else if resp.dragged() {
                let d = resp.drag_delta();
                let mut pl = cur;
                pl.cx += d.x / hero.width().max(1.0);
                pl.cy += d.y / hero.height().max(1.0);
                a.live.logo = Some(pl.clamped());
            }
            if hresp.dragged() {
                // The corner follows the pointer, the centre stays put.
                let d = hresp.drag_delta();
                let k = (r.width() / r.height().max(1.0)).max(0.1);
                let mut pl = cur;
                pl.h += (d.y + d.x / k) / hero.height().max(1.0);
                a.live.logo = Some(pl.clamped());
            }
            if resp.drag_stopped() || hresp.drag_stopped() {
                save = true;
            }
            if resp.hovered() && !resp.dragged() && scroll.abs() > 0.1 {
                ui.ctx().input_mut(|i| i.smooth_scroll_delta.y = 0.0);
                let mut pl = cur;
                pl.h *= (1.0 + scroll * 0.002).clamp(0.8, 1.25);
                a.live.logo = Some(pl.clamped());
                a.unsaved_at = Some(now);
            }
        }
        if save && let Some(a) = &mut self.arrange {
            a.unsaved_at = None;
            save_layout(st, id, a.live);
        }

        // ---- the floating bar
        let (has_logo, has_bg) =
            self.arrange.as_ref().map_or((false, false), |a| (a.live.logo.is_some(), a.live.bg.is_some()));
        let mut done = false;
        let mut reset_logo = false;
        let mut reset_bg = false;
        // Top centre; on a page with a logo, the right-hand corner away from the logo.
        let low = self
            .arrange
            .as_mut()
            .map(|a| *a.bar_low.get_or_insert(logo.is_some_and(|r| r.center().y < hero.center().y)));
        let (pivot, at) = match (logo, low) {
            (None, _) => (egui::Align2::CENTER_TOP, pos2(hero.center().x, hero.top() + 12.0)),
            (Some(_), Some(true)) => (egui::Align2::RIGHT_BOTTOM, pos2(hero.right() - 12.0, hero.bottom() - 12.0)),
            (Some(_), _) => (egui::Align2::RIGHT_TOP, pos2(hero.right() - 12.0, hero.top() + 12.0)),
        };
        egui::Area::new(egui::Id::NULL.with(("ls_arrange_bar", id)))
            .order(egui::Order::Foreground)
            .pivot(pivot)
            .fixed_pos(at)
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(pal.surface_alt)
                    .stroke(Stroke::new(1.0, pal.border))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .shadow(ui.visuals().popup_shadow)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            done = super::super::dialogs::primary_button(ui, "Done", &pal, true)
                                .on_hover_text("Esc")
                                .clicked();
                            // Only where the logo can be arranged (not the Grid / List panel).
                            if logo.is_some() {
                                reset_logo = ui
                                    .add_enabled(has_logo, egui::Button::new("Reset logo").min_size(vec2(0.0, 30.0)))
                                    .clicked();
                            }
                            reset_bg = ui
                                .add_enabled(has_bg, egui::Button::new("Reset background").min_size(vec2(0.0, 30.0)))
                                .clicked();
                        });
                    });
            });
        if let Some(a) = &mut self.arrange {
            if reset_logo {
                a.live.logo = None;
            }
            if reset_bg {
                a.live.bg = None;
            }
            if reset_logo || reset_bg {
                a.unsaved_at = None;
                save_layout(st, id, a.live);
            }
            if done {
                a.done = true;
            }
        }
    }
}

/// "Arrange logo and background" for the ••• menu.
pub(crate) fn arrange_menu_entry(ui: &mut Ui, arranging: bool) -> bool {
    let text = if arranging { "Done arranging" } else { "Arrange logo and background" };
    ui.button(text).clicked()
}
