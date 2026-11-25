//! Playnite resources → a Lodestar palette. Playnite's two modes use different keys
//! (desktop: TextBrush / GlyphBrush / WindowBackgourndBrush …; fullscreen: TextBrush /
//! GlyphBrush / ControlBackgroundBrush / MainBackgourndBrush / SelectionBrush …). Keys the
//! theme itself defines win over Playnite's defaults; colours that would be unreadable are
//! worked out from the others instead.

use super::Mode;
use super::look::LookBrush;
use super::xaml::{Brush, Resources};
use crate::theme::{Color, Palette};

pub(crate) fn over(fg: Color, bg: Color) -> Color {
    let a = fg.0[3] as f32 / 255.0;
    let ch = |i: usize| (fg.0[i] as f32 * a + bg.0[i] as f32 * (1.0 - a)).round() as u8;
    Color([ch(0), ch(1), ch(2), 255])
}

pub(crate) fn mix(a: Color, b: Color, t: f32) -> Color {
    let ch = |i: usize| (a.0[i] as f32 + (b.0[i] as f32 - a.0[i] as f32) * t).round() as u8;
    Color([ch(0), ch(1), ch(2), 255])
}

pub(crate) fn luminance(c: Color) -> f32 {
    let lin = |v: u8| {
        let s = v as f32 / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c.0[0]) + 0.7152 * lin(c.0[1]) + 0.0722 * lin(c.0[2])
}

/// Dark enough for light text: the line between a dark and a light theme.
pub(crate) fn is_dark(c: Color) -> bool {
    luminance(c) < 0.35
}

pub(crate) fn contrast(a: Color, b: Color) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn dist(a: Color, b: Color) -> u8 {
    (0..3).map(|i| a.0[i].abs_diff(b.0[i])).max().unwrap_or(0)
}

/// The mapped palette plus what it was made from (for the report).
pub(crate) struct Mapped {
    pub palette: Palette,
    /// Palette roles taken from the theme's own resources ("text", "accent" …).
    pub from_theme: Vec<&'static str>,
    /// Roles Playnite's defaults or Lodestar had to fill in.
    pub worked_out: Vec<&'static str>,
    /// The window background brush (for gradients / pictures).
    pub background: Option<Brush>,
    /// The background brush is a gradient (shown as one colour in desktop mode).
    pub gradient: bool,
}

impl Mapped {
    /// The background brush with everything a renderer needs: a gradient keeps all of its
    /// stops, its direction and its radial centre / radius (no averaging); a picture its file.
    pub fn background_look(&self, files: &[(String, u64)]) -> Option<LookBrush> {
        self.background.as_ref().map(|b| b.to_look(files))
    }
}

/// A brush resource with its full geometry (see [`Mapped::background_look`]).
pub(crate) fn brush_look(r: &Resources, key: &str, files: &[(String, u64)]) -> Option<LookBrush> {
    r.brush(key).map(|b| b.to_look(files))
}

/// Colours of `keys`: the ones the theme defines first (in order), then Playnite's defaults.
fn candidates(r: &Resources, keys: &[&str]) -> Vec<(Color, bool)> {
    let mut out = vec![];
    for pass in [true, false] {
        for k in keys {
            if r.theme_defines(k) == pass
                && let Some(c) = r.color(k)
            {
                out.push((c, pass));
            }
        }
    }
    out
}

struct Builder<'a> {
    r: &'a Resources,
    from_theme: Vec<&'static str>,
    worked_out: Vec<&'static str>,
}

impl Builder<'_> {
    /// The first candidate of `keys` that passes `ok` (composited over `base`), else `derived`.
    fn role(
        &mut self,
        role: &'static str,
        keys: &[&str],
        base: Color,
        ok: impl Fn(Color) -> bool,
        derived: Color,
    ) -> Color {
        for (c, ours) in candidates(self.r, keys) {
            if c.0[3] < 48 {
                continue;
            }
            let c = over(c, base);
            if ok(c) {
                if ours {
                    self.from_theme.push(role)
                } else {
                    self.worked_out.push(role)
                }
                return c;
            }
        }
        self.worked_out.push(role);
        derived
    }
}

/// The resource keys each palette role is read from, best first.
struct RoleKeys {
    surface: &'static [&'static str],
    alt: &'static [&'static str],
    dim: &'static [&'static str],
    selection: &'static [&'static str],
    border: &'static [&'static str],
}

