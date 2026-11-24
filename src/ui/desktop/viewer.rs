//! Full-window artwork viewer (click the cover on a game's page, or the magnifier on the cover /
//! background): the original file at full resolution over a dimmed backdrop, fit to the window.
//! ←/→ (keys or the side buttons, or the chips at the bottom) step through the game's artwork,
//! the wheel zooms around the cursor, dragging pans a zoomed image (at fit, a sideways drag goes
//! to the previous / next picture), double-click toggles fit / 100 %, Esc / a click on the
//! backdrop / × closes. Zoom math: ui/art_viewer.rs.

use super::Pal;
use crate::app::AppState;
use crate::art::ArtSlot;
use crate::model::{Game, GameId};
use crate::ui::art_viewer::{self as av, ArtEntry, Dims, Zoom};
use crate::ui::widgets::{Icon, paint_icon, paint_lazy_spinner, with_alpha};
use egui::{Color32, CursorIcon, FontFamily, FontId, Id, Key, Modifiers, Order, Rect, Sense, Stroke, Ui, pos2, vec2};

pub(crate) struct ViewerDlg {
    pub game: GameId,
    name: String,
    entries: Vec<ArtEntry>,
    idx: usize,
    /// None = fit to the window.
    zoom: Option<Zoom>,
    /// Image size the zoom was made for (a stand-in texture swapped for the real one, or the
    /// header read late, resets it).
    zoom_img: Option<egui::Vec2>,
    dims: Dims,
    /// Clicks on the viewer so far: the second click of a double-click that opened it (on the
    /// cover) is the viewer's first, and must not zoom.
    clicks: u32,
    /// A drag on the fitted picture so far (points): a swipe to the previous / next one.
    swipe: egui::Vec2,
    /// Testing hook (`LODESTAR_DESKTOP_OPEN=viewer-…-zoom`): go to 100 % once the image is in.
    pub debug_zoom: bool,
}

impl ViewerDlg {
    /// None when the game has no artwork file.
    pub fn open(ctx: &egui::Context, g: &Game, slot: ArtSlot) -> Option<ViewerDlg> {
        let entries = av::existing_art(&g.art);
        if entries.is_empty() {
            return None;
        }
        let dims = Dims::spawn(ctx, entries.iter().map(|e| e.path.clone()).collect());
        Some(ViewerDlg {
            game: g.id,
            name: g.display_name().to_string(),
            idx: av::start_index(&entries, slot),
            entries,
            zoom: None,
            zoom_img: None,
            dims,
            clicks: 0,
            swipe: egui::Vec2::ZERO,
            debug_zoom: false,
        })
    }

    fn go(&mut self, delta: i32) {
        self.idx = av::step_index(self.idx, delta, self.entries.len());
        self.zoom = None;
        self.zoom_img = None;
        self.swipe = egui::Vec2::ZERO;
    }
}

const SIDE: f32 = 76.0;
const TOP: f32 = 64.0;
const BOTTOM: f32 = 70.0;

/// Draw the viewer; false once it is closed.
pub(super) fn viewer(ctx: &egui::Context, st: &mut AppState, d: &mut ViewerDlg, pal: &Pal, bold: &FontFamily) -> bool {
    if d.idx >= d.entries.len() || st.library.get(d.game).is_none() {
        return false;
    }
    let mut open = true;
    let n = d.entries.len();
    ctx.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::Escape) {
            open = false;
        }
        if n > 1 && i.consume_key(Modifiers::NONE, Key::ArrowLeft) {
            d.go(-1);
        }
        if n > 1 && i.consume_key(Modifiers::NONE, Key::ArrowRight) {
            d.go(1);
        }
    });
    if !open {
        return false;
    }
    let screen = ctx.content_rect();
    egui::Area::new(Id::NULL.with("ls_art_viewer"))
        .order(Order::Foreground)
        .fixed_pos(screen.min)
        .constrain(false)
        .show(ctx, |ui| {
            ui.set_min_size(screen.size());
            open = viewer_ui(ui, st, d, pal, bold, screen);
        });
    open
}

/// Where the viewer draws this frame.
#[derive(Clone, Copy)]
struct Geom {
    screen: Rect,
    /// The picture's area at fit (between the bars and the side buttons).
    area: Rect,
    /// A zoomed image spans the full window width (under the side buttons) but leaves the top
    /// and bottom bars readable: it is drawn clipped to `view` and pans across all of it.
    view: Rect,
    /// One physical pixel in points.
    one: f32,
}

