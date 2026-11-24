//! Which part of a background picture shows (Arrange mode): the picture always
//! fills its box (`object-fit: cover`, never an empty edge); the user's [`BackgroundPlacement`]
//! says which point of the picture to keep in the middle and how far to zoom in. The same
//! placement is used by every box the background shows in (the game page's header, the ambient
//! window background, controller mode), whatever their shapes.
//!
//! Everything here is pure math in texture uv space (0..1 on both axes).

use egui::{Rect, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};

/// A hand-placed background: the picture point (fractions of its width / height) kept in the
/// middle of the box as far as the picture allows, and the zoom over "just fills the box".
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackgroundPlacement {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

impl Default for BackgroundPlacement {
    fn default() -> Self {
        BackgroundPlacement { x: 0.5, y: 0.5, zoom: 1.0 }
    }
}

/// Lenient `Option<BackgroundPlacement>`: anything that isn't one (a newer build's shape, a
/// hand edit) reads as None instead of failing the whole edits row.
pub fn deserialize_background_lenient<'de, D>(d: D) -> Result<Option<BackgroundPlacement>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value::<BackgroundPlacement>(v).ok().map(BackgroundPlacement::clamped))
}

impl BackgroundPlacement {
    pub const MAX_ZOOM: f32 = 4.0;

    /// Inside the picture and at a sane zoom whatever a hand-edited value says.
    pub fn clamped(self) -> BackgroundPlacement {
        let ok = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        BackgroundPlacement {
            x: ok(self.x, 0.0, 1.0, 0.5),
            y: ok(self.y, 0.0, 1.0, 0.5),
            zoom: ok(self.zoom, 1.0, Self::MAX_ZOOM, 1.0),
        }
    }

    /// The part of a `tex`-sized picture shown in a `target`-sized box (uv rect).
    pub fn uv(self, tex: Vec2, target: Vec2) -> Rect {
        let p = self.clamped();
        let s = fill_size(tex, target) / p.zoom;
        Rect::from_center_size(pos2(centre_in(p.x, s.x), centre_in(p.y, s.y)), s)
    }

    /// The placement showing `uv` in this box: where Arrange starts from when the game has no
    /// placement yet (the automatic crop). `keep` is the focus used on an axis the picture
    /// can't move along in this box (it still matters in boxes of other shapes).
    pub fn from_uv(uv: Rect, tex: Vec2, target: Vec2, keep: Vec2) -> BackgroundPlacement {
        let fill = fill_size(tex, target);
        let zoom = if uv.width() > 0.0 { fill.x / uv.width() } else { 1.0 };
        let mut p = BackgroundPlacement { x: keep.x, y: keep.y, zoom }.clamped();
        let s = fill / p.zoom;
        if s.x < 1.0 - EPS {
            p.x = uv.center().x;
        }
        if s.y < 1.0 - EPS {
            p.y = uv.center().y;
        }
        p.clamped()
    }

    /// Which ways the picture can move in this box: (left/right, up/down).
    pub fn can_pan(self, tex: Vec2, target: Vec2) -> (bool, bool) {
        let s = fill_size(tex, target) / self.clamped().zoom;
        (s.x < 1.0 - EPS, s.y < 1.0 - EPS)
    }

    /// Dragged by `delta` box points: the picture follows the pointer. The focus stops at the
    /// picture's edges (dragging further doesn't build up an invisible offset); an axis the
    /// picture can't move along keeps its value.
    pub fn panned(self, tex: Vec2, target: Vec2, delta: Vec2) -> BackgroundPlacement {
        if target.x <= 0.0 || target.y <= 0.0 {
            return self;
        }
        let uv = self.uv(tex, target);
        let d = vec2(delta.x / target.x * uv.width(), delta.y / target.y * uv.height());
        let c = uv.center() - d;
        self.settle(tex, target, vec2(c.x, c.y), self.clamped().zoom)
    }

