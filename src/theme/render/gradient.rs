//! N-stop, angled and radial gradients (`theme::Gradient`) drawn as one egui mesh, cached.
//!
//! Linear gradients are exact: the shape (a convex rounded rectangle) is cut into bands
//! between the stops, and the colour is linear inside each band, so the GPU's vertex-colour
//! interpolation reproduces it. Radial gradients use elliptic rings at the stops (exact along
//! every ray) and 64 segments around. Rounded edges get a one-pixel anti-aliasing fringe.
//!
//! `GradientCache` keeps the meshes: built once per (size, radius, gradient) in local
//! coordinates, and once more per (position, opacity) as a ready `Arc<Mesh>`. A static
//! background costs one hash lookup per frame and no allocation.

use crate::theme::{Gradient, GradientStop};
use egui::epaint::{Mesh, Shape};
use egui::{Color32, Painter, Pos2, Rect, Vec2, pos2, vec2};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

/// Corner radii [top-left, top-right, bottom-right, bottom-left] in points.
pub type Corners = [f32; 4];

/// Segments around a radial gradient.
const RADIAL_SEGMENTS: usize = 64;

/// The outline of a rounded rectangle, clockwise (screen space) from the top-left corner.
pub fn rounded_polygon(rect: Rect, radii: Corners) -> Vec<Pos2> {
    let max_r = rect.width().min(rect.height()) / 2.0;
    let mut out = Vec::with_capacity(4 * 13);
    let centres = [
        (rect.left_top(), vec2(1.0, 1.0), 180.0f32),
        (rect.right_top(), vec2(-1.0, 1.0), 270.0),
        (rect.right_bottom(), vec2(-1.0, -1.0), 0.0),
        (rect.left_bottom(), vec2(1.0, -1.0), 90.0),
    ];
    for (k, (corner, inward, start)) in centres.iter().enumerate() {
        let r = radii[k].clamp(0.0, max_r.max(0.0));
        if r < 0.5 {
            out.push(*corner);
            continue;
        }
        let c = *corner + *inward * r;
        let n = ((r / 2.0).ceil() as usize).clamp(3, 12);
        for i in 0..=n {
            let a = (start + 90.0 * i as f32 / n as f32).to_radians();
            out.push(c + vec2(a.cos(), a.sin()) * r);
        }
    }
    out
}

/// Keep the part of convex polygon `poly` where `f(p) >= 0` (`f` linear).
fn clip_half(poly: &[Pos2], f: impl Fn(Pos2) -> f32) -> Vec<Pos2> {
    let mut out = Vec::with_capacity(poly.len() + 2);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (fa, fb) = (f(a), f(b));
        if fa >= 0.0 {
            out.push(a);
        }
        if (fa >= 0.0) != (fb >= 0.0) {
            let t = fa / (fa - fb);
            out.push(a + (b - a) * t);
        }
    }
    out
}

/// Clip convex polygon `subject` to convex polygon `clip` (clockwise in screen space).
fn clip_convex(subject: &[Pos2], clip: &[Pos2]) -> Vec<Pos2> {
    let mut out = subject.to_vec();
    for i in 0..clip.len() {
        if out.len() < 3 {
            return Vec::new();
        }
        let (a, b) = (clip[i], clip[(i + 1) % clip.len()]);
        if (b - a).length_sq() < 1e-8 {
            continue;
        }
        // Inside = right of a→b in screen space (y down, clockwise outline).
        out = clip_half(&out, |p| {
            let (d, e) = (b - a, p - a);
            d.x * e.y - d.y * e.x
        });
    }
    out
}

fn color32(stops: &[GradientStop], t: f32) -> Color32 {
    crate::theme::gradient_color_at(stops, t).to_egui()
}

fn fan(mesh: &mut Mesh, poly: &[Pos2], color: impl Fn(Pos2) -> Color32) {
    if poly.len() < 3 {
        return;
    }
    let base = mesh.vertices.len() as u32;
    for p in poly {
        mesh.colored_vertex(*p, color(*p));
    }
    for i in 1..poly.len() as u32 - 1 {
        mesh.add_triangle(base, base + i, base + i + 1);
    }
}