fn viewer_ui(ui: &mut Ui, st: &mut AppState, d: &mut ViewerDlg, pal: &Pal, bold: &FontFamily, screen: Rect) -> bool {
    let bg = ui.interact(screen, Id::NULL.with("ls_viewer_bg"), Sense::click_and_drag());
    if bg.clicked() {
        d.clicks += 1;
    }
    let p = ui.painter().clone();
    p.rect_filled(screen, 0.0, Color32::from_black_alpha(244));

    let area = Rect::from_min_max(screen.min + vec2(SIDE, TOP), screen.max - vec2(SIDE, BOTTOM));
    let view = Rect::from_x_y_ranges(screen.x_range(), area.y_range());
    let g = Geom { screen, area, view, one: 1.0 / ui.ctx().pixels_per_point() };
    let entry = d.entries[d.idx].clone();
    let shown = av::shown(&mut st.images, &entry.path);
    // Prefetch the neighbours so stepping through is instant.
    if d.entries.len() > 1 {
        for k in [1, -1] {
            let j = av::step_index(d.idx, k, d.entries.len());
            st.images.prefetch(&d.entries[j].path, crate::images::ImgSize::Original);
        }
    }
    let header = d.dims.get(&entry.path);
    let img = av::image_size(header, if shown.loading { None } else { shown.tex.as_ref() })
        .or_else(|| shown.tex.as_ref().map(|t| t.size_vec2()));
    let image_rect = img.map(|img| show_image(ui, d, &p, &bg, &g, img, &shown));
    if bg.drag_stopped() {
        let step = av::swipe_step(std::mem::take(&mut d.swipe), view.width());
        if d.zoom.is_none() && step != 0 && d.entries.len() > 1 {
            d.go(step);
        }
    } else if !bg.dragged() {
        d.swipe = egui::Vec2::ZERO;
    }
    if d.swipe != egui::Vec2::ZERO {
        // Keep animating while the drag is held still (no input, no repaint otherwise).
        ui.ctx().request_repaint();
    }
    if shown.loading {
        let c = image_rect.map(|r| r.intersect(view).center()).unwrap_or(area.center());
        let sr = Rect::from_center_size(c, vec2(44.0, 44.0));
        p.circle_filled(sr.center(), 26.0, Color32::from_black_alpha(150));
        paint_lazy_spinner(ui, sr, Color32::from_gray(235));
    } else if shown.failed && shown.tex.is_none() {
        let g =
            p.layout_no_wrap("Couldn't open this image".into(), FontId::proportional(16.0), Color32::from_gray(200));
        p.galley(area.center() - g.size() / 2.0, g, Color32::from_gray(200));
    }

    let size_text = match (header, shown.tex.as_ref()) {
        (Some((w, h)), _) => format!("{w} × {h} px"),
        (None, Some(t)) if !shown.loading && !shown.failed => {
            let [w, h] = t.size();
            format!("{w} × {h} px")
        }
        _ => String::new(),
    };
    paint_title(&p, &d.name, entry.slot, size_text, screen, bold);
    let mut keep = true;
    let close = Rect::from_center_size(pos2(screen.right() - 36.0, screen.top() + 34.0), vec2(40.0, 40.0));
    if round_button(ui, close, "ls_viewer_close", Icon::Close, "Close (Esc)", pal) {
        keep = false;
    }
    // Sides: previous / next.
    if d.entries.len() > 1 {
        let y = area.center().y;
        let prev = Rect::from_center_size(pos2(screen.left() + SIDE / 2.0, y), vec2(48.0, 48.0));
        let next = Rect::from_center_size(pos2(screen.right() - SIDE / 2.0, y), vec2(48.0, 48.0));
        if round_button(ui, prev, "ls_viewer_prev", Icon::ChevronLeft, "Previous (←)", pal) {
            d.go(-1);
        }
        if round_button(ui, next, "ls_viewer_next", Icon::ChevronRight, "Next (→)", pal) {
            d.go(1);
        }
    }
    bottom_bar(ui, d, &p, &g, img, pal);

    // A click on the backdrop (not the image, not a drag) closes, except the second click of
    // the double-click on the cover that opened the viewer.
    let opening_double = bg.double_clicked() && d.clicks <= 1;
    if bg.clicked()
        && !opening_double
        && let Some(pos) = bg.interact_pointer_pos()
        && !image_rect.is_some_and(|r| r.intersect(view).contains(pos))
    {
        keep = false;
    }
    keep
}

