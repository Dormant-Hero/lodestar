//! Drawing primitives shared by the theme renderer and the controller UI's built-in screens.

use crate::images::ImgSize;
use crate::theme::{Align, Fit, TextStyle, VAlign};
use egui::epaint::{Mesh, RectShape, Shadow, TextShape};
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Color32, CornerRadius, FontFamily, FontId, Galley, Painter, Pos2, Rect, TextureHandle, Vec2, pos2, vec2};
use std::sync::Arc;

/// Which part of a picture `Fit::Cover` shows (a game's background placement, see
/// model_crop.rs); bindings supply it (`Binder::image_crop`).
pub type Crop = crate::model::BackgroundPlacement;

pub fn cr(r: f32) -> CornerRadius {
    CornerRadius::same(r.round().clamp(0.0, 255.0) as u8)
}

/// Multiply a colour's alpha (premultiplied-correct).
pub fn fade(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a.clamp(0.0, 1.0))
}

/// A muted colour for a game without artwork, the same for every game of a console (or, for
/// PC games, of a store), so a row of them doesn't look broken and still tells them apart.
pub fn placeholder_tint(g: &crate::model::Game) -> Color32 {
    const TINTS: [Color32; 8] = [
        Color32::from_rgb(52, 92, 168),
        Color32::from_rgb(118, 62, 150),
        Color32::from_rgb(40, 120, 112),
        Color32::from_rgb(156, 74, 64),
        Color32::from_rgb(70, 112, 60),
        Color32::from_rgb(150, 110, 46),
        Color32::from_rgb(64, 82, 120),
        Color32::from_rgb(130, 58, 96),
    ];
    let key = g.emu.as_ref().map(|e| e.platform.as_str()).or(g.platform.as_deref()).unwrap_or_else(|| g.source.key());
    // FNV-1a: stable across runs and builds (the colour mustn't change between starts).
    let h = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
    TINTS[(h % TINTS.len() as u64) as usize]
}

