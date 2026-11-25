//! Panels of per-game texts that push each other around (a long name, a missing fact) →
//! Lodestar's `stack` container, so what follows moves as in Playnite.

use super::emit::{el_table, prompt_table, set_f, set_pair};
use super::*;

/// Whether flowing panels become `stack` containers: always, except in the placement tests,
/// which check absolute positions.
pub(super) fn enabled() -> bool {
    #[cfg(test)]
    return super::tests::STACKS.with(|f| f.get());
    #[cfg(not(test))]
    true
}

/// Children flow one after the other (StackPanel, or a DockPanel docking everything to the top).
pub(super) fn is_flow(n: &LNode) -> bool {
    match &n.panel {
        Panel::Stack { .. } => true,
        Panel::Dock { .. } => n.children.iter().filter(|c| c.vis != Vis::Collapsed).all(|c| c.dock == Dock::Top),
        _ => false,
    }
}

/// It holds a per-game part that can push what follows: a text (wrapping, or not the last part)
/// or a part shown only for some games.
pub(super) fn varies(n: &LNode) -> bool {
    let visible: Vec<&LNode> = n.children.iter().filter(|c| c.vis == Vis::Visible).collect();
    if visible.len() < 2 {
        return false;
    }
    // A part shown only for some games (Play for installed ones, Install for the others)
    // leaves its place to what follows.
    if visible[..visible.len() - 1].iter().any(|c| c.data.cond.visible_if.is_some() || c.data.cond.hidden_if.is_some())
    {
        return true;
    }
    let mut found = false;
    for (i, c) in visible.iter().enumerate() {
        layout::walk(c, &mut |k, _| {
            if let Kind::Text(t) = &k.data.kind
                && t.bound
                && (t.wrap || i + 1 < visible.len())
            {
                found = true;
            }
        });
    }
    found
}

/// A button whose template paints a band narrower than its box (`Face::band`): the band's height,
/// the rest of the box kept as margin above and below so the flow doesn't move.
fn narrow_to_band(t: &mut Table, n: &LNode, (top, h): (f32, f32)) {
    let (top, h) = (top * n.scale, h * n.scale);
    if h >= n.rect.h {
        return;
    }
    let mut m = [0.0f32; 4];
    if let Some(a) = t.get("margin").and_then(T::as_array) {
        for (i, v) in a.iter().take(4).enumerate() {
            m[i] = v.as_float().unwrap_or(0.0) as f32;
        }
    }
    m[1] += top / H;
    m[3] += (n.rect.h - top - h).max(0.0) / H;
    t.insert("margin".into(), T::Array(m.iter().map(|v| T::Float(round(*v) as f64)).collect()));
    set_pair(t, "size", [n.rect.w / W, h / H]);
}

pub(super) fn set_margin(t: &mut Table, n: &LNode) {
    let m = n.margin;
    if m.iter().any(|v| *v != 0.0) {
        t.insert("margin".into(), T::Array(m.iter().map(|v| T::Float(round(*v * n.scale / H) as f64)).collect()));
    }
}