/// Build the mesh for `g` over `rect` with rounded corners (`feather` = AA fringe width in
/// points, 0 = none). Colours at full opacity.
pub fn gradient_mesh(rect: Rect, radii: Corners, g: &Gradient, feather: f32) -> Mesh {
    let mut mesh = Mesh::default();
    let stops = g.sorted_stops();
    if stops.is_empty() || !(rect.width() > 0.0 && rect.height() > 0.0) {
        return mesh;
    }
    let rounded = radii.iter().any(|r| *r >= 0.5);
    let feather = if rounded { feather.max(0.0) } else { 0.0 };
    let outline = rounded_polygon(rect, radii);
    // The fill stops half a fringe inside the outline.
    let inner: Vec<Pos2> = if feather > 0.0 { offset_polygon(&outline, -feather * 0.5) } else { outline.clone() };
    let param = |p: Pos2| g.param((p.x - rect.left()) / rect.width(), (p.y - rect.top()) / rect.height());
    let color = |p: Pos2| color32(&stops, param(p));

    if let Some([cx, cy, rx, ry]) = g.radial {
        let c = pos2(rect.left() + cx * rect.width(), rect.top() + cy * rect.height());
        let ax = vec2(rx.abs().max(1e-4) * rect.width(), 0.0);
        let ay = vec2(0.0, ry.abs().max(1e-4) * rect.height());
        // Rings at every stop inside the shape, and one past its farthest corner.
        let t_max = inner.iter().map(|p| param(*p)).fold(0.0f32, f32::max) * 1.01 + 0.01;
        let mut rings: Vec<f32> = vec![0.0];
        rings.extend(stops.iter().map(|s| s.offset).filter(|o| *o > 1e-4 && *o < t_max));
        rings.push(t_max);
        rings.dedup_by(|a, b| (*a - *b).abs() < 1e-5);
        let at = |t: f32, k: usize| {
            let a = std::f32::consts::TAU * k as f32 / RADIAL_SEGMENTS as f32;
            c + ax * (t * a.cos()) + ay * (t * a.sin())
        };
        for w in rings.windows(2) {
            for k in 0..RADIAL_SEGMENTS {
                let quad = [at(w[0], k), at(w[1], k), at(w[1], k + 1), at(w[0], k + 1)];
                // Make the quad clockwise like the clip outline (its winding follows the axes).
                let mut q: Vec<Pos2> = quad.to_vec();
                if signed_area(&q) < 0.0 {
                    q.reverse();
                }
                let piece = clip_convex(&q, &inner);
                fan(&mut mesh, &piece, color);
            }
        }
    } else {
        // Linear: t(p) = A·p + B. Bands between consecutive stops (padded at both ends).
        let [dx, dy] = g.direction();
        let l = (dx.abs() + dy.abs()).max(1e-6);
        let ga = vec2(dx / (l * rect.width()), dy / (l * rect.height()));
        let t_of = |p: Pos2| ga.x * (p.x - rect.center().x) + ga.y * (p.y - rect.center().y) + 0.5;
        let (lo, hi) = inner.iter().map(|p| t_of(*p)).fold((f32::MAX, f32::MIN), |(a, b), t| (a.min(t), b.max(t)));
        let mut cuts: Vec<f32> = stops.iter().map(|s| s.offset).filter(|o| *o > lo && *o < hi).collect();
        cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        let mut edges = vec![f32::NEG_INFINITY];
        edges.extend(cuts);
        edges.push(f32::INFINITY);
        for w in edges.windows(2) {
            let (a, b) = (w[0], w[1]);
            let mut band = inner.clone();
            if a.is_finite() {
                band = clip_half(&band, |p| t_of(p) - a);
            }
            if b.is_finite() {
                band = clip_half(&band, |p| b - t_of(p));
            }
            // Just inside the band, so a hard stop (two stops at one offset) stays sharp.
            let (ia, ib) = (a + 1e-5, b - 1e-5);
            fan(&mut mesh, &band, |p| color32(&stops, t_of(p).clamp(ia, ib.max(ia))));
        }
    }

    if feather > 0.0 {
        // Fringe: from the fill's edge out to half a fringe past the outline, fading out.
        let outer = offset_polygon(&outline, feather * 0.5);
        let n = inner.len() as u32;
        let base = mesh.vertices.len() as u32;
        for (p, q) in inner.iter().zip(&outer) {
            mesh.colored_vertex(*p, color(*p));
            mesh.colored_vertex(*q, Color32::TRANSPARENT);
        }
        for i in 0..n {
            let j = (i + 1) % n;
            let (pi, qi, pj, qj) = (base + 2 * i, base + 2 * i + 1, base + 2 * j, base + 2 * j + 1);
            mesh.add_triangle(pi, qi, pj);
            mesh.add_triangle(pj, qi, qj);
        }
    }
    mesh
}

fn signed_area(p: &[Pos2]) -> f32 {
    let mut a = 0.0;
    for i in 0..p.len() {
        let (u, v) = (p[i], p[(i + 1) % p.len()]);
        a += u.x * v.y - v.x * u.y;
    }
    a * 0.5
}