/// Multiply two colours channel by channel (tints).
pub fn mul_color(a: Color32, b: Color32) -> Color32 {
    let m = |x: u8, y: u8| ((x as u16 * y as u16 + 127) / 255) as u8;
    Color32::from_rgba_premultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

/// Thumbnail size class for something drawn `px` physical pixels on its longest side.
pub fn size_class(px: f32) -> ImgSize {
    if px <= ImgSize::Thumb.max_px() as f32 * 1.15 {
        ImgSize::Thumb
    } else if px <= ImgSize::Medium.max_px() as f32 * 1.15 {
        ImgSize::Medium
    } else {
        ImgSize::Full
    }
}

/// Perimeter of a rounded rectangle (clockwise in screen space).
fn rounded_points(rect: Rect, r: f32, out: &mut Vec<Pos2>) {
    let r = r.min(rect.width() / 2.0).min(rect.height() / 2.0).max(0.0);
    if r < 0.5 {
        out.extend([rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()]);
        return;
    }
    let n = ((r / 2.0).ceil() as usize).clamp(3, 12);
    let corners = [
        (pos2(rect.right() - r, rect.top() + r), -90.0f32),
        (pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (pos2(rect.left() + r, rect.bottom() - r), 90.0),
        (pos2(rect.left() + r, rect.top() + r), 180.0),
    ];
    // Start at the top-left corner's end so the order is clockwise starting near top-left.
    for (c, start) in corners.iter().cycle().skip(3).take(4) {
        for i in 0..=n {
            let a = (start + 90.0 * i as f32 / n as f32).to_radians();
            out.push(*c + vec2(a.cos(), a.sin()) * r);
        }
    }
}

/// Filled (optionally rounded) rectangle with a two-stop gradient and anti-aliased edges.
pub fn gradient_rect(painter: &Painter, rect: Rect, radius: f32, colors: [Color32; 2], horizontal: bool) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 || (colors[0].a() == 0 && colors[1].a() == 0) {
        return;
    }
    if radius < 0.5 {
        let mut mesh = Mesh::default();
        let (a, b) = (colors[0], colors[1]);
        let (tl, tr, br, bl) = if horizontal { (a, b, b, a) } else { (a, a, b, b) };
        mesh.colored_vertex(rect.left_top(), tl);
        mesh.colored_vertex(rect.right_top(), tr);
        mesh.colored_vertex(rect.right_bottom(), br);
        mesh.colored_vertex(rect.left_bottom(), bl);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(mesh);
        return;
    }
    let mut pts = Vec::with_capacity(56);
    rounded_points(rect, radius, &mut pts);
    let color_at = |p: Pos2| {
        let t = if horizontal { (p.x - rect.left()) / rect.width() } else { (p.y - rect.top()) / rect.height() };
        lerp_color(colors[0], colors[1], t)
    };
    let feather = 1.0 / painter.pixels_per_point();
    let c = rect.center();
    let n = pts.len() as u32;
    let mut mesh = Mesh::default();
    mesh.reserve_vertices(pts.len() * 2 + 1);
    mesh.colored_vertex(c, color_at(c));
    for (i, p) in pts.iter().enumerate() {
        let prev = pts[(i + pts.len() - 1) % pts.len()];
        let next = pts[(i + 1) % pts.len()];
        let perp = |d: Vec2| vec2(d.y, -d.x);
        let mut nrm = (perp(*p - prev).normalized() + perp(next - *p).normalized()).normalized();
        if nrm.dot(*p - c) < 0.0 {
            nrm = -nrm;
        }
        mesh.colored_vertex(*p - nrm * feather * 0.5, color_at(*p));
        mesh.colored_vertex(*p + nrm * feather * 0.5, Color32::TRANSPARENT);
    }
    for i in 0..n {
        let j = (i + 1) % n;
        let (pi, qi, pj, qj) = (1 + 2 * i, 2 + 2 * i, 1 + 2 * j, 2 + 2 * j);
        mesh.add_triangle(0, pi, pj);
        mesh.add_triangle(pi, qi, pj);
        mesh.add_triangle(pj, qi, qj);
    }
    painter.add(mesh);
}

/// Where an image of `tex_size` lands inside `rect` for a fit mode: (draw rect, uv rect).
/// `contain` places the picture at `align` (0..1 per axis) inside `rect`; `cover` shows the
/// part `crop` picks (a game's background placement) when there is one.
pub fn fit_rects_aligned(rect: Rect, tex_size: Vec2, fit: Fit, align: Vec2, crop: Option<Crop>) -> (Rect, Rect) {
    let full = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    if tex_size.x <= 0.0 || tex_size.y <= 0.0 || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return (rect, full);
    }
    let ta = tex_size.x / tex_size.y;
    let ra = rect.width() / rect.height();
    match fit {
        Fit::Stretch => (rect, full),
        Fit::Contain => {
            let size =
                if ta > ra { vec2(rect.width(), rect.width() / ta) } else { vec2(rect.height() * ta, rect.height()) };
            let a = vec2(align.x.clamp(0.0, 1.0), align.y.clamp(0.0, 1.0));
            (Rect::from_min_size(rect.min + (rect.size() - size) * a, size), full)
        }
        Fit::Cover if crop.is_some() => {
            (rect, crate::model::background_uv(crop, tex_size, rect.size(), vec2(0.5, 0.35)))
        }
        Fit::Cover => {
            let uv = if ta > ra {
                let w = ra / ta;
                Rect::from_min_max(pos2(0.5 - w / 2.0, 0.0), pos2(0.5 + w / 2.0, 1.0))
            } else {
                let h = ta / ra;
                // Bias crops towards the top a little: faces and logos live there.
                let top = ((1.0 - h) * 0.35).max(0.0);
                Rect::from_min_max(pos2(0.0, top), pos2(1.0, top + h))
            };
            (rect, uv)
        }
    }
}

