//! WPF measure / arrange: the layout rules Playnite's views are written for, at any size. The
//! Playnite view compiler (`playnite::compile`) runs it once per Fullscreen view, at import time.
//!
//! Covered: Grid (pixel / Auto / star rows and columns with Min / Max, spans), StackPanel,
//! DockPanel (LastChildFill), Canvas (Left / Top / Right / Bottom), Border-like single-child
//! hosts (padding, border thickness), Viewbox (Uniform / UniformToFill / Fill), ScrollViewer
//! (content measured unbounded along its scroll direction), WrapPanel, and on every element
//! Width / Height, Min / Max, Margin, Horizontal / VerticalAlignment, Visibility (Collapsed
//! takes no space, Hidden does), Panel.ZIndex and a static TranslateTransform. Leaves are text
//! (measured by a `Measurer`, Lodestar's font metrics), images (natural size + Stretch) and
//! fixed boxes. The rules follow FrameworkElement.MeasureCore / ArrangeCore: the desired size is
//! clamped to what was offered, an element larger than its slot keeps its size and overflows,
//! Stretch acts like Center when a Max / Width stops it filling the slot.
//!
//! Pure data in, rectangles out: the caller builds the tree, this lays it out, nothing runs.

const INF: f32 = f32::INFINITY;

/// A Grid row / column size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Len {
    Px(f32),
    Auto,
    Star(f32),
}

impl Len {
    /// `Auto`, `*`, `2*`, `120` (WPF GridLength text).
    pub fn parse(s: &str) -> Len {
        let t = s.trim();
        if t.eq_ignore_ascii_case("auto") {
            return Len::Auto;
        }
        if let Some(w) = t.strip_suffix('*') {
            let w = if w.trim().is_empty() { 1.0 } else { w.trim().parse::<f32>().unwrap_or(1.0) };
            return Len::Star(if w.is_finite() && w > 0.0 { w } else { 1.0 });
        }
        match t.parse::<f32>() {
            Ok(v) if v.is_finite() && v >= 0.0 => Len::Px(v),
            _ => Len::Star(1.0),
        }
    }
}

/// A RowDefinition / ColumnDefinition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Def {
    pub len: Len,
    pub min: f32,
    pub max: f32,
}

impl Def {
    pub fn new(len: Len) -> Def {
        Def { len, min: 0.0, max: INF }
    }
}

/// Alignment along one axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Al {
    #[default]
    Stretch,
    Start,
    Center,
    End,
}

impl Al {
    pub fn parse(s: &str) -> Option<Al> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "stretch" => Al::Stretch,
            "left" | "top" => Al::Start,
            "center" => Al::Center,
            "right" | "bottom" => Al::End,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Vis {
    #[default]
    Visible,
    Hidden,
    Collapsed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Dock {
    #[default]
    Left,
    Top,
    Right,
    Bottom,
}

/// Image / Viewbox Stretch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Scale {
    None,
    #[default]
    Uniform,
    UniformToFill,
    Fill,
}

impl Scale {
    pub fn parse(s: &str) -> Option<Scale> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "none" => Scale::None,
            "uniform" => Scale::Uniform,
            "uniformtofill" => Scale::UniformToFill,
            "fill" => Scale::Fill,
            _ => return None,
        })
    }
}

/// How an element arranges its children.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum Panel {
    /// Children overlap in the content box (Border, ContentControl, a Grid without
    /// definitions, anything unknown).
    #[default]
    Single,
    Grid {
        rows: Vec<Def>,
        cols: Vec<Def>,
    },
    Stack {
        horizontal: bool,
    },
    Dock {
        fill_last: bool,
    },
    Canvas,
    Wrap {
        horizontal: bool,
    },
    Viewbox {
        stretch: Scale,
    },
    Scroll {
        horizontal: bool,
        vertical: bool,
    },
}

/// Text to measure.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextSpec {
    pub text: String,
    /// px
    pub size: f32,
    pub bold: bool,
    pub wrap: bool,
    pub max_lines: Option<u32>,
}

/// What a leaf measures as.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum Content {
    #[default]
    None,
    Text(TextSpec),
    /// A picture of this natural size (None: no picture yet → 0 × 0, as WPF).
    Image {
        natural: Option<[f32; 2]>,
        stretch: Scale,
    },
    /// A box that wants this size when nothing constrains it (lists, controls Lodestar draws).
    Fixed([f32; 2]),
}

