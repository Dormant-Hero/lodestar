//! Ambient background: the selected game's art, blurred and dimmed, behind the whole window,
//! with translucent panels on top (like Playnite's "background image" option).
//!
//! Cheap by design: the art is loaded as `ImgSize::Backdrop` (<= 160 px, blurred once on the
//! decode thread) and stretched by the GPU; repaints happen only during the short crossfade.
//!
//! Also the theme's window backdrop (`DesktopStyle::window_gradient` / `background_picture`)
//! and the panel brushes (`topbar_fill`, `sidebar_fill`, `details_fill`): a gradient panel gets
//! a transparent frame and its mesh is painted into a slot reserved before the panel
//! (`panel_slot` / `paint_panel`). Meshes are cached (theme::render::gradient).

use super::DesktopUi;
use crate::app::AppState;
use crate::config::DesktopView;
use crate::images::ImgSize;
use crate::model::BackgroundPlacement;
use crate::theme::render::gradient::GradientCache;
use crate::theme::{Brush, DesktopStyle, Gradient, gradient_color_at};
use crate::ui::widgets::{mix, vgradient_mesh, with_alpha};
use egui::{Color32, Rect, Stroke, vec2};
use std::path::PathBuf;

const FADE_SECS: f64 = 0.45;
/// Lodestar's own corner radius of the boxes on a game page.
const CARD_RADIUS: u8 = 8;

#[derive(Default)]
pub(crate) struct Ambient {
    /// The picture and the game's background placement (the same crop as its page header).
    cur: Option<(PathBuf, Option<BackgroundPlacement>)>,
    prev: Option<(PathBuf, Option<BackgroundPlacement>)>,
    /// When `cur`'s texture first became available (crossfade start).
    shown_at: Option<f64>,
    /// Gradient panels this frame: (gradient, opacity) per `Panel`.
    panel_grads: [Option<(Gradient, f32)>; 5],
    grads: GradientCache,
    /// The theme's background picture: (theme folder, theme path, resolved file). Keyed by
    /// the folder too: two themes can name the same relative file.
    picture: Option<(Option<PathBuf>, String, Option<PathBuf>)>,
    /// The game's art waits for the top bar: its slot behind the panels and the window's rect.
    art_slot: Option<(egui::layers::ShapeIdx, Rect)>,
}

/// Panels a theme brush can fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Panel {
    /// Top bar and status bar.
    Bar = 0,
    Sidebar = 1,
    /// The details panel (grid / list views).
    Details = 2,
    /// The game page (details view).
    Page = 3,
    /// The game list (details view list column).
    List = 4,
}

/// Panel fills for this frame (opaque normally, translucent over an ambient background).
#[derive(Clone, Copy)]
pub(crate) struct Fills {
    pub bar: Color32,
    pub surface: Color32,
    pub center: Color32,
    /// Left explorer panel.
    pub sidebar: Color32,
    pub list: Color32,
    /// List view: column header and alternate-row stripe.
    pub header: Color32,
    pub stripe: Color32,
    /// The game page of the details view.
    pub page: Color32,
    /// Boxes on the game page (the facts next to the description), with their border and
    /// corner radius.
    pub card: Color32,
    pub card_border: Stroke,
    pub card_radius: u8,
}

impl Fills {
    pub(crate) const BLACK: Fills = Fills {
        bar: Color32::BLACK,
        surface: Color32::BLACK,
        center: Color32::BLACK,
        sidebar: Color32::BLACK,
        list: Color32::BLACK,
        header: Color32::BLACK,
        stripe: Color32::BLACK,
        page: Color32::BLACK,
        card: Color32::BLACK,
        card_border: Stroke::NONE,
        card_radius: CARD_RADIUS,
    };
}

/// A theme colour as a panel fill (translucent at `o` over an ambient background).
fn solid(c: Color32, o: Option<f32>) -> Color32 {
    match o {
        None => c,
        Some(o) => {
            let a = c.to_srgba_unmultiplied()[3];
            with_alpha(c, a.min((o.clamp(0.0, 1.0) * 255.0).round() as u8))
        }
    }
}

