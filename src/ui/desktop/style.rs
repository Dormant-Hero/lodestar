//! egui style built from the desktop theme's palette.

use crate::theme::{CoverTitle, DesktopStyle, Palette, Theme};
use crate::ui::widgets::{contrast_ratio, mix, with_alpha};
use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Visuals, vec2};

/// The palette as egui colours, plus a few derived shades the desktop UI uses everywhere.
#[derive(Clone, Copy, Debug)]
pub struct Pal {
    pub bg: Color32,
    pub surface: Color32,
    pub surface_alt: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub selection: Color32,
    pub border: Color32,
    pub fav: Color32,
    pub danger: Color32,
    /// Top/status bar background (a touch darker than `bg`).
    pub bar: Color32,
    /// Hovered row / control.
    pub hover: Color32,
    /// Selected row background.
    pub sel_bg: Color32,
    /// The Play button and its label.
    pub play: Color32,
    pub play_text: Color32,
}

impl Pal {
    pub fn from_palette(p: &Palette) -> Pal {
        let bg = p.background.to_egui();
        let surface = p.surface.to_egui();
        let surface_alt = p.surface_alt.to_egui();
        let text = p.text.to_egui();
        let accent = p.accent.to_egui();
        let towards = if p.dark { Color32::BLACK } else { Color32::WHITE };
        Pal {
            bg,
            surface,
            surface_alt,
            text,
            dim: p.text_dim.to_egui(),
            accent,
            accent_text: p.accent_text.to_egui(),
            selection: p.selection.to_egui(),
            border: p.border.to_egui(),
            fav: p.favorite.to_egui(),
            danger: p.danger.to_egui(),
            bar: mix(bg, towards, 0.28),
            hover: mix(surface_alt, text, 0.07),
            sel_bg: mix(surface_alt, accent, 0.38),
            play: accent,
            play_text: p.accent_text.to_egui(),
        }
    }

    /// The palette plus what the theme's desktop style recolours (the Play button).
    pub fn of_theme(theme: &Theme) -> Pal {
        let mut pal = Pal::from_palette(&theme.file.palette);
        if let Some(fill) = &theme.file.desktop.play_fill {
            pal.play = fill.average().to_egui();
            let dark = Color32::from_rgb(0x10, 0x10, 0x12);
            pal.play_text = if contrast_ratio(Color32::WHITE, pal.play) >= contrast_ratio(dark, pal.play) {
                Color32::WHITE
            } else {
                dark
            };
        }
        pal
    }
}

impl Default for Pal {
    fn default() -> Self {
        Pal::from_palette(&Palette::default())
    }
}

/// How grid covers look (`DesktopStyle`), resolved once per frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoverLook {
    pub radius: u8,
    /// Gap between covers (None = from the cover width).
    pub gap: Option<f32>,
    pub title: CoverTitle,
    pub sel_color: Color32,
    pub sel_width: f32,
    /// Hovered covers darken by this much instead of lightening.
    pub hover_darken: Option<f32>,
    /// Painted behind each cover and its name.
    pub tile: Option<Color32>,
    /// Painted behind the name under each cover.
    pub caption: Option<Color32>,
}

impl CoverLook {
    pub fn of(d: &DesktopStyle, pal: &Pal) -> CoverLook {
        let sel = d.selection_border;
        CoverLook {
            radius: d.cover_radius(),
            gap: d.cover_gap.filter(|g| g.is_finite()).map(|g| g.clamp(0.0, 80.0)),
            title: d.cover_title(),
            sel_color: sel.and_then(|s| s.color).map_or(pal.selection, |c| c.to_egui()),
            sel_width: sel.map_or(3.0, |s| if s.width.is_finite() { s.width.clamp(0.0, 12.0) } else { 3.0 }),
            hover_darken: d.hover_darken.filter(|h| h.is_finite()).map(|h| h.clamp(0.0, 1.0)),
            tile: d.tile_fill.as_ref().map(|b| b.average().to_egui()),
            caption: d.caption_fill.as_ref().map(|b| b.average().to_egui()),
        }
    }
}

impl Default for CoverLook {
    fn default() -> Self {
        CoverLook::of(&DesktopStyle::default(), &Pal::default())
    }
}