/// Text measurement (Lodestar's font metrics in the importer; a simple one in tests).
pub(crate) trait Measurer {
    /// The text's size when laid out in at most `max_w` px (INF: no wrapping).
    fn measure(&mut self, t: &TextSpec, max_w: f32) -> [f32; 2];
}

/// Rough metrics (average glyph width 0.52 em, line height 1.33 em) for tests.
#[cfg(test)]
pub(crate) struct Rough;

#[cfg(test)]
impl Measurer for Rough {
    fn measure(&mut self, t: &TextSpec, max_w: f32) -> [f32; 2] {
        let per = t.size * if t.bold { 0.56 } else { 0.52 };
        let one = t.text.chars().count() as f32 * per;
        let line_h = t.size * 1.33;
        if t.text.is_empty() {
            return [0.0, line_h];
        }
        if !t.wrap || !max_w.is_finite() || one <= max_w {
            return [one, line_h];
        }
        let per_line = (max_w / per).floor().max(1.0);
        let mut lines = (t.text.chars().count() as f32 / per_line).ceil().max(1.0);
        if let Some(m) = t.max_lines {
            lines = lines.min(m.max(1) as f32);
        }
        [max_w, lines * line_h]
    }
}

/// An axis-aligned rectangle (x, y, w, h), px.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn area(&self) -> f32 {
        self.w.max(0.0) * self.h.max(0.0)
    }

    /// The part inside `o` (zero size when apart).
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let y1 = (self.y + self.h).min(o.y + o.h);
        Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
    }

    fn deflate(&self, t: [f32; 4]) -> Rect {
        Rect::new(self.x + t[0], self.y + t[1], self.w - t[0] - t[2], self.h - t[1] - t[3])
    }
}

/// One element of the tree. `T` is the compiler's payload.
#[derive(Clone, Debug)]
pub(crate) struct Node<T> {
    pub panel: Panel,
    pub content: Content,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub min: [f32; 2],
    pub max: [f32; 2],
    /// left, top, right, bottom
    pub margin: [f32; 4],
    pub padding: [f32; 4],
    pub border: [f32; 4],
    pub ha: Al,
    pub va: Al,
    pub vis: Vis,
    pub z: i32,
    pub row: usize,
    pub col: usize,
    pub row_span: usize,
    pub col_span: usize,
    pub dock: Dock,
    /// Canvas.Left / Top / Right / Bottom
    pub canvas: [Option<f32>; 4],
    /// RenderTransform translation (px).
    pub translate: [f32; 2],
    pub children: Vec<Node<T>>,
    pub data: T,

    // Results.
    /// Desired size with margin, clamped to what was offered.
    pub desired: [f32; 2],
    /// Desired size without margin, before clamping to the offer.
    unclipped: [f32; 2],
    /// The element's box on the screen (after arrange, Viewbox scaling and translation).
    pub rect: Rect,
    /// The space the element was given (its slot minus its margin): where longer text of
    /// the same element would grow into.
    pub slot: Rect,
    /// Scale of the element's content (Viewboxes above it), 1 = as designed.
    pub scale: f32,
    /// Grid: the Auto column / row sizes found while measuring.
    grid_auto: (Vec<f32>, Vec<f32>),
}

impl<T: Default> Default for Node<T> {
    fn default() -> Self {
        Node::new(T::default())
    }
}

impl<T> Node<T> {
    pub fn new(data: T) -> Node<T> {
        Node {
            panel: Panel::Single,
            content: Content::None,
            width: None,
            height: None,
            min: [0.0, 0.0],
            max: [INF, INF],
            margin: [0.0; 4],
            padding: [0.0; 4],
            border: [0.0; 4],
            ha: Al::Stretch,
            va: Al::Stretch,
            vis: Vis::Visible,
            z: 0,
            row: 0,
            col: 0,
            row_span: 1,
            col_span: 1,
            dock: Dock::Left,
            canvas: [None; 4],
            translate: [0.0, 0.0],
            children: vec![],
            data,
            desired: [0.0, 0.0],
            unclipped: [0.0, 0.0],
            rect: Rect::default(),
            slot: Rect::default(),
            scale: 1.0,
            grid_auto: (vec![], vec![]),
        }
    }

    /// (min, max) for one axis after Width / Height (WPF MinMax).
    fn min_max(&self, axis: usize) -> (f32, f32) {
        let exp = if axis == 0 { self.width } else { self.height };
        let (mn, mx) = (self.min[axis].max(0.0), self.max[axis].max(0.0));
        let maxv = exp.unwrap_or(INF).min(mx).max(mn);
        let minv = exp.unwrap_or(0.0).min(maxv).max(mn);
        (minv, maxv)
    }

