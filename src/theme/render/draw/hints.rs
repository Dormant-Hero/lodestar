//! Controller hints: button glyphs in the style of the pad in use (Xbox letters, PlayStation
//! shapes, Nintendo letters; see `input::style`) and the one-row hint bar built from them.
//! Every mark that isn't a plain letter is drawn as shapes, so no font has to cover it.

use super::{cr, lerp_color};
pub use crate::input::style::{Button, Prompts};
use crate::input::style::{Face, Mark};
use crate::theme::Align;
use egui::{Color32, FontFamily, FontId, Galley, Painter, Pos2, Rect, Stroke, pos2, vec2};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct Hint {
    pub button: Button,
    pub label: String,
    /// The only way out of where the user is (B back to the in-game menu): shown even by a
    /// theme help bar that lists only some buttons and not this one.
    pub always: bool,
}

impl Hint {
    pub fn new(button: Button, label: impl Into<String>) -> Hint {
        Hint { button, label: label.into(), always: false }
    }

    /// This hint must show whatever buttons a theme's help bars list (`always`).
    pub fn always(mut self) -> Hint {
        self.always = true;
        self
    }
}

/// Body of a coloured face button (Xbox / PlayStation): dark like the real buttons, so the
/// coloured mark reads whatever colour the theme gives hints.
const FACE_BODY: Color32 = Color32::from_rgb(30, 32, 38);

/// Colour of a button's mark: the face's own colour, else `text`. Keeps `text`'s opacity.
pub fn mark_color(face: &Face, text: Color32) -> Color32 {
    match face.color {
        Some(c) => c.gamma_multiply(text.a() as f32 / 255.0),
        None => text,
    }
}

/// Draw the button `b` (in `prompts`' style) centred at `c` with height `h`: a `fill` disc or
/// pill with the mark in `text`; coloured face buttons are a dark disc ringed in `fill` with
/// the mark in its own colour. Returns its width.
#[allow(clippy::too_many_arguments)]
pub fn button_glyph(
    painter: &Painter,
    c: Pos2,
    h: f32,
    b: Button,
    prompts: Prompts,
    fill: Color32,
    text: Color32,
    family: FontFamily,
) -> f32 {
    let face = prompts.face(b);
    let ink = mark_color(&face, text);
    let stroke = |w: f32| Stroke::new((h * w).max(1.0), ink);
    let disc = || {
        if face.color.is_some() {
            let ring = (h * 0.08).max(1.0);
            painter.circle_filled(c, h / 2.0, fill);
            painter.circle_filled(
                c,
                h / 2.0 - ring,
                lerp_color(FACE_BODY, fill, 0.08).gamma_multiply(fill.a() as f32 / 255.0),
            );
        } else {
            painter.circle_filled(c, h / 2.0, fill);
        }
    };
    match face.mark {
        Mark::Text(t) => {
            let galley = painter.layout_no_wrap(
                t.to_string(),
                FontId::new(if t.chars().count() > 1 { h * 0.46 } else { h * 0.56 }, family),
                ink,
            );
            let w = if face.round { h } else { (galley.size().x + h * 0.6).max(h * 1.3) };
            let rect = Rect::from_center_size(c, vec2(w, h));
            if face.round {
                disc();
            } else {
                painter.rect_filled(rect, cr(h / 2.0), fill);
            }
            painter.galley(rect.center() - galley.size() / 2.0, galley, ink);
            return w;
        }
        Mark::Cross => {
            disc();
            let s = h * 0.19;
            painter.line_segment([c + vec2(-s, -s), c + vec2(s, s)], stroke(0.09));
            painter.line_segment([c + vec2(-s, s), c + vec2(s, -s)], stroke(0.09));
        }
        Mark::Circle => {
            disc();
            painter.circle_stroke(c, h * 0.21, stroke(0.085));
        }
        Mark::Triangle => {
            disc();
            let r = h * 0.25;
            let o = c + vec2(0.0, h * 0.035);
            let pts: Vec<Pos2> = [-90.0f32, 30.0, 150.0]
                .iter()
                .map(|a| o + vec2(a.to_radians().cos(), a.to_radians().sin()) * r)
                .collect();
            painter.add(egui::Shape::closed_line(pts, stroke(0.08)));
        }
        Mark::Square => {
            disc();
            let s = h * 0.38;
            painter.rect_stroke(Rect::from_center_size(c, vec2(s, s)), cr(0.0), stroke(0.08), egui::StrokeKind::Middle);
        }
        Mark::Menu => {
            disc();
            let (w, t) = (h * 0.42, (h * 0.075).max(1.0));
            for k in -1..=1 {
                let y = c.y + k as f32 * h * 0.14;
                painter.rect_filled(Rect::from_center_size(pos2(c.x, y), vec2(w, t)), cr(t / 2.0), ink);
            }
        }
        Mark::View => {
            disc();
            let s = h * 0.26;
            painter.rect_stroke(
                Rect::from_center_size(c + vec2(-s * 0.22, -s * 0.22), vec2(s, s)),
                cr(s * 0.15),
                stroke(0.07),
                egui::StrokeKind::Middle,
            );
            painter.rect_filled(Rect::from_center_size(c + vec2(s * 0.28, s * 0.28), vec2(s, s)), cr(s * 0.15), ink);
        }
        Mark::Create => {
            // PlayStation's Create / Share mark: three short rays fanning up from a point.
            disc();
            // Wide fan and a gap at the base, so small glyphs don't read as a down arrow.
            let base = c + vec2(0.0, h * 0.25);
            for a in [-90.0f32, -140.0, -40.0] {
                let d = vec2(a.to_radians().cos(), a.to_radians().sin());
                painter.line_segment([base + d * h * 0.25, base + d * h * 0.43], stroke(0.085));
            }
        }
        Mark::Plus | Mark::Minus => {
            disc();
            let (w, t) = (h * 0.44, (h * 0.1).max(1.0));
            painter.rect_filled(Rect::from_center_size(c, vec2(w, t)), cr(t / 2.0), ink);
            if face.mark == Mark::Plus {
                painter.rect_filled(Rect::from_center_size(c, vec2(t, w)), cr(t / 2.0), ink);
            }
        }
    }
    h
}

