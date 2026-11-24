//! Laid-out nodes → Lodestar element tables: paint, texts, images, the game list, the help
//! bar, buttons and the filter tabs.

use super::bind::{bind_art, parse_binding};
use super::stack::{is_flow, varies};
use super::*;

impl Default for Ctx {
    fn default() -> Self {
        Ctx { opacity: 1.0, clip: Rect::new(0.0, 0.0, W, H), cond: Cond::default(), scroll: None, row: None }
    }
}

impl Rect {
    pub(super) fn deflate_pub(&self, t: [f32; 4]) -> Rect {
        Rect::new(self.x + t[0], self.y + t[1], (self.w - t[0] - t[2]).max(0.0), (self.h - t[1] - t[3]).max(0.0))
    }
}

pub(super) fn tile_panel(r: Rect, aspect: f32) -> Tiles {
    let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.5 };
    let horizontal = matches!(setting("Fullscreen.HorizontalLayout"), Some(SVal::Bool(true)));
    let (w, h) = (r.w.max(1.0), r.h.max(1.0));
    if horizontal {
        let mut iw = w / (settings::COLUMNS + 0.5);
        let mut ih = iw * aspect;
        if ih > h {
            ih = h;
            iw = ih / aspect;
        }
        let rows = (h / ih).floor().max(1.0) as usize;
        Tiles { item: [iw, ih], rows, columns: (w / iw).floor().max(1.0) as usize }
    } else {
        let mut ih = h / (settings::ROWS + 0.5);
        let mut iw = ih / aspect;
        if iw > w {
            iw = w;
            ih = iw * aspect;
        }
        let columns = (w / iw).floor().max(1.0) as usize;
        Tiles { item: [iw, ih], rows: (h / ih).floor().max(1.0) as usize, columns }
    }
}

/// Everything under `n` that shows is something Lodestar can't draw (at least one such part,
/// nothing drawable).
pub(super) fn frames_dropped(n: &LNode) -> bool {
    fn scan(n: &LNode, drop: &mut bool, draws: &mut bool) {
        if n.vis != Vis::Visible || n.data.opacity <= 0.01 {
            return;
        }
        match &n.data.kind {
            Kind::Drop(_) => *drop = true,
            Kind::None => {}
            _ => *draws = true,
        }
        for c in &n.children {
            if c.data.fill.is_some() && matches!(c.data.kind, Kind::None) && c.children.is_empty() {
                continue;
            }
            scan(c, drop, draws);
        }
    }
    let (mut drop, mut draws) = (false, false);
    for c in &n.children {
        scan(c, &mut drop, &mut draws);
    }
    drop && !draws
}

pub(super) fn stretch_fit(s: Stretch) -> &'static str {
    match s {
        Stretch::UniformToFill => "cover",
        Stretch::Fill => "stretch",
        _ => "contain",
    }
}

pub(super) fn set_f(t: &mut Table, k: &str, v: f32) {
    t.insert(k.into(), T::Float(round(v) as f64));
}

pub(super) fn set_pair(t: &mut Table, k: &str, v: [f32; 2]) {
    t.insert(k.into(), T::Array(v.iter().map(|x| T::Float(round(*x) as f64)).collect()));
}

/// A help bar showing the screen's hints (not a theme's single prompt glyph).
pub(super) fn is_hint_bar(e: &Table) -> bool {
    e.get("type").and_then(T::as_str) == Some("helpbar") && e.get("glyphs").and_then(T::as_bool) != Some(true)
}

/// A theme's button prompt in `r`: one glyph, as tall as the box.
pub(super) fn prompt_table(button: &str, color: Option<Color>, r: Rect) -> Table {
    let mut t = el_table("helpbar", r);
    t.insert("buttons".into(), T::Array(vec![T::String(button.into())]));
    t.insert("glyphs".into(), T::Boolean(true));
    set_f(&mut t, "button_scale", 1.0);
    let mut st = Table::new();
    set_f(&mut st, "font_size", (r.h / H).clamp(0.004, 0.2));
    st.insert("align".into(), T::String("center".into()));
    if let Some(c) = color {
        st.insert("color".into(), T::String(c.to_hex()));
        t.insert("icon_color".into(), T::String(c.to_hex()));
    }
    t.insert("style".into(), T::Table(st));
    // No room of its own around the glyph (a bar pads its hints by default).
    t.insert("padding".into(), T::Array(vec![T::Float(0.0), T::Float(0.0)]));
    t
}