/// Zoom (wheel, pinch, double-click), pan / swipe, and the picture itself. Returns where the
/// image is drawn.
fn show_image(
    ui: &Ui,
    d: &mut ViewerDlg,
    p: &egui::Painter,
    bg: &egui::Response,
    g: &Geom,
    img: egui::Vec2,
    shown: &av::Shown,
) -> Rect {
    let Geom { area, view, one, .. } = *g;
    if d.zoom_img != Some(img) {
        d.zoom = None;
        d.zoom_img = Some(img);
    }
    // The window may have been resized since the zoom was made.
    if let Some(z) = &mut d.zoom {
        z.clamp(view.size(), img);
    }
    if d.debug_zoom && !shown.loading {
        d.debug_zoom = false;
        d.zoom = av::toggle_100(None, area.center(), area, view.size(), img, one);
    }
    // Wheel / pinch zoom around the cursor (anywhere over the viewer).
    if let Some(cursor) = bg.hover_pos() {
        let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let factor = pinch * (scroll / 300.0).exp();
        if (factor - 1.0).abs() > 1e-4 {
            let c = if view.contains(cursor) { cursor } else { area.center() };
            d.zoom = av::zoom_by(d.zoom, factor, c, area, view.size(), img, one);
            d.zoom_img = Some(img);
        }
    }
    if bg.dragged() {
        match &mut d.zoom {
            Some(z) => z.pan(bg.drag_delta(), view.size(), img),
            // At fit the picture follows a sideways drag (with more than one picture).
            None if d.entries.len() > 1 => d.swipe += bg.drag_delta(),
            None => {}
        }
    }
    if d.zoom.is_some() {
        d.swipe = egui::Vec2::ZERO;
    }
    let z = d.zoom.unwrap_or_else(|| Zoom::fit(img, area.size(), one));
    let mut r = z.image_rect(area, img);
    if d.zoom.is_none() {
        // Damped, so a short drag that turns out not to be a swipe barely moves it.
        r = r.translate(vec2(d.swipe.x * 0.6, 0.0));
    }
    if let Some(t) = &shown.tex {
        let clip = p.with_clip_rect(view);
        clip.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    if bg.double_clicked()
        && !bg.triple_clicked()
        && d.clicks >= 2
        && let Some(pos) = bg.interact_pointer_pos()
        && r.intersect(view).contains(pos)
    {
        d.zoom = av::toggle_100(d.zoom, pos, area, view.size(), img, one);
        d.zoom_img = Some(img);
    }
    if bg.hovered() && d.zoom.is_some() {
        ui.ctx().set_cursor_icon(if bg.dragged() { CursorIcon::Grabbing } else { CursorIcon::Grab });
    }
    r
}

/// Top left: the game's name, which artwork and its size.
fn paint_title(p: &egui::Painter, name: &str, slot: ArtSlot, size_text: String, screen: Rect, bold: &FontFamily) {
    let name = p.layout_no_wrap(name.to_string(), FontId::proportional(13.0), Color32::from_gray(170));
    p.galley(pos2(screen.left() + 22.0, screen.top() + 14.0), name, Color32::WHITE);
    let label = p.layout_no_wrap(slot.label().to_string(), FontId::new(18.0, bold.clone()), Color32::WHITE);
    let ly = screen.top() + 32.0;
    let lw = label.size().x;
    p.galley(pos2(screen.left() + 22.0, ly), label, Color32::WHITE);
    if !size_text.is_empty() {
        let g = p.layout_no_wrap(size_text, FontId::proportional(14.0), Color32::from_gray(180));
        p.galley(pos2(screen.left() + 22.0 + lw + 12.0, ly + 3.0), g, Color32::WHITE);
    }
}

/// Bottom: one chip per artwork (click to jump), the zoom level and how to zoom.
fn bottom_bar(ui: &Ui, d: &mut ViewerDlg, p: &egui::Painter, g: &Geom, img: Option<egui::Vec2>, pal: &Pal) {
    let Geom { screen, area, one, .. } = *g;
    let chip_font = FontId::proportional(13.0);
    let chips: Vec<_> =
        d.entries.iter().map(|e| p.layout_no_wrap(e.slot.label().into(), chip_font.clone(), Color32::WHITE)).collect();
    let gap = 8.0;
    let chip_w: Vec<f32> = chips.iter().map(|g| g.size().x + 26.0).collect();
    let total = chip_w.iter().sum::<f32>() + gap * (chips.len().saturating_sub(1)) as f32;
    let cy = screen.bottom() - BOTTOM / 2.0;
    let mut x = screen.center().x - total / 2.0;
    let mut jump = None;
    for (i, g) in chips.into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(x, cy - 14.0), vec2(chip_w[i], 28.0));
        x += chip_w[i] + gap;
        let resp = ui
            .interact(r, Id::NULL.with(("ls_viewer_chip", i)), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let cur = i == d.idx;
        let fill = if cur {
            with_alpha(pal.accent, 220)
        } else if resp.hovered() {
            Color32::from_white_alpha(40)
        } else {
            Color32::from_white_alpha(18)
        };
        p.rect_filled(r, 14.0, fill);
        let fg = if cur { pal.accent_text } else { Color32::from_gray(225) };
        p.galley(r.center() - g.size() / 2.0, g, fg);
        if resp.clicked() {
            jump = Some(i);
        }
    }
    if let Some(i) = jump
        && i != d.idx
    {
        d.idx = i;
        d.zoom = None;
        d.zoom_img = None;
    }
    if let Some(img) = img {
        let z = d.zoom.unwrap_or_else(|| Zoom::fit(img, area.size(), one));
        let text =
            if d.zoom.is_none() { format!("Fit · {}", av::percent(z.scale, one)) } else { av::percent(z.scale, one) };
        let g = p.layout_no_wrap(text, FontId::proportional(13.0), Color32::from_gray(200));
        p.galley(pos2(screen.right() - 22.0 - g.size().x, cy - g.size().y / 2.0), g, Color32::WHITE);
    }
    let hint = p.layout_no_wrap(
        hint_text(d.zoom.is_some(), d.entries.len()).into(),
        FontId::proportional(12.0),
        Color32::from_gray(130),
    );
    if screen.left() + 22.0 + hint.size().x + 16.0 < screen.center().x - total / 2.0 {
        p.galley(pos2(screen.left() + 22.0, cy - hint.size().y / 2.0), hint, Color32::WHITE);
    }
}