impl DesktopUi {
    pub(crate) fn ambient_enabled(st: &AppState) -> bool {
        let d = &st.desktop_theme.file.desktop;
        let in_view = !d.background_image_details_only || st.config.desktop.view == DesktopView::Details;
        st.config.desktop.background_image.unwrap_or(d.background_image && in_view)
    }

    pub(crate) fn fills(&mut self, st: &AppState, ambient: bool) -> Fills {
        let d = &st.desktop_theme.file.desktop;
        let mut f = self.base_fills(st, ambient);
        // Over the game's art Lodestar's themes turn their panels translucent. A Playnite
        // theme's panels show the window only where the theme paints no fill of its own: its
        // brushes are drawn as they are, opaque ones stay opaque.
        let o = (ambient && !d.see_through_panels).then(|| d.panel_opacity.clamp(0.0, 1.0));
        if !ambient && !d.see_through_panels {
            // The theme's window background shows through the game area.
            if let Some(Brush::Solid(c)) = &d.window_gradient {
                f.center = c.to_egui();
                f.page = f.center;
            }
            if d.has_backdrop() {
                f.center = Color32::TRANSPARENT;
                f.page = Color32::TRANSPARENT;
                f.list = with_alpha(f.list, 140);
                f.header = with_alpha(f.header, 200);
                f.stripe = with_alpha(f.stripe, 90);
            }
        }
        let regions = [
            (Panel::Bar, &d.topbar_fill),
            (Panel::Sidebar, &d.sidebar_fill),
            (Panel::Details, &d.details_fill),
            (Panel::Page, &d.details_fill),
            (Panel::List, &d.list_fill),
        ];
        for (panel, brush) in regions {
            let field = match panel {
                Panel::Bar => &mut f.bar,
                Panel::Sidebar => &mut f.sidebar,
                Panel::Details => &mut f.surface,
                Panel::Page => &mut f.page,
                Panel::List => &mut f.list,
            };
            let slot = &mut self.ambient.panel_grads[panel as usize];
            match brush.as_ref().map(|b| (b, b.gradient())) {
                Some((_, Some(g))) => {
                    let op = o.unwrap_or(1.0);
                    // Cloned only when the theme changes (no per-frame allocation).
                    if slot.as_ref().is_none_or(|(sg, so)| sg != g || *so != op) {
                        *slot = Some((g.clone(), op));
                    }
                    *field = Color32::TRANSPARENT;
                }
                Some((b, None)) => {
                    *slot = None;
                    *field = solid(b.average().to_egui(), o);
                }
                None => *slot = None,
            }
        }
        self.card_style(d, &mut f);
        f
    }

    /// The theme's own facts box on the game page: its fill, border and corners.
    fn card_style(&self, d: &DesktopStyle, f: &mut Fills) {
        if let Some(b) = &d.card_fill {
            f.card = b.average().to_egui();
        }
        if let Some(b) = d.card_border.filter(|b| b.width.is_finite() && b.width > 0.0) {
            let color = b.color.map_or(self.pal.border, |c| c.to_egui());
            f.card_border = Stroke::new(b.width.clamp(0.0, 8.0), color);
        }
        if let Some(r) = d.card_radius {
            f.card_radius = crate::theme::radius_u8(r);
        }
    }