pub(super) fn el_table(kind: &str, r: Rect) -> Table {
    let mut t = Table::new();
    t.insert("type".into(), T::String(kind.into()));
    set_pair(&mut t, "pos", [r.x / W, r.y / H]);
    set_pair(&mut t, "size", [r.w.max(0.0) / W, r.h.max(0.0) / H]);
    t
}

/// Cut an element's box to the clip (scroll viewers, the screen).
pub(super) fn clip_box(t: &mut Table, r: Rect, clip: Rect) {
    let c = r.intersect(&clip);
    if c != r && c.area() > 0.0 && (c.w < r.w * 0.999 || c.h < r.h * 0.999) {
        // Pictures keep their own box (cropping would rescale them); only shapes are cut.
        if t.get("type").and_then(T::as_str) == Some("rect") {
            set_pair(t, "pos", [c.x / W, c.y / H]);
            set_pair(t, "size", [c.w / W, c.h / H]);
        }
    }
}

impl<'b, 't, 'i> Builder<'b, 't, 'i> {
    /// Write `n` and what's under it (by Panel.ZIndex) as elements onto `out`.
    pub(super) fn emit(&mut self, n: &LNode, cx: &Ctx, out: &mut Vec<Table>) {
        if n.vis != Vis::Visible {
            return;
        }
        let opacity = cx.opacity * n.data.opacity;
        if opacity <= 0.01 {
            return;
        }
        let cond = cx.cond.and(&n.data.cond);
        let r = n.rect;
        let area = r.intersect(&cx.clip).area();
        let deco = Deco { n, opacity, cond: &cond };
        // A panel of per-game texts that push each other around: Lodestar's `stack`, so a
        // long name or a missing fact moves what follows as in Playnite.
        if self.stacks && matches!(n.data.kind, Kind::None) && area > 0.0 && is_flow(n) && varies(n) {
            let saved = self.stats.clone();
            let sub = Ctx { opacity: 1.0, clip: cx.clip, cond: Cond::default(), scroll: cx.scroll, row: None };
            if let Some(mut t) = self.stack_node(n, &sub, true, false) {
                self.decorate(&mut t, &deco, out.len(), "stack");
                out.push(t);
                self.stats.area_drawn += area;
                self.stats.stacks += 1;
                return;
            }
            self.stats = saved;
        }
        // A frame around things Lodestar can't draw (an add-on's panel, a trailer's box) goes
        // with them.
        if (n.data.fill.is_some() || n.data.stroke.is_some()) && matches!(n.data.kind, Kind::None) && frames_dropped(n)
        {
            self.stats.drop("add-on panel", area);
            return;
        }
        if area > 0.0 && (n.data.fill.is_some() || n.data.stroke.is_some()) {
            self.emit_paint(n, &deco, cx, area, out);
        }
        if self.emit_kind(n, &deco, cx, area, out) {
            return;
        }
        // Children, by Panel.ZIndex (stable).
        let mut kids: Vec<&LNode> = n.children.iter().collect();
        kids.sort_by_key(|k| k.z);
        let clip = if n.data.clip { cx.clip.intersect(&r) } else { cx.clip };
        let row = if matches!(n.panel, Panel::Stack { horizontal: true })
            && n.children.iter().any(|c| matches!(c.data.kind, Kind::Help(_)))
        {
            Some(r)
        } else {
            cx.row
        };
        let scroll = if matches!(n.panel, Panel::Scroll { .. }) { Some(r.intersect(&cx.clip)) } else { cx.scroll };
        let sub = Ctx { opacity, clip, cond, row, scroll };
        for k in kids {
            self.emit(k, &sub, out);
        }
    }