/// Draw a texture into `rect` with fit/tint/rounding/rotation. Returns the rect actually covered.
pub fn paint_image(
    painter: &Painter,
    rect: Rect,
    tex: &TextureHandle,
    fit: Fit,
    tint: Color32,
    radius: f32,
    angle_deg: f32,
) -> Rect {
    paint_image_aligned(painter, rect, tex, fit, tint, radius, angle_deg, vec2(0.5, 0.5), None)
}

/// `paint_image` with the contain-alignment of `fit_rects_aligned`.
#[allow(clippy::too_many_arguments)]
pub fn paint_image_aligned(
    painter: &Painter,
    rect: Rect,
    tex: &TextureHandle,
    fit: Fit,
    tint: Color32,
    radius: f32,
    angle_deg: f32,
    align: Vec2,
    crop: Option<Crop>,
) -> Rect {
    let (r, uv) = fit_rects_aligned(rect, tex.size_vec2(), fit, align, crop);
    if angle_deg != 0.0 {
        // Rotated textured rect shapes get their UVs from the unrotated box, so use a mesh.
        let mut mesh = Mesh::with_texture(tex.id());
        mesh.add_rect_with_uv(r, uv, tint);
        mesh.rotate(egui::emath::Rot2::from_angle(angle_deg.to_radians()), r.center());
        painter.add(mesh);
        return r;
    }
    painter.add(RectShape::filled(r, cr(radius), tint).with_texture(tex.id(), uv));
    r
}

