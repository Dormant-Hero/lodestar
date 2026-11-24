//! Placing elements: boxes in screen fractions, dragging and resizing with snapping.
//!
//! Every position here is a fraction of the view's frame (0..1 across and down), the units of
//! the theme format. A box is an `egui::Rect` in those units.

use crate::theme::Common;
use egui::{Pos2, Rect, Vec2, pos2, vec2};

/// The smallest box a resize leaves (a fraction of the screen).
pub const MIN_SIZE: f32 = 0.004;

/// The element's box. A size component of 0 is "automatic": `natural` (also fractions) gives
/// it, keeping the aspect ratio when `keep_aspect` (pictures), else the natural extent (text).
/// Mirrors the renderer's `elem_rect_mode`.
pub fn element_box(c: &Common, natural: Vec2, keep_aspect: bool) -> Rect {
    let mut w = c.size[0];
    let mut h = c.size[1];
    let nat = natural;
    if w <= 0.0 && h <= 0.0 {
        w = nat.x;
        h = nat.y;
    } else if w <= 0.0 {
        w = if !keep_aspect {
            nat.x
        } else if nat.y > 0.0 {
            h * nat.x / nat.y
        } else {
            0.0
        };
    } else if h <= 0.0 {
        h = if !keep_aspect {
            nat.y
        } else if nat.x > 0.0 {
            w * nat.y / nat.x
        } else {
            0.0
        };
    }
    let min = pos2(c.pos[0] - c.origin[0] * w, c.pos[1] - c.origin[1] * h);
    Rect::from_min_size(min, vec2(w.max(0.0), h.max(0.0)))
}

/// What a drag on the preview moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Move,
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

impl Handle {
    pub const CORNERS_AND_EDGES: [Handle; 8] =
        [Handle::NW, Handle::N, Handle::NE, Handle::E, Handle::SE, Handle::S, Handle::SW, Handle::W];

    /// Which edges it moves: (left, right, top, bottom).
    fn edges(self) -> (bool, bool, bool, bool) {
        match self {
            Handle::Move => (true, true, true, true),
            Handle::N => (false, false, true, false),
            Handle::S => (false, false, false, true),
            Handle::E => (false, true, false, false),
            Handle::W => (true, false, false, false),
            Handle::NE => (false, true, true, false),
            Handle::NW => (true, false, true, false),
            Handle::SE => (false, true, false, true),
            Handle::SW => (true, false, false, true),
        }
    }

    /// The handle's spot on a box.
    pub fn point(self, r: Rect) -> Pos2 {
        let (l, rt, t, b) = self.edges();
        let x = if l {
            r.left()
        } else if rt {
            r.right()
        } else {
            r.center().x
        };
        let y = if t {
            r.top()
        } else if b {
            r.bottom()
        } else {
            r.center().y
        };
        pos2(x, y)
    }

    /// Changes the width (false: only the height, or a move).
    pub fn sizes_x(self) -> bool {
        let (l, r, _, _) = self.edges();
        self != Handle::Move && (l || r)
    }

    pub fn sizes_y(self) -> bool {
        let (_, _, t, b) = self.edges();
        self != Handle::Move && (t || b)
    }

    pub fn cursor(self) -> egui::CursorIcon {
        use egui::CursorIcon as C;
        match self {
            Handle::Move => C::Grab,
            Handle::N | Handle::S => C::ResizeVertical,
            Handle::E | Handle::W => C::ResizeHorizontal,
            Handle::NE | Handle::SW => C::ResizeNeSw,
            Handle::NW | Handle::SE => C::ResizeNwSe,
        }
    }
}

/// Lines a dragged box snaps to (fractions), and how close counts.
#[derive(Clone, Debug, Default)]
pub struct Snap {
    pub xs: Vec<f32>,
    pub ys: Vec<f32>,
    /// Distance that snaps, per axis (a few pixels as a fraction).
    pub reach: Vec2,
}