    /// The node's background, fill and border: a theme picture, the game's art (an
    /// ImageBrush bound to it) or a rect.
    fn emit_paint(&mut self, n: &LNode, deco: &Deco, cx: &Ctx, area: f32, out: &mut Vec<Table>) {
        let r = n.rect;
        let opacity = deco.opacity;
        let radius = if n.data.radius < 0.0 { r.w.min(r.h) / 2.0 } else { n.data.radius * n.scale };
        match &n.data.fill {
            Some(LookBrush::Image { file: Some(f), stretch, opacity: o, .. }) => {
                let path = self.theme_image(f);
                let mut t = el_table("image", r);
                t.insert("path".into(), T::String(path));
                t.insert("fit".into(), T::String(stretch_fit(*stretch).into()));
                self.decorate(&mut t, deco, out.len(), "picture");
                if *o < 0.999 {
                    set_f(&mut t, "opacity", opacity * o);
                }
                clip_box(&mut t, r, cx.clip);
                out.push(t);
                self.stats.area_drawn += area;
            }
            // An ImageBrush bound to the game's art (a cover drawn as a Border's background).
            Some(LookBrush::Image { source, stretch, opacity: o, .. })
                if let Markup::Binding(raw) = tree::markup(source)
                    && let Some(art) = bind_art(&parse_binding(&raw)) =>
            {
                let mut t = el_table("image", r);
                t.insert("path".into(), T::String(art.into()));
                if art == "{art.hero}" {
                    t.insert("fallbacks".into(), T::Array(vec![T::String("{art.banner}".into())]));
                }
                t.insert("fit".into(), T::String(stretch_fit(*stretch).into()));
                if radius > 0.0 {
                    set_f(&mut t, "corner_radius", (radius / H).min(0.5));
                }
                set_f(&mut t, "fade", 0.25);
                self.decorate(&mut t, deco, out.len(), "picture");
                if *o < 0.999 {
                    set_f(&mut t, "opacity", opacity * o);
                }
                clip_box(&mut t, r, cx.clip);
                out.push(t);
                if let Some((c, w)) = n.data.stroke {
                    let mut b = el_table("rect", r);
                    b.insert("color".into(), T::String(Color::TRANSPARENT.to_hex()));
                    if radius > 0.0 {
                        set_f(&mut b, "corner_radius", (radius / H).min(0.5));
                    }
                    b.insert("border_color".into(), T::String(c.to_hex()));
                    set_f(&mut b, "border_width", (w * n.scale / H).max(0.0008));
                    self.decorate(&mut b, deco, out.len(), "box");
                    clip_box(&mut b, r, cx.clip);
                    out.push(b);
                }
                self.stats.area_drawn += area;
            }
            Some(LookBrush::Image { .. }) => self.stats.drop("picture bound to something Lodestar doesn't have", area),
            fill => {
                let mut t = el_table("rect", r);
                match fill {
                    Some(b) if crate::theme::playnite::fill::paint(&mut t, b) => {}
                    _ => {
                        t.insert("color".into(), T::String(Color::TRANSPARENT.to_hex()));
                    }
                }
                if radius > 0.0 {
                    set_f(&mut t, "corner_radius", (radius / H).min(0.5));
                }
                if let Some((c, w)) = n.data.stroke {
                    t.insert("border_color".into(), T::String(c.to_hex()));
                    set_f(&mut t, "border_width", (w * n.scale / H).max(0.0008));
                }
                self.decorate(&mut t, deco, out.len(), "box");
                clip_box(&mut t, r, cx.clip);
                out.push(t);
                self.stats.area_drawn += area;
            }
        }
    }

    /// What the node draws itself. True: its children aren't drawn (it's a leaf).
    fn emit_kind(&mut self, n: &LNode, deco: &Deco, cx: &Ctx, area: f32, out: &mut Vec<Table>) -> bool {
        let r = n.rect;
        match &n.data.kind {
            Kind::None => {}
            Kind::Drop(what) => {
                if area > 0.0 {
                    self.stats.drop(what, area);
                }
                return true;
            }
            Kind::Video => {
                if area > 0.0 {
                    let mut t = el_table("image", r);
                    t.insert("path".into(), T::String(String::new()));
                    t.insert("fit".into(), T::String("cover".into()));
                    t.insert("video".into(), T::String("@main_video".into()));
                    self.decorate(&mut t, deco, out.len(), "video");
                    out.push(t);
                }
                return true;
            }
            Kind::Text(t) | Kind::Clock(t) if area > 0.0 => {
                let clock = matches!(n.data.kind, Kind::Clock(_));
                let mut tbl = self.text_table(n, t, cx, clock);
                self.decorate(&mut tbl, deco, out.len(), if clock { "clock" } else { "text" });
                out.push(tbl);
                self.stats.area_drawn += area;
            }
            Kind::Image(img) if area > 0.0 => {
                let mut t = image_table(img, r);
                self.decorate(&mut t, deco, out.len(), "image");
                clip_box(&mut t, r, cx.clip);
                out.push(t);
                self.stats.area_drawn += area;
            }
            Kind::List => {
                if area > W * H * 0.01 {
                    self.stats.list = true;
                    let mut t = self.list_table(r);
                    self.decorate(&mut t, deco, out.len(), "list");
                    t.insert("name".into(), T::String("pn_list".into()));
                    out.push(t);
                    self.stats.area_drawn += area;
                }
                return true;
            }
            Kind::Tabs(tabs) if area > 0.0 => {
                let mut tbl = self.tabs_table(n, tabs);
                self.decorate(&mut tbl, deco, out.len(), "tabs");
                out.push(tbl);
                self.stats.area_drawn += area;
                return true;
            }
            Kind::Prompt(b, color) if area > 0.0 => {
                let mut tbl = prompt_table(b, *color, r);
                self.decorate(&mut tbl, deco, out.len(), "prompt");
                out.push(tbl);
                self.stats.area_drawn += area;
                return true;
            }
            Kind::Help(t) if area > 0.0 => {
                // One help bar for the row of bottom buttons (the first one places it).
                if !out.iter().any(is_hint_bar) {
                    let mut tbl = help_bar(t, cx.row.unwrap_or(r), n.scale);
                    self.decorate(&mut tbl, deco, out.len(), "help");
                    tbl.insert("name".into(), T::String("pn_help".into()));
                    out.push(tbl);
                }
                self.stats.area_drawn += area;
                return true;
            }
            Kind::Button(bt) if area > 0.0 => {
                self.emit_button(n, bt, deco, cx, out);
                self.stats.area_drawn += area;
                return true;
            }
            _ => {}
        }
        false
    }