    /// Lodestar's own panel fills (no theme brushes).
    fn base_fills(&self, st: &AppState, ambient: bool) -> Fills {
        let p = &self.pal;
        let list = mix(p.bg, p.surface, 0.5);
        let header = mix(p.bg, p.surface, 0.6);
        let stripe = mix(p.bg, p.surface, 0.35);
        let d = &st.desktop_theme.file.desktop;
        if d.see_through_panels {
            let clear = Color32::TRANSPARENT;
            let a = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u8;
            return Fills {
                bar: clear,
                surface: clear,
                center: clear,
                sidebar: clear,
                list: clear,
                // The list view's column header and row stripes still need to read as such.
                header: with_alpha(header, a(d.panel_opacity + 0.08)),
                stripe: with_alpha(p.text, 8),
                page: clear,
                // Where the theme paints no box around the facts (`card_fill`), a faint one keeps
                // them grouped without a slab of the theme's popup colour on the window.
                card: with_alpha(p.surface, a(d.panel_opacity * 0.5)),
                card_border: Stroke::NONE,
                card_radius: CARD_RADIUS,
            };
        }
        if !ambient {
            return Fills {
                bar: p.bar,
                surface: p.surface,
                center: p.bg,
                sidebar: p.surface,
                list,
                header,
                stripe,
                page: p.bg,
                card: p.surface,
                card_border: Stroke::NONE,
                card_radius: CARD_RADIUS,
            };
        }
        let o = d.panel_opacity.clamp(0.0, 1.0);
        let a = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u8;
        let center = with_alpha(p.bg, a(o * 0.3));
        Fills {
            page: center,
            bar: with_alpha(p.bar, a(o + 0.12)),
            surface: with_alpha(p.surface, a(o)),
            // The game grid shows the most of the background.
            center: with_alpha(p.bg, a(o * 0.3)),
            // Side panels stay readable but let the art through, like the rest of the window.
            sidebar: with_alpha(p.bg, a(o * 0.5)),
            list: with_alpha(p.bg, a(o * 0.55)),
            header: with_alpha(header, a(o + 0.08)),
            // Opaque stripes over the background read as a hard black-and-blue zebra; keep
            // them as faint as the rows between them.
            stripe: with_alpha(stripe, a(o * 0.45)),
            card: p.surface,
            card_border: Stroke::NONE,
            card_radius: CARD_RADIUS,
        }
    }

    /// Reserve the paint slot behind a panel (call right before showing it).
    pub(crate) fn panel_slot(&self, ui: &egui::Ui) -> egui::layers::ShapeIdx {
        ui.painter().add(egui::Shape::Noop)
    }

    /// Paint a panel's theme gradient into its slot, once the panel's rect is known.
    pub(crate) fn paint_panel(&mut self, ui: &egui::Ui, slot: egui::layers::ShapeIdx, rect: Rect, panel: Panel) {
        let Some((g, o)) = &self.ambient.panel_grads[panel as usize] else {
            return;
        };
        let feather = 1.0 / ui.ctx().pixels_per_point();
        if let Some(m) = self.ambient.grads.mesh(rect, [0.0; 4], g, *o, feather) {
            ui.painter().set(slot, egui::Shape::Mesh(m));
        }
    }

    /// The window needs painting before the panels: an ambient background or the theme's
    /// window backdrop.
    pub(crate) fn backdrop_wanted(st: &AppState) -> bool {
        let d = &st.desktop_theme.file.desktop;
        d.has_backdrop() || d.see_through_panels
    }

    /// Paint the background over the whole window (the theme's backdrop and picture) and,
    /// when `art`, reserve the place of the selected game's art over it (`paint_art` fills
    /// it once the top bar is placed). Call before any panel.
    pub(crate) fn paint_ambient(&mut self, ui: &egui::Ui, st: &mut AppState, art: bool) {
        let rect = ui.max_rect();
        let p = ui.painter();
        self.ambient.grads.next_frame();
        let d = &st.desktop_theme.file.desktop;
        let base = match &d.window_gradient {
            Some(Brush::Solid(c)) => c.to_egui(),
            _ => self.pal.bg,
        };
        p.rect_filled(rect, 0.0, base);
        if let Some(g) = d.window_gradient.as_ref().and_then(Brush::gradient)
            && let Some(m) = self.ambient.grads.mesh(rect, [0.0; 4], g, 1.0, 0.0)
        {
            p.add(egui::Shape::Mesh(m));
        }
        let picture_over_art = art && d.background_picture_over_art;
        if !picture_over_art {
            self.paint_picture(p, st, rect);
        }
        self.ambient.art_slot = art.then(|| (p.add(egui::Shape::Noop), rect));
        if picture_over_art {
            self.paint_picture(p, st, rect);
        }
    }