impl RoleKeys {
    fn of(mode: Mode) -> RoleKeys {
        match mode {
            Mode::Fullscreen => RoleKeys {
                surface: &["ControlBackgroundDarkBrush", "ControlBackgroundColorDark"],
                alt: &["ControlBackgroundBrush", "ControlBackgroundColor"],
                dim: &["TextBrushDark", "TextColorDark"],
                selection: &["SelectionBrush", "GlyphLightBrush", "GlyphLightColor"],
                border: &[],
            },
            Mode::Desktop => RoleKeys {
                // Playnite's desktop panels are see-through over the window; its popups / menus
                // are the closest thing to a panel colour. GridItemBackgroundBrush fills cover
                // tiles only, often in the theme's accent: last.
                surface: &["PopupBackgroundBrush", "NormalBrushDark", "GridItemBackgroundBrush"],
                alt: &["NormalBrush", "PopupBackgroundBrush", "TooltipBackgroundBrush", "ButtonBackgroundBrush"],
                dim: &["TextBrushDarker", "TextColorDarker"],
                selection: &["HighlightGlyphBrush", "GlyphBrush"],
                border: &["PopupBorderBrush", "PanelSeparatorBrush", "NormalBorderBrush"],
            },
        }
    }
}

/// The palette for the theme's resources in `mode`.
///
/// `picture`: the average colour of the theme's background picture, if it has one (the
/// background colour then).
///
/// `window`: the colour the theme's main view really paints (the look's window / main view
/// brush, averaged), which wins over the background resource keys: a theme may style its
/// window with its own brush and leave Playnite's default key untouched.
///
/// `play`: the colour the theme's Play button is filled with (desktop), the accent before the
/// glyph colour (most themes keep their glyphs white or grey).
pub(crate) fn palette(
    r: &Resources,
    mode: Mode,
    picture: Option<Color>,
    window: Option<Color>,
    play: Option<Color>,
) -> Mapped {
    let mut b = Builder { r, from_theme: vec![], worked_out: vec![] };
    let (mut background, mut bg_from_theme, mut bg) = window_background(r, mode, picture, window);
    let gradient = matches!(background, Some(Brush::Gradient { .. }));
    let text = candidates(r, &["TextBrush", "TextColor"])
        .into_iter()
        .next()
        .map(|(c, ours)| {
            if ours {
                b.from_theme.push("text")
            } else {
                b.worked_out.push("text")
            }
            over(c, bg)
        })
        .unwrap_or(Color([0xf2, 0xf2, 0xf2, 255]));

    // The background must be readable under the text (a light theme that only changed the
    // text colour, say): then take a theme colour that is, or a plain one.
    if contrast(text, bg) < 3.0 {
        bg = readable_background(r, text);
        bg_from_theme = false;
        background = Some(Brush::Solid(bg));
    }
    if bg_from_theme {
        b.from_theme.push("background")
    } else {
        b.worked_out.push("background")
    }

    let readable = |min: f32| move |c: Color| contrast(text, c) >= min;
    let keys = RoleKeys::of(mode);
    let surface = b.role(
        "panels",
        keys.surface,
        bg,
        // Panels stay a quiet step from the background (never an accent-coloured slab).
        |c| dist(c, bg) >= 5 && contrast(c, bg) < 2.0 && readable(3.0)(c),
        mix(bg, text, 0.05),
    );
    let surface_alt = b.role(
        "cards",
        keys.alt,
        bg,
        |c| dist(c, surface) >= 5 && dist(c, bg) >= 5 && readable(3.0)(c),
        mix(bg, text, 0.11),
    );
    let text_dim =
        b.role("dim text", keys.dim, bg, |c| dist(c, text) >= 16 && contrast(c, bg) >= 2.2, mix(text, bg, 0.42));
    let accent = match play.map(|c| over(c, bg)).filter(|c| contrast(*c, bg) >= 1.6) {
        Some(c) => {
            b.from_theme.push("accent");
            c
        }
        None => b.role("accent", &["GlyphBrush", "GlyphColor"], bg, |c| contrast(c, bg) >= 1.6, mix(text, bg, 0.2)),
    };
    let selection = b.role("selection", keys.selection, bg, |c| contrast(c, bg) >= 1.8, mix(accent, text, 0.25));
    let border =
        b.role("borders", keys.border, bg, |c| dist(c, bg) >= 8 && contrast(c, bg) < 6.0, mix(surface_alt, text, 0.12));
    let danger = b.role("warnings", &["WarningBrush"], bg, |c| contrast(c, bg) >= 1.8, Color([0xff, 0x6b, 0x6b, 255]));
    let white = Color([255, 255, 255, 255]);
    let dark_text = Color([0x10, 0x10, 0x12, 255]);
    let accent_text = if contrast(white, accent) >= contrast(dark_text, accent) { white } else { dark_text };
    let palette = Palette {
        dark: is_dark(bg),
        background: bg,
        surface,
        surface_alt,
        text,
        text_dim,
        accent,
        accent_text,
        selection,
        border,
        favorite: Palette::default().favorite,
        danger,
    };
    Mapped { palette, from_theme: b.from_theme, worked_out: b.worked_out, background, gradient }
}