    /// A button: its background, then its label(s).
    fn emit_button(&mut self, n: &LNode, bt: &ButtonOut, deco: &Deco, cx: &Ctx, out: &mut Vec<Table>) {
        let r = n.rect;
        let rad = (bt.radius * n.scale / H).min(0.5);
        let mut bg = el_table("rect", r);
        match &bt.fill {
            Some(b) if crate::theme::playnite::fill::paint(&mut bg, b) => {}
            _ => {
                let c = self
                    .t
                    .resource("ButtonBackgroundBrush")
                    .or_else(|| self.t.resource("ControlBackgroundBrush"))
                    .and_then(|r| self.t.eval_node(r, 0))
                    .and_then(|v| v.color())
                    .unwrap_or(Color([255, 255, 255, 40]));
                bg.insert("color".into(), T::String(c.to_hex()));
            }
        }
        if rad > 0.0 {
            set_f(&mut bg, "corner_radius", rad);
        }
        if let Some((c, w)) = bt.border {
            bg.insert("border_color".into(), T::String(c.to_hex()));
            set_f(&mut bg, "border_width", (w * n.scale / H).max(0.0008));
        }
        self.decorate(&mut bg, deco, out.len(), "button");
        out.push(bg);
        for (label, c) in self.button_labels(bt) {
            let mut lt = bt.label.clone();
            lt.text = label;
            let inner = Rect::new(r.x, r.y + (r.h - lt.size * 1.34 * n.scale) / 2.0, r.w, lt.size * 1.34 * n.scale);
            let mut tbl = self.text_table_at(inner, inner, &lt, n.scale, cx.clip, false);
            self.decorate(&mut tbl, deco, out.len(), "button_label");
            let both = deco.cond.and(&c);
            if let Some(v) = both.visible_if {
                tbl.insert("visible_if".into(), T::String(v));
            }
            if let Some(h) = both.hidden_if {
                tbl.insert("hidden_if".into(), T::String(h));
            }
            out.push(tbl);
        }
    }

    /// The label(s) of a button: Play / Install by the game's state for PART_ButtonContext.
    pub(super) fn button_labels(&self, bt: &ButtonOut) -> Vec<(String, Cond)> {
        if bt.context {
            let play = self.loc.text("LOCPlayGame").unwrap_or_else(|| "Play".into());
            let install = self.loc.text("LOCInstallGame").unwrap_or_else(|| "Install".into());
            vec![
                (play, Cond { visible_if: Some("{game.installed}".into()), hidden_if: None }),
                (install, Cond { visible_if: None, hidden_if: Some("{game.installed}".into()) }),
            ]
        } else {
            vec![(bt.label.text.clone(), Cond::default())]
        }
    }