impl<'b, 't, 'i> Builder<'b, 't, 'i> {
    /// `n` as a `stack` (None: something in it can't flow).
    /// `in_row`: it is a part of a horizontal stack.
    pub(super) fn stack_node(&mut self, n: &LNode, cx: &Ctx, root: bool, in_row: bool) -> Option<Table> {
        let horizontal = match &n.panel {
            Panel::Stack { horizontal } => *horizontal,
            _ => false,
        };
        // A column of one-line texts in a row of columns (a label over its value) with no
        // width of its own is as wide as its widest text, as WPF's StackPanel makes it: a
        // longer value widens the column instead of being cut. Its texts span the column.
        let hug = !horizontal
            && in_row
            && n.width.is_none()
            && n.min[0] <= 0.0
            && n.children
                .iter()
                .filter(|c| c.vis == Vis::Visible)
                .all(|c| matches!(&c.data.kind, Kind::Text(t) if !t.wrap));
        let mut kids: Vec<&LNode> = n.children.iter().collect();
        kids.sort_by_key(|k| k.z);
        let mut children = vec![];
        for c in kids {
            if let Some(t) = self.stack_child(c, cx, horizontal, hug)? {
                children.push(T::Table(t));
            }
        }
        if children.is_empty() {
            return None;
        }
        let r = n.rect;
        let mut t = Table::new();
        t.insert("type".into(), T::String("stack".into()));
        if horizontal {
            t.insert("direction".into(), T::String("horizontal".into()));
        }
        // Children sit where most of them sit across the flow.
        let across = n.children.iter().filter(|c| c.vis == Vis::Visible).map(|c| if horizontal { c.va } else { c.ha });
        let mut count = [0usize; 3];
        for a in across {
            count[match a {
                Al::Center => 1,
                Al::End => 2,
                _ => 0,
            }] += 1;
        }
        let align = if hug {
            "stretch"
        } else if count[1] > count[0].max(count[2]) {
            "center"
        } else if count[2] > count[0] {
            "end"
        } else {
            "start"
        };
        t.insert("align".into(), T::String(align.into()));
        if let Some(b) = &n.data.fill
            && let Some(v) = crate::theme::playnite::fill::brush_value(b)
        {
            t.insert("background".into(), v);
            if n.data.radius > 0.0 {
                set_f(&mut t, "corner_radius", (n.data.radius * n.scale / H).min(0.5));
            }
        }
        if let Some((c, w)) = n.data.stroke {
            t.insert("border_color".into(), T::String(c.to_hex()));
            set_f(&mut t, "border_width", (w * n.scale / H).max(0.0008));
        }
        let inset = [n.padding[0] + n.border[0], n.padding[1] + n.border[1]];
        if inset[0] > 0.0 || inset[1] > 0.0 {
            set_pair(&mut t, "padding", [inset[0] * n.scale / H, inset[1] * n.scale / H]);
        }
        if root {
            // Anchored where the panel is anchored, growing away from it.
            let (oy, y) = match n.va {
                Al::End => (1.0, r.y + r.h),
                Al::Center => (0.5, r.y + r.h / 2.0),
                _ => (0.0, r.y),
            };
            let (ox, x) = match (horizontal, n.ha) {
                (true, Al::End) => (1.0, r.x + r.w),
                (true, Al::Center) => (0.5, r.x + r.w / 2.0),
                _ => (0.0, r.x),
            };
            set_pair(&mut t, "pos", [x / W, y / H]);
            set_pair(&mut t, "origin", [ox, oy]);
            set_pair(&mut t, "size", [if horizontal { 0.0 } else { r.w / W }, 0.0]);
        } else {
            set_pair(&mut t, "size", [if horizontal || hug { 0.0 } else { r.w / W }, 0.0]);
            set_margin(&mut t, n);
        }
        t.insert("children".into(), T::Array(children));
        Some(t)
    }