/// Sizes in a hint bar, in pixels: button height, room between a button and its label, and
/// between hints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HintMetrics {
    pub glyph_h: f32,
    pub gap: f32,
    pub spacing: f32,
}

impl HintMetrics {
    /// Lodestar's own proportions for labels of `px`.
    pub fn standard(px: f32) -> HintMetrics {
        HintMetrics { glyph_h: px * 1.35, gap: px * 0.45, spacing: px * 1.5 }
    }
}

/// The button a theme names in a hint bar's `buttons` list.
pub fn button_named(name: &str) -> Option<Button> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "a" => Button::A,
        "b" => Button::B,
        "x" => Button::X,
        "y" => Button::Y,
        "start" => Button::Start,
        "select" => Button::Select,
        "lb" => Button::LB,
        "rb" => Button::RB,
        _ => return None,
    })
}

/// How many hints (the first ones: they are listed most important first) a bar shows and at
/// what label size, given `width(count, px)` and the `room` it has: all at `px` when they fit;
/// else all, smaller, down to 80 % of `px`; else fewer at the largest size that fits (down to
/// that 80 %); a single hint that still doesn't fit gets as small as it must.
pub fn fit_hints(n: usize, px: f32, room: f32, width: impl Fn(usize, f32) -> f32) -> (usize, f32) {
    const MIN_SCALE: f32 = 0.8;
    if n == 0 || width(n, px) <= room {
        return (n, px);
    }
    for k in (1..=n).rev() {
        // Width grows about linearly with the label size; check, and trim a little more if the
        // fixed spacing makes it grow slower.
        let mut p = px * (room / width(k, px).max(1.0)).min(1.0);
        for _ in 0..6 {
            if width(k, p) <= room {
                break;
            }
            p *= 0.95;
        }
        if p >= px * MIN_SCALE || k == 1 {
            return (k, p.max(1.0));
        }
    }
    (n, px)
}

/// Slot a hint's glyph takes in the bar (pills are wider than discs).
fn glyph_w(prompts: Prompts, b: Button, glyph_h: f32) -> f32 {
    if prompts.face(b).round { glyph_h } else { glyph_h * 1.5 }
}

/// Width `paint_hints` needs for `hints` at `px`.
pub fn hints_width(painter: &Painter, hints: &[Hint], prompts: Prompts, family: FontFamily, px: f32) -> f32 {
    hints_width_with(painter, hints, prompts, family, px, HintMetrics::standard(px))
}