/// `paint_image_aligned` with a tint that runs from `tint[0]` to `tint[1]` across the
/// picture (top → bottom, or left → right with `horizontal`).
#[allow(clippy::too_many_arguments)]
pub fn paint_image_tinted(
    painter: &Painter,
    rect: Rect,
    tex: &TextureHandle,
    fit: Fit,
    tint: [Color32; 2],
    horizontal: bool,
    angle_deg: f32,
    align: Vec2,
    crop: Option<Crop>,
) -> Rect {
    let (r, uv) = fit_rects_aligned(rect, tex.size_vec2(), fit, align, crop);
    let mut mesh = Mesh::with_texture(tex.id());
    let corners = [
        (r.left_top(), uv.left_top(), tint[0]),
        (r.right_top(), uv.right_top(), if horizontal { tint[1] } else { tint[0] }),
        (r.right_bottom(), uv.right_bottom(), tint[1]),
        (r.left_bottom(), uv.left_bottom(), if horizontal { tint[0] } else { tint[1] }),
    ];
    for (pos, uv, color) in corners {
        mesh.vertices.push(egui::epaint::Vertex { pos, uv, color });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    if angle_deg != 0.0 {
        mesh.rotate(egui::emath::Rot2::from_angle(angle_deg.to_radians()), r.center());
    }
    painter.add(mesh);
    r
}

/// Draw a texture whose alpha fades to 0 towards the edges: `mask` = [left, top, right,
/// bottom] fade widths as fractions of the drawn rect. Returns the rect covered.
#[allow(clippy::too_many_arguments)]
pub fn paint_image_masked(
    painter: &Painter,
    rect: Rect,
    tex: &TextureHandle,
    fit: Fit,
    tint: Color32,
    mask: [f32; 4],
    align: Vec2,
    crop: Option<Crop>,
) -> Rect {
    let (r, uv) = fit_rects_aligned(rect, tex.size_vec2(), fit, align, crop);
    let [l, t, rt, b] = mask.map(|m| m.clamp(0.0, 1.0));
    // Stops along each axis with the alpha factor at that stop (smooth-ish: 4 stops per fade).
    fn stops(a: f32, b: f32) -> Vec<(f32, f32)> {
        let mut v = vec![];
        let ease = |x: f32| x * x * (3.0 - 2.0 * x);
        if a > 0.0 {
            for k in 0..4 {
                let f = k as f32 / 4.0;
                v.push((a * f, ease(f)));
            }
        }
        v.push((a, 1.0));
        if b > 0.0 {
            v.push((1.0 - b, 1.0));
            for k in 1..=4 {
                let f = k as f32 / 4.0;
                v.push((1.0 - b + b * f, ease(1.0 - f)));
            }
        } else {
            v.push((1.0, 1.0));
        }
        v.dedup_by(|x, y| (x.0 - y.0).abs() < 1e-6);
        v
    }
    let xs = stops(l, rt);
    let ys = stops(t, b);
    let mut mesh = Mesh::with_texture(tex.id());
    for &(fy, ay) in &ys {
        for &(fx, ax) in &xs {
            let pos = pos2(r.left() + r.width() * fx, r.top() + r.height() * fy);
            let uvp = pos2(uv.left() + uv.width() * fx, uv.top() + uv.height() * fy);
            mesh.vertices.push(egui::epaint::Vertex { pos, uv: uvp, color: tint.gamma_multiply(ax * ay) });
        }
    }
    let w = xs.len() as u32;
    for yi in 0..ys.len() as u32 - 1 {
        for xi in 0..w - 1 {
            let i = yi * w + xi;
            mesh.add_triangle(i, i + 1, i + w);
            mesh.add_triangle(i + 1, i + w + 1, i + w);
        }
    }
    painter.add(mesh);
    r
}

/// Cheap blur: the texture (usually a small thumbnail) drawn several times with offsets,
/// blended so every tap has equal weight. Clipped to `rect`.
pub fn paint_blurred(
    painter: &Painter,
    rect: Rect,
    tex: &TextureHandle,
    fit: Fit,
    tint: Color32,
    blur_px: f32,
    crop: Option<Crop>,
) {
    let painter = painter.with_clip_rect(rect);
    let grown = rect.expand(blur_px * 1.2);
    let mut taps: Vec<Vec2> = vec![Vec2::ZERO];
    for (ring, count) in [(0.5f32, 6usize), (1.0, 10)] {
        for i in 0..count {
            let a = (i as f32 / count as f32 + ring * 0.13) * std::f32::consts::TAU;
            taps.push(vec2(a.cos(), a.sin()) * blur_px * ring);
        }
    }
    for (i, off) in taps.iter().enumerate() {
        let a = 1.0 / (i as f32 + 1.0);
        paint_image_aligned(&painter, grown.translate(*off), tex, fit, fade(tint, a), 0.0, 0.0, vec2(0.5, 0.5), crop);
    }
}

/// Five-pointed star centred at `c` (fan from the centre: the star is star-shaped around it).
pub fn star(painter: &Painter, c: Pos2, r: f32, color: Color32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(c, color);
    for i in 0..10 {
        let a = (-90.0 + 36.0 * i as f32).to_radians();
        let rr = if i % 2 == 0 { r } else { r * 0.45 };
        mesh.colored_vertex(c + vec2(a.cos(), a.sin()) * rr, color);
    }
    for i in 0..10u32 {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % 10);
    }
    painter.add(mesh);
}

/// Filled gamepad silhouette centred at `c`, `w` wide (about 0.66 w tall). The d-pad and the
/// face buttons are cut out in `cut` (the colour behind the glyph).
pub fn gamepad_glyph(painter: &Painter, c: Pos2, w: f32, color: Color32, cut: Color32) {
    let u = w / 16.0;
    // Drawn on a 16-unit grid; the shape spans y -4.8..5.6, so shift it to centre on `c`.
    let p = |x: f32, y: f32| c + vec2(x * u, (y - 0.4) * u);
    // Body (a pill) plus two round grips.
    painter.rect_filled(Rect::from_center_size(p(0.0, -1.0), vec2(16.0 * u, 7.6 * u)), cr(3.8 * u), color);
    for x in [-4.7, 4.7] {
        painter.circle_filled(p(x, 2.1), 3.5 * u, color);
    }
    // D-pad and two face buttons.
    let t = (1.5 * u).max(1.0);
    let d = p(-4.6, -0.7);
    painter.rect_filled(Rect::from_center_size(d, vec2(4.4 * u, t)), cr(0.0), cut);
    painter.rect_filled(Rect::from_center_size(d, vec2(t, 4.4 * u)), cr(0.0), cut);
    let br = (1.05 * u).max(0.8);
    painter.circle_filled(p(3.5, 0.3), br, cut);
    painter.circle_filled(p(5.6, -1.8), br, cut);
}