/// How a game list row is drawn (`DesktopStyle::rows`), resolved once per frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowLook {
    pub sel_fill: Color32,
    /// Width and colour of a bar along the selected row's left edge.
    pub sel_bar: Option<(f32, Color32)>,
    pub sel_text: Color32,
    pub text: Color32,
    pub text_uninstalled: Color32,
    /// None: the list's own corners.
    pub radius: Option<u8>,
    /// The selected row's text wins over an uninstalled game's (a theme's IsSelected trigger
    /// comes after its IsInstalled ones); Lodestar keeps uninstalled games dim.
    selected_wins: bool,
}

impl RowLook {
    pub fn of(d: &DesktopStyle, pal: &Pal) -> RowLook {
        let own = RowLook {
            sel_fill: pal.sel_bg,
            sel_bar: None,
            sel_text: pal.text,
            text: mix(pal.text, pal.dim, 0.15),
            text_uninstalled: pal.dim,
            radius: None,
            selected_wins: false,
        };
        let Some(r) = &d.rows else { return own };
        let width = |w: f32| if w.is_finite() { w.clamp(1.0, 12.0) } else { 3.0 };
        let sel_bar = r.selected_bar.map(|b| (width(b.width), b.color.map_or(pal.selection, |c| c.to_egui())));
        // With neither a fill nor a bar the selected row would look like any other: it keeps
        // Lodestar's own fill.
        let unmarked = if sel_bar.is_none() { own.sel_fill } else { Color32::TRANSPARENT };
        RowLook {
            sel_fill: r.selected_fill.map_or(unmarked, |c| c.to_egui()),
            sel_bar,
            sel_text: r.selected_text.map_or(own.sel_text, |c| c.to_egui()),
            text: r.text.map_or(own.text, |c| c.to_egui()),
            text_uninstalled: r.text_uninstalled.map_or(own.text_uninstalled, |c| c.to_egui()),
            radius: r.radius.map(crate::theme::radius_u8),
            selected_wins: true,
        }
    }

    /// A row's text colour.
    pub fn text_for(&self, selected: bool, installed: bool) -> Color32 {
        match (selected, installed) {
            (true, _) if self.selected_wins => self.sel_text,
            (_, false) => self.text_uninstalled,
            (true, true) => self.sel_text,
            (false, true) => self.text,
        }
    }

    /// The selected row's fill and bar over `rect` (`radius`: the list's own corners).
    pub fn paint_selected(&self, p: &egui::Painter, rect: egui::Rect, radius: u8) {
        p.rect_filled(rect, self.radius.unwrap_or(radius), self.sel_fill);
        if let Some((w, c)) = self.sel_bar {
            p.rect_filled(egui::Rect::from_min_size(rect.min, vec2(w, rect.height())), 0.0, c);
        }
    }
}

/// Desktop text sizes [small, body, heading, monospace] in points (`DesktopStyle::font_scale`;
/// Lodestar's own sizes when the theme sets none).
pub fn text_sizes(d: &DesktopStyle) -> [f32; 4] {
    match d.font_scale {
        None => [11.5, 14.0, 20.0, 13.0],
        Some(fs) => [fs.pt(fs.small), fs.pt(1.0), fs.pt(fs.larger), fs.pt(13.0 / 14.0)],
    }
}

/// A fixed point size scaled like the theme's normal text (`font_scale.normal`).
pub fn scaled(d: &DesktopStyle, pt: f32) -> f32 {
    match d.font_scale {
        None => pt,
        Some(fs) => pt * fs.pt(1.0) / 14.0,
    }
}

/// … and a game title's size (`font_scale.largest`, relative to Lodestar's own largest).
pub fn scaled_title(d: &DesktopStyle, pt: f32) -> f32 {
    match d.font_scale {
        None => pt,
        Some(fs) => {
            let base = crate::theme::FontScale::default().largest;
            pt * fs.pt(fs.largest) / (14.0 * base)
        }
    }
}

/// Heading font families from the theme (fall back to the default proportional font).
#[derive(Clone, Debug)]
pub struct Fonts {
    pub bold: FontFamily,
    pub title: FontFamily,
}

impl Default for Fonts {
    fn default() -> Self {
        Fonts { bold: FontFamily::Proportional, title: FontFamily::Proportional }
    }
}