    /// The theme's background picture (`background_picture`), dimmed as it says.
    fn paint_picture(&mut self, p: &egui::Painter, st: &mut AppState, rect: Rect) {
        let Some(pic) = &st.desktop_theme.file.desktop.background_picture else {
            return;
        };
        let theme = &st.desktop_theme;
        if self.ambient.picture.as_ref().is_none_or(|(dir, k, _)| *k != pic.path || *dir != theme.dir) {
            self.ambient.picture = Some((theme.dir.clone(), pic.path.clone(), theme.resolve(&pic.path)));
        }
        if let Some((_, _, Some(path))) = &self.ambient.picture
            && let Some(tex) = st.images.get(path, ImgSize::Full)
        {
            let (r, uv) =
                crate::theme::render::draw::fit_rects_aligned(rect, tex.size_vec2(), pic.fit, vec2(0.5, 0.5), None);
            p.add(egui::epaint::RectShape::filled(r, 0, Color32::WHITE).with_texture(tex.id(), uv));
            let dim = if pic.dim.is_finite() { pic.dim.clamp(0.0, 1.0) } else { 0.0 };
            if dim > 0.0 {
                p.rect_filled(rect, 0.0, Color32::from_black_alpha((dim * 255.0).round() as u8));
            }
        }
    }

    /// The selected game's art in the place `paint_ambient` kept for it, crossfading between
    /// games: over the whole window, or under the top bar (`top_bar`, its rect this frame)
    /// when the theme says so.
    pub(crate) fn paint_art(&mut self, ui: &egui::Ui, st: &mut AppState, top_bar: Rect) {
        let Some((slot, window)) = self.ambient.art_slot.take() else {
            return;
        };
        let d = &st.desktop_theme.file.desktop;
        let rect = if d.background_image_below_bar {
            Rect::from_min_max(
                egui::pos2(window.left(), top_bar.bottom().clamp(window.top(), window.bottom())),
                window.max,
            )
        } else {
            window
        };
        self.follow_selection(st);

        let now = ui.input(|i| i.time);
        let cur_tex = self.ambient.cur.as_ref().and_then(|(path, _)| st.images.get(path, ImgSize::Backdrop));
        if cur_tex.is_some() && self.ambient.shown_at.is_none() {
            self.ambient.shown_at = Some(now);
        }
        let t = self.ambient.shown_at.map_or(0.0, |s| ((now - s) / FADE_SECS).clamp(0.0, 1.0)) as f32;
        let ease = t * t * (3.0 - 2.0 * t);

        let look = ArtLook::of(d, rect);
        let mut shapes = vec![];
        if ease < 1.0
            && let Some((prev, crop)) = &self.ambient.prev
            && let Some(tex) = st.images.get(prev, ImgSize::Backdrop)
        {
            look.picture(&mut shapes, &tex, *crop, 1.0);
        }
        if let Some(tex) = &cur_tex {
            let crop = self.ambient.cur.as_ref().and_then(|c| c.1);
            look.picture(&mut shapes, tex, crop, ease);
        }
        if self.ambient.shown_at.is_some() && ease < 1.0 {
            ui.ctx().request_repaint();
        } else if ease >= 1.0 {
            self.ambient.prev = None;
        }
        if look.dim_color.is_none() {
            // Dim for legibility, a little more towards the bottom.
            let a = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u8;
            shapes.push(egui::Shape::rect_filled(rect, 0.0, with_alpha(self.pal.bg, a(look.dim))));
            shapes.push(egui::Shape::mesh(vgradient_mesh(
                rect,
                Color32::TRANSPARENT,
                with_alpha(self.pal.bg, a(look.dim * 0.6)),
            )));
        }
        for layer in &d.background_image_layers {
            match layer.gradient() {
                Some(g) => shapes.extend(self.ambient.grads.mesh(rect, [0.0; 4], g, 1.0, 0.0).map(egui::Shape::Mesh)),
                None => shapes.push(egui::Shape::rect_filled(rect, 0.0, layer.average().to_egui())),
            }
        }
        ui.painter().set(slot, egui::Shape::Vec(shapes));
    }