/// Colour of a Steam Deck rating mark (green / amber / grey, as Steam shows them).
pub fn deck_color(r: crate::model::DeckRating) -> Color32 {
    use crate::model::DeckRating;
    match r {
        DeckRating::Verified => Color32::from_rgb(0x6c, 0xc9, 0x52),
        DeckRating::Playable => Color32::from_rgb(0xf0, 0xb4, 0x3c),
        DeckRating::Unsupported => Color32::from_rgb(0x8d, 0x94, 0x9e),
    }
}

/// Steam Deck rating mark: a coloured disc of radius `r` with a check (verified), an "i"
/// (playable) or a slash (unsupported) in `cut`.
pub fn deck_glyph(painter: &Painter, c: Pos2, r: f32, rating: crate::model::DeckRating, cut: Color32) {
    use crate::model::DeckRating;
    let col = deck_color(rating);
    painter.circle_filled(c, r, col);
    let st = egui::Stroke::new((r * 0.26).max(1.0), cut);
    let p = |x: f32, y: f32| c + vec2(x * r, y * r);
    match rating {
        DeckRating::Verified => {
            painter.add(egui::Shape::line(vec![p(-0.45, 0.02), p(-0.12, 0.36), p(0.48, -0.32)], st));
        }
        DeckRating::Playable => {
            painter.circle_filled(p(0.0, -0.4), r * 0.15, cut);
            painter.rect_filled(
                Rect::from_center_size(p(0.0, 0.17), vec2(st.width, r * 0.72)),
                cr(st.width / 2.0),
                cut,
            );
        }
        DeckRating::Unsupported => {
            painter.line_segment([p(-0.42, 0.42), p(0.42, -0.42)], st);
        }
    }
}

/// Small dark pill with a gamepad glyph (the "full controller support" badge on covers),
/// `h` tall, its bottom-right corner at `corner`. Returns the pill rect.
pub fn controller_badge(painter: &Painter, corner: Pos2, h: f32, alpha: f32) -> Rect {
    let pill = Rect::from_min_max(corner - vec2(h * 1.6, h), corner);
    painter.rect_filled(pill, cr(h / 2.0), fade(Color32::from_black_alpha(165), alpha));
    // Opaque glyph colours (dimmed instead of faded): the glyph is built from overlapping
    // shapes, and translucent overlaps would show as brighter seams.
    let dark = Color32::from_rgb(24, 26, 31);
    gamepad_glyph(painter, pill.center(), h * 0.92, lerp_color(dark, Color32::from_gray(238), alpha), dark);
    pill
}