/// Apply the desktop theme to egui (both the dark and light style slots, so it does not
/// matter which one egui picks from the system preference).
pub fn apply_style(ctx: &egui::Context, theme: &Theme) {
    let p = &theme.file.palette;
    let d = &theme.file.desktop;
    let pal = Pal::from_palette(p);
    let [small, body, heading, mono] = text_sizes(d);
    ctx.set_theme(if p.dark { egui::Theme::Dark } else { egui::Theme::Light });
    ctx.all_styles_mut(|style| {
        let mut v = if p.dark { Visuals::dark() } else { Visuals::light() };
        v.dark_mode = p.dark;
        v.override_text_color = None;
        v.weak_text_color = Some(mix(pal.dim, pal.bg, 0.3));
        v.panel_fill = pal.bg;
        v.window_fill = pal.surface;
        v.window_stroke = Stroke::new(1.0, pal.border);
        v.window_corner_radius = CornerRadius::same(d.window_radius());
        v.menu_corner_radius = CornerRadius::same(d.menu_radius());
        v.window_shadow = egui::epaint::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(if p.dark { 110 } else { 40 }),
        };
        v.popup_shadow = egui::epaint::Shadow {
            offset: [0, 4],
            blur: 12,
            spread: 0,
            color: Color32::from_black_alpha(if p.dark { 90 } else { 30 }),
        };
        v.faint_bg_color = mix(pal.bg, pal.surface, 0.5);
        v.extreme_bg_color = mix(pal.bg, if p.dark { Color32::BLACK } else { Color32::WHITE }, 0.25);
        v.text_edit_bg_color = Some(mix(pal.bg, if p.dark { Color32::BLACK } else { Color32::WHITE }, 0.18));
        v.code_bg_color = pal.surface_alt;
        v.hyperlink_color = pal.selection;
        v.warn_fg_color = pal.fav;
        v.error_fg_color = pal.danger;
        v.selection.bg_fill = with_alpha(pal.accent, if p.dark { 200 } else { 150 });
        v.selection.stroke = Stroke::new(1.0, pal.accent_text);
        v.text_cursor.stroke = Stroke::new(2.0, pal.selection);
        v.striped = false;
        v.slider_trailing_fill = true;
        v.indent_has_left_vline = false;

        let r = CornerRadius::same(d.control_radius());
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = pal.surface;
        w.noninteractive.weak_bg_fill = pal.surface;
        w.noninteractive.bg_stroke = Stroke::new(1.0, pal.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, pal.text);
        w.noninteractive.corner_radius = r;

        w.inactive.bg_fill = pal.surface_alt;
        w.inactive.weak_bg_fill = pal.surface_alt;
        // A faint outline so text fields and checkboxes read as controls on dialog surfaces
        // (surface_alt alone is barely distinguishable from surface).
        w.inactive.bg_stroke = Stroke::new(1.0, mix(pal.border, pal.text, 0.08));
        w.inactive.fg_stroke = Stroke::new(1.0, mix(pal.text, pal.dim, 0.25));
        w.inactive.corner_radius = r;
        w.inactive.expansion = 0.0;

        w.hovered.bg_fill = pal.hover;
        w.hovered.weak_bg_fill = pal.hover;
        w.hovered.bg_stroke = Stroke::new(1.0, mix(pal.border, pal.text, 0.15));
        w.hovered.fg_stroke = Stroke::new(1.5, pal.text);
        w.hovered.corner_radius = r;
        w.hovered.expansion = 0.0;

        w.active.bg_fill = mix(pal.surface_alt, pal.accent, 0.55);
        w.active.weak_bg_fill = mix(pal.surface_alt, pal.accent, 0.55);
        w.active.bg_stroke = Stroke::new(1.0, pal.accent);
        w.active.fg_stroke = Stroke::new(2.0, pal.accent_text);
        w.active.corner_radius = r;
        w.active.expansion = 0.0;

        w.open.bg_fill = pal.surface_alt;
        w.open.weak_bg_fill = pal.hover;
        w.open.bg_stroke = Stroke::new(1.0, pal.border);
        w.open.fg_stroke = Stroke::new(1.0, pal.text);
        w.open.corner_radius = r;

        style.visuals = v;

        let s = &mut style.spacing;
        s.item_spacing = vec2(8.0, 6.0);
        s.button_padding = vec2(10.0, 4.0);
        s.interact_size = vec2(36.0, 24.0);
        s.window_margin = Margin::same(14);
        s.menu_margin = Margin::same(6);
        s.combo_height = 320.0;
        s.slider_width = 120.0;
        s.icon_width = 16.0;
        s.icon_width_inner = 9.0;
        s.icon_spacing = 6.0;
        s.text_edit_width = 240.0;
        s.scroll = egui::style::ScrollStyle::floating();
        s.scroll.bar_width = 8.0;

        style.text_styles = [
            (TextStyle::Small, FontId::new(small, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(body, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(body, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(heading, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(mono, FontFamily::Monospace)),
        ]
        .into();
        style.animation_time = 0.12;
        style.interaction.tooltip_delay = 0.35;
        style.interaction.selectable_labels = false;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Color, FontScale, SelectionBorder};

    #[test]
    fn default_desktop_style_keeps_lodestars_sizes() {
        let d = DesktopStyle::default();
        assert_eq!(text_sizes(&d), [11.5, 14.0, 20.0, 13.0]);
        assert_eq!(scaled(&d, 34.0), 34.0);
        assert_eq!(scaled_title(&d, 34.0), 34.0);
        let look = CoverLook::default();
        assert_eq!((look.radius, look.gap, look.title, look.sel_width), (6, None, CoverTitle::Under, 3.0));
        assert_eq!(look.sel_color, Pal::default().selection);
        assert_eq!(look.hover_darken, None);
    }

    #[test]
    fn list_rows_follow_the_theme() {
        let pal = Pal::default();
        // Lodestar's own rows: the palette's selection fill, uninstalled games stay dim.
        let own = RowLook::of(&DesktopStyle::default(), &pal);
        assert_eq!((own.sel_fill, own.sel_bar, own.radius), (pal.sel_bg, None, None));
        assert_eq!(own.text_for(true, false), pal.dim);
        assert_eq!(own.text_for(true, true), pal.text);
        // A theme's: a bar and no fill; its selected text wins.
        let rows = crate::theme::RowStyle {
            selected_bar: Some(SelectionBorder { color: Some(Color::parse("#55cdff").unwrap()), width: 5.0 }),
            selected_text: Some(Color::parse("#55cdff").unwrap()),
            text_uninstalled: Some(Color::parse("#a3a3a3").unwrap()),
            ..Default::default()
        };
        let d = DesktopStyle { rows: Some(rows), ..Default::default() };
        let look = RowLook::of(&d, &pal);
        assert_eq!(look.sel_fill, Color32::TRANSPARENT);
        assert_eq!(look.sel_bar, Some((5.0, Color32::from_rgb(0x55, 0xcd, 0xff))));
        assert_eq!(look.text_for(true, false), Color32::from_rgb(0x55, 0xcd, 0xff));
        assert_eq!(look.text_for(false, false), Color32::from_gray(0xa3));
        // Neither a fill nor a bar: the selected row keeps the palette's fill.
        let rows = crate::theme::RowStyle { text: Some(Color::parse("#a3a3a3").unwrap()), ..Default::default() };
        let look = RowLook::of(&DesktopStyle { rows: Some(rows), ..Default::default() }, &pal);
        assert_eq!((look.sel_fill, look.sel_bar), (pal.sel_bg, None));
    }

    #[test]
    fn theme_desktop_style_changes_the_look() {
        let d = DesktopStyle {
            cover_radius: Some(14.0),
            cover_gap: Some(30.0),
            cover_title: Some(CoverTitle::Hover),
            selection_border: Some(SelectionBorder { color: Some(Color::parse("#ff8800").unwrap()), width: 2.0 }),
            hover_darken: Some(1.7),
            font_scale: Some(FontScale { normal: 1.2, larger: 2.0, ..Default::default() }),
            caption_fill: Some(crate::theme::Brush::Solid(Color::parse("#202020").unwrap())),
            ..Default::default()
        };
        let look = CoverLook::of(&d, &Pal::default());
        assert_eq!(look.caption, Some(Color32::from_gray(0x20)));
        assert_eq!((look.radius, look.gap, look.title, look.sel_width), (14, Some(30.0), CoverTitle::Hover, 2.0));
        assert_eq!(look.sel_color, Color32::from_rgb(255, 136, 0));
        assert_eq!(look.hover_darken, Some(1.0));
        let [_, body, heading, _] = text_sizes(&d);
        assert!((body - 16.8).abs() < 1e-3 && (heading - 33.6).abs() < 1e-3);
        assert!((scaled(&d, 10.0) - 12.0).abs() < 1e-3);
    }
}