    fn inset(&self) -> [f32; 4] {
        let mut t = [0.0; 4];
        for (i, v) in t.iter_mut().enumerate() {
            *v = self.padding[i] + self.border[i];
        }
        t
    }
}

/// Lay out `root` in a `size` area at the origin.
pub(crate) fn layout<T>(root: &mut Node<T>, size: [f32; 2], m: &mut dyn Measurer) {
    measure(root, size, m);
    arrange(root, Rect::new(0.0, 0.0, size[0], size[1]), m);
}

/// FrameworkElement.MeasureCore.
pub(crate) fn measure<T>(n: &mut Node<T>, avail: [f32; 2], m: &mut dyn Measurer) {
    if n.vis == Vis::Collapsed {
        n.desired = [0.0, 0.0];
        n.unclipped = [0.0, 0.0];
        return;
    }
    let mw = n.margin[0] + n.margin[2];
    let mh = n.margin[1] + n.margin[3];
    let (minw, maxw) = n.min_max(0);
    let (minh, maxh) = n.min_max(1);
    let fa =
        [(avail[0] - mw).max(0.0).clamp(minw, maxw.max(minw)), (avail[1] - mh).max(0.0).clamp(minh, maxh.max(minh))];
    let d = measure_override(n, fa, m);
    let d = [d[0].max(minw).min(maxw), d[1].max(minh).min(maxh)];
    n.unclipped = d;
    let mut out = [(d[0] + mw).max(0.0), (d[1] + mh).max(0.0)];
    for (i, o) in out.iter_mut().enumerate() {
        if avail[i].is_finite() && *o > avail[i] {
            *o = avail[i].max(0.0);
        }
    }
    n.desired = out;
}

fn measure_override<T>(n: &mut Node<T>, c: [f32; 2], m: &mut dyn Measurer) -> [f32; 2] {
    let inset = n.inset();
    let iw = inset[0] + inset[2];
    let ih = inset[1] + inset[3];
    let inner = [(c[0] - iw).max(0.0), (c[1] - ih).max(0.0)];
    let content = match &n.content {
        Content::None => [0.0, 0.0],
        Content::Text(t) => {
            let s = m.measure(t, inner[0]);
            [s[0], s[1]]
        }
        Content::Image { natural, stretch } => image_size(*natural, *stretch, inner),
        Content::Fixed(sz) => [sz[0].min(inner[0]), sz[1].min(inner[1])],
    };
    let kids = match n.panel.clone() {
        Panel::Single => {
            let mut d: [f32; 2] = [0.0, 0.0];
            for ch in n.children.iter_mut() {
                measure(ch, inner, m);
                d = [d[0].max(ch.desired[0]), d[1].max(ch.desired[1])];
            }
            d
        }
        Panel::Scroll { horizontal, vertical } => {
            let offer = [if horizontal { INF } else { inner[0] }, if vertical { INF } else { inner[1] }];
            let mut d: [f32; 2] = [0.0, 0.0];
            for ch in n.children.iter_mut() {
                measure(ch, offer, m);
                d = [d[0].max(ch.desired[0]), d[1].max(ch.desired[1])];
            }
            [d[0].min(inner[0]), d[1].min(inner[1])]
        }
        Panel::Stack { horizontal } => {
            let along = if horizontal { 0 } else { 1 };
            let a = 1 - along; // the axis across
            let mut offer = inner;
            offer[along] = INF;
            let mut d = [0.0f32; 2];
            for ch in n.children.iter_mut() {
                measure(ch, offer, m);
                d[along] += ch.desired[along];
                d[a] = d[a].max(ch.desired[a]);
            }
            d
        }
        Panel::Wrap { horizontal } => {
            let along = if horizontal { 0 } else { 1 };
            let a = 1 - along;
            let mut offer = inner;
            offer[along] = INF;
            let (mut line, mut line_a, mut tot_a, mut max_along) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for ch in n.children.iter_mut() {
                measure(ch, [inner[0], inner[1]], m);
                let s = ch.desired;
                if line > 0.0 && line + s[along] > inner[along] {
                    tot_a += line_a;
                    max_along = max_along.max(line);
                    line = 0.0;
                    line_a = 0.0;
                }
                line += s[along];
                line_a = line_a.max(s[a]);
            }
            let _ = offer;
            tot_a += line_a;
            max_along = max_along.max(line);
            let mut d = [0.0; 2];
            d[along] = max_along;
            d[a] = tot_a;
            d
        }
        Panel::Dock { .. } => {
            let (mut pw, mut ph, mut aw, mut ah) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for ch in n.children.iter_mut() {
                let offer = [(inner[0] - aw).max(0.0), (inner[1] - ah).max(0.0)];
                measure(ch, offer, m);
                let d = ch.desired;
                match ch.dock {
                    Dock::Left | Dock::Right => {
                        ph = ph.max(ah + d[1]);
                        aw += d[0];
                    }
                    Dock::Top | Dock::Bottom => {
                        pw = pw.max(aw + d[0]);
                        ah += d[1];
                    }
                }
            }
            [pw.max(aw), ph.max(ah)]
        }
        Panel::Canvas => {
            for ch in n.children.iter_mut() {
                measure(ch, [INF, INF], m);
            }
            [0.0, 0.0]
        }
        Panel::Viewbox { stretch } => {
            let mut d: [f32; 2] = [0.0, 0.0];
            for ch in n.children.iter_mut() {
                measure(ch, [INF, INF], m);
                d = [d[0].max(ch.desired[0]), d[1].max(ch.desired[1])];
            }
            let s = viewbox_scale(d, inner, stretch);
            [d[0] * s[0], d[1] * s[1]]
        }
        Panel::Grid { rows, cols } => grid_measure(n, &rows, &cols, inner, m),
    };
    [content[0].max(kids[0]) + iw, content[1].max(kids[1]) + ih]
}