/// The favourite star's colour: the theme accent, lifted towards white when it is too dark to
/// read on the badge's dark disc (some themes use a deep blue or red accent). Opaque.
pub fn favorite_star_color(accent: Color32) -> Color32 {
    let [r, g, b, _] = accent.to_array();
    // Relative luminance (sRGB weights on the gamma values: close enough to judge "too dark").
    let lum = (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0;
    const MIN: f32 = 0.68;
    let opaque = Color32::from_rgb(r, g, b);
    if lum >= MIN {
        opaque
    } else {
        lerp_color(opaque, Color32::WHITE, ((MIN - lum) / (1.0 - lum).max(1e-3)).clamp(0.0, 1.0))
    }
}

/// Where the favourite badge sits on an item drawn at `item`: the top-right corner, a disc `h`
/// across inset by a third of `h`. Returns the disc's centre and radius.
pub fn favorite_badge_spot(item: Rect, h: f32) -> (Pos2, f32) {
    let inset = h * 0.34;
    (pos2(item.right() - inset - h / 2.0, item.top() + inset + h / 2.0), h / 2.0)
}

/// The favourite badge (a ★ in `accent` on a dark disc with a faint light rim, readable on any
/// cover) in the top-right corner of `item`, `h` across. Returns the disc's bounding rect.
pub fn favorite_badge(painter: &Painter, item: Rect, h: f32, accent: Color32, alpha: f32) -> Rect {
    let (c, rad) = favorite_badge_spot(item, h);
    painter.circle_filled(c, rad, fade(Color32::from_black_alpha(170), alpha));
    painter.circle_stroke(c, rad - 0.5, egui::Stroke::new(1.0, fade(Color32::from_white_alpha(70), alpha)));
    // Dimmed by mixing towards the disc, not by alpha (the fan's triangles overlap at the centre).
    let dark = Color32::from_rgb(14, 15, 18);
    star(painter, c + vec2(0.0, rad * 0.04), rad * 0.68, lerp_color(dark, favorite_star_color(accent), alpha));
    Rect::from_center_size(c, vec2(h, h))
}

/// Row height used for a font size and a theme `line_spacing` multiplier.
pub fn line_height(px: f32, line_spacing: f32) -> f32 {
    px * line_spacing.clamp(0.6, 4.0) * 1.12
}

/// Lay out text with a theme TextStyle at `px` points.
#[allow(clippy::too_many_arguments)]
pub fn layout(
    painter: &Painter,
    text: &str,
    family: FontFamily,
    px: f32,
    color: Color32,
    style: &TextStyle,
    wrap_width: Option<f32>,
    max_rows: Option<u32>,
) -> Arc<Galley> {
    let text = if style.uppercase { text.to_uppercase() } else { text.to_string() };
    let format = TextFormat {
        font_id: FontId::new(px.max(1.0), family),
        color,
        extra_letter_spacing: style.letter_spacing * px,
        line_height: Some(line_height(px, style.line_spacing)),
        ..Default::default()
    };
    let mut job = LayoutJob::single_section(text, format);
    job.wrap = match wrap_width {
        Some(w) => TextWrapping {
            max_width: w.max(1.0),
            max_rows: max_rows.map(|r| r.max(1) as usize).unwrap_or(usize::MAX),
            break_anywhere: false,
            overflow_character: Some('…'),
        },
        None => TextWrapping::no_max_width(),
    };
    job.halign = match style.align {
        Align::Left => egui::Align::LEFT,
        Align::Center => egui::Align::Center,
        Align::Right => egui::Align::RIGHT,
    };
    painter.layout_job(job)
}

/// Single line, truncated with "…" at `max_width`.
pub fn layout_line(painter: &Painter, text: &str, font: FontId, color: Color32, max_width: f32) -> Arc<Galley> {
    let mut job =
        LayoutJob::single_section(text.to_string(), TextFormat { font_id: font, color, ..Default::default() });
    job.wrap = TextWrapping::truncate_at_width(max_width.max(1.0));
    painter.layout_job(job)
}

/// Top-left paint position of a galley laid out with `align` inside `rect`.
pub fn galley_pos(rect: Rect, galley: &Galley, align: Align, valign: VAlign) -> Pos2 {
    let x = match align {
        Align::Left => rect.left(),
        Align::Center => rect.center().x,
        Align::Right => rect.right(),
    };
    let h = galley.size().y;
    let y = match valign {
        VAlign::Top => rect.top(),
        VAlign::Center => rect.center().y - h / 2.0,
        VAlign::Bottom => rect.bottom() - h,
    };
    pos2(x, y)
}

/// Top-left paint position of a LEFT-aligned galley (e.g. from `layout_line`) inside `rect`.
pub fn line_pos(rect: Rect, galley: &Galley, align: Align, valign: VAlign) -> Pos2 {
    let w = galley.size().x;
    let x = match align {
        Align::Left => rect.left(),
        Align::Center => rect.center().x - w / 2.0,
        Align::Right => rect.right() - w,
    };
    pos2(x, galley_pos(rect, galley, Align::Left, valign).y)
}

/// Paint a galley, with an optional soft shadow, opacity and rotation.
pub fn paint_galley(
    painter: &Painter,
    pos: Pos2,
    galley: &Arc<Galley>,
    color: Color32,
    shadow: Option<Color32>,
    angle_deg: f32,
) {
    let px = galley.rows.first().and_then(|r| r.row.glyphs.first()).map(|g| g.font_height).unwrap_or(16.0);
    if let Some(s) = shadow {
        let off = (px * 0.06).clamp(1.0, 4.0);
        for (d, a) in [(vec2(0.0, off), 0.55), (vec2(off * 0.5, off * 1.6), 0.3)] {
            let mut ts = TextShape::new(pos + d, galley.clone(), s);
            ts.override_text_color = Some(fade(s, a));
            ts.angle = angle_deg.to_radians();
            painter.add(ts);
        }
    }
    let mut ts = TextShape::new(pos, galley.clone(), color);
    ts.angle = angle_deg.to_radians();
    painter.add(ts);
}

/// Soft glow/shadow around a rect.
pub fn glow(painter: &Painter, rect: Rect, radius: f32, color: Color32, blur: f32) {
    let s = Shadow { offset: [0, 0], blur: blur.clamp(0.0, 255.0) as u8, spread: 0, color };
    painter.add(s.as_shape(rect, cr(radius)));
}

/// Shadow under a card (offset downwards).
pub fn drop_shadow(painter: &Painter, rect: Rect, radius: f32, alpha: f32) {
    let blur = (rect.height() * 0.08).clamp(6.0, 40.0);
    let s = Shadow {
        offset: [0, (blur * 0.35).clamp(0.0, 127.0) as i8],
        blur: blur as u8,
        spread: 0,
        color: Color32::from_black_alpha((alpha.clamp(0.0, 1.0) * 150.0) as u8),
    };
    painter.add(s.as_shape(rect, cr(radius)));
}

/// Rotate a point around a pivot.
pub fn rotate_around(p: Pos2, pivot: Pos2, angle_deg: f32) -> Pos2 {
    pivot + egui::emath::Rot2::from_angle(angle_deg.to_radians()) * (p - pivot)
}

// Controller hints (button glyphs in the style of the pad in use) live in hints.rs.
mod hints;
pub use hints::*;

/// Most tiles `paint_tiled` draws per element and frame.
const MAX_TILES: f32 = 20_000.0;

/// Repeat `tex` (`natural` = one tile's size) over `rect`, as one mesh. Themes may tint a
/// 16 px white square into a coloured panel, which is thousands of tiles on a 1080p or 4K
/// screen: past `MAX_TILES` the tiles grow instead (a plain square looks the same).
pub fn paint_tiled(painter: &Painter, rect: Rect, tex: &TextureHandle, natural: Vec2, tint: Color32) {
    if !(rect.width() > 0.0 && rect.height() > 0.0) {
        return;
    }
    let (ts, nx, ny) = tile_grid(rect.size(), natural);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    let mut mesh = Mesh::with_texture(tex.id());
    mesh.reserve_triangles(nx * ny * 2);
    mesh.reserve_vertices(nx * ny * 4);
    for y in 0..ny {
        for x in 0..nx {
            let r = Rect::from_min_size(rect.min + vec2(x as f32 * ts.x, y as f32 * ts.y), ts);
            mesh.add_rect_with_uv(r, uv, tint);
        }
    }
    painter.with_clip_rect(rect.intersect(painter.clip_rect())).add(mesh);
}

/// Tile size and columns × rows for `paint_tiled` (never more than `MAX_TILES`).
pub fn tile_grid(area: Vec2, natural: Vec2) -> (Vec2, usize, usize) {
    let mut ts = natural.max(vec2(4.0, 4.0));
    let count = (area.x / ts.x).ceil() * (area.y / ts.y).ceil();
    if count > MAX_TILES {
        ts *= (count / MAX_TILES).sqrt().ceil();
    }
    (ts, (area.x / ts.x).ceil() as usize, (area.y / ts.y).ceil() as usize)
}
