//! Small shared drawing helpers (both UIs may add here; keep functions self-contained).
//!
//! Everything here is pure painting: no state, no I/O. Icons are drawn with shapes so they look
//! the same whatever fonts a theme installs.

use egui::epaint::{Mesh, RectShape, Shadow};
use egui::{
    Color32, CornerRadius, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, TextureHandle, Ui, Vec2,
    pos2, vec2,
};

// ---------------------------------------------------------------- colours

/// Linear mix of two (opaque-ish) colours in unmultiplied sRGB space. `t` = 0 → `a`, 1 → `b`.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let [ar, ag, ab, aa] = a.to_srgba_unmultiplied();
    let [br, bg, bb, ba] = b.to_srgba_unmultiplied();
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(l(ar, br), l(ag, bg), l(ab, bb), l(aa, ba))
}

/// Same colour with a new alpha (0..=255), unmultiplied.
pub fn with_alpha(c: Color32, a: u8) -> Color32 {
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// Perceived luminance 0..1 (for picking readable text over a colour).
pub fn luminance(c: Color32) -> f32 {
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32) / 255.0
}

/// WCAG contrast ratio of two opaque colours (1 = identical, 21 = black on white).
pub fn contrast_ratio(a: Color32, b: Color32) -> f32 {
    fn rel(c: Color32) -> f32 {
        let [r, g, b, _] = c.to_srgba_unmultiplied();
        let lin = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }
    let (la, lb) = (rel(a), rel(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// `preferred` when it stands out from `bg` (contrast at least 3:1, the WCAG minimum for
/// icons), else `fallback`. Imported themes often give the selection colour almost the same
/// shade as the surfaces it is drawn on, which made selected icons vanish.
pub fn readable_on(preferred: Color32, bg: Color32, fallback: Color32) -> Color32 {
    if contrast_ratio(preferred, bg) >= 3.0 { preferred } else { fallback }
}

/// FNV-1a of a string (stable across runs; used to derive placeholder tints).
pub fn hash_str(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// HSV (all 0..1) to an opaque colour.
pub fn hsv(h: f32, s: f32, v: f32) -> Color32 {
    let h = (h.fract() + 1.0).fract() * 6.0;
    let i = h.floor() as i32;
    let f = h - i as f32;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match i {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// A muted colour derived from `name`, blended into `base` so it sits in the palette.
pub fn name_tint(name: &str, base: Color32) -> Color32 {
    let h = hash_str(name);
    let hue = (h % 360) as f32 / 360.0;
    let base_l = luminance(base);
    let v = if base_l < 0.5 { 0.42 } else { 0.85 };
    mix(base, hsv(hue, 0.45, v), 0.55)
}

// ---------------------------------------------------------------- geometry

/// UV sub-rectangle that makes a texture of `tex` size fill `target` (CSS `object-fit: cover`).
/// `focus` picks which part survives the crop (0.5,0.5 = centre; 0.5,0.0 = keep the top).
pub fn cover_uv(tex: Vec2, target: Vec2, focus: Vec2) -> Rect {
    crate::model::aligned_uv(tex, target, focus)
}

/// Largest rect with the aspect of `tex` that fits inside `bounds`, centred (`object-fit: contain`).
pub fn contain_rect(tex: Vec2, bounds: Rect) -> Rect {
    if tex.x <= 0.0 || tex.y <= 0.0 {
        return bounds;
    }
    let scale = (bounds.width() / tex.x).min(bounds.height() / tex.y);
    Rect::from_center_size(bounds.center(), tex * scale)
}

/// Same as [`contain_rect`] but anchored: `align` 0..1 on each axis (0.5 = centred).
pub fn contain_rect_aligned(tex: Vec2, bounds: Rect, align: Vec2) -> Rect {
    let r = contain_rect(tex, bounds);
    let slack = bounds.size() - r.size();
    Rect::from_min_size(bounds.min + vec2(slack.x * align.x, slack.y * align.y), r.size())
}

// ---------------------------------------------------------------- painting

/// Paint `tex` cropped to fill `rect`, with rounded corners and a tint (WHITE = untouched).
pub fn paint_cover(
    painter: &Painter,
    rect: Rect,
    tex: &TextureHandle,
    radius: impl Into<CornerRadius>,
    tint: Color32,
    focus: Vec2,
) {
    let uv = cover_uv(tex.size_vec2(), rect.size(), focus);
    painter.add(RectShape::filled(rect, radius, tint).with_texture(tex.id(), uv));
}

/// The `uv` part of `tex` across `rect` (no rounding), fading to fully transparent over the bottom
/// `fade` fraction, so it melts into whatever is behind (ambient backgrounds).
pub fn paint_uv_fading(painter: &Painter, rect: Rect, tex: &TextureHandle, uv: Rect, fade: f32) {
    let f = fade.clamp(0.0, 1.0);
    let split_y = rect.bottom() - rect.height() * f;
    let split_v = uv.max.y - uv.height() * f;
    let mut m = Mesh::with_texture(tex.id());
    let rows = [
        (rect.top(), uv.min.y, Color32::WHITE),
        (split_y, split_v, Color32::WHITE),
        (rect.bottom(), uv.max.y, Color32::TRANSPARENT),
    ];
    for (y, v, c) in rows {
        m.vertices.push(egui::epaint::Vertex { pos: pos2(rect.left(), y), uv: pos2(uv.min.x, v), color: c });
        m.vertices.push(egui::epaint::Vertex { pos: pos2(rect.right(), y), uv: pos2(uv.max.x, v), color: c });
    }
    for r in 0..2u32 {
        let i = r * 2;
        m.add_triangle(i, i + 1, i + 2);
        m.add_triangle(i + 1, i + 2, i + 3);
    }
    painter.add(Shape::mesh(m));
}

/// Paint `tex` letterboxed inside `bounds`. Returns the rect actually covered.
pub fn paint_contain(painter: &Painter, bounds: Rect, tex: &TextureHandle, tint: Color32, align: Vec2) -> Rect {
    let r = contain_rect_aligned(tex.size_vec2(), bounds, align);
    let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    painter.add(RectShape::filled(r, 0.0, tint).with_texture(tex.id(), uv));
    r
}

/// Vertical gradient from `top` to `bottom` colour (premultiplied interpolation, so fading a
/// colour into transparency blends correctly).
pub fn vgradient(painter: &Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut m = Mesh::default();
    m.colored_vertex(rect.left_top(), top);
    m.colored_vertex(rect.right_top(), top);
    m.colored_vertex(rect.left_bottom(), bottom);
    m.colored_vertex(rect.right_bottom(), bottom);
    m.add_triangle(0, 1, 2);
    m.add_triangle(1, 2, 3);
    painter.add(Shape::mesh(m));
}

/// Vertical gradient filling a rounded rect: solid rounded caps at the top and bottom with a
/// gradient between them, so the corners stay rounded and there is no visible seam.
pub fn vgradient_rounded(painter: &Painter, rect: Rect, radius: u8, top: Color32, bottom: Color32) {
    let r = (radius as f32).min(rect.height() / 4.0).min(rect.width() / 2.0);
    if r < 0.5 {
        vgradient(painter, rect, top, bottom);
        return;
    }
    let ru = r.round() as u8;
    let cap_h = 2.0 * r;
    painter.rect_filled(
        Rect::from_min_size(rect.min, vec2(rect.width(), cap_h)),
        CornerRadius { nw: ru, ne: ru, sw: 0, se: 0 },
        top,
    );
    painter.rect_filled(
        Rect::from_min_max(pos2(rect.left(), rect.bottom() - cap_h), rect.max),
        CornerRadius { nw: 0, ne: 0, sw: ru, se: ru },
        bottom,
    );
    // The middle band ends exactly in the cap colours, overlapping half of each cap.
    vgradient(
        painter,
        Rect::from_min_max(pos2(rect.left(), rect.top() + r), pos2(rect.right(), rect.bottom() - r)),
        top,
        bottom,
    );
}

/// Up to two initials for a monogram: "Football Life 2026" → "FL", "mgo2.exe" → "M",
/// "2048" → "2". Words are split on whitespace and common separators; digit-led words only
/// count when nothing else does.
pub fn initials(name: &str) -> String {
    let firsts: Vec<char> = name
        .split(|c: char| c.is_whitespace() || matches!(c, '-' | ':' | '_'))
        .filter_map(|w| w.chars().find(|c| c.is_alphanumeric()))
        .collect();
    let letters: String = firsts.iter().filter(|c| c.is_alphabetic()).take(2).flat_map(|c| c.to_uppercase()).collect();
    if !letters.is_empty() {
        return letters;
    }
    firsts.first().map(|c| c.to_string()).unwrap_or_default()
}

/// Soft drop shadow under a rounded rect.
pub fn soft_shadow(painter: &Painter, rect: Rect, radius: u8, blur: u8, offset_y: i8, alpha: u8) {
    let s = Shadow { offset: [0, offset_y], blur, spread: 0, color: Color32::from_black_alpha(alpha) };
    painter.add(s.as_shape(rect, radius));
}

/// Five-pointed star (filled) centred at `c` with outer radius `r`.
pub fn star(painter: &Painter, c: Pos2, r: f32, fill: Color32) {
    let mut m = Mesh::default();
    m.colored_vertex(c, fill);
    for k in 0..10 {
        let rad = if k % 2 == 0 { r } else { r * 0.45 };
        let a = -std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::PI / 5.0;
        m.colored_vertex(c + vec2(a.cos(), a.sin()) * rad, fill);
    }
    for k in 0..10u32 {
        m.add_triangle(0, 1 + k, 1 + (k + 1) % 10);
    }
    painter.add(Shape::mesh(m));
}

/// Star outline.
pub fn star_outline(painter: &Painter, c: Pos2, r: f32, stroke: Stroke) {
    let pts: Vec<Pos2> = (0..10)
        .map(|k| {
            let rad = if k % 2 == 0 { r } else { r * 0.45 };
            let a = -std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::PI / 5.0;
            c + vec2(a.cos(), a.sin()) * rad
        })
        .collect();
    painter.add(Shape::closed_line(pts, stroke));
}

/// Filled right-pointing triangle (play symbol) inside `rect`.
pub fn play_triangle(painter: &Painter, rect: Rect, fill: Color32) {
    let s = rect.width().min(rect.height());
    let r = Rect::from_center_size(rect.center() + vec2(s * 0.06, 0.0), vec2(s * 0.8, s));
    painter.add(Shape::convex_polygon(vec![r.left_top(), r.right_center(), r.left_bottom()], fill, Stroke::NONE));
}

/// Filled gamepad silhouette centred in `rect` (drawn in a 16×16 box scaled to the rect): body
/// and grips in `fill`, d-pad and face buttons "cut out" in `cut` (paint it in the colour of
/// whatever is behind the glyph, e.g. the badge's background).
pub fn gamepad_solid(painter: &Painter, rect: Rect, fill: Color32, cut: Color32) {
    let s = rect.width().min(rect.height());
    let u = s / 16.0;
    // Optically centred: the grips hang below the body.
    let c = rect.center() + vec2(0.0, -0.6 * u);
    let p = |x: f32, y: f32| c + vec2(x * u, y * u);
    painter.rect_filled(Rect::from_min_max(p(-7.8, -4.6), p(7.8, 2.2)), 3.4 * u, fill);
    for x in [-4.9, 4.9] {
        painter.circle_filled(p(x, 2.4), 3.1 * u, fill);
    }
    // D-pad (left) and two face buttons (right).
    let arm = 1.15 * u;
    painter.rect_filled(Rect::from_center_size(p(-4.4, -1.0), vec2(4.4 * u, arm)), 0.0, cut);
    painter.rect_filled(Rect::from_center_size(p(-4.4, -1.0), vec2(arm, 4.4 * u)), 0.0, cut);
    painter.circle_filled(p(3.3, 0.1), 1.05 * u, cut);
    painter.circle_filled(p(5.4, -2.0), 1.05 * u, cut);
}

/// The cover badge for "full controller support": a small dark translucent pill with a gamepad
/// glyph, its bottom-right corner at `corner`. `h` is the pill height. Returns its rect.
pub fn gamepad_badge(painter: &Painter, corner: Pos2, h: f32) -> Rect {
    let rect = Rect::from_min_max(corner - vec2(h * 1.55, h), corner);
    let bg = Color32::from_black_alpha(150);
    painter.rect_filled(rect, h / 2.0, bg);
    // The cut-outs show (roughly) what the translucent pill shows: dark.
    gamepad_solid(
        painter,
        Rect::from_center_size(rect.center(), vec2(h * 0.9, h * 0.9)),
        Color32::from_gray(235),
        Color32::from_gray(38),
    );
    rect
}

// ---------------------------------------------------------------- icons

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Menu,
    Grid,
    List,
    Details,
    Search,
    SortAsc,
    SortDesc,
    Stop,
    Sidebar,
    Panel,
    Close,
    Gamepad,
    Folder,
    Check,
    More,
    Download,
    Pencil,
    /// Four arrows pointing out (Arrange: move the logo and the background).
    Arrange,
    Image,
    /// Circle with a check mark (e.g. "Steam Deck Verified").
    CheckCircle,
    /// Circle with an "i" (e.g. "Steam Deck Playable").
    InfoCircle,
    /// Circle with a slash (e.g. "Steam Deck Unsupported").
    BlockCircle,
    /// Small downward chevron (drop-down arrows).
    ChevronDown,
    /// Two chain links (web links).
    Link,
    /// Counter-clockwise arrow ("reset to the source's value").
    Reset,
    ArrowUp,
    ArrowDown,
    /// Rubbish bin (remove an entry).
    Trash,
    /// Small right-pointing chevron (collapsed tree rows).
    ChevronRight,
    /// Small left-pointing chevron (previous).
    ChevronLeft,
    /// Jigsaw piece (add-ons).
    Puzzle,
    /// Triangle with "!" (warnings, errors).
    Warning,
    /// A die showing five (random game).
    Dice,
}

/// Paint an icon centred in `rect` (drawn in a ~16px box scaled to the rect).
pub fn paint_icon(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let u = s / 16.0; // unit
    let st = Stroke::new((1.5 * u).max(1.0), color);
    let p = |x: f32, y: f32| c + vec2(x * u, y * u);
    match icon {
        Icon::Menu => {
            for y in [-4.5, 0.0, 4.5] {
                painter.line_segment([p(-6.0, y), p(6.0, y)], st);
            }
        }
        Icon::Grid => {
            for (x, y) in [(-6.0, -6.0), (1.0, -6.0), (-6.0, 1.0), (1.0, 1.0)] {
                painter.rect_stroke(Rect::from_min_size(p(x, y), vec2(5.0 * u, 5.0 * u)), 1.0, st, StrokeKind::Inside);
            }
        }
        Icon::List => {
            for y in [-5.0, 0.0, 5.0] {
                painter.circle_filled(p(-6.0, y), 1.1 * u, color);
                painter.line_segment([p(-3.0, y), p(6.5, y)], st);
            }
        }
        Icon::Details => {
            painter.rect_stroke(Rect::from_min_max(p(-7.0, -6.0), p(-1.0, 6.0)), 1.0, st, StrokeKind::Inside);
            for y in [-4.5, -1.5, 1.5] {
                painter.line_segment([p(1.5, y), p(7.0, y)], st);
            }
        }
        Icon::Search => {
            painter.circle_stroke(p(-1.5, -1.5), 4.5 * u, st);
            painter.line_segment([p(1.8, 1.8), p(6.0, 6.0)], Stroke::new(st.width * 1.2, color));
        }
        Icon::SortAsc | Icon::SortDesc => {
            let up = icon == Icon::SortAsc;
            // bars of increasing length + arrow
            let lens = if up { [3.0, 5.5, 8.0] } else { [8.0, 5.5, 3.0] };
            for (i, l) in lens.iter().enumerate() {
                let y = -4.5 + i as f32 * 4.5;
                painter.line_segment([p(-1.0, y), p(-1.0 + l, y)], st);
            }
            painter.line_segment([p(-5.0, -6.0), p(-5.0, 6.0)], st);
            if up {
                painter.line_segment([p(-7.5, -3.5), p(-5.0, -6.0)], st);
                painter.line_segment([p(-2.5, -3.5), p(-5.0, -6.0)], st);
            } else {
                painter.line_segment([p(-7.5, 3.5), p(-5.0, 6.0)], st);
                painter.line_segment([p(-2.5, 3.5), p(-5.0, 6.0)], st);
            }
        }
        Icon::Stop => {
            painter.rect_filled(Rect::from_center_size(c, vec2(10.0 * u, 10.0 * u)), 1.5 * u, color);
        }
        Icon::Sidebar | Icon::Panel => {
            // A filled strip on the side the pane opens on: a thin divider alone made the two
            // toggles mirror images that look the same at toolbar size.
            let r = Rect::from_center_size(c, vec2(14.0 * u, 12.0 * u));
            painter.rect_stroke(r, 2.0 * u, st, StrokeKind::Inside);
            let strip = if icon == Icon::Sidebar {
                Rect::from_min_max(r.min, pos2(r.left() + 5.0 * u, r.bottom()))
            } else {
                Rect::from_min_max(pos2(r.right() - 6.0 * u, r.top()), r.max)
            };
            painter.rect_filled(strip, 2.0 * u, color);
        }
        Icon::Close => {
            painter.line_segment([p(-5.0, -5.0), p(5.0, 5.0)], st);
            painter.line_segment([p(5.0, -5.0), p(-5.0, 5.0)], st);
        }
        Icon::Gamepad => {
            let body = Rect::from_center_size(c + vec2(0.0, 0.5 * u), vec2(15.0 * u, 9.0 * u));
            painter.rect_stroke(body, 4.0 * u, st, StrokeKind::Inside);
            painter.line_segment([p(-5.5, 0.5), p(-2.5, 0.5)], st);
            painter.line_segment([p(-4.0, -1.0), p(-4.0, 2.0)], st);
            painter.circle_filled(p(3.0, -0.5), 0.9 * u, color);
            painter.circle_filled(p(5.0, 1.5), 0.9 * u, color);
        }
        Icon::Folder => {
            let pts = vec![p(-7.0, -5.0), p(-2.5, -5.0), p(-1.0, -3.0), p(7.0, -3.0), p(7.0, 5.5), p(-7.0, 5.5)];
            painter.add(Shape::closed_line(pts, st));
        }
        Icon::Check => {
            painter.add(Shape::line(vec![p(-6.0, 0.5), p(-2.0, 4.5), p(6.0, -4.5)], st));
        }
        Icon::More => {
            for x in [-5.0, 0.0, 5.0] {
                painter.circle_filled(p(x, 0.0), 1.5 * u, color);
            }
        }
        Icon::Download => {
            painter.line_segment([p(0.0, -7.0), p(0.0, 3.0)], st);
            painter.add(Shape::line(vec![p(-4.0, -1.0), p(0.0, 3.0), p(4.0, -1.0)], st));
            painter.line_segment([p(-6.0, 6.5), p(6.0, 6.5)], st);
        }
        Icon::Pencil => {
            // Diagonal pencil, tip bottom-left: body outline, tip line, eraser band.
            let d = vec2(1.0, -1.0) / std::f32::consts::SQRT_2;
            let n = vec2(1.0, 1.0) / std::f32::consts::SQRT_2 * 2.1 * u;
            let tip = p(-6.5, 6.5);
            let neck = tip + d * 4.0 * u;
            let end = p(5.5, -5.5);
            let band = end - d * 3.0 * u;
            painter.add(Shape::closed_line(vec![tip, neck + n, end + n, end - n, neck - n], st));
            painter.line_segment([neck + n, neck - n], st);
            painter.line_segment([band + n, band - n], st);
        }
        Icon::Arrange => {
            // A cross with an arrow head at each end.
            painter.line_segment([p(-7.0, 0.0), p(7.0, 0.0)], st);
            painter.line_segment([p(0.0, -7.0), p(0.0, 7.0)], st);
            for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                let tip = p(7.0 * dx, 7.0 * dy);
                let (bx, by) = (4.5 * dx, 4.5 * dy);
                let (nx, ny) = (2.5 * dy, 2.5 * dx);
                painter.add(Shape::line(vec![p(bx + nx, by + ny), tip, p(bx - nx, by - ny)], st));
            }
        }
        Icon::Image => {
            let r = Rect::from_min_max(p(-7.0, -6.0), p(7.0, 6.0));
            painter.rect_stroke(r, 1.5 * u, st, StrokeKind::Inside);
            painter.circle_filled(p(3.0, -2.5), 1.5 * u, color);
            painter.add(Shape::line(vec![p(-6.0, 4.5), p(-2.0, 0.0), p(1.0, 3.0), p(3.0, 1.5), p(6.0, 4.5)], st));
        }
        Icon::CheckCircle => {
            painter.circle_stroke(c, 6.8 * u, st);
            painter.add(Shape::line(vec![p(-3.2, 0.2), p(-0.9, 2.6), p(3.4, -2.2)], st));
        }
        Icon::InfoCircle => {
            painter.circle_stroke(c, 6.8 * u, st);
            painter.circle_filled(p(0.0, -3.0), 1.0 * u, color);
            painter.line_segment([p(0.0, -0.6), p(0.0, 3.6)], st);
        }
        Icon::BlockCircle => {
            painter.circle_stroke(c, 6.8 * u, st);
            painter.line_segment([p(-4.6, 4.6), p(4.6, -4.6)], st);
        }
        Icon::ChevronRight => {
            painter.add(Shape::line(vec![p(-2.0, -4.5), p(2.5, 0.0), p(-2.0, 4.5)], st));
        }
        Icon::ChevronLeft => {
            painter.add(Shape::line(vec![p(2.0, -4.5), p(-2.5, 0.0), p(2.0, 4.5)], st));
        }
        Icon::Puzzle => {
            // "Extensions": three blocks and a fourth lifted off the corner.
            let sq = |x: f32, y: f32| Rect::from_min_size(p(x, y), vec2(5.6 * u, 5.6 * u));
            for r in [sq(-6.8, -0.8), sq(-0.8, -0.8), sq(-6.8, -6.8)] {
                painter.rect_stroke(r, 1.2 * u, st, StrokeKind::Middle);
            }
            painter.rect_filled(sq(1.4, -8.0), 1.2 * u, color);
        }
        Icon::Warning => {
            painter.add(Shape::closed_line(vec![p(0.0, -6.5), p(7.0, 5.5), p(-7.0, 5.5)], st));
            painter.line_segment([p(0.0, -2.0), p(0.0, 1.8)], st);
            painter.circle_filled(p(0.0, 3.8), 0.95 * u, color);
        }
        Icon::Dice => {
            let r = Rect::from_center_size(c, vec2(13.0 * u, 13.0 * u));
            painter.rect_stroke(r, 3.0 * u, st, egui::StrokeKind::Middle);
            for (x, y) in [(-3.0, -3.0), (3.0, -3.0), (0.0, 0.0), (-3.0, 3.0), (3.0, 3.0)] {
                painter.circle_filled(p(x, y), 1.25 * u, color);
            }
        }
        Icon::ChevronDown => {
            painter.add(Shape::line(vec![p(-4.5, -2.0), p(0.0, 2.5), p(4.5, -2.0)], st));
        }
        Icon::Link => {
            // Two chain links: rounded capsules tilted 45°, overlapping in the middle.
            let d = vec2(1.0, -1.0) / std::f32::consts::SQRT_2;
            let n = vec2(d.y, -d.x);
            let r = 2.6 * u;
            let half = 2.4 * u;
            for k in [-1.0f32, 1.0] {
                let mid = c + d * (k * 2.9 * u);
                let (a, b) = (mid - d * half, mid + d * half);
                let mut pts = Vec::with_capacity(20);
                for i in 0..=8 {
                    let t = std::f32::consts::PI * i as f32 / 8.0;
                    pts.push(b + (n * t.cos() + d * t.sin()) * r);
                }
                for i in 0..=8 {
                    let t = std::f32::consts::PI * i as f32 / 8.0;
                    pts.push(a + (-n * t.cos() - d * t.sin()) * r);
                }
                painter.add(Shape::closed_line(pts, st));
            }
        }
        Icon::Reset => {
            // Arc from ~1 o'clock round to ~10 o'clock, arrow head at the start.
            let r = 5.5 * u;
            let pts: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let a = (-60.0 + i as f32 * (270.0 / 16.0)).to_radians();
                    c + vec2(a.cos(), a.sin()) * r
                })
                .collect();
            let head = pts[0];
            painter.add(Shape::line(pts, st));
            painter.line_segment([head, head + vec2(-3.2 * u, -0.6 * u)], st);
            painter.line_segment([head, head + vec2(0.4 * u, -3.4 * u)], st);
        }
        Icon::ArrowUp | Icon::ArrowDown => {
            let k = if icon == Icon::ArrowUp { 1.0 } else { -1.0 };
            painter.line_segment([p(0.0, -5.5 * k), p(0.0, 5.5 * k)], st);
            painter.add(Shape::line(vec![p(-4.0, -1.5 * k), p(0.0, -5.5 * k), p(4.0, -1.5 * k)], st));
        }
        Icon::Trash => {
            painter.line_segment([p(-6.0, -4.0), p(6.0, -4.0)], st);
            painter.line_segment([p(-2.0, -6.0), p(2.0, -6.0)], st);
            painter.add(Shape::closed_line(vec![p(-4.5, -4.0), p(4.5, -4.0), p(3.5, 6.0), p(-3.5, 6.0)], st));
            painter.line_segment([p(-1.2, -1.0), p(-1.2, 3.5)], st);
            painter.line_segment([p(1.2, -1.0), p(1.2, 3.5)], st);
        }
    }
}

/// Square icon button (flat until hovered). `selected` draws it pressed in accent.
pub fn icon_button(ui: &mut Ui, icon: Icon, selected: bool, size: f32, tooltip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    if ui.is_rect_visible(rect) {
        let v = ui.visuals();
        let (fill, fg) = if selected {
            let bg = v.widgets.inactive.weak_bg_fill;
            (bg, readable_on(v.hyperlink_color, bg, v.strong_text_color()))
        } else if resp.is_pointer_button_down_on() {
            (v.widgets.active.weak_bg_fill, v.widgets.active.fg_stroke.color)
        } else if resp.hovered() || resp.has_focus() {
            // Keyboard focus (Tab) looks like hover, so Enter / Space has a visible target.
            (v.widgets.hovered.weak_bg_fill, v.widgets.hovered.fg_stroke.color)
        } else {
            (Color32::TRANSPARENT, v.widgets.inactive.fg_stroke.color)
        };
        let painter = ui.painter();
        painter.rect_filled(rect, 4.0, fill);
        paint_icon(painter, rect.shrink(size * 0.22), icon, fg);
    }
    if tooltip.is_empty() { resp } else { resp.on_hover_text(tooltip) }
}

/// iOS-style on / off switch (`size` = height). Toggles `on` when clicked; the knob slides
/// (egui's own animation, so no repaint loop once it has settled).
pub fn toggle_switch(ui: &mut Ui, on: &mut bool, size: f32, accent: Color32) -> Response {
    let desired = vec2(size * 1.8, size);
    let (rect, mut resp) = ui.allocate_exact_size(desired, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Switch, ui.is_enabled(), *on, ""));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(resp.id, *on);
        let v = ui.visuals();
        let enabled = ui.is_enabled();
        // Off: a track that stands out from cards and panels alike.
        let off_fill = mix(v.widgets.inactive.bg_fill, v.widgets.inactive.fg_stroke.color, 0.28);
        let mut fill = mix(off_fill, accent, t);
        if !enabled {
            fill = with_alpha(fill, 110);
        }
        let r = rect.height() / 2.0;
        let painter = ui.painter();
        painter.rect_filled(rect, r, fill);
        if resp.hovered() && enabled {
            painter.rect_filled(rect, r, Color32::from_white_alpha(18));
        }
        if resp.has_focus() {
            painter.rect_stroke(rect.expand(2.0), r + 2.0, v.selection.stroke, StrokeKind::Outside);
        }
        let knob_r = r - 2.5;
        let x = egui::lerp((rect.left() + r)..=(rect.right() - r), t);
        let knob = if enabled { Color32::from_gray(250) } else { Color32::from_gray(170) };
        painter.circle_filled(pos2(x, rect.center().y + 0.5), knob_r, Color32::from_black_alpha(40));
        painter.circle_filled(pos2(x, rect.center().y), knob_r, knob);
    }
    resp
}