/// An image's size in `avail` (WPF Image.MeasureOverride / ArrangeOverride).
fn image_size(natural: Option<[f32; 2]>, stretch: Scale, avail: [f32; 2]) -> [f32; 2] {
    let Some([nw, nh]) = natural.filter(|n| n[0] > 0.0 && n[1] > 0.0) else {
        return [0.0, 0.0];
    };
    let sx = avail[0] / nw;
    let sy = avail[1] / nh;
    let (fx, fy) = (avail[0].is_finite(), avail[1].is_finite());
    let s = match stretch {
        Scale::None => return [nw, nh],
        Scale::Uniform => match (fx, fy) {
            (true, true) => sx.min(sy),
            (true, false) => sx,
            (false, true) => sy,
            _ => 1.0,
        },
        Scale::UniformToFill => match (fx, fy) {
            (true, true) => sx.max(sy),
            (true, false) => sx,
            (false, true) => sy,
            _ => 1.0,
        },
        Scale::Fill => {
            return [if fx { avail[0] } else { nw }, if fy { avail[1] } else { nh }];
        }
    };
    let out = [nw * s, nh * s];
    if stretch == Scale::UniformToFill {
        return [out[0].min(if fx { avail[0] } else { INF }), out[1].min(if fy { avail[1] } else { INF })];
    }
    out
}

fn viewbox_scale(d: [f32; 2], avail: [f32; 2], stretch: Scale) -> [f32; 2] {
    if d[0] <= 0.0 || d[1] <= 0.0 {
        return [1.0, 1.0];
    }
    let sx = if avail[0].is_finite() { avail[0] / d[0] } else { INF };
    let sy = if avail[1].is_finite() { avail[1] / d[1] } else { INF };
    let pick = |a: f32, b: f32, uniform_fill: bool| {
        let v = match (a.is_finite(), b.is_finite()) {
            (true, true) => {
                if uniform_fill {
                    a.max(b)
                } else {
                    a.min(b)
                }
            }
            (true, false) => a,
            (false, true) => b,
            _ => 1.0,
        };
        [v, v]
    };
    match stretch {
        Scale::None => [1.0, 1.0],
        Scale::Uniform => pick(sx, sy, false),
        Scale::UniformToFill => pick(sx, sy, true),
        Scale::Fill => [if sx.is_finite() { sx } else { 1.0 }, if sy.is_finite() { sy } else { 1.0 }],
    }
}

// ------------------------------------------------------------------------------------ Grid

fn defs_or_one(d: &[Def]) -> Vec<Def> {
    if d.is_empty() { vec![Def::new(Len::Star(1.0))] } else { d.to_vec() }
}

fn span(i: usize, n: usize, len: usize) -> std::ops::Range<usize> {
    let a = i.min(len - 1);
    let b = (a + n.max(1)).min(len);
    a..b
}