/// The window background: the theme's background brush (its own key first, then Playnite's
/// default), and the colour under the text — the background picture's, else what the window
/// really paints, else the brush's average. Returns (brush, whether the theme chose it,
/// colour).
fn window_background(
    r: &Resources,
    mode: Mode,
    picture: Option<Color>,
    window: Option<Color>,
) -> (Option<Brush>, bool, Color) {
    let bg_keys: &[&str] = if mode == Mode::Fullscreen {
        &["MainBackgourndBrush", "MainBackgroundBrush"]
    } else {
        &["WindowBackgourndBrush", "WindowBackgroundBrush"]
    };
    let mut background = None;
    let mut bg_from_theme = false;
    for pass in [true, false] {
        for k in bg_keys {
            if background.is_none()
                && r.theme_defines(k) == pass
                && let Some(br) = r.brush(k)
            {
                bg_from_theme = pass;
                background = Some(br);
            }
        }
    }
    let black = Color([0, 0, 0, 255]);
    if window.is_some() && picture.is_none() {
        bg_from_theme = true;
    }
    let bg = match (&background, picture) {
        (Some(Brush::Image(_)) | None, Some(p)) => over(p, black),
        _ if window.is_some() => over(window.unwrap_or(black), black),
        _ => background
            .as_ref()
            .and_then(Brush::average)
            .map(|c| over(c, black))
            .unwrap_or(Color([0x0d, 0x12, 0x25, 255])),
    };
    (background, bg_from_theme, bg)
}

/// A background `text` is readable on: an opaque theme colour of a surface, else a plain
/// dark or light one.
fn readable_background(r: &Resources, text: Color) -> Color {
    let black = Color([0, 0, 0, 255]);
    // BackgroundToneColor last: it tints surfaces, it isn't one.
    let keys = [
        "PopupBackgroundBrush",
        "PopupBackgroundColor",
        "GridItemBackgroundBrush",
        "NormalBrushDark",
        "MainColorDark",
        "ControlBackgroundDarkBrush",
        "ControlBackgroundBrush",
        "BackgroundToneColor",
    ];
    candidates(r, &keys)
        .into_iter()
        .filter(|(c, ours)| *ours && c.0[3] > 200)
        .map(|(c, _)| over(c, black))
        .find(|c| contrast(text, *c) >= 4.5)
        .unwrap_or(if luminance(text) > 0.4 { Color([0x14, 0x16, 0x1c, 255]) } else { Color([0xf4, 0xf4, 0xf2, 255]) })
}

/// Recolour a layout: every colour string equal (RGB) to a colour of the `from` palette
/// becomes the matching colour of `to`, keeping its alpha. Plain whites / blacks used for
/// text over artwork stay as they are unless they are palette colours.
pub(crate) fn recolor(v: &mut toml::Value, pairs: &[([u8; 3], Color)]) {
    match v {
        toml::Value::String(s) => {
            if s.starts_with('#')
                && (s.len() == 7 || s.len() == 9)
                && let Some(c) = Color::parse(s)
                && let Some((_, to)) = pairs.iter().find(|(rgb, _)| *rgb == [c.0[0], c.0[1], c.0[2]])
            {
                *s = to.with_alpha(c.0[3]).to_hex();
            }
        }
        toml::Value::Array(a) => a.iter_mut().for_each(|x| recolor(x, pairs)),
        toml::Value::Table(t) => t.iter_mut().for_each(|(_, x)| recolor(x, pairs)),
        _ => {}
    }
}

/// (old RGB, new colour) for the palette roles a layout paints with.
pub(crate) fn pairs(from: &Palette, to: &Palette) -> Vec<([u8; 3], Color)> {
    let roles = |p: &Palette| {
        [p.background, p.surface, p.surface_alt, p.text, p.text_dim, p.accent, p.selection, p.border, p.danger]
    };
    let mut out: Vec<([u8; 3], Color)> = vec![];
    for (a, b) in roles(from).into_iter().zip(roles(to)) {
        let rgb = [a.0[0], a.0[1], a.0[2]];
        if !out.iter().any(|(x, _)| *x == rgb) {
            out.push((rgb, b));
        }
    }
    out
}
