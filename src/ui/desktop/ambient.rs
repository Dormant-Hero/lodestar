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
use crate::images::ImgSize;
use crate::model::BackgroundPlacement;
use crate::theme::render::gradient::GradientCache;
use crate::theme::{Brush, Gradient};
use crate::ui::widgets::{mix, vgradient, with_alpha};
use egui::{Color32, Rect, vec2};
use std::path::PathBuf;

const FADE_SECS: f64 = 0.45;

#[derive(Default)]
pub(crate) struct Ambient {
    /// The picture and the game's background placement (the same crop as its page header).
    cur: Option<(PathBuf, Option<BackgroundPlacement>)>,
    prev: Option<(PathBuf, Option<BackgroundPlacement>)>,
    /// When `cur`'s texture first became available (crossfade start).
    shown_at: Option<f64>,
    /// Gradient panels this frame: (gradient, opacity) per `Panel`.
    panel_grads: [Option<(Gradient, f32)>; 4],
    grads: GradientCache,
    /// The theme's background picture: (theme folder, theme path, resolved file). Keyed by
    /// the folder too: two themes can name the same relative file.
    picture: Option<(Option<PathBuf>, String, Option<PathBuf>)>,
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
        st.config.desktop.background_image.unwrap_or(st.desktop_theme.file.desktop.background_image)
    }

    pub(crate) fn fills(&mut self, st: &AppState, ambient: bool) -> Fills {
        let d = &st.desktop_theme.file.desktop;
        let mut f = self.base_fills(st, ambient);
        let o = ambient.then(|| d.panel_opacity.clamp(0.0, 1.0));
        if !ambient {
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
        ];
        for (panel, brush) in regions {
            let field = match panel {
                Panel::Bar => &mut f.bar,
                Panel::Sidebar => &mut f.sidebar,
                Panel::Details => &mut f.surface,
                Panel::Page => &mut f.page,
            };
            let slot = &mut self.ambient.panel_grads[panel as usize];
            match brush {
                Some(b) if b.gradient().is_some() => {
                    let g = b.gradient().expect("checked");
                    let op = o.unwrap_or(1.0);
                    // Cloned only when the theme changes (no per-frame allocation).
                    if slot.as_ref().is_none_or(|(sg, so)| sg != g || *so != op) {
                        *slot = Some((g.clone(), op));
                    }
                    *field = Color32::TRANSPARENT;
                }
                Some(b) => {
                    *slot = None;
                    *field = solid(b.average().to_egui(), o);
                }
                None => *slot = None,
            }
        }
        f
    }

    /// Lodestar's own panel fills (no theme brushes).
    fn base_fills(&self, st: &AppState, ambient: bool) -> Fills {
        let p = &self.pal;
        let list = mix(p.bg, p.surface, 0.5);
        let header = mix(p.bg, p.surface, 0.6);
        let stripe = mix(p.bg, p.surface, 0.35);
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
            };
        }
        let o = st.desktop_theme.file.desktop.panel_opacity.clamp(0.0, 1.0);
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
        st.desktop_theme.file.desktop.has_backdrop()
    }

    /// Paint the background over the whole window (the theme's backdrop, then the selected
    /// game's art when `art`). Call before any panel.
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
        if let Some(pic) = &d.background_picture {
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
        if !art {
            return;
        }

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
        // Keep the last background while nothing (or an art-less game) is selected.
        if target.is_some() && target != self.ambient.cur {
            // Mid-fade switches start from whatever is fully shown (avoid a flash).
            if self.ambient.shown_at.is_some() {
                self.ambient.prev = self.ambient.cur.take();
            }
            self.ambient.cur = target;
            self.ambient.shown_at = None;
        }

        let now = ui.input(|i| i.time);
        let cur_tex = self.ambient.cur.as_ref().and_then(|(path, _)| st.images.get(path, ImgSize::Backdrop));
        if cur_tex.is_some() && self.ambient.shown_at.is_none() {
            self.ambient.shown_at = Some(now);
        }
        let t = self.ambient.shown_at.map_or(0.0, |s| ((now - s) / FADE_SECS).clamp(0.0, 1.0)) as f32;
        let ease = t * t * (3.0 - 2.0 * t);
        let focus = vec2(0.5, 0.35);

        let paint = |tex: &egui::TextureHandle, crop: Option<BackgroundPlacement>, tint: Color32| {
            let uv = crate::model::background_uv(crop, tex.size_vec2(), rect.size(), focus);
            p.add(egui::epaint::RectShape::filled(rect, 0, tint).with_texture(tex.id(), uv));
        };
        if ease < 1.0
            && let Some((prev, crop)) = &self.ambient.prev
            && let Some(tex) = st.images.get(prev, ImgSize::Backdrop)
        {
            paint(&tex, *crop, Color32::WHITE);
        }
        if let Some(tex) = &cur_tex {
            let crop = self.ambient.cur.as_ref().and_then(|c| c.1);
            paint(tex, crop, Color32::WHITE.gamma_multiply(ease));
        }
        if self.ambient.shown_at.is_some() && ease < 1.0 {
            ui.ctx().request_repaint();
        } else if ease >= 1.0 {
            self.ambient.prev = None;
        }

        // Dim for legibility, a little more towards the bottom.
        let dim = st.desktop_theme.file.desktop.background_dim.clamp(0.0, 1.0);
        let a = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u8;
        p.rect_filled(rect, 0.0, with_alpha(self.pal.bg, a(dim)));
        vgradient(p, rect, Color32::TRANSPARENT, with_alpha(self.pal.bg, a(dim * 0.6)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DesktopView;
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
        // Over an ambient background the theme's solid colours turn translucent.
        let f = ui.fills(&st, true);
        assert!(f.sidebar.a() < 255);
        assert!((ui.ambient.panel_grads[Panel::Bar as usize].as_ref().unwrap().1 - 0.78).abs() < 1e-4);
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