/// Sizes of definitions for `avail` px: pixel and Auto (from `auto`) first, then stars share
/// what is left in proportion to their weight, within their Min / Max.
fn resolve(defs: &[Def], auto: &[f32], star_content: &[f32], avail: f32) -> Vec<f32> {
    let mut out = vec![0.0f32; defs.len()];
    let mut fixed = 0.0;
    for (i, d) in defs.iter().enumerate() {
        out[i] = match d.len {
            Len::Px(p) => p.clamp(d.min, d.max.max(d.min)),
            Len::Auto => auto[i].clamp(d.min, d.max.max(d.min)),
            Len::Star(_) => 0.0,
        };
        if !matches!(d.len, Len::Star(_)) {
            fixed += out[i];
        }
    }
    let stars: Vec<usize> = (0..defs.len()).filter(|i| matches!(defs[*i].len, Len::Star(_))).collect();
    if stars.is_empty() {
        return out;
    }
    if !avail.is_finite() {
        // Unbounded (or a desired size): stars take what their content needs, as WPF's
        // definition minimum sizes.
        for i in stars {
            out[i] = star_content[i].clamp(defs[i].min, defs[i].max.max(defs[i].min));
        }
        return out;
    }
    let mut left = (avail - fixed).max(0.0);
    let mut open: Vec<usize> = stars;
    // Min / Max: fix the ones that hit a limit, share the rest again.
    for _ in 0..defs.len() + 1 {
        let total: f32 = open.iter().map(|i| weight(&defs[*i])).sum();
        if total <= 0.0 {
            break;
        }
        let mut changed = false;
        let mut next = vec![];
        for &i in &open {
            let share = left * weight(&defs[i]) / total;
            let d = &defs[i];
            if share < d.min {
                out[i] = d.min;
                left -= d.min;
                changed = true;
            } else if share > d.max {
                out[i] = d.max;
                left -= d.max;
                changed = true;
            } else {
                next.push(i);
            }
        }
        if !changed {
            for &i in &next {
                out[i] = left * weight(&defs[i]) / total;
            }
            break;
        }
        left = left.max(0.0);
        open = next;
        if open.is_empty() {
            break;
        }
    }
    out
}

fn weight(d: &Def) -> f32 {
    match d.len {
        Len::Star(w) => w,
        _ => 1.0,
    }
}

fn spans_star(defs: &[Def], r: std::ops::Range<usize>) -> bool {
    defs[r].iter().any(|d| matches!(d.len, Len::Star(_)))
}

fn spans_auto(defs: &[Def], r: std::ops::Range<usize>) -> bool {
    defs[r].iter().any(|d| d.len == Len::Auto)
}

/// Grow the Auto (else star) definitions in `r` so they hold `need` px between them.
fn grow(sizes: &mut [f32], defs: &[Def], r: std::ops::Range<usize>, need: f32, star_too: bool) {
    let have: f32 = sizes[r.clone()].iter().sum();
    if need <= have {
        return;
    }
    let targets: Vec<usize> =
        r.clone().filter(|i| defs[*i].len == Len::Auto || (star_too && matches!(defs[*i].len, Len::Star(_)))).collect();
    if targets.is_empty() {
        return;
    }
    let add = (need - have) / targets.len() as f32;
    for i in targets {
        sizes[i] += add;
    }
}

/// What star definitions need for their content (WPF's definition MinSize): single-span
/// children first, then spanning ones add only what the range still lacks.
fn star_mins(defs: &[Def], ents: &[(std::ops::Range<usize>, f32)], auto: &[f32]) -> Vec<f32> {
    let mut mins = vec![0.0f32; defs.len()];
    let is_star = |i: usize| matches!(defs[i].len, Len::Star(_));
    for (r, d) in ents.iter().filter(|(r, _)| r.len() == 1) {
        if is_star(r.start) {
            mins[r.start] = mins[r.start].max(*d);
        }
    }
    let mut multi: Vec<&(std::ops::Range<usize>, f32)> = ents.iter().filter(|(r, _)| r.len() > 1).collect();
    multi.sort_by_key(|(r, _)| r.len());
    for (r, d) in multi {
        let have: f32 = r
            .clone()
            .map(|i| match defs[i].len {
                Len::Star(_) => mins[i],
                Len::Px(p) => p,
                Len::Auto => auto[i],
            })
            .sum();
        let stars: Vec<usize> = r.clone().filter(|i| is_star(*i)).collect();
        if *d > have && !stars.is_empty() {
            let add = (*d - have) / stars.len() as f32;
            for i in stars {
                mins[i] += add;
            }
        }
    }
    mins
}

