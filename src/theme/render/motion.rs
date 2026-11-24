//! Transitions (`Common::transitions`): elements animating in / out (`appear`, `disappear`) and
//! list items animating to their focused look (`focus`, `unfocus`).
//!
//! Cheap by design: an element without transitions takes the old drawing path untouched; one
//! that animates draws as usual and its shapes are then moved / scaled in the paint list
//! (`transform_after_rounding_range`) with the painter's opacity lowered while it draws. Frames
//! are requested only while something animates (`Output::animating`).

use super::*;
use egui::emath::TSTransform;
use egui::layers::ShapeIdx;

/// Per-element appear / disappear bookkeeping.
#[derive(Default)]
pub(super) struct FxState {
    /// egui frame the element was last considered in (drawn or hidden).
    last_frame: u64,
    seen: bool,
    shown: bool,
    appear_t0: Option<f64>,
    hide_t0: Option<f64>,
    /// Selection the element was last drawn for (`Replay::Select`).
    sel: u64,
}

/// Focus animation of a list: the focused item and the one that just lost the focus.
#[derive(Default)]
pub(super) struct FocusAnim {
    init: bool,
    list: u64,
    cur: Option<u64>,
    prev: Option<u64>,
    /// How far the item that lost the focus had come when it lost it.
    prev_from: f32,
    t0: f64,
}

/// Focus amounts of a list's items this frame (0 = normal look, 1 = focused look).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FocusAmounts {
    pub cur: Option<(u64, f32)>,
    pub prev: Option<(u64, f32)>,
}

impl FocusAmounts {
    pub fn of(&self, id: u64) -> f32 {
        match (self.cur, self.prev) {
            (Some((c, a)), _) if c == id => a,
            (_, Some((p, a))) if p == id => a,
            _ => 0.0,
        }
    }
}

impl FocusAnim {
    /// Advance to `now` with the focus on `cur` (None = the list doesn't have the focus).
    /// The first frame, and a new list, show the focus at once.
    pub(super) fn step(
        &mut self,
        list: u64,
        cur: Option<u64>,
        now: f64,
        tr: &Transitions,
        instant: bool,
    ) -> (FocusAmounts, bool) {
        let (Some(fin), Some(fout)) = (tr.focus.as_ref(), tr.unfocus_timing()) else {
            return (FocusAmounts { cur: cur.map(|c| (c, 1.0)), prev: None }, false);
        };
        if !self.init || self.list != list {
            *self = FocusAnim { init: true, list, cur, prev: None, prev_from: 0.0, t0: f64::NEG_INFINITY };
        } else if self.cur != cur {
            // Where the outgoing item stands now (it may still have been animating in).
            let had = if self.t0.is_finite() { fin.progress(now - self.t0) } else { 1.0 };
            self.prev = self.cur;
            self.prev_from = if self.cur.is_some() { had } else { 0.0 };
            self.cur = cur;
            self.t0 = now;
        }
        let el = now - self.t0;
        if instant || !self.t0.is_finite() || (fin.done(el) && fout.done(el)) {
            self.prev = None;
            return (FocusAmounts { cur: self.cur.map(|c| (c, 1.0)), prev: None }, false);
        }
        let a_in = fin.progress(el);
        let a_out = self.prev_from * (1.0 - fout.progress(el));
        if fout.done(el) {
            self.prev = None;
        }
        (FocusAmounts { cur: self.cur.map(|c| (c, a_in)), prev: self.prev.map(|p| (p, a_out)) }, true)
    }
}

/// State key of child `i` of the element (or item template) with key `parent`.
pub(super) fn child_key(parent: usize, i: usize) -> usize {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (parent, i, 0x5eed_u32).hash(&mut h);
    (h.finish() as usize) | (1 << (usize::BITS - 1))
}

/// The selection a scene shows (for `Replay::Select`).
pub(super) fn selection_sig(scene: &Scene) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (scene.list_id, scene.selected, scene.extra.row).hash(&mut h);
    scene.game.map(|g| g.id).hash(&mut h);
    h.finish()
}

/// The desktop asks for no animations (KDE: Settings → Animation speed "Instant").
pub fn desktop_reduces_motion() -> bool {
    #[cfg(test)]
    {
        false
    }
    #[cfg(not(test))]
    {
        static ONCE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ONCE.get_or_init(|| {
            let dir = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| dirs::home_dir().map(|h| h.join(".config")));
            dir.and_then(|d| std::fs::read_to_string(d.join("kdeglobals")).ok())
                .is_some_and(|t| kde_instant_animations(&t))
        })
    }
}

/// `[KDE] AnimationDurationFactor=0` in kdeglobals.
pub(super) fn kde_instant_animations(text: &str) -> bool {
    let mut in_kde = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_kde = l == "[KDE]";
        } else if in_kde && let Some(v) = l.strip_prefix("AnimationDurationFactor=") {
            return v.trim().parse::<f32>().is_ok_and(|f| f <= 0.0);
        }
    }
    false
}