impl Snap {
    /// The screen's edges and centre, and the edges and centres of `others`.
    pub fn new(others: &[Rect], reach: Vec2) -> Snap {
        let mut xs = vec![0.0, 0.5, 1.0];
        let mut ys = vec![0.0, 0.5, 1.0];
        for r in others {
            xs.extend([r.left(), r.center().x, r.right()]);
            ys.extend([r.top(), r.center().y, r.bottom()]);
        }
        Snap { xs, ys, reach }
    }
}

/// The lines a box snapped to (drawn as guides).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Guides {
    pub xs: Vec<f32>,
    pub ys: Vec<f32>,
}

/// Best shift (target − value) of any of `values` onto `targets` within `reach`.
fn snap_shift(values: &[f32], targets: &[f32], reach: f32) -> Option<(f32, f32)> {
    let mut best: Option<(f32, f32)> = None;
    for &v in values {
        for &t in targets {
            let d = t - v;
            if d.abs() <= reach && best.is_none_or(|(b, _)| d.abs() < b.abs()) {
                best = Some((d, t));
            }
        }
    }
    best
}

/// The box after dragging `handle` by `delta` from `start`, snapped (None: free).
pub fn drag(start: Rect, handle: Handle, delta: Vec2, snap: Option<&Snap>) -> (Rect, Guides) {
    let mut guides = Guides::default();
    let (l, r, t, b) = handle.edges();
    let mut out = start;
    if handle == Handle::Move {
        out = start.translate(delta);
        if let Some(s) = snap {
            if let Some((d, line)) = snap_shift(&[out.left(), out.center().x, out.right()], &s.xs, s.reach.x) {
                out = out.translate(vec2(d, 0.0));
                guides.xs.push(line);
            }
            if let Some((d, line)) = snap_shift(&[out.top(), out.center().y, out.bottom()], &s.ys, s.reach.y) {
                out = out.translate(vec2(0.0, d));
                guides.ys.push(line);
            }
        }
        return (out, guides);
    }
    let edge = |v: f32, d: f32, targets: Option<(&[f32], f32)>, lines: &mut Vec<f32>| -> f32 {
        let mut n = v + d;
        if let Some((ts, reach)) = targets
            && let Some((s, line)) = snap_shift(&[n], ts, reach)
        {
            n += s;
            lines.push(line);
        }
        n
    };
    let sx = snap.map(|s| (s.xs.as_slice(), s.reach.x));
    let sy = snap.map(|s| (s.ys.as_slice(), s.reach.y));
    if l {
        out.min.x = edge(start.min.x, delta.x, sx, &mut guides.xs).min(start.max.x - MIN_SIZE);
    }
    if r {
        out.max.x = edge(start.max.x, delta.x, sx, &mut guides.xs).max(start.min.x + MIN_SIZE);
    }
    if t {
        out.min.y = edge(start.min.y, delta.y, sy, &mut guides.ys).min(start.max.y - MIN_SIZE);
    }
    if b {
        out.max.y = edge(start.max.y, delta.y, sy, &mut guides.ys).max(start.min.y + MIN_SIZE);
    }
    (out, guides)
}

/// Round to 4 decimals (0.0001 of a 1920-pixel screen is a fifth of a pixel): tidy files.
pub fn tidy(v: f32) -> f32 {
    (v * 10_000.0).round() / 10_000.0
}

/// Write box `r` into the element: its `pos` (where its `origin` lands) and its `size`. An
/// automatic size (0) stays automatic unless the drag changed that side (`sized`).
pub fn apply_box(c: &mut Common, r: Rect, sized: [bool; 2]) {
    let keep = [c.size[0] <= 0.0 && !sized[0], c.size[1] <= 0.0 && !sized[1]];
    c.pos = [tidy(r.min.x + c.origin[0] * r.width()), tidy(r.min.y + c.origin[1] * r.height())];
    if !keep[0] {
        c.size[0] = tidy(r.width());
    }
    if !keep[1] {
        c.size[1] = tidy(r.height());
    }
}

/// Move by `step` (keyboard nudges).
pub fn nudge(c: &mut Common, step: Vec2) {
    c.pos = [tidy(c.pos[0] + step.x), tidy(c.pos[1] + step.y)];
}