    /// Name, opacity, show / hide rules and the appear transition of an element.
    pub(super) fn decorate(&mut self, t: &mut Table, d: &Deco, idx: usize, kind: &str) {
        let n = d.n;
        let label = if n.data.name.is_empty() { kind.to_string() } else { n.data.name.clone() };
        t.insert("name".into(), T::String(format!("pnc_{label}_{idx}")));
        if d.opacity < 0.999 {
            set_f(t, "opacity", d.opacity);
        }
        if let Some(v) = &d.cond.visible_if {
            t.insert("visible_if".into(), T::String(v.clone()));
        }
        if let Some(h) = &d.cond.hidden_if {
            t.insert("hidden_if".into(), T::String(h.clone()));
        }
        if let Some((ms, delay)) = n.data.appear_ms {
            let mut a = Table::new();
            set_f(&mut a, "opacity", 0.0);
            a.insert("duration_ms".into(), T::Integer(ms as i64));
            if delay > 0 {
                a.insert("delay_ms".into(), T::Integer(delay as i64));
            }
            a.insert("easing".into(), T::String("ease_out".into()));
            a.insert("replay".into(), T::String("select".into()));
            let mut tr = Table::new();
            tr.insert("appear".into(), T::Table(a));
            t.insert("transitions".into(), T::Table(tr));
        }
    }

    pub(super) fn text_table(&mut self, n: &LNode, t: &TextOut, cx: &Ctx, clock: bool) -> Table {
        let mut t = t.clone();
        let mut r = n.rect.deflate_pub(n.padding);
        if let Some(view) = cx.scroll.filter(|_| t.wrap) {
            // Text in a scroll viewer: the viewer's box, scrolling.
            r = Rect::new(r.x, r.y.max(view.y), r.w.min(view.w), (view.y + view.h - r.y.max(view.y)).max(0.0));
            t.scroll = true;
        }
        let mut tbl = self.text_table_at(r, n.slot, &t, n.scale, cx.clip, clock);
        if clock {
            tbl.insert("type".into(), T::String("datetime".into()));
            tbl.insert("clock".into(), T::Boolean(true));
            tbl.insert("format".into(), T::String("%H:%M".into()));
            tbl.remove("text");
            tbl.remove("max_lines");
            tbl.remove("wrap");
            tbl.remove("scroll");
        }
        tbl
    }

    /// A text element for `t` drawn in `r`; a per-game text may grow within `slot`.
    pub(super) fn text_table_at(&self, r: Rect, slot: Rect, t: &TextOut, scale: f32, clip: Rect, clock: bool) -> Table {
        let mut tbl = Table::new();
        tbl.insert("type".into(), T::String("text".into()));
        tbl.insert("text".into(), T::String(t.text.clone()));
        let px = t.size * scale;
        let mut align = t.align.unwrap_or("left");
        // Single-line text bound per game: the box spans what its slot allows, so a longer
        // name grows the way WPF would place it (left / centre / right of the slot).
        let mut b = r;
        if !t.wrap && (t.bound || clock) && slot.w > r.w + 1.0 {
            if t.align.is_none() {
                align = t.anchor;
            }
            let (lo, hi) = (slot.x.max(clip.x), (slot.x + slot.w).min(clip.x + clip.w));
            b = match align {
                "right" => Rect::new(lo, r.y, r.x + r.w - lo, r.h),
                "center" => {
                    let c = r.x + r.w / 2.0;
                    let half = (c - lo).min(hi - c).max(r.w / 2.0);
                    Rect::new(c - half, r.y, half * 2.0, r.h)
                }
                _ => Rect::new(r.x, r.y, hi - r.x, r.h),
            };
            if b.w < r.w {
                b = r;
            }
        }
        // Lodestar's glyphs run a little wider than WPF's for the same size: a few pixels of air.
        if !t.wrap && !t.bound {
            let extra = (px * 0.6).max(b.w * 0.08);
            b = match align {
                "right" => Rect::new(b.x - extra, b.y, b.w + extra, b.h),
                "center" => Rect::new(b.x - extra / 2.0, b.y, b.w + extra, b.h),
                _ => Rect::new(b.x, b.y, b.w + extra, b.h),
            };
        }
        set_pair(&mut tbl, "pos", [b.x / W, b.y / H]);
        let line = px * 1.344;
        if t.wrap {
            set_pair(&mut tbl, "size", [b.w.max(1.0) / W, b.h.max(line) / H]);
            tbl.insert("wrap".into(), T::Boolean(true));
            if t.scroll {
                tbl.insert("scroll".into(), T::Boolean(true));
            }
        } else {
            set_pair(&mut tbl, "size", [b.w.max(1.0) / W, 0.0]);
            if !clock {
                tbl.insert("max_lines".into(), T::Integer(1));
            }
        }
        let mut st = Table::new();
        set_f(&mut st, "font_size", (px / H).clamp(0.006, 0.2));
        if let Some(c) = t.color {
            st.insert("color".into(), T::String(c.to_hex()));
        }
        if let Some(f) = t.font {
            st.insert("font".into(), T::String(f.into()));
        }
        if align != "left" {
            st.insert("align".into(), T::String(align.into()));
        }
        if t.upper {
            st.insert("uppercase".into(), T::Boolean(true));
        }
        if let Some(s) = t.shadow {
            st.insert("shadow".into(), T::String(s.to_hex()));
        }
        tbl.insert("style".into(), T::Table(st));
        tbl
    }