impl Renderer {
    /// Draw element `el` (state key `key`) with its visibility rules and appear / disappear
    /// transitions. `visible` = its `visible` / `visible_if` / `hidden_if` say it shows.
    pub(super) fn draw_fx(&mut self, f: &mut Frame, key: usize, el: &Element, focused: bool, visible: bool) {
        let tr = el.common().transitions.as_deref().filter(|t| t.appear.is_some() || t.disappear.is_some());
        let Some(tr) = tr else {
            if visible {
                self.draw_element(f, key, el, focused);
            }
            return;
        };
        match self.element_fx(f, key, tr, visible) {
            None => {}
            Some(fx) if fx.is_none() => self.draw_element(f, key, el, focused),
            Some(fx) => self.draw_transformed(f, fx, |r, f| r.draw_element(f, key, el, focused)),
        }
    }

    /// The transform of an element with appear / disappear transitions this frame (None = not
    /// drawn).
    fn element_fx(&mut self, f: &mut Frame, key: usize, tr: &Transitions, visible: bool) -> Option<Fx> {
        let now = f.now;
        let frame = f.frame;
        let instant = self.opts.reduce_motion;
        let sel = selection_sig(f.scene);
        let st = self.fx_states.entry((f.scene.kind, key)).or_default();
        // Not considered last frame: the view was just (re)opened.
        let fresh = !st.seen || st.last_frame + 1 < frame;
        st.seen = true;
        st.last_frame = frame;
        if visible {
            let replay = tr.appear.as_ref().is_some_and(|a| a.replay == Replay::Select) && st.sel != sel && !fresh;
            if tr.appear.is_some() && (fresh || !st.shown || replay) {
                st.appear_t0 = Some(now);
            }
            st.shown = true;
            st.hide_t0 = None;
            st.sel = sel;
            let (Some(a), Some(t0)) = (tr.appear.as_ref(), st.appear_t0) else {
                return Some(Fx::NONE);
            };
            let el = now - t0;
            if instant || a.done(el) {
                st.appear_t0 = None;
                return Some(Fx::NONE);
            }
            f.out.animating = true;
            Some(a.at(1.0 - a.progress(el)))
        } else {
            if st.shown && !fresh && tr.disappear.is_some() {
                st.hide_t0 = Some(now);
            }
            st.shown = false;
            st.appear_t0 = None;
            let (Some(d), Some(t0)) = (tr.disappear.as_ref(), st.hide_t0) else {
                return None;
            };
            let el = now - t0;
            if instant || d.done(el) {
                st.hide_t0 = None;
                return None;
            }
            f.out.animating = true;
            Some(d.at(d.progress(el)))
        }
    }

    /// Run `draw` with `fx` applied: the painter's opacity lowered, then the shapes it added
    /// scaled around their centre and moved.
    pub(super) fn draw_transformed(&mut self, f: &mut Frame, fx: Fx, draw: impl FnOnce(&mut Self, &mut Frame)) {
        if fx.opacity <= 0.0 {
            return;
        }
        let start = f.painter.add(egui::Shape::Noop);
        let saved = (fx.opacity < 1.0).then(|| {
            let s = f.painter.clone();
            f.painter.multiply_opacity(fx.opacity);
            s
        });
        draw(self, f);
        if let Some(s) = saved {
            f.painter = s;
        }
        if !fx.moves() {
            return;
        }
        let end = f.painter.add(egui::Shape::Noop);
        let layer = f.painter.layer_id();
        let shift = vec2(fx.translate[0] * f.sw(), fx.translate[1] * f.sh());
        let fallback = f.screen.center();
        f.painter.ctx().graphics_mut(|g| {
            let list = g.entry(layer);
            let bounds = list
                .all_entries()
                .skip(start.0)
                .take(end.0.saturating_sub(start.0))
                .fold(Rect::NOTHING, |r, c| r.union(c.shape.visual_bounding_rect()));
            let ts = fx_transform(if bounds.is_positive() { bounds.center() } else { fallback }, fx.scale, shift);
            list.transform_after_rounding_range(start, ShapeIdx(end.0), ts);
        });
    }
}

/// Scale by `s` around `c`, then move by `shift`.
pub(super) fn fx_transform(c: Pos2, s: f32, shift: Vec2) -> TSTransform {
    let s = if s.is_finite() { s.max(0.0) } else { 1.0 };
    TSTransform::new(c.to_vec2() * (1.0 - s) + shift, s)
}

/// An item rect at focus `amount` of `tr`'s focus look (scale around its centre, moved by
/// `translate`), and its opacity between `rest` and the focus opacity.
pub(super) fn focus_item(
    rect: Rect,
    rest_alpha: f32,
    focus_alpha: f32,
    scale: f32,
    translate: Vec2,
    amount: f32,
) -> (Rect, f32) {
    let a = amount.clamp(0.0, 1.0);
    let s = 1.0 + (scale - 1.0) * a;
    let r = Rect::from_center_size(rect.center() + translate * a, rect.size() * s);
    (r, rest_alpha + (focus_alpha - rest_alpha) * a)
}