/// Move each vertex of a convex outline along its normal (positive = outwards).
fn offset_polygon(poly: &[Pos2], d: f32) -> Vec<Pos2> {
    let n = poly.len();
    let c = poly.iter().fold(Vec2::ZERO, |acc, p| acc + p.to_vec2()) / n.max(1) as f32;
    let c = c.to_pos2();
    (0..n)
        .map(|i| {
            let (prev, p, next) = (poly[(i + n - 1) % n], poly[i], poly[(i + 1) % n]);
            let perp = |v: Vec2| vec2(v.y, -v.x);
            let e1 = p - prev;
            let e2 = next - p;
            let mut nrm = Vec2::ZERO;
            if e1.length_sq() > 1e-8 {
                nrm += perp(e1).normalized();
            }
            if e2.length_sq() > 1e-8 {
                nrm += perp(e2).normalized();
            }
            let mut nrm = if nrm.length_sq() > 1e-8 { nrm.normalized() } else { (p - c).normalized() };
            if nrm.dot(p - c) < 0.0 {
                nrm = -nrm;
            }
            p + nrm * d
        })
        .collect()
}

fn hash_gradient(h: &mut impl Hasher, g: &Gradient) {
    for s in &g.stops {
        s.color.hash(h);
        s.offset.to_bits().hash(h);
    }
    g.angle.map(f32::to_bits).hash(h);
    g.radial.map(|r| r.map(f32::to_bits)).hash(h);
}

/// Meshes of the gradients drawn recently (see the module comment).
#[derive(Default)]
pub struct GradientCache {
    /// (size, radii, feather, gradient) → mesh at the origin, full opacity.
    local: HashMap<u64, (Arc<Mesh>, u64)>,
    /// (local key, position, opacity) → ready mesh.
    placed: HashMap<u64, (Arc<Mesh>, u64)>,
    frame: u64,
}

impl GradientCache {
    /// Call once per frame (drops meshes not drawn for a while).
    pub fn next_frame(&mut self) {
        self.frame += 1;
        let now = self.frame;
        if self.placed.len() > 96 {
            self.placed.retain(|_, (_, f)| now - *f <= 2);
        }
        if self.local.len() > 64 {
            self.local.retain(|_, (_, f)| now - *f <= 600);
        }
    }

    /// Paint `g` over `rect` (rounded `radii`) at `opacity`.
    pub fn paint(&mut self, painter: &Painter, rect: Rect, radii: Corners, g: &Gradient, opacity: f32) {
        if let Some(m) = self.mesh(rect, radii, g, opacity, 1.0 / painter.pixels_per_point()) {
            painter.add(Shape::Mesh(m));
        }
    }