    /// The filter preset tabs as a row of texts: each of `TABS`, plain or (when its collection
    /// is the open one) bold in the current colour, then the open collection's own name when
    /// it is none of them.
    pub(super) fn tabs_table(&self, n: &LNode, tabs: &TabsOut) -> Table {
        let r = n.rect.deflate_pub(n.padding);
        let key = "{collection.key}";
        let text = |label: &str, current: bool| -> T {
            let mut lt = tabs.label.clone();
            lt.text = label.to_string();
            lt.bound = false;
            lt.align = None;
            if current {
                lt.color = tabs.current.or(lt.color);
                lt.font = Some("bold");
            }
            let mut t = self.text_table_at(r, r, &lt, n.scale, Rect::new(0.0, 0.0, W, H), false);
            t.remove("pos");
            set_pair(&mut t, "size", [0.0, 0.0]);
            T::Table(t)
        };
        let mut children = vec![];
        for (k, label) in TABS {
            for current in [false, true] {
                let T::Table(mut t) = text(label, current) else { continue };
                let rule = format!("{key} == {k}");
                t.insert((if current { "visible_if" } else { "hidden_if" }).into(), T::String(rule));
                children.push(T::Table(t));
            }
        }
        if let T::Table(mut other) = text("{collection.name}", true) {
            let rule: Vec<String> = TABS.iter().map(|(k, _)| format!("{key} != {k}")).collect();
            other.insert("visible_if".into(), T::String(rule.join(" && ")));
            children.push(T::Table(other));
        }
        let mut t = Table::new();
        t.insert("type".into(), T::String("stack".into()));
        t.insert("direction".into(), T::String("horizontal".into()));
        t.insert("align".into(), T::String("center".into()));
        set_f(&mut t, "gap", tabs.label.size * n.scale * 1.2 / H);
        let (ox, x) = match n.ha {
            Al::End => (1.0, r.x + r.w),
            Al::Center => (0.5, r.x + r.w / 2.0),
            _ => (0.0, r.x),
        };
        let line = tabs.label.size * n.scale * 1.344;
        let y = (r.y + (r.h.min(line * 2.0) / 2.0)).min(r.y + r.h);
        set_pair(&mut t, "pos", [x / W, y / H]);
        set_pair(&mut t, "origin", [ox, 0.5]);
        set_pair(&mut t, "size", [0.0, 0.0]);
        t.insert("children".into(), T::Array(children));
        t
    }