fn grid_measure<T>(n: &mut Node<T>, rows: &[Def], cols: &[Def], c: [f32; 2], m: &mut dyn Measurer) -> [f32; 2] {
    let rows = defs_or_one(rows);
    let cols = defs_or_one(cols);
    let (nr, nc) = (rows.len(), cols.len());
    let mut auto_c = vec![0.0f32; nc];
    let mut auto_r = vec![0.0f32; nr];
    let star_c = vec![0.0f32; nc];
    let star_r = vec![0.0f32; nr];
    // (span, desired) of every child, for the stars' content sizes.
    let mut ent_c: Vec<(std::ops::Range<usize>, f32)> = vec![];
    let mut ent_r: Vec<(std::ops::Range<usize>, f32)> = vec![];
    // Star definitions in an unbounded direction behave like Auto.
    let star_is_auto = [!c[0].is_finite(), !c[1].is_finite()];
    let px = |d: &[Def], r: std::ops::Range<usize>| -> Option<f32> {
        let mut s = 0.0;
        for x in &d[r] {
            match x.len {
                Len::Px(p) => s += p.clamp(x.min, x.max.max(x.min)),
                _ => return None,
            }
        }
        Some(s)
    };
    // Pass 1: children in cells without stars (in a bounded direction): offer pixel sizes or
    // unbounded.
    let mut later = vec![];
    for (k, ch) in n.children.iter_mut().enumerate() {
        let cr = span(ch.col, ch.col_span, nc);
        let rr = span(ch.row, ch.row_span, nr);
        let star_col = spans_star(&cols, cr.clone()) && !star_is_auto[0];
        let star_row = spans_star(&rows, rr.clone()) && !star_is_auto[1];
        if star_col || star_row {
            later.push(k);
            continue;
        }
        let ow = px(&cols, cr.clone()).unwrap_or(INF);
        let oh = px(&rows, rr.clone()).unwrap_or(INF);
        measure(ch, [ow, oh], m);
        grow(&mut auto_c, &cols, cr.clone(), ch.desired[0], false);
        grow(&mut auto_r, &rows, rr.clone(), ch.desired[1], false);
        ent_c.push((cr, ch.desired[0]));
        ent_r.push((rr, ch.desired[1]));
    }
    // Pass 2: stars share what is left; children in star cells get their share.
    let mut wc = resolve(&cols, &auto_c, &star_c, c[0]);
    let mut wr = resolve(&rows, &auto_r, &star_r, c[1]);
    for k in later {
        let ch = &mut n.children[k];
        let cr = span(ch.col, ch.col_span, nc);
        let rr = span(ch.row, ch.row_span, nr);
        // In an unbounded direction stars are Auto: the child gets all it wants (a Grid
        // without row definitions in a vertical StackPanel sizes to its content).
        let ow = if star_is_auto[0] {
            px(&cols, cr.clone()).unwrap_or(INF)
        } else if spans_auto(&cols, cr.clone()) && !spans_star(&cols, cr.clone()) {
            INF
        } else {
            wc[cr.clone()].iter().sum()
        };
        let oh = if star_is_auto[1] {
            px(&rows, rr.clone()).unwrap_or(INF)
        } else if spans_auto(&rows, rr.clone()) && !spans_star(&rows, rr.clone()) {
            INF
        } else {
            wr[rr.clone()].iter().sum()
        };
        measure(ch, [ow, oh], m);
        let before_c = auto_c.clone();
        let before_r = auto_r.clone();
        grow(
            &mut auto_c,
            &cols,
            cr.clone(),
            ch.desired[0]
                - wc[cr.clone()]
                    .iter()
                    .zip(&cols[cr.clone()])
                    .filter(|(_, d)| d.len != Len::Auto)
                    .map(|(w, _)| *w)
                    .sum::<f32>(),
            false,
        );
        grow(
            &mut auto_r,
            &rows,
            rr.clone(),
            ch.desired[1]
                - wr[rr.clone()]
                    .iter()
                    .zip(&rows[rr.clone()])
                    .filter(|(_, d)| d.len != Len::Auto)
                    .map(|(w, _)| *w)
                    .sum::<f32>(),
            false,
        );
        ent_c.push((cr, ch.desired[0]));
        ent_r.push((rr, ch.desired[1]));
        if before_c != auto_c || before_r != auto_r {
            wc = resolve(&cols, &auto_c, &star_c, c[0]);
            wr = resolve(&rows, &auto_r, &star_r, c[1]);
        }
    }
    // Desired: definitions at their content size (stars at what their content needs).
    let dc = resolve(&cols, &auto_c, &star_mins(&cols, &ent_c, &auto_c), INF);
    let dr = resolve(&rows, &auto_r, &star_mins(&rows, &ent_r, &auto_r), INF);
    let w: f32 = dc.iter().sum();
    let h: f32 = dr.iter().sum();
    // Remember the Auto sizes for arrange.
    n.grid_auto = (auto_c, auto_r);
    [w, h]
}