/// Rounded square tile with a name's initials on a colour derived from the name (an add-on or
/// integration without an icon of its own).
pub fn initials_tile(painter: &Painter, rect: Rect, name: &str, base: Color32, fg: Color32) {
    let tint = name_tint(name, base);
    let radius = (rect.width() * 0.22) as u8;
    vgradient_rounded(painter, rect, radius, mix(tint, Color32::WHITE, 0.06), mix(tint, Color32::BLACK, 0.18));
    let text = initials(name);
    let font = egui::FontId::proportional(rect.height() * 0.38);
    let g = painter.layout_no_wrap(text, font, fg);
    painter.galley(rect.center() - g.size() / 2.0, g, fg);
}

/// Steps per second of [`lazy_spinner`]; it repaints at this rate, not at the monitor's.
pub const SPINNER_STEPS_PER_SEC: f64 = 8.0;

/// Busy indicator that only asks for a repaint every 1/[`SPINNER_STEPS_PER_SEC`] s (egui's
/// `Spinner` repaints every frame — 60-240 fps for as long as a background job runs).
/// Eight dots around a circle; the "head" moves one dot per step with a fading tail.
pub fn lazy_spinner(ui: &mut Ui, size: f32, color: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    if ui.is_rect_visible(rect) {
        paint_lazy_spinner(ui, rect, color);
    }
    resp
}