    /// The game list at `r`: one row of covers (carousel) or a grid, shaped the way Playnite's
    /// FullscreenTilePanel fills the list's box at the default settings.
    pub(super) fn list_table(&self, r: Rect) -> Table {
        let item = &self.look.item;
        let spacing = item.spacing.map(|s| s[0] + s[2]).unwrap_or(settings::ITEM_SPACING).clamp(0.0, 80.0);
        let aspect = match item.size {
            Some([w, h]) if w > 0.0 && h > 0.0 => h / w,
            _ => 1.5,
        };
        let tiles = tile_panel(r, aspect);
        let mut t = Table::new();
        if tiles.rows <= 1 {
            // The theme's own item size when its slot holds it, else the slot's cover.
            let scale = item.focus_scale.unwrap_or(1.06).clamp(1.0, 1.5);
            let (iw, ih) = match item.size {
                Some([w, h]) if w > 0.0 && h > 0.0 && h <= r.h * 1.05 && w <= tiles.item[0] * 1.5 => (w, h),
                _ => {
                    let h = (tiles.item[1] - spacing).min(r.h / scale * 0.95).max(40.0);
                    (h / aspect, h)
                }
            };
            t.insert("type".into(), T::String("carousel".into()));
            t.insert("orientation".into(), T::String("horizontal".into()));
            set_pair(&mut t, "pos", [r.x / W, r.y / H]);
            set_pair(&mut t, "size", [r.w / W, r.h / H]);
            set_pair(&mut t, "item_size", [iw / W, ih / H]);
            let slot = iw + spacing;
            set_f(&mut t, "visible_items", (r.w / slot).max(1.0));
            // The selection starts at the left like Playnite's list (its first item's slot).
            set_f(&mut t, "selected_position", ((slot * 0.5) / r.w.max(1.0)).clamp(0.02, 0.5));
            set_f(&mut t, "item_align", 0.5);
            set_f(&mut t, "selected_scale", scale);
            set_f(&mut t, "unselected_opacity", 1.0);
            t.insert("item_fit".into(), T::String("cover".into()));
        } else {
            t.insert("type".into(), T::String("grid".into()));
            set_pair(&mut t, "pos", [r.x / W, r.y / H]);
            set_pair(&mut t, "size", [r.w / W, r.h / H]);
            t.insert("columns".into(), T::Integer(tiles.columns.clamp(2, 12) as i64));
            set_f(&mut t, "item_aspect", aspect);
            set_f(&mut t, "spacing", spacing / H);
            set_f(&mut t, "selected_scale", item.focus_scale.unwrap_or(1.06).clamp(1.0, 1.5));
            set_f(&mut t, "unselected_opacity", 1.0);
            t.insert("item_fit".into(), T::String("cover".into()));
        }
        t.insert("item_image".into(), T::String("{art.cover}".into()));
        t.insert("item_text".into(), T::String("{game.name}".into()));
        let mut st = Table::new();
        set_f(&mut st, "font_size", 0.0167);
        st.insert("align".into(), T::String("center".into()));
        t.insert("item_text_style".into(), T::Table(st));
        t.insert("controller_badge".into(), T::Boolean(true));
        t.insert("favorite_badge".into(), T::Boolean(true));
        if let Some(s) = item.focus_scale.filter(|s| (*s - 1.0).abs() > 0.005) {
            let mut f = Table::new();
            set_f(&mut f, "scale", s);
            f.insert("duration_ms".into(), T::Integer(100));
            f.insert("easing".into(), T::String("ease_out".into()));
            let mut tr = Table::new();
            tr.insert("focus".into(), T::Table(f));
            t.insert("transitions".into(), T::Table(tr));
        }
        t
    }
}

/// An image element for `img` at `r`.
fn image_table(img: &ImageOut, r: Rect) -> Table {
    let mut t = el_table("image", r);
    t.insert("path".into(), T::String(img.path.clone()));
    if !img.fallbacks.is_empty() {
        t.insert("fallbacks".into(), T::Array(img.fallbacks.iter().map(|f| T::String(f.clone())).collect()));
    }
    t.insert("fit".into(), T::String(if img.fit.is_empty() { "contain" } else { img.fit }.into()));
    if let Some(m) = img.mask {
        t.insert("mask".into(), T::Array(m.iter().map(|v| T::Float(round(*v) as f64)).collect()));
    }
    if img.blur > 0.5 {
        set_f(&mut t, "blur", (img.blur / H).clamp(0.0, 0.05));
    }
    if img.placeholder {
        t.insert("placeholder".into(), T::Boolean(true));
    }
    if img.path.starts_with('{') && !img.picture {
        // Per-game art: fade between games like the theme's FadeImage.
        set_f(&mut t, "fade", 0.25);
    }
    t
}

/// The help bar in place of the row of bottom buttons `row`, in the buttons' text look.
fn help_bar(t: &TextOut, row: Rect, scale: f32) -> Table {
    let mut tbl = el_table("helpbar", row);
    set_pair(&mut tbl, "pos", [row.x / W, (row.y + row.h / 2.0) / H]);
    set_pair(&mut tbl, "origin", [0.0, 0.5]);
    let mut st = Table::new();
    set_f(&mut st, "font_size", (t.size * scale / H).clamp(0.012, 0.04));
    if let Some(c) = t.color {
        st.insert("color".into(), T::String(c.to_hex()));
        tbl.insert("icon_color".into(), T::String(c.to_hex()));
    }
    tbl.insert("style".into(), T::Table(st));
    tbl
}