/// `hints_width` with the bar's own sizes.
pub fn hints_width_with(
    painter: &Painter,
    hints: &[Hint],
    prompts: Prompts,
    family: FontFamily,
    px: f32,
    m: HintMetrics,
) -> f32 {
    if hints.is_empty() {
        return 0.0;
    }
    let HintMetrics { glyph_h, gap, spacing } = m;
    hints
        .iter()
        .map(|h| {
            let g = painter.layout_no_wrap(h.label.clone(), FontId::new(px, family.clone()), Color32::WHITE);
            glyph_w(prompts, h.button, glyph_h) + gap + g.size().x
        })
        .sum::<f32>()
        + spacing * (hints.len() - 1) as f32
}

/// Lay out hints in one row inside `rect`. Returns total width used.
#[allow(clippy::too_many_arguments)]
pub fn paint_hints(
    painter: &Painter,
    rect: Rect,
    hints: &[Hint],
    prompts: Prompts,
    family: FontFamily,
    px: f32,
    text: Color32,
    icon_fill: Color32,
    icon_text: Color32,
    align: Align,
) -> f32 {
    let m = HintMetrics::standard(px);
    paint_hints_shadow(painter, rect, hints, prompts, family, (px, m), (text, None), icon_fill, icon_text, align)
}

/// `paint_hints` with a soft shadow behind the labels (a theme's `HelpBar` style shadow).
#[allow(clippy::too_many_arguments)]
pub fn paint_hints_shadow(
    painter: &Painter,
    rect: Rect,
    hints: &[Hint],
    prompts: Prompts,
    family: FontFamily,
    (px, m): (f32, HintMetrics),
    (text, shadow): (Color32, Option<Color32>),
    icon_fill: Color32,
    icon_text: Color32,
    align: Align,
) -> f32 {
    if hints.is_empty() {
        return 0.0;
    }
    let HintMetrics { glyph_h, gap, spacing } = m;
    let galleys: Vec<Arc<Galley>> =
        hints.iter().map(|h| painter.layout_no_wrap(h.label.clone(), FontId::new(px, family.clone()), text)).collect();
    let total: f32 =
        hints.iter().zip(&galleys).map(|(h, g)| glyph_w(prompts, h.button, glyph_h) + gap + g.size().x).sum::<f32>()
            + spacing * (hints.len().saturating_sub(1)) as f32;
    let mut x = match align {
        Align::Left => rect.left(),
        Align::Center => rect.center().x - total / 2.0,
        Align::Right => rect.right() - total,
    };
    let cy = rect.center().y;
    for (h, g) in hints.iter().zip(galleys) {
        let w = glyph_w(prompts, h.button, glyph_h);
        let drawn = button_glyph(
            painter,
            pos2(x + w / 2.0, cy),
            glyph_h,
            h.button,
            prompts,
            icon_fill,
            icon_text,
            family.clone(),
        );
        x += drawn.max(w) + gap;
        let p = pos2(x, cy - g.size().y / 2.0);
        match shadow {
            Some(sh) => super::paint_galley(painter, p, &g, text, Some(sh), 0.0),
            None => painter.galley(p, g.clone(), text),
        }
        x += g.size().x + spacing;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::ButtonStyle;

    #[test]
    fn hints_fit_by_shrinking_a_little_then_dropping_the_last() {
        // 100 px per hint at size 10, linear in the size.
        let w = |k: usize, px: f32| k as f32 * px * 10.0;
        assert_eq!(fit_hints(4, 10.0, 400.0, w), (4, 10.0));
        let (k, px) = fit_hints(4, 10.0, 360.0, w);
        assert_eq!(k, 4);
        assert!((px - 9.0).abs() < 1e-4);
        // 80 % is the limit: 4 hints would need 7.5, so the last one goes.
        let (k, px) = fit_hints(4, 10.0, 300.0, w);
        assert_eq!((k, px), (3, 10.0));
        let (k, px) = fit_hints(4, 10.0, 50.0, w);
        assert_eq!(k, 1);
        assert!(px <= 5.0 + 1e-4);
        assert_eq!(fit_hints(0, 10.0, 1.0, w), (0, 10.0));
        assert_eq!(button_named(" Start "), Some(Button::Start));
        assert_eq!(button_named("up/down"), None);
    }

    #[test]
    fn mark_colours() {
        let y = ButtonStyle::Xbox.face(Button::Y);
        assert_eq!(mark_color(&y, Color32::BLACK), y.color.unwrap());
        // Plain faces use the hint's text colour.
        let n = ButtonStyle::Nintendo.face(Button::A);
        assert_eq!(mark_color(&n, Color32::RED), Color32::RED);
    }
}
