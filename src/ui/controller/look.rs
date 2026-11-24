//! Palette + fonts of the current controller theme, resolved once per frame for the
//! built-in (non-theme) parts of controller mode: menus, keyboard, details fallback.

use crate::app::AppState;
use crate::theme::Align;
use crate::theme::render::draw::{Hint, fade, paint_hints};
use egui::{Color32, FontFamily, FontId, Rect};

pub struct Look {
    pub bg: Color32,
    pub surface: Color32,
    pub surface_alt: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub selection: Color32,
    pub favorite: Color32,
    pub danger: Color32,
    pub regular: FontFamily,
    pub medium: FontFamily,
    pub bold: FontFamily,
    pub title: FontFamily,
    /// Screen height in points (font sizes are fractions of it).
    pub sh: f32,
    pub sw: f32,
    /// Button prompt style (the pad in use or the setting) and confirm / back swap.
    pub prompts: crate::input::Prompts,
    /// Settings → Visuals → Text size.
    pub text_scale: f32,
}

impl Look {
    pub fn new(st: &AppState, screen: Rect) -> Look {
        let t = &st.controller_theme;
        let p = &t.file.palette;
        let medium = if t.file.fonts.extra.contains_key("medium") { Some("medium") } else { Some("bold") };
        Look {
            bg: p.background.to_egui(),
            surface: p.surface.to_egui(),
            surface_alt: p.surface_alt.to_egui(),
            text: p.text.to_egui(),
            dim: p.text_dim.to_egui(),
            accent: p.accent.to_egui(),
            accent_text: p.accent_text.to_egui(),
            selection: p.selection.to_egui(),
            favorite: p.favorite.to_egui(),
            danger: p.danger.to_egui(),
            regular: st.fonts.family(t, None),
            medium: st.fonts.family(t, medium),
            bold: st.fonts.family(t, Some("bold")),
            title: st.fonts.family(t, Some("title")),
            sh: screen.height().max(1.0),
            sw: screen.width().max(1.0),
            prompts: st.prompts(),
            text_scale: st.config.controller.text_size.factor(),
        }
    }

    pub fn font(&self, frac: f32) -> FontId {
        FontId::new((frac * self.sh * self.text_scale).max(6.0), self.regular.clone())
    }
    pub fn medium(&self, frac: f32) -> FontId {
        FontId::new((frac * self.sh * self.text_scale).max(6.0), self.medium.clone())
    }
    pub fn bold(&self, frac: f32) -> FontId {
        FontId::new((frac * self.sh * self.text_scale).max(6.0), self.bold.clone())
    }
    pub fn title(&self, frac: f32) -> FontId {
        FontId::new((frac * self.sh * self.text_scale).max(6.0), self.title.clone())
    }
    /// Fraction of screen height in points.
    pub fn h(&self, frac: f32) -> f32 {
        frac * self.sh
    }

    /// The button legend along the bottom right of a full-screen page (built-in details, the
    /// artwork viewer, Now playing, the empty library).
    pub fn paint_screen_hints(&self, p: &egui::Painter, rect: Rect, hints: &[Hint], text: Color32) {
        paint_hints(
            p,
            rect,
            hints,
            self.prompts,
            self.medium.clone(),
            self.h(0.02),
            text,
            fade(Color32::WHITE, 0.92),
            Color32::from_rgb(20, 22, 28),
            Align::Right,
        );
    }
}