/// The bottom-left hint: dragging moves the picture only when it is zoomed in; at fit a sideways
/// drag goes to the previous / next picture (when there is one).
fn hint_text(zoomed: bool, pictures: usize) -> &'static str {
    if zoomed {
        "Drag to move · scroll to zoom · double-click: fit"
    } else if pictures > 1 {
        "Scroll or double-click to zoom · drag sideways for the next picture"
    } else {
        "Scroll or double-click to zoom"
    }
}

/// Round translucent icon button painted at `rect`. True when clicked.
fn round_button(ui: &mut Ui, rect: Rect, id: &str, icon: Icon, tip: &str, pal: &Pal) -> bool {
    let resp = ui.interact(rect, Id::NULL.with(id), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    let p = ui.painter();
    let hov = resp.hovered();
    // Dark and translucent: readable over the backdrop and over a zoomed image alike.
    let fill = if hov {
        crate::ui::widgets::mix(Color32::from_gray(30), pal.accent, 0.45)
    } else {
        Color32::from_rgba_unmultiplied(28, 30, 36, 170)
    };
    let rad = rect.width() / 2.0;
    p.circle_filled(rect.center(), rad, fill);
    p.circle_stroke(rect.center(), rad - 0.5, Stroke::new(1.0, Color32::from_white_alpha(60)));
    paint_icon(p, rect.shrink(rect.width() * 0.28), icon, Color32::from_gray(240));
    let clicked = resp.clicked();
    resp.on_hover_text(tip);
    clicked
}

#[cfg(test)]
mod tests {
    use super::hint_text;

    #[test]
    fn hint_mentions_dragging_to_move_only_when_zoomed() {
        assert!(hint_text(true, 3).starts_with("Drag to move"));
        assert!(hint_text(true, 1).starts_with("Drag to move"));
        assert!(!hint_text(false, 3).contains("move"));
        assert!(hint_text(false, 3).contains("drag sideways"));
        assert!(!hint_text(false, 1).contains("drag"));
    }
}