    /// Zoomed by `factor` around `anchor` (fractions of the box: the picture point under the
    /// pointer stays under it as far as the edges allow). Never below "just fills the box".
    pub fn zoomed(self, tex: Vec2, target: Vec2, anchor: Vec2, factor: f32) -> BackgroundPlacement {
        if !factor.is_finite() || factor <= 0.0 {
            return self;
        }
        let uv = self.uv(tex, target);
        let a = vec2(anchor.x.clamp(0.0, 1.0), anchor.y.clamp(0.0, 1.0));
        let under = uv.min + a * uv.size();
        let zoom = (self.clamped().zoom * factor).clamp(1.0, Self::MAX_ZOOM);
        let s = fill_size(tex, target) / zoom;
        let c = under - a * s + s / 2.0;
        self.settle(tex, target, vec2(c.x, c.y), zoom)
    }

    /// `centre` / `zoom` with each movable axis kept inside the picture; fixed axes unchanged.
    fn settle(self, tex: Vec2, target: Vec2, centre: Vec2, zoom: f32) -> BackgroundPlacement {
        let mut p = BackgroundPlacement { zoom, ..self.clamped() };
        let s = fill_size(tex, target) / p.zoom;
        if s.x < 1.0 - EPS {
            p.x = centre_in(centre.x, s.x);
        }
        if s.y < 1.0 - EPS {
            p.y = centre_in(centre.y, s.y);
        }
        p.clamped()
    }
}

const EPS: f32 = 1e-4;

/// uv size of a picture just filling a box (`object-fit: cover` at zoom 1): the full height of a
/// wider picture, the full width of a taller one.
pub fn fill_size(tex: Vec2, target: Vec2) -> Vec2 {
    if tex.x <= 0.0 || tex.y <= 0.0 || target.x <= 0.0 || target.y <= 0.0 {
        return vec2(1.0, 1.0);
    }
    let (ta, ra) = (tex.x / tex.y, target.x / target.y);
    if ta > ra { vec2(ra / ta, 1.0) } else { vec2(1.0, ta / ra) }
}

/// Size (box points) of a picture fitted inside a box (`object-fit: contain`).
pub fn fit_size(tex: Vec2, target: Vec2) -> Vec2 {
    if tex.x <= 0.0 || tex.y <= 0.0 {
        return target;
    }
    tex * (target.x / tex.x).min(target.y / tex.y)
}

/// The automatic crop: the picture filling the box, its spare part split by `align` (0 = keep
/// the left / top edge, 1 = the right / bottom one).
pub fn aligned_uv(tex: Vec2, target: Vec2, align: Vec2) -> Rect {
    let s = fill_size(tex, target);
    let min = pos2((1.0 - s.x) * align.x.clamp(0.0, 1.0), (1.0 - s.y) * align.y.clamp(0.0, 1.0));
    Rect::from_min_size(min, s)
}

/// A game's background crop: its placement, else the automatic one at `align`.
pub fn background_uv(placement: Option<BackgroundPlacement>, tex: Vec2, target: Vec2, align: Vec2) -> Rect {
    match placement {
        Some(p) => p.uv(tex, target),
        None => aligned_uv(tex, target, align),
    }
}

