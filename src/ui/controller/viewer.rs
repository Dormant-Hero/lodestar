//! Controller-mode artwork viewer (the details page's Artwork button, or the game menu's "View
//! artwork"): the original image fit to the screen over a dark backdrop. D-pad ←/→ (or LB / RB)
//! steps through the game's artwork, X / Y toggle a closer look (100 %, centred; the d-pad moves
//! it, ←/→ only while the image is wider than the screen), B closes.
//! The shared parts (artwork list, sizes, zoom math) are in ui/art_viewer.rs.

use super::look::Look;
use crate::art::ArtSlot;
use crate::images::{ImageCache, ImgSize};
use crate::input::Nav;
use crate::model::{Game, GameId};
use crate::theme::render::draw::{Button, Hint, fade};
use crate::ui::art_viewer::{self as av, ArtEntry, Dims, Zoom};
use egui::{Color32, Rect, Stroke, pos2, vec2};

#[derive(Clone, Copy, PartialEq)]
struct Frame {
    area: Rect,
    view: egui::Vec2,
    img: egui::Vec2,
    one: f32,
}

pub struct CtlViewer {
    pub game: GameId,
    entries: Vec<ArtEntry>,
    idx: usize,
    zoom: Option<Zoom>,
    /// The viewing area, the full-width view a zoomed image is clipped to and pans across, the
    /// image size and 100 % of the last frame (for zooming / panning on input).
    last: Option<Frame>,
    dims: Dims,
}

impl CtlViewer {
    /// None when the game has no artwork file.
    pub fn open(ctx: &egui::Context, g: &Game) -> Option<CtlViewer> {
        let entries = av::existing_art(&g.art);
        if entries.is_empty() {
            return None;
        }
        let dims = Dims::spawn(ctx, entries.iter().map(|e| e.path.clone()).collect());
        Some(CtlViewer {
            game: g.id,
            idx: av::start_index(&entries, ArtSlot::Cover),
            entries,
            zoom: None,
            last: None,
            dims,
        })
    }

    /// Handle one input; false when the viewer closes.
    pub fn nav(&mut self, nav: Nav) -> bool {
        let n = self.entries.len();
        match nav {
            Nav::Back | Nav::Menu | Nav::Guide => return false,
            // Zoomed into an image wider than the screen: ←/→ move along it.
            Nav::Left | Nav::Right
                if self.zoom.is_some_and(|z| self.last.is_some_and(|f| f.img.x * z.scale > f.view.x + 0.5)) =>
            {
                if let (Some(z), Some(f)) = (&mut self.zoom, self.last) {
                    let d = f.view.x * 0.25 * if nav == Nav::Left { 1.0 } else { -1.0 };
                    z.pan(vec2(d, 0.0), f.view, f.img);
                }
            }
            Nav::Left | Nav::Right | Nav::PageUp | Nav::PageDown if n > 1 => {
                let back = matches!(nav, Nav::Left | Nav::PageUp);
                self.idx = av::step_index(self.idx, if back { -1 } else { 1 }, n);
                self.zoom = None;
                self.last = None;
            }
            Nav::Favorite | Nav::Search | Nav::Accept => {
                if let Some(f) = self.last {
                    self.zoom = av::toggle_centered(self.zoom, f.area, f.view, f.img, f.one);
                }
            }
            Nav::Up | Nav::Down => {
                if let (Some(z), Some(f)) = (&mut self.zoom, self.last) {
                    let d = f.area.height() * 0.25 * if nav == Nav::Up { 1.0 } else { -1.0 };
                    z.pan(vec2(0.0, d), f.view, f.img);
                }
            }
            _ => {}
        }
        true
    }