    /// Follow the selected game's art (kept while nothing, or an art-less game, is selected).
    fn follow_selection(&mut self, st: &AppState) {
        let target = self.selected.and_then(|id| st.library.get(id)).and_then(|g| {
            // While arranging, the crop being dragged right now.
            let crop = match &self.arrange {
                Some(a) if a.id == g.id => a.bg,
                _ => g.edits.background_placement,
            };
            Some((g.art.background()?.clone(), crop))
        });
        // Same picture, another crop (Arrange): follow it at once, no crossfade.
        if let (Some((path, crop)), Some(cur)) = (&target, &mut self.ambient.cur)
            && *path == cur.0
        {
            cur.1 = *crop;
        }
        if target.is_some() && target != self.ambient.cur {
            // Mid-fade switches start from whatever is fully shown (avoid a flash).
            if self.ambient.shown_at.is_some() {
                self.ambient.prev = self.ambient.cur.take();
            }
            self.ambient.cur = target;
            self.ambient.shown_at = None;
        }
    }
}

/// How the game's art is drawn this frame.
struct ArtLook<'a> {
    rect: Rect,
    /// The theme's opacity for the art.
    opacity: f32,
    dim: f32,
    /// The colour the art is darkened towards inside the picture (where its masks let it
    /// through), as Playnite's darkening brush does; None: Lodestar's dimming of the whole
    /// window towards the background colour.
    dim_color: Option<Color32>,
    masks: &'a [Gradient],
}

/// Vertices per side of the mesh that carries a masked picture's fade.
const MASK_STEPS: usize = 24;

impl<'a> ArtLook<'a> {
    fn of(d: &'a DesktopStyle, rect: Rect) -> ArtLook<'a> {
        let opacity = d.background_image_opacity.filter(|o| o.is_finite()).map_or(1.0, |o| o.clamp(0.0, 1.0));
        let dim = if d.background_dim.is_finite() { d.background_dim.clamp(0.0, 1.0) } else { 0.0 };
        ArtLook {
            rect,
            opacity,
            dim,
            dim_color: d.background_dim_color.map(|c| c.to_egui()),
            masks: &d.background_image_masks,
        }
    }

    /// One picture at `alpha` (the crossfade).
    fn picture(
        &self,
        shapes: &mut Vec<egui::Shape>,
        tex: &egui::TextureHandle,
        crop: Option<BackgroundPlacement>,
        alpha: f32,
    ) {
        let uv = crate::model::background_uv(crop, tex.size_vec2(), self.rect.size(), vec2(0.5, 0.35));
        let alpha = self.opacity * alpha;
        let Some(dim_color) = self.dim_color else {
            // Lodestar's own themes: the picture alone (the dimming covers the whole window).
            if self.masks.is_empty() {
                let tint = Color32::WHITE.gamma_multiply(alpha);
                shapes.push(egui::epaint::RectShape::filled(self.rect, 0, tint).with_texture(tex.id(), uv).into());
            } else {
                let tint = |a: f32| Color32::WHITE.gamma_multiply(alpha * a);
                shapes.push(egui::Shape::mesh(self.mesh(Some((tex.id(), uv)), tint)));
            }
            return;
        };
        // The darkening is part of the picture: the texture times (1 - dim), plus the dim
        // colour's share added on top (premultiplied, alpha 0 = additive).
        let keep = 1.0 - self.dim;
        let shade = |a: f32| {
            let a = alpha * a;
            let k = (keep * a * 255.0).round() as u8;
            Color32::from_rgba_premultiplied(k, k, k, (a * 255.0).round() as u8)
        };
        shapes.push(egui::Shape::mesh(self.mesh(Some((tex.id(), uv)), shade)));
        if dim_color != Color32::BLACK && self.dim > 0.0 {
            let [r, g, b, _] = dim_color.to_array();
            let add = |a: f32| {
                let f = self.dim * alpha * a;
                let c = |v: u8| (v as f32 * f).round() as u8;
                Color32::from_rgba_premultiplied(c(r), c(g), c(b), 0)
            };
            shapes.push(egui::Shape::mesh(self.mesh(None, add)));
        }
    }

    /// A grid over the art's rect whose vertex colours come from `color(mask alpha there)`.
    fn mesh(&self, tex: Option<(egui::TextureId, Rect)>, color: impl Fn(f32) -> Color32) -> egui::Mesh {
        let steps = if self.masks.is_empty() { 2 } else { MASK_STEPS };
        let stops: Vec<_> = self.masks.iter().map(|g| (g, g.sorted_stops())).collect();
        let mask_at = |u: f32, v: u32| -> f32 {
            let v = v as f32 / (steps - 1) as f32;
            stops.iter().map(|(g, s)| gradient_color_at(s, g.param(u, v)).0[3] as f32 / 255.0).product()
        };
        let mut mesh = match tex {
            Some((id, _)) => egui::Mesh::with_texture(id),
            None => egui::Mesh::default(),
        };
        let r = self.rect;
        for yi in 0..steps as u32 {
            for xi in 0..steps as u32 {
                let (u, v) = (xi as f32 / (steps - 1) as f32, yi as f32 / (steps - 1) as f32);
                let pos = egui::pos2(r.left() + r.width() * u, r.top() + r.height() * v);
                let uv = tex.map_or(egui::epaint::WHITE_UV, |(_, uv)| {
                    egui::pos2(uv.left() + uv.width() * u, uv.top() + uv.height() * v)
                });
                mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: color(mask_at(u, yi)) });
            }
        }
        let w = steps as u32;
        for yi in 0..w - 1 {
            for xi in 0..w - 1 {
                let i = yi * w + xi;
                mesh.add_triangle(i, i + 1, i + w);
                mesh.add_triangle(i + 1, i + w + 1, i + w);
            }
        }
        mesh
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;
    use crate::theme::{
        BackgroundPicture, Color, CoverTitle, DesktopStyle, FontScale, Gradient, GradientStop, SelectionBorder, Theme,
        ThemeFile,
    };
    use std::sync::Arc;