// ------------------------------------------------------------------------------------ arrange

/// FrameworkElement.ArrangeCore into `slot`.
pub(crate) fn arrange<T>(n: &mut Node<T>, slot: Rect, m: &mut dyn Measurer) {
    if n.vis == Vis::Collapsed {
        n.rect = Rect::new(slot.x, slot.y, 0.0, 0.0);
        for ch in n.children.iter_mut() {
            collapse(ch, slot.x, slot.y);
        }
        return;
    }
    let client = slot.deflate(n.margin);
    n.slot = Rect::new(client.x + n.translate[0], client.y + n.translate[1], client.w, client.h);
    let mut size = [client.w.max(0.0), client.h.max(0.0)];
    let aligns = [n.ha, n.va];
    for i in 0..2 {
        if size[i] < n.unclipped[i] || aligns[i] != Al::Stretch {
            size[i] = n.unclipped[i];
        }
        let (mn, mx) = n.min_max(i);
        size[i] = size[i].clamp(mn, mx.max(mn));
    }
    let cl = [client.w, client.h];
    let mut off = [0.0f32; 2];
    for i in 0..2 {
        let mut a = aligns[i];
        if a == Al::Stretch && size[i] > cl[i] {
            a = Al::Start;
        }
        off[i] = match a {
            Al::Start => 0.0,
            Al::Stretch | Al::Center => (cl[i] - size[i]) / 2.0,
            Al::End => cl[i] - size[i],
        };
    }
    let r = Rect::new(client.x + off[0] + n.translate[0], client.y + off[1] + n.translate[1], size[0], size[1]);
    n.rect = r;
    arrange_override(n, r, m);
}

fn collapse<T>(n: &mut Node<T>, x: f32, y: f32) {
    n.rect = Rect::new(x, y, 0.0, 0.0);
    for ch in n.children.iter_mut() {
        collapse(ch, x, y);
    }
}