    /// Draw over `screen`; true while something animates (the loading spinner).
    pub fn draw(&mut self, ui: &egui::Ui, screen: Rect, look: &Look, images: &mut ImageCache) -> bool {
        let Some(entry) = self.entries.get(self.idx).cloned() else {
            return false;
        };
        let p = ui.painter_at(screen);
        // Opaque: the page's own button hints would otherwise show through under the viewer's.
        p.rect_filled(screen, 0.0, Color32::from_rgb(6, 7, 9));
        let one = 1.0 / ui.ctx().pixels_per_point();
        let margin_x = look.h(0.1);
        let area =
            Rect::from_min_max(screen.min + vec2(margin_x, look.h(0.11)), screen.max - vec2(margin_x, look.h(0.11)));
        let shown = av::shown(images, &entry.path);
        if self.entries.len() > 1 {
            for k in [1, -1] {
                let j = av::step_index(self.idx, k, self.entries.len());
                images.prefetch(&self.entries[j].path, ImgSize::Original);
            }
        }
        let header = self.dims.get(&entry.path);
        let img = av::image_size(header, if shown.loading { None } else { shown.tex.as_ref() })
            .or_else(|| shown.tex.as_ref().map(|t| t.size_vec2()));
        let mut animating = false;
        if let Some(img) = img {
            let view = Rect::from_x_y_ranges(screen.x_range(), area.y_range());
            let frame = Frame { area, view: view.size(), img, one };
            if self.last.is_some_and(|f| f != frame) {
                self.zoom = None;
            }
            self.last = Some(frame);
            let z = self.zoom.unwrap_or_else(|| Zoom::fit(img, area.size(), one));
            if let Some(t) = &shown.tex {
                let clip = p.with_clip_rect(view);
                clip.image(
                    t.id(),
                    z.image_rect(area, img),
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }
        if shown.loading {
            let r = Rect::from_center_size(area.center(), vec2(look.h(0.06), look.h(0.06)));
            p.circle_filled(r.center(), look.h(0.04), Color32::from_black_alpha(150));
            crate::ui::widgets::paint_lazy_spinner(ui, r, look.text);
            animating = true;
        } else if shown.failed && shown.tex.is_none() {
            let g = p.layout_no_wrap("Couldn't open this image".into(), look.medium(0.026), look.dim);
            p.galley(area.center() - g.size() / 2.0, g, look.dim);
        }

        // Top: which artwork, its size, n / m.
        let x0 = screen.left() + look.h(0.05);
        let y0 = screen.top() + look.h(0.035);
        let label = p.layout_no_wrap(entry.slot.label().into(), look.bold(0.034), look.text);
        let lw = label.size().x;
        let lh = label.size().y;
        p.galley(pos2(x0, y0), label, look.text);
        let mut info = String::new();
        if let Some((w, h)) = header {
            info = format!("{w} × {h} px");
        }
        if self.entries.len() > 1 {
            if !info.is_empty() {
                info.push_str("   ·   ");
            }
            info.push_str(&format!("{} / {}", self.idx + 1, self.entries.len()));
        }
        if !info.is_empty() {
            let g = p.layout_no_wrap(info, look.medium(0.022), look.dim);
            let gy = y0 + lh - g.size().y - look.h(0.004);
            p.galley(pos2(x0 + lw + look.h(0.025), gy), g, look.dim);
        }

        // Side chevrons when there is more than one image (d-pad ←/→).
        if self.entries.len() > 1 {
            let s = look.h(0.028);
            let y = area.center().y;
            let col = fade(look.text, 0.7);
            let st = Stroke::new(look.h(0.006), col);
            let lx = screen.left() + margin_x / 2.0;
            let rx = screen.right() - margin_x / 2.0;
            p.line_segment([pos2(lx + s * 0.5, y - s), pos2(lx - s * 0.5, y)], st);
            p.line_segment([pos2(lx - s * 0.5, y), pos2(lx + s * 0.5, y + s)], st);
            p.line_segment([pos2(rx - s * 0.5, y - s), pos2(rx + s * 0.5, y)], st);
            p.line_segment([pos2(rx + s * 0.5, y), pos2(rx - s * 0.5, y + s)], st);
        }

        let hints =
            [Hint::new(Button::B, "Close"), Hint::new(Button::X, if self.zoom.is_some() { "Fit" } else { "Zoom" })];
        let hb = Rect::from_min_max(
            pos2(screen.left() + 0.04 * look.sw, screen.bottom() - 0.075 * look.sh),
            pos2(screen.right() - 0.045 * look.sw, screen.bottom() - 0.015 * look.sh),
        );
        look.paint_screen_hints(&p, hb, &hints, fade(look.text, 0.85));
        animating
    }
}