    fn grad() -> Gradient {
        Gradient {
            stops: vec![
                GradientStop::new(Color::parse("#102040").unwrap(), 0.0),
                GradientStop::new(Color::parse("#406080").unwrap(), 0.6),
                GradientStop::new(Color::parse("#000000").unwrap(), 1.0),
            ],
            angle: Some(70.0),
            radial: None,
        }
    }

    fn themed(dir: &std::path::Path) -> Theme {
        let desktop = DesktopStyle {
            control_radius: Some(2.0),
            window_radius: Some(12.0),
            cover_radius: Some(10.0),
            cover_gap: Some(26.0),
            cover_title: Some(CoverTitle::Overlay),
            selection_border: Some(SelectionBorder { color: Some(Color::parse("#ff8800").unwrap()), width: 2.0 }),
            hover_darken: Some(0.3),
            window_gradient: Some(Brush::Gradient(Gradient::radial(grad().stops, [0.5, 0.0], [1.0, 1.0]))),
            sidebar_fill: Some(Brush::Solid(Color::parse("#101418").unwrap())),
            topbar_fill: Some(Brush::Gradient(grad())),
            details_fill: Some(Brush::Gradient(grad())),
            background_picture: Some(BackgroundPicture {
                path: "images/bg.png".into(),
                fit: crate::theme::Fit::Cover,
                dim: 0.3,
            }),
            details_background_dim: Some(0.4),
            details_background_fade: Some(0.8),
            font_scale: Some(FontScale { normal: 1.1, ..Default::default() }),
            tile_fill: Some(Brush::Solid(Color::parse("#151e3d").unwrap())),
            caption_fill: Some(Brush::Solid(Color::parse("#202020").unwrap())),
            list_fill: Some(Brush::Gradient(grad())),
            card_fill: Some(Brush::Solid(Color::parse("#18141e").unwrap())),
            card_border: Some(SelectionBorder { color: Some(Color::parse("#c8a040").unwrap()), width: 1.0 }),
            card_radius: Some(4.0),
            rows: Some(crate::theme::RowStyle {
                selected_fill: Some(Color::parse("#303a50").unwrap()),
                selected_bar: Some(SelectionBorder { color: None, width: 3.0 }),
                radius: Some(4.0),
                ..Default::default()
            }),
            play_fill: Some(Brush::Gradient(grad())),
            background_dim_color: Some(Color::parse("#100808").unwrap()),
            background_image_masks: vec![grad()],
            background_image_layers: vec![Brush::Gradient(grad()), Brush::Solid(Color::parse("#00000040").unwrap())],
            background_image_below_bar: true,
            background_picture_over_art: true,
            ..Default::default()
        };
        let file = ThemeFile { desktop, ..Default::default() };
        Theme { key: "pn".into(), file, dir: Some(dir.to_path_buf()) }
    }