    /// One child of a stack: Ok(None) = nothing to draw, Err = it can't flow. `hug`: the
    /// stack is a column as wide as its widest text (`stack_node`).
    pub(super) fn stack_child(&mut self, c: &LNode, cx: &Ctx, in_row: bool, hug: bool) -> Option<Option<Table>> {
        if c.vis != Vis::Visible || c.data.opacity <= 0.01 {
            return Some(None);
        }
        let deco = Deco { n: c, opacity: c.data.opacity, cond: &c.data.cond };
        let r = c.rect;
        let mut t = match &c.data.kind {
            Kind::Text(tx) => {
                let mut t = self.text_table(c, tx, cx, false);
                let w = if (in_row || hug) && !tx.wrap { 0.0 } else { r.w / W };
                if hug
                    && tx.align.is_none()
                    && tx.anchor != "left"
                    && let Some(T::Table(st)) = t.get_mut("style")
                {
                    // Spanning the column, it sits where WPF placed its box.
                    st.insert("align".into(), T::String(tx.anchor.into()));
                }
                // Wrapping text keeps its box (a description scrolls in what its viewer shows).
                let h = match (tx.wrap, cx.scroll) {
                    (true, Some(v)) => v.h / H,
                    (true, None) => r.h / H,
                    _ => 0.0,
                };
                set_pair(&mut t, "size", [w, h]);
                t
            }
            Kind::Prompt(b, color) => prompt_table(b, *color, r),
            Kind::Clock(_) | Kind::List | Kind::Mirror(_) | Kind::Help { .. } | Kind::Video(_) | Kind::Tabs(_) => {
                return None;
            }
            Kind::Drop(what) => {
                // Left out of the flow (counted like anywhere else).
                self.stats.drop(what, c.rect.intersect(&cx.clip).area());
                return Some(None);
            }
            Kind::Image(img) => {
                let mut t = el_table("image", r);
                t.insert("path".into(), T::String(img.path.clone()));
                t.insert("fit".into(), T::String(if img.fit.is_empty() { "contain" } else { img.fit }.into()));
                if !img.fallbacks.is_empty() {
                    t.insert(
                        "fallbacks".into(),
                        T::Array(img.fallbacks.iter().map(|f| T::String(f.clone())).collect()),
                    );
                }
                t
            }
            // One look per kind of game needs two backgrounds in place: not a flowing part.
            Kind::Button(bt) if bt.installed.is_some() => return None,
            Kind::Button(bt) => {
                self.stats.play_button |= bt.context;
                let mut t = Table::new();
                t.insert("type".into(), T::String("stack".into()));
                t.insert("align".into(), T::String("center".into()));
                if let Some(v) = bt.fill.as_ref().and_then(crate::theme::playnite::fill::brush_value) {
                    t.insert("background".into(), v);
                }
                if bt.radius > 0.0 {
                    set_f(&mut t, "corner_radius", (bt.radius * c.scale / H).min(0.5));
                }
                if let Some((col, w)) = bt.border {
                    t.insert("border_color".into(), T::String(col.to_hex()));
                    set_f(&mut t, "border_width", (w * c.scale / H).max(0.0008));
                }
                set_pair(&mut t, "size", [r.w / W, r.h / H]);
                set_pair(&mut t, "padding", [c.padding[0] / H, c.padding[1] / H]);
                let mut labels = vec![];
                for (label, cond) in self.button_labels(bt) {
                    let mut lt = bt.label.clone();
                    lt.text = label;
                    let mut tt = self.text_table_at(r, r, &lt, c.scale, cx.clip, false);
                    set_pair(&mut tt, "size", [0.0, 0.0]);
                    if let Some(v) = cond.visible_if {
                        tt.insert("visible_if".into(), T::String(v));
                    }
                    if let Some(h) = cond.hidden_if {
                        tt.insert("hidden_if".into(), T::String(h));
                    }
                    labels.push(T::Table(tt));
                }
                t.insert("children".into(), T::Array(labels));
                t
            }
            Kind::None if c.children.is_empty() => {
                let Some(fill) = &c.data.fill else { return Some(None) };
                let mut t = el_table("rect", r);
                if !crate::theme::playnite::fill::paint(&mut t, fill) {
                    return None;
                }
                t
            }
            Kind::None => {
                let flows = is_flow(c)
                    || matches!(c.panel, Panel::Single | Panel::Scroll { .. } | Panel::Grid { .. })
                        && c.children.iter().filter(|k| k.vis == Vis::Visible).count() <= 1;
                if !flows {
                    return None;
                }
                let sub = if matches!(c.panel, Panel::Scroll { .. }) {
                    Ctx { scroll: Some(r), ..cx.clone() }
                } else {
                    cx.clone()
                };
                match self.stack_node(c, &sub, false, in_row) {
                    // A plain wrapper (Grid / Border / ScrollViewer without paint) around one
                    // part: that part, with the wrapper's margin added to its own.
                    Some(mut t)
                        if !is_flow(c)
                            && c.data.fill.is_none()
                            && c.data.stroke.is_none()
                            && t.get("children").and_then(T::as_array).is_some_and(|a| a.len() == 1) =>
                    {
                        let Some(T::Table(mut only)) =
                            t.remove("children").and_then(|a| a.as_array().and_then(|a| a.first().cloned()))
                        else {
                            return None;
                        };
                        let own = |t: &Table| -> [f32; 4] {
                            let mut m = [0.0; 4];
                            if let Some(a) = t.get("margin").and_then(T::as_array) {
                                for (i, v) in a.iter().take(4).enumerate() {
                                    m[i] = v.as_float().unwrap_or(0.0) as f32;
                                }
                            }
                            m
                        };
                        let (a, b) = (own(&only), c.margin.map(|v| v * c.scale / H));
                        let sum: Vec<T> = (0..4).map(|i| T::Float(round(a[i] + b[i]) as f64)).collect();
                        if sum.iter().any(|v| v.as_float() != Some(0.0)) {
                            only.insert("margin".into(), T::Array(sum));
                        }
                        if let Some(v) = &c.data.cond.visible_if {
                            only.entry("visible_if").or_insert_with(|| T::String(v.clone()));
                        }
                        if let Some(h) = &c.data.cond.hidden_if {
                            only.entry("hidden_if").or_insert_with(|| T::String(h.clone()));
                        }
                        return Some(Some(only));
                    }
                    Some(t) => t,
                    None if c.children.iter().all(|k| k.vis != Vis::Visible) => return Some(None),
                    None => return None,
                }
            }
        };
        if !matches!(c.data.kind, Kind::None) || c.children.is_empty() {
            set_margin(&mut t, c);
        }
        if let Kind::Button(ButtonOut { band: Some(band), .. }) = &c.data.kind {
            narrow_to_band(&mut t, c, *band);
        }
        let idx = self.names;
        self.names += 1;
        self.decorate(&mut t, &deco, idx, "part");
        Some(Some(t))
    }
}