fn arrange_override<T>(n: &mut Node<T>, r: Rect, m: &mut dyn Measurer) {
    let inner = r.deflate(n.inset());
    match n.panel.clone() {
        Panel::Single => {
            for ch in n.children.iter_mut() {
                arrange(ch, inner, m);
            }
        }
        Panel::Scroll { horizontal, vertical } => {
            for ch in n.children.iter_mut() {
                let w = if horizontal { inner.w.max(ch.desired[0]) } else { inner.w };
                let h = if vertical { inner.h.max(ch.desired[1]) } else { inner.h };
                arrange(ch, Rect::new(inner.x, inner.y, w, h), m);
            }
        }
        Panel::Stack { horizontal } => {
            let mut at = if horizontal { inner.x } else { inner.y };
            for ch in n.children.iter_mut() {
                let slot = if horizontal {
                    Rect::new(at, inner.y, ch.desired[0], inner.h)
                } else {
                    Rect::new(inner.x, at, inner.w, ch.desired[1])
                };
                at += if horizontal { ch.desired[0] } else { ch.desired[1] };
                arrange(ch, slot, m);
            }
        }
        Panel::Wrap { horizontal } => {
            let along = if horizontal { 0 } else { 1 };
            let (mut x, mut y, mut line_a) = (0.0f32, 0.0f32, 0.0f32);
            let lim = if horizontal { inner.w } else { inner.h };
            for ch in n.children.iter_mut() {
                let s = ch.desired;
                if x > 0.0 && x + s[along] > lim {
                    y += line_a;
                    x = 0.0;
                    line_a = 0.0;
                }
                let slot = if horizontal {
                    Rect::new(inner.x + x, inner.y + y, s[0], s[1])
                } else {
                    Rect::new(inner.x + y, inner.y + x, s[0], s[1])
                };
                x += s[along];
                line_a = line_a.max(s[1 - along]);
                arrange(ch, slot, m);
            }
        }
        Panel::Dock { fill_last } => {
            let mut rem = inner;
            let count = n.children.len();
            for (i, ch) in n.children.iter_mut().enumerate() {
                let d = ch.desired;
                if fill_last && i + 1 == count {
                    arrange(ch, rem, m);
                    break;
                }
                let slot = match ch.dock {
                    Dock::Left => {
                        let w = d[0].min(rem.w.max(0.0));
                        let s = Rect::new(rem.x, rem.y, w, rem.h);
                        rem.x += w;
                        rem.w -= w;
                        s
                    }
                    Dock::Right => {
                        let w = d[0].min(rem.w.max(0.0));
                        let s = Rect::new(rem.x + rem.w - w, rem.y, w, rem.h);
                        rem.w -= w;
                        s
                    }
                    Dock::Top => {
                        let h = d[1].min(rem.h.max(0.0));
                        let s = Rect::new(rem.x, rem.y, rem.w, h);
                        rem.y += h;
                        rem.h -= h;
                        s
                    }
                    Dock::Bottom => {
                        let h = d[1].min(rem.h.max(0.0));
                        let s = Rect::new(rem.x, rem.y + rem.h - h, rem.w, h);
                        rem.h -= h;
                        s
                    }
                };
                arrange(ch, slot, m);
            }
        }
        Panel::Canvas => {
            for ch in n.children.iter_mut() {
                let d = ch.desired;
                let x = match (ch.canvas[0], ch.canvas[2]) {
                    (Some(l), _) => inner.x + l,
                    (None, Some(rt)) => inner.x + inner.w - rt - d[0],
                    _ => inner.x,
                };
                let y = match (ch.canvas[1], ch.canvas[3]) {
                    (Some(t), _) => inner.y + t,
                    (None, Some(b)) => inner.y + inner.h - b - d[1],
                    _ => inner.y,
                };
                arrange(ch, Rect::new(x, y, d[0], d[1]), m);
            }
        }
        Panel::Viewbox { stretch } => {
            for ch in n.children.iter_mut() {
                let d = ch.desired;
                let s = viewbox_scale(d, [inner.w, inner.h], stretch);
                let (w, h) = (d[0] * s[0], d[1] * s[1]);
                let ox = inner.x + (inner.w - w) / 2.0;
                let oy = inner.y + (inner.h - h) / 2.0;
                arrange(ch, Rect::new(0.0, 0.0, d[0], d[1]), m);
                transform(ch, ox, oy, s);
            }
        }
        Panel::Grid { rows, cols } => {
            let rows = defs_or_one(&rows);
            let cols = defs_or_one(&cols);
            let (mut auto_c, mut auto_r) = n.grid_auto.clone();
            auto_c.resize(cols.len(), 0.0);
            auto_r.resize(rows.len(), 0.0);
            let zero_c = vec![0.0; cols.len()];
            let zero_r = vec![0.0; rows.len()];
            let wc = resolve(&cols, &auto_c, &zero_c, inner.w.max(0.0));
            let wr = resolve(&rows, &auto_r, &zero_r, inner.h.max(0.0));
            let start = |sizes: &[f32], i: usize| -> f32 { sizes[..i].iter().sum() };
            let (nc, nr) = (cols.len(), rows.len());
            for ch in n.children.iter_mut() {
                let cr = span(ch.col, ch.col_span, nc);
                let rr = span(ch.row, ch.row_span, nr);
                let slot = Rect::new(
                    inner.x + start(&wc, cr.start),
                    inner.y + start(&wr, rr.start),
                    wc[cr].iter().sum(),
                    wr[rr].iter().sum(),
                );
                arrange(ch, slot, m);
            }
        }
    }
}

/// Scale a laid-out subtree by `s` around the origin, then move it to (ox, oy).
fn transform<T>(n: &mut Node<T>, ox: f32, oy: f32, s: [f32; 2]) {
    let r = n.rect;
    n.rect = Rect::new(ox + r.x * s[0], oy + r.y * s[1], r.w * s[0], r.h * s[1]);
    let sl = n.slot;
    n.slot = Rect::new(ox + sl.x * s[0], oy + sl.y * s[1], sl.w * s[0], sl.h * s[1]);
    n.scale *= s[0].min(s[1]);
    for ch in n.children.iter_mut() {
        transform(ch, ox, oy, s);
    }
}

/// Visit every node with its depth, parents first.
pub(crate) fn walk<T>(n: &Node<T>, f: &mut dyn FnMut(&Node<T>, usize)) {
    fn go<T>(n: &Node<T>, d: usize, f: &mut dyn FnMut(&Node<T>, usize)) {
        f(n, d);
        for ch in &n.children {
            go(ch, d + 1, f);
        }
    }
    go(n, 0, f);
}