/// [`lazy_spinner`] painted into `rect` (no layout), e.g. on a painter-drawn overlay.
pub fn paint_lazy_spinner(ui: &Ui, rect: egui::Rect, color: Color32) {
    let size = rect.width().min(rect.height());
    let time = ui.input(|i| i.time);
    let step = (time * SPINNER_STEPS_PER_SEC).floor();
    let head = step.rem_euclid(8.0) as usize;
    let base_a = color.to_srgba_unmultiplied()[3] as u32;
    let painter = ui.painter();
    let r = size * 0.36;
    let dot = (size * 0.11).max(1.0);
    for k in 0..8usize {
        let a = k as f32 / 8.0 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        let age = (head + 8 - k) % 8; // 0 = head
        let alpha = [255u8, 190, 130, 90, 70, 60, 55, 50][age];
        painter.circle_filled(
            rect.center() + vec2(a.cos(), a.sin()) * r,
            dot,
            with_alpha(color, ((base_a * alpha as u32) / 255) as u8),
        );
    }
    let next = (step + 1.0) / SPINNER_STEPS_PER_SEC - time;
    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(next.clamp(0.01, 1.0)));
}

/// Small rounded "pill" label painted at `pos` (top-left). Returns its rect.
pub fn pill(painter: &Painter, pos: Pos2, text: &str, size: f32, fg: Color32, bg: Color32) -> Rect {
    let galley = painter.layout_no_wrap(text.to_owned(), egui::FontId::proportional(size), fg);
    let pad = vec2(size * 0.55, size * 0.22);
    let rect = Rect::from_min_size(pos, galley.size() + pad * 2.0);
    painter.rect_filled(rect, rect.height() / 2.0, bg);
    painter.galley(rect.min + pad, galley, fg);
    rect
}