    #[test]
    fn theme_brushes_become_panel_fills() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let mut ui = DesktopUi::new();
        // Lodestar's own look: nothing changes.
        let plain = ui.fills(&st, false);
        assert_eq!(plain.bar, ui.pal.bar);
        assert_eq!(plain.center, ui.pal.bg);
        assert_eq!(plain.page, ui.pal.bg);
        assert!(ui.ambient.panel_grads.iter().all(Option::is_none));
        st.desktop_theme = Arc::new(themed(dir.path()));
        let f = ui.fills(&st, false);
        // Gradient panels: transparent frames, the mesh goes behind them.
        assert_eq!(f.bar, Color32::TRANSPARENT);
        assert_eq!(f.surface, Color32::TRANSPARENT);
        assert!(ui.ambient.panel_grads[Panel::Bar as usize].is_some());
        assert!(ui.ambient.panel_grads[Panel::Details as usize].is_some());
        assert!(ui.ambient.panel_grads[Panel::Sidebar as usize].is_none());
        // A solid sidebar is its colour; the window backdrop shows through the game area.
        assert_eq!(f.sidebar, Color32::from_rgb(0x10, 0x14, 0x18));
        assert_eq!(f.center, Color32::TRANSPARENT);
        // The facts box: the theme's fill, border and corners.
        assert_eq!(f.card, Color32::from_rgb(0x18, 0x14, 0x1e));
        assert_eq!(f.card_border, Stroke::new(1.0, Color32::from_rgb(0xc8, 0xa0, 0x40)));
        assert_eq!(f.card_radius, 4);
        assert!(ui.ambient.panel_grads[Panel::List as usize].is_some());
        // Over an ambient background the theme's solid colours turn translucent.
        let f = ui.fills(&st, true);
        assert!(f.sidebar.a() < 255);
        assert!((ui.ambient.panel_grads[Panel::Bar as usize].as_ref().unwrap().1 - 0.78).abs() < 1e-4);
        // … but a Playnite theme's panels are drawn as the theme paints them: opaque stays opaque.
        let mut pn = themed(dir.path());
        pn.file.desktop.see_through_panels = true;
        st.desktop_theme = Arc::new(pn);
        let f = ui.fills(&st, true);
        assert_eq!(f.sidebar, Color32::from_rgb(0x10, 0x14, 0x18));
        assert_eq!(ui.ambient.panel_grads[Panel::Bar as usize].as_ref().unwrap().1, 1.0);
    }

    /// The art's masks fade it: the mesh's vertex alpha follows the mask.
    #[test]
    fn masked_art_fades_towards_the_masks_edge() {
        let fade = Gradient {
            stops: vec![
                GradientStop::new(Color::parse("#000000ff").unwrap(), 0.0),
                GradientStop::new(Color::parse("#00000000").unwrap(), 1.0),
            ],
            angle: Some(90.0),
            radial: None,
        };
        let d = DesktopStyle {
            background_dim: 0.5,
            background_dim_color: Some(Color::BLACK),
            background_image_masks: vec![fade],
            ..Default::default()
        };
        let rect = Rect::from_min_size(egui::Pos2::ZERO, vec2(100.0, 100.0));
        let look = ArtLook::of(&d, rect);
        let mesh = look.mesh(None, |a| Color32::from_white_alpha((a * 255.0) as u8));
        let alpha_at = |y: f32| mesh.vertices.iter().find(|v| (v.pos.y - y).abs() < 0.01).map(|v| v.color.a());
        assert_eq!(alpha_at(0.0), Some(255), "opaque at the top");
        assert_eq!(alpha_at(100.0), Some(0), "gone at the bottom");
        assert_eq!(mesh.vertices.len(), MASK_STEPS * MASK_STEPS);
        // Unmasked: a plain quad.
        let plain = DesktopStyle { background_image_masks: vec![], ..d };
        assert_eq!(ArtLook::of(&plain, rect).mesh(None, |_| Color32::WHITE).vertices.len(), 4);
    }

    /// A Playnite import: panels without a fill of their own show the window through, and the
    /// game art is on by default in the details view only (the user's setting covers all views).
    #[test]
    fn see_through_panels_and_details_only_art() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let mut ui = DesktopUi::new();
        let desktop = DesktopStyle {
            see_through_panels: true,
            background_image: true,
            background_image_details_only: true,
            sidebar_fill: Some(Brush::Solid(Color::parse("#101418").unwrap())),
            window_gradient: Some(Brush::Solid(Color::parse("#202020").unwrap())),
            ..Default::default()
        };
        st.desktop_theme =
            Arc::new(Theme { key: "pn".into(), file: ThemeFile { desktop, ..Default::default() }, dir: None });
        assert!(DesktopUi::backdrop_wanted(&st), "the window is painted before the clear panels");
        let f = ui.fills(&st, false);
        for clear in [f.bar, f.surface, f.center, f.list, f.page] {
            assert_eq!(clear, Color32::TRANSPARENT);
        }
        assert_eq!(f.sidebar, Color32::from_rgb(0x10, 0x14, 0x18), "the theme's own fill stays");
        assert!(f.card.a() > 0 && f.card.a() < 255, "the facts box stays faint: {:?}", f.card);
        st.config.desktop.view = DesktopView::Details;
        assert!(DesktopUi::ambient_enabled(&st));
        st.config.desktop.view = DesktopView::Grid;
        assert!(!DesktopUi::ambient_enabled(&st));
        st.config.desktop.background_image = Some(true);
        assert!(DesktopUi::ambient_enabled(&st));
    }

    /// Two themes naming the same relative background picture: switching shows the new one.
    #[test]
    fn background_picture_follows_the_theme_folder() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let mut st = AppState::for_test(a.path());
        let mut ui = DesktopUi::new();
        let ctx = st.ctx().clone();
        for dir in [a.path(), b.path()] {
            st.desktop_theme = Arc::new(themed(dir));
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 600.0))),
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |eui| ui.show(eui, &mut st));
            let resolved = ui.ambient.picture.as_ref().and_then(|(_, _, p)| p.clone());
            assert_eq!(resolved, Some(dir.join("images/bg.png")));
        }
    }

    /// Every desktop view with a theme that sets every `DesktopStyle` field: draws, and the
    /// panel gradients reach the screen.
    #[test]
    fn themed_desktop_draws_every_view() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let games =
            (1..=30).map(|i| crate::ui::desktop::tests::fake_game(i, &format!("Game {i}"), Source::Steam)).collect();
        st.library = crate::library::Library::new(games);
        st.desktop_theme = Arc::new(themed(dir.path()));
        let mut ui = DesktopUi::new();
        let ctx = st.ctx().clone();
        for (view, ambient) in [(DesktopView::Grid, false), (DesktopView::List, true), (DesktopView::Details, false)] {
            st.config.desktop.view = view;
            st.config.desktop.background_image = Some(ambient);
            let mut meshes = 0;
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1400.0, 800.0))),
                    ..Default::default()
                };
                let out = ctx.run_ui(input, |eui| ui.show(eui, &mut st));
                meshes = out.shapes.iter().filter(|s| matches!(s.shape, egui::Shape::Mesh(_))).count();
            }
            assert!(meshes >= 2, "{view:?}: {meshes}");
        }
    }
}