    /// The ready mesh (None when there is nothing to draw).
    pub fn mesh(&mut self, rect: Rect, radii: Corners, g: &Gradient, opacity: f32, feather: f32) -> Option<Arc<Mesh>> {
        let opacity = opacity.clamp(0.0, 1.0);
        if opacity <= 0.0 || g.stops.is_empty() || g.is_invisible() || !(rect.width() > 0.0 && rect.height() > 0.0) {
            return None;
        }
        let mut h = DefaultHasher::new();
        rect.width().to_bits().hash(&mut h);
        rect.height().to_bits().hash(&mut h);
        radii.map(f32::to_bits).hash(&mut h);
        feather.to_bits().hash(&mut h);
        hash_gradient(&mut h, g);
        let lkey = h.finish();
        rect.min.x.to_bits().hash(&mut h);
        rect.min.y.to_bits().hash(&mut h);
        opacity.to_bits().hash(&mut h);
        let pkey = h.finish();
        let now = self.frame;
        if let Some((m, f)) = self.placed.get_mut(&pkey) {
            *f = now;
            return Some(m.clone());
        }
        let local = match self.local.get_mut(&lkey) {
            Some((m, f)) => {
                *f = now;
                m.clone()
            }
            None => {
                let m = Arc::new(gradient_mesh(Rect::from_min_size(Pos2::ZERO, rect.size()), radii, g, feather));
                self.local.insert(lkey, (m.clone(), now));
                m
            }
        };
        let mut m = (*local).clone();
        m.translate(rect.min.to_vec2());
        if opacity < 1.0 {
            for v in &mut m.vertices {
                v.color = v.color.gamma_multiply(opacity);
            }
        }
        let m = Arc::new(m);
        self.placed.insert(pkey, (m.clone(), now));
        Some(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Color;

    fn c(s: &str) -> Color {
        Color::parse(s).unwrap()
    }

    fn area(mesh: &Mesh) -> f32 {
        mesh.indices
            .chunks(3)
            .map(|t| {
                let [a, b, c] = [0, 1, 2].map(|k| mesh.vertices[t[k] as usize].pos);
                ((b - a).x * (c - a).y - (b - a).y * (c - a).x).abs() * 0.5
            })
            .sum()
    }

    fn three_stops(angle: Option<f32>) -> Gradient {
        Gradient {
            stops: vec![
                GradientStop::new(c("#ff0000"), 0.0),
                GradientStop::new(c("#00ff00"), 0.5),
                GradientStop::new(c("#0000ff"), 1.0),
            ],
            angle,
            radial: None,
        }
    }

    #[test]
    fn linear_bands_cover_the_rect_exactly() {
        let r = Rect::from_min_size(pos2(10.0, 20.0), vec2(200.0, 100.0));
        for angle in [None, Some(0.0), Some(30.0), Some(135.0), Some(-90.0)] {
            let m = gradient_mesh(r, [0.0; 4], &three_stops(angle), 0.0);
            assert!((area(&m) - 20_000.0).abs() < 1.0, "{angle:?}: {}", area(&m));
            // Every vertex has the gradient's colour at its position.
            let g = three_stops(angle);
            for v in &m.vertices {
                let t = g.param((v.pos.x - r.left()) / r.width(), (v.pos.y - r.top()) / r.height());
                assert_eq!(v.color, g.color_at(t).to_egui());
            }
        }
        // Top → bottom: the middle stop is green along the middle row.
        let m = gradient_mesh(r, [0.0; 4], &three_stops(None), 0.0);
        assert!(m.vertices.iter().any(|v| (v.pos.y - 70.0).abs() < 1e-3 && v.color == Color32::from_rgb(0, 255, 0)));
    }

    #[test]
    fn rounded_and_radial_shapes() {
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 60.0));
        let m = gradient_mesh(r, [12.0; 4], &three_stops(Some(45.0)), 0.0);
        let rounded_area = 100.0 * 60.0 - (4.0 - std::f32::consts::PI) * 144.0;
        assert!((area(&m) - rounded_area).abs() < 8.0, "{}", area(&m));
        let rad = Gradient::radial(three_stops(None).stops, [0.5, 0.5], [0.5, 0.5]);
        let m = gradient_mesh(r, [0.0; 4], &rad, 0.0);
        assert!((area(&m) - 6000.0).abs() < 6.0, "{}", area(&m));
        // The centre is the first colour, the corners the last.
        let near = |p: Pos2| m.vertices.iter().find(|v| (v.pos - p).length() < 0.01).map(|v| v.color);
        assert_eq!(near(pos2(50.0, 30.0)), Some(Color32::from_rgb(255, 0, 0)));
        assert_eq!(near(pos2(0.0, 0.0)), Some(Color32::from_rgb(0, 0, 255)));
        // A fringe adds transparent outer vertices.
        let m = gradient_mesh(r, [8.0; 4], &rad, 1.0);
        assert!(m.vertices.iter().any(|v| v.color == Color32::TRANSPARENT));
    }

    #[test]
    fn cache_reuses_meshes() {
        let mut cache = GradientCache::default();
        let g = three_stops(Some(10.0));
        let r = Rect::from_min_size(pos2(5.0, 5.0), vec2(300.0, 40.0));
        let a = cache.mesh(r, [4.0; 4], &g, 1.0, 1.0).unwrap();
        cache.next_frame();
        let b = cache.mesh(r, [4.0; 4], &g, 1.0, 1.0).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        // Moved: same local mesh, translated.
        let moved = cache.mesh(r.translate(vec2(10.0, 0.0)), [4.0; 4], &g, 1.0, 1.0).unwrap();
        assert_eq!(moved.vertices.len(), a.vertices.len());
        assert!((moved.vertices[0].pos.x - a.vertices[0].pos.x - 10.0).abs() < 1e-4);
        assert_eq!(cache.local.len(), 1);
        // Invisible / empty: nothing.
        assert!(cache.mesh(r, [0.0; 4], &Gradient::default(), 1.0, 1.0).is_none());
        assert!(cache.mesh(r, [0.0; 4], &g, 0.0, 1.0).is_none());
        // Many frames of moving meshes stay bounded.
        for i in 0..500 {
            cache.mesh(r.translate(vec2(i as f32, 0.0)), [4.0; 4], &g, 1.0, 1.0);
            cache.next_frame();
        }
        assert!(cache.placed.len() <= 100);
    }
}