/// Lay out `text` on one line, cut with "…" to `max_w`.
pub fn truncated(
    painter: &Painter,
    text: &str,
    font: egui::FontId,
    color: Color32,
    max_w: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(1.0));
    painter.layout_job(job)
}

/// Lay out `text` wrapped to `max_w`, at most `rows` rows (cut with "…").
/// With `Align::Center` the galley is centred on x = 0: paint it at the desired centre x.
pub fn wrapped(
    painter: &Painter,
    text: &str,
    font: egui::FontId,
    color: Color32,
    max_w: f32,
    rows: usize,
    halign: egui::Align,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, max_w.max(1.0));
    job.wrap.max_rows = rows.max(1);
    job.wrap.break_anywhere = false;
    job.halign = halign;
    painter.layout_job(job)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_uv_crops_the_long_side() {
        // 2:1 texture into a square: keep the middle half horizontally.
        let uv = cover_uv(vec2(200.0, 100.0), vec2(50.0, 50.0), vec2(0.5, 0.5));
        assert!((uv.min.x - 0.25).abs() < 1e-5 && (uv.max.x - 0.75).abs() < 1e-5);
        assert_eq!((uv.min.y, uv.max.y), (0.0, 1.0));
        // tall texture into a wide box, focus top.
        let uv = cover_uv(vec2(100.0, 400.0), vec2(100.0, 100.0), vec2(0.5, 0.0));
        assert_eq!(uv.min.y, 0.0);
        assert!((uv.max.y - 0.25).abs() < 1e-5);
        // degenerate sizes never produce NaN
        let uv = cover_uv(vec2(0.0, 0.0), vec2(10.0, 10.0), vec2(0.5, 0.5));
        assert_eq!(uv, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)));
    }

    #[test]
    fn contain_fits_and_centres() {
        let b = Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0));
        let r = contain_rect(vec2(200.0, 100.0), b);
        assert_eq!(r.size(), vec2(100.0, 50.0));
        assert_eq!(r.center(), b.center());
        let r = contain_rect_aligned(vec2(200.0, 100.0), b, vec2(0.5, 1.0));
        assert_eq!(r.bottom(), 100.0);
    }

    #[test]
    fn monogram_initials() {
        assert_eq!(initials("Football Life 2026"), "FL");
        assert_eq!(initials("Football Life 2026 Settings"), "FL");
        assert_eq!(initials("mgo2.exe"), "M");
        assert_eq!(initials("DOOM"), "D");
        assert_eq!(initials("Half-Life: Alyx"), "HL");
        assert_eq!(initials("2048"), "2");
        assert_eq!(initials("  "), "");
        assert_eq!(initials("élan vital"), "ÉV");
    }

    #[test]
    fn mix_and_tint() {
        assert_eq!(mix(Color32::BLACK, Color32::WHITE, 0.0), Color32::BLACK);
        assert_eq!(mix(Color32::BLACK, Color32::WHITE, 1.0), Color32::WHITE);
        let a = name_tint("Portal 2", Color32::from_rgb(30, 30, 30));
        let b = name_tint("Portal 2", Color32::from_rgb(30, 30, 30));
        assert_eq!(a, b);
        assert_ne!(a, name_tint("Half-Life", Color32::from_rgb(30, 30, 30)));
    }

    /// Selected toolbar icons stay visible with imported themes whose selection colour is
    /// almost the surface colour (Helium: #3e809d on #345e74): the fallback takes over.
    #[test]
    fn selected_icons_fall_back_to_a_readable_colour() {
        let (selection, surface, text) =
            (Color32::from_rgb(0x3e, 0x80, 0x9d), Color32::from_rgb(0x34, 0x5e, 0x74), Color32::WHITE);
        assert!(contrast_ratio(selection, surface) < 3.0);
        assert_eq!(readable_on(selection, surface, text), text);
        let accent = Color32::from_rgb(0x6c, 0xb4, 0xff);
        let dark = Color32::from_rgb(0x1a, 0x1d, 0x24);
        assert_eq!(readable_on(accent, dark, text), accent, "a readable selection colour is kept");
        assert!((contrast_ratio(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
    }
}