/// Centre of a `size`-long window on 0..1 kept inside it.
fn centre_in(c: f32, size: f32) -> f32 {
    if size >= 1.0 { 0.5 } else { c.clamp(size / 2.0, 1.0 - size / 2.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Rect, b: Rect) -> bool {
        (a.min - b.min).length() < 1e-4 && (a.max - b.max).length() < 1e-4
    }
    fn inside(r: Rect) -> bool {
        r.min.x >= -1e-5 && r.min.y >= -1e-5 && r.max.x <= 1.0 + 1e-5 && r.max.y <= 1.0 + 1e-5
    }

    #[test]
    fn fill_and_fit_sizes() {
        // A 2:1 picture in a 1:1 box: half its width, all its height.
        assert_eq!(fill_size(vec2(200.0, 100.0), vec2(50.0, 50.0)), vec2(0.5, 1.0));
        // A 1:2 picture in a 2:1 box: all its width, a quarter of its height.
        assert_eq!(fill_size(vec2(100.0, 200.0), vec2(100.0, 50.0)), vec2(1.0, 0.25));
        assert_eq!(fill_size(vec2(100.0, 100.0), vec2(30.0, 30.0)), vec2(1.0, 1.0));
        assert_eq!(fill_size(vec2(0.0, 100.0), vec2(30.0, 30.0)), vec2(1.0, 1.0));
        assert_eq!(fit_size(vec2(200.0, 100.0), vec2(100.0, 100.0)), vec2(100.0, 50.0));
        assert_eq!(fit_size(vec2(100.0, 200.0), vec2(100.0, 100.0)), vec2(50.0, 100.0));
    }

    #[test]
    fn default_placement_matches_centred_cover() {
        let tex = vec2(1920.0, 620.0);
        for target in [vec2(800.0, 400.0), vec2(1600.0, 300.0), vec2(300.0, 300.0)] {
            let a = BackgroundPlacement::default().uv(tex, target);
            assert!(close(a, aligned_uv(tex, target, vec2(0.5, 0.5))), "{target:?}");
        }
    }

    #[test]
    fn crop_never_leaves_the_picture() {
        let tex = vec2(1920.0, 1080.0);
        let target = vec2(900.0, 300.0); // wider than the picture: up/down only at zoom 1
        for (x, y, zoom) in [(0.0, 0.0, 1.0), (1.0, 1.0, 1.0), (0.0, 1.0, 3.0), (0.5, 0.5, 9.0), (f32::NAN, -4.0, 0.2)]
        {
            let uv = BackgroundPlacement { x, y, zoom }.uv(tex, target);
            assert!(inside(uv), "{x} {y} {zoom}: {uv:?}");
            // Same aspect as the box (no stretching).
            let a = uv.width() * tex.x / (uv.height() * tex.y);
            assert!((a - 3.0).abs() < 1e-3);
        }
        // Focus at the top: the crop starts at the top edge.
        let top = BackgroundPlacement { x: 0.5, y: 0.0, zoom: 1.0 }.uv(tex, target);
        assert!(top.min.y.abs() < 1e-5 && (top.width() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn zoom_shrinks_the_crop_around_the_focus() {
        let tex = vec2(1000.0, 1000.0);
        let target = vec2(500.0, 500.0);
        let uv = BackgroundPlacement { x: 0.5, y: 0.5, zoom: 2.0 }.uv(tex, target);
        assert!(close(uv, Rect::from_min_max(pos2(0.25, 0.25), pos2(0.75, 0.75))));
        // Near an edge the window slides back inside.
        let uv = BackgroundPlacement { x: 0.1, y: 0.95, zoom: 2.0 }.uv(tex, target);
        assert!(close(uv, Rect::from_min_max(pos2(0.0, 0.5), pos2(0.5, 1.0))));
    }

    #[test]
    fn pan_follows_the_pointer_and_stops_at_edges() {
        let tex = vec2(1920.0, 1080.0);
        let target = vec2(1920.0, 270.0); // shows a quarter of the height
        let p = BackgroundPlacement { x: 0.5, y: 0.5, zoom: 1.0 };
        assert_eq!(p.can_pan(tex, target), (false, true));
        // Dragging down by a whole box height shows the part above: a quarter of the picture.
        let down = p.panned(tex, target, vec2(0.0, 270.0));
        assert!((down.y - 0.25).abs() < 1e-4, "{down:?}");
        // Left/right can't move here: x keeps its value.
        let side = BackgroundPlacement { x: 0.3, ..p }.panned(tex, target, vec2(500.0, 0.0));
        assert_eq!(side.x, 0.3);
        // Far past the top edge: stops at the top (focus = half the window from the edge).
        let far = p.panned(tex, target, vec2(0.0, 10_000.0));
        assert!((far.y - 0.125).abs() < 1e-4, "{far:?}");
        // …so dragging back moves the picture at once (no build-up).
        let back = far.panned(tex, target, vec2(0.0, -27.0));
        assert!(back.y > far.y);
    }

    #[test]
    fn zoom_keeps_the_point_under_the_pointer() {
        let tex = vec2(1000.0, 1000.0);
        let target = vec2(400.0, 400.0);
        let p = BackgroundPlacement::default();
        let anchor = vec2(0.75, 0.25);
        let before = p.uv(tex, target);
        let z = p.zoomed(tex, target, anchor, 2.0);
        assert_eq!(z.zoom, 2.0);
        let after = z.uv(tex, target);
        let under = |r: Rect| r.min + anchor * r.size();
        assert!((under(before) - under(after)).length() < 1e-4);
        // Never below filling the box, never beyond the maximum.
        assert_eq!(p.zoomed(tex, target, anchor, 0.1).zoom, 1.0);
        assert_eq!(p.zoomed(tex, target, anchor, 100.0).zoom, BackgroundPlacement::MAX_ZOOM);
        // Zoomed in, both axes can move.
        assert_eq!(z.can_pan(tex, target), (true, true));
    }

    #[test]
    fn from_uv_round_trips_the_automatic_crop() {
        let tex = vec2(1920.0, 620.0);
        let target = vec2(700.0, 400.0); // narrower than the picture: left/right only
        let auto = aligned_uv(tex, target, vec2(0.5, 0.3));
        let p = BackgroundPlacement::from_uv(auto, tex, target, vec2(0.5, 0.3));
        assert!(close(p.uv(tex, target), auto));
        assert_eq!(p.y, 0.3); // the fixed axis keeps the automatic focus
        assert!((p.zoom - 1.0).abs() < 1e-4);
    }

    #[test]
    fn background_uv_uses_the_placement_or_the_automatic_crop() {
        let tex = vec2(1000.0, 2000.0);
        let target = vec2(1000.0, 500.0);
        let auto = background_uv(None, tex, target, vec2(0.5, 0.2));
        assert!(close(auto, aligned_uv(tex, target, vec2(0.5, 0.2))));
        let pl = BackgroundPlacement { x: 0.5, y: 0.9, zoom: 1.0 };
        assert!(close(background_uv(Some(pl), tex, target, vec2(0.5, 0.2)), pl.uv(tex, target)));
    }

    #[test]
    fn serde_is_lenient_and_skips_none() {
        #[derive(Serialize, Deserialize, Default, Debug)]
        #[serde(default)]
        struct Holder {
            #[serde(
                default,
                skip_serializing_if = "Option::is_none",
                deserialize_with = "deserialize_background_lenient"
            )]
            bg: Option<BackgroundPlacement>,
        }
        assert_eq!(serde_json::to_string(&Holder::default()).unwrap(), "{}");
        let h: Holder = serde_json::from_str(r#"{"bg":{"x":0.2,"y":0.8,"zoom":1.5}}"#).unwrap();
        assert_eq!(h.bg, Some(BackgroundPlacement { x: 0.2, y: 0.8, zoom: 1.5 }));
        // Missing fields default; out-of-range values are clamped; nonsense is dropped.
        let h: Holder = serde_json::from_str(r#"{"bg":{"y":7,"zoom":0.1}}"#).unwrap();
        assert_eq!(h.bg, Some(BackgroundPlacement { x: 0.5, y: 1.0, zoom: 1.0 }));
        let h: Holder = serde_json::from_str(r#"{"bg":"top"}"#).unwrap();
        assert_eq!(h.bg, None);
        let h: Holder = serde_json::from_str(r#"{"bg":null}"#).unwrap();
        assert_eq!(h.bg, None);
    }
}
