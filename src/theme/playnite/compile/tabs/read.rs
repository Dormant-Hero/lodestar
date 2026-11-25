//! Reading the theme's `ItemFilterQuickPreset` template for one preset in one state: which of
//! its triggers hold, and the values its parts take then. WPF decides this per preset button
//! (a CheckBoxEx): `IsChecked` is the open preset, `AlternationIndex` its place in the strip,
//! and a part bound to `ElementName=FilterIcon, Path=Source` depends on whether the theme
//! ships a picture for that preset's name. Nothing in the strip has the focus while the game
//! list does, so focus and mouse-over triggers rest unset.

use super::super::bind::parse_binding;
use super::super::visibility::{is_null, truthy, vis_of};
use super::*;

/// How deep a part's style trigger may follow other parts' visibility (`ElementName=X,
/// Path=Visibility` in both directions ends here).
const MAX_DEPTH: u8 = 4;

/// One preset button in one state.
#[derive(Clone, Copy, Debug)]
pub(super) struct Item {
    /// The names its pictures are looked up by (`PRESET_NAMES`; none for a collection that
    /// isn't one of the tabs).
    pub names: &'static [&'static str],
    /// Its place in the strip (`AlternationIndex`).
    pub index: usize,
    /// It is the open preset (`IsChecked`).
    pub open: bool,
}

/// The template of the preset buttons and the trigger setters that hold for one item.
pub(super) struct Reading<'t, 'i> {
    /// The ItemFilterQuickPreset style (the button's own setters).
    pub style: X<'t, 'i>,
    /// The template's visual root.
    pub root: X<'t, 'i>,
    /// The FilterPresetSelector (bindings to the view's elements are read from it).
    pub selector: X<'t, 'i>,
    pub item: Item,
    /// The strip numbers its items (`AlternationCount` set): `AlternationIndex` is the item's
    /// place, else 0 for every item.
    pub alternation: bool,
    /// (TargetName, empty for the button itself; property) → value.
    set: HashMap<(String, String), Value<'t, 'i>>,
}

/// The last segment of a property name (`ItemsControl.Height` → `Height`).
fn short(prop: &str) -> &str {
    let p = prop.trim();
    p.rsplit('.').next().unwrap_or(p)
}

/// Properties no preset button has while the game list holds the focus.
fn unfocused(prop: &str) -> bool {
    matches!(
        prop,
        "IsFocused" | "IsKeyboardFocused" | "IsKeyboardFocusWithin" | "IsMouseOver" | "IsPressed" | "IsSelected"
    )
}

/// A value equals what a trigger asks for.
fn value_is(v: &Value, want: &str) -> bool {
    match v {
        Value::Bool(b) => truthy(want) == Some(*b),
        Value::Num(n) => want.trim().parse::<f64>().ok() == Some(*n),
        Value::Text(t) => tree::same_value(t, want),
        Value::Null => is_null(want),
        _ => false,
    }
}

/// Where a storyboard's animations leave Opacity: (TargetName, empty for the button; value).
fn opacity_ends<'a>(sb: X<'a, '_>) -> impl Iterator<Item = (String, f64)> + 'a {
    sb.descendants().filter(|a| a.is_element() && tag(*a) == "DoubleAnimation").filter_map(|a| {
        let prop = attr(a, "Storyboard.TargetProperty").map(short)?;
        let to = attr(a, "To")?.trim().parse::<f64>().ok()?;
        (prop == "Opacity").then(|| (attr(a, "Storyboard.TargetName").unwrap_or("").to_string(), to))
    })
}

/// One condition of a trigger: on a property of the button or of a part (`SourceName`), or on
/// a binding.
struct Condition<'a> {
    binding: Option<&'a str>,
    property: Option<&'a str>,
    source: Option<&'a str>,
    value: &'a str,
}

/// A trigger's conditions.
fn conditions<'a>(tr: X<'a, '_>) -> Vec<Condition<'a>> {
    let one = |c: X<'a, '_>| Condition {
        binding: attr(c, "Binding"),
        property: attr(c, "Property"),
        source: attr(c, "SourceName"),
        value: attr(c, "Value").unwrap_or(""),
    };
    match tag(tr) {
        "Trigger" | "DataTrigger" => vec![one(tr)],
        "MultiTrigger" | "MultiDataTrigger" => {
            prop_elem(tr, "Conditions").map(|cs| content(cs).map(one).collect()).unwrap_or_default()
        }
        _ => vec![],
    }
}

impl<'b, 't, 'i> Builder<'b, 't, 'i> {
    /// The template read for `item`: the setters (and storyboard ends) of every trigger that
    /// holds. The button's style triggers outrank its template's (WPF's value precedence),
    /// and an animation outranks both.
    pub(super) fn read_item(
        &self,
        style: X<'t, 'i>,
        selector: X<'t, 'i>,
        item: Item,
        alternation: bool,
    ) -> Option<Reading<'t, 'i>> {
        let tpl = self.t.template(style)?;
        let root = self.t.template_root(tpl)?;
        // Conditions are read before any setter applies: a trigger testing another part's
        // state sees it as written (triggers reading each other's results are rare in presets).
        let mut r = Reading { style, root, selector, item, alternation, set: HashMap::new() };
        let mut set = HashMap::new();
        let mut animated = vec![];
        for tr in self.t.triggers(tpl).into_iter().chain(self.t.triggers(style)) {
            if !self.holds(&r, tr, None, 0) {
                continue;
            }
            for s in tree::setters(tr) {
                let Some(prop) = attr(s, "Property").map(short) else { continue };
                if let Some(v) = self.t.setter_value(s, prop) {
                    set.insert((attr(s, "TargetName").unwrap_or("").to_string(), prop.to_string()), v);
                }
            }
            for sb in self.t.enter_storyboards(tr) {
                animated.extend(opacity_ends(sb));
            }
        }
        for (target, to) in animated {
            set.insert((target, "Opacity".to_string()), Value::Num(to));
        }
        r.set = set;
        Some(r)
    }

    /// Every condition of trigger `tr` holds for the item. `subject`: the part whose own
    /// style holds the trigger (None: the button's style or template).
    fn holds(&self, r: &Reading<'t, 'i>, tr: X<'t, 'i>, subject: Option<X<'t, 'i>>, depth: u8) -> bool {
        let conds = conditions(tr);
        !conds.is_empty()
            && conds.into_iter().all(|c| match (c.property, c.binding) {
                (Some(p), _) => {
                    let target = c.source.and_then(|s| self.t.named(r.root, s)).or(subject);
                    self.state_is(r, target, p, c.value, depth)
                }
                (None, Some(b)) => self.binding_is(r, b, c.value, subject, depth),
                (None, None) => false,
            })
    }

    /// A binding in a trigger condition equals `value` for the item.
    fn binding_is(
        &self,
        r: &Reading<'t, 'i>,
        binding: &str,
        value: &str,
        subject: Option<X<'t, 'i>>,
        depth: u8,
    ) -> bool {
        let Markup::Binding(raw) = tree::markup(binding) else {
            return self.test(r.selector, binding, value) == Test::Always;
        };
        let b = parse_binding(&raw);
        let path = b.path.trim().trim_start_matches("Path=");
        if b.relative && b.element.is_none() {
            // `RelativeSource Self` in a part's own style is that part; in the button's
            // triggers (and `TemplatedParent` anywhere) it is the button.
            let part = subject.filter(|_| !raw.contains("TemplatedParent"));
            return self.state_is(r, part, path, value, depth);
        }
        if let Some(name) = &b.element
            && let Some(part) = self.t.named(r.root, name)
        {
            return self.state_is(r, Some(part), path, value, depth);
        }
        self.test(r.selector, binding, value) == Test::Always
    }

    /// The button's (`part` None) or a part's property `prop` equals `value` for the item.
    fn state_is(&self, r: &Reading<'t, 'i>, part: Option<X<'t, 'i>>, prop: &str, value: &str, depth: u8) -> bool {
        let prop = short(prop);
        if unfocused(prop) {
            return truthy(value) == Some(false);
        }
        let Some(part) = part else {
            return match prop {
                "IsChecked" => truthy(value) == Some(r.item.open),
                "IsEnabled" | "IsVisible" => truthy(value) == Some(true),
                "AlternationIndex" => {
                    value.trim().parse::<usize>().ok() == Some(if r.alternation { r.item.index } else { 0 })
                }
                "Tag" => self.t.setter(r.style, "Tag").map_or(is_null(value), |v| value_is(&v, value)),
                _ => false,
            };
        };
        match prop {
            "Source" => {
                let shows = match path_format(part) {
                    Some(format) => self.preset_file(&format, r.item.names).is_some(),
                    None => self.t.local(part, "Source").is_some(),
                };
                is_null(value) != shows
            }
            "Visibility" if depth < MAX_DEPTH => {
                let own = self.item_value(r, part, "Visibility", depth + 1);
                vis_of(value) == own.and_then(|v| v.text().map(vis_of)).unwrap_or(Vis::Visible)
            }
            _ => false,
        }
    }

    /// The button's own property for the item: a trigger's setter, else its style's.
    pub(super) fn button_value(&self, r: &Reading<'t, 'i>, prop: &str) -> Option<Value<'t, 'i>> {
        match r.set.get(&(String::new(), prop.to_string())) {
            Some(v) => Some(v.clone()),
            None => self.t.setter(r.style, prop),
        }
    }

    /// A template part's property for the item: a template trigger's setter, else as written
    /// (a TemplateBinding is the button's value), else its own style's triggers and setters.
    pub(super) fn item_value(
        &self,
        r: &Reading<'t, 'i>,
        part: X<'t, 'i>,
        prop: &str,
        depth: u8,
    ) -> Option<Value<'t, 'i>> {
        if let Some(n) = name_of(part)
            && let Some(v) = r.set.get(&(n.to_string(), prop.to_string()))
        {
            return Some(v.clone());
        }
        match self.t.local(part, prop) {
            Some(Value::Binding(b)) if b.starts_with("TemplateBinding:") => {
                self.button_value(r, b.trim_start_matches("TemplateBinding:").trim())
            }
            Some(v) => Some(v),
            None => {
                let style = self.t.style_of(part)?;
                let triggered = self
                    .t
                    .triggers(style)
                    .into_iter()
                    .filter(|tr| depth < MAX_DEPTH && self.holds(r, *tr, Some(part), depth + 1))
                    .flat_map(tree::setters)
                    .filter(|s| attr(*s, "Property").map(short) == Some(prop) && attr(*s, "TargetName").is_none())
                    .last();
                triggered.and_then(|s| self.t.setter_value(s, prop)).or_else(|| self.t.setter(style, prop))
            }
        }
    }

    pub(super) fn item_num(&self, r: &Reading<'t, 'i>, part: X<'t, 'i>, prop: &str) -> Option<f32> {
        self.item_value(r, part, prop, 0).and_then(|v| v.num()).map(|v| v as f32).filter(|v| v.is_finite())
    }

    /// The part's sides (`Margin`, `Padding`, `BorderThickness`), 0 when unset.
    pub(super) fn item_sides(&self, r: &Reading<'t, 'i>, part: X<'t, 'i>, prop: &str) -> [f32; 4] {
        self.item_value(r, part, prop, 0).and_then(|v| v.sides()).unwrap_or([0.0; 4])
    }

    /// The part and every part holding it are drawn for the item (not Collapsed, Hidden or
    /// fully transparent).
    pub(super) fn item_shown(&self, r: &Reading<'t, 'i>, part: X<'t, 'i>) -> bool {
        self.item_chain(r, part).into_iter().all(|p| {
            let hidden = self
                .item_value(r, p, "Visibility", 0)
                .and_then(|v| v.text().map(vis_of))
                .is_some_and(|v| v != Vis::Visible);
            !hidden && self.item_num(r, p, "Opacity").is_none_or(|o| o > 0.01)
        })
    }

    /// The part's opacity with that of every part holding it.
    pub(super) fn item_opacity(&self, r: &Reading<'t, 'i>, part: X<'t, 'i>) -> f32 {
        self.item_chain(r, part)
            .into_iter()
            .map(|p| self.item_num(r, p, "Opacity").unwrap_or(1.0).clamp(0.0, 1.0))
            .product()
    }

    /// `part` and the parts holding it, up to the template's root.
    pub(super) fn item_chain(&self, r: &Reading<'t, 'i>, part: X<'t, 'i>) -> Vec<X<'t, 'i>> {
        let mut out = vec![part];
        let mut cur = part;
        while cur != r.root {
            let Some(p) = cur.parent_element() else { break };
            out.push(p);
            cur = p;
        }
        out
    }

    /// Room between the button's edge and `part`, [left, top, right, bottom]: the margins of
    /// `part` and of the parts holding it, the paddings and borders of those holders (and a
    /// TextBlock's own padding, which lies between its edge and its text).
    pub(super) fn inset(&self, r: &Reading<'t, 'i>, part: X<'t, 'i>) -> [f32; 4] {
        let chain = self.item_chain(r, part);
        let mut out = self.button_value(r, "Margin").and_then(|v| v.sides()).unwrap_or([0.0; 4]);
        for (i, p) in chain.iter().enumerate() {
            let mut add = self.item_sides(r, *p, "Margin");
            if i > 0 || tag(*p) == "TextBlock" {
                let (pad, border) = (self.item_sides(r, *p, "Padding"), self.item_sides(r, *p, "BorderThickness"));
                for s in 0..4 {
                    add[s] += pad[s] + if tag(*p) == "Border" { border[s] } else { 0.0 };
                }
            }
            for s in 0..4 {
                out[s] += add[s];
            }
        }
        out
    }

    /// The room `part` gets along one axis (`dim` "Height" / "Width") for the item: its own
    /// size, else the nearest holder's size less what lies between (a star row of a Grid
    /// its share), else the button's.
    pub(super) fn room(&self, r: &Reading<'t, 'i>, part: X<'t, 'i>, dim: &str) -> Option<f32> {
        let sides = if dim == "Height" { [1, 3] } else { [0, 2] };
        let cap = |p: X<'t, 'i>| {
            self.item_num(r, p, dim).or_else(|| self.item_num(r, p, &format!("Max{dim}"))).filter(|v| *v > 0.0)
        };
        if let Some(own) = cap(part) {
            return Some(own);
        }
        let (mut lost, mut share) = (0.0, 1.0);
        let mut cur = part;
        loop {
            let m = self.item_sides(r, cur, "Margin");
            lost += m[sides[0]] + m[sides[1]];
            if cur == r.root {
                let own = self.button_value(r, dim).and_then(|v| v.num()).map(|v| v as f32).filter(|v| *v > 0.0)?;
                return Some(((own - lost) * share).max(0.0));
            }
            let holder = cur.parent_element()?;
            if dim == "Height" {
                match row_share(holder, cur) {
                    RowShare::Fixed(px) => return Some((px - lost).max(0.0)),
                    RowShare::Star(s) => share *= s,
                    RowShare::Auto => return None,
                }
            }
            let (pad, border) = (self.item_sides(r, holder, "Padding"), self.item_sides(r, holder, "BorderThickness"));
            lost += pad[sides[0]] + pad[sides[1]];
            if tag(holder) == "Border" {
                lost += border[sides[0]] + border[sides[1]];
            }
            if let Some(size) = cap(holder) {
                return Some(((size - lost) * share).max(0.0));
            }
            cur = holder;
        }
    }
}

/// How much of a Grid's height its child's row gets.
enum RowShare {
    Fixed(f32),
    /// A fraction of the height the star rows share (1 when the holder isn't a Grid with rows).
    Star(f32),
    Auto,
}

fn row_share(holder: X, child: X) -> RowShare {
    let rows: Vec<String> = match prop_elem(holder, "RowDefinitions").filter(|_| tag(holder) == "Grid") {
        Some(defs) => content(defs).map(|d| attr(d, "Height").unwrap_or("*").trim().to_ascii_lowercase()).collect(),
        None => return RowShare::Star(1.0),
    };
    let star = |h: &str| match h.strip_suffix('*') {
        Some("") => Some(1.0),
        Some(n) => n.trim().parse::<f32>().ok(),
        None => None,
    };
    let row = attr(child, "Grid.Row").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
    let Some(own) = rows.get(row) else { return RowShare::Star(1.0) };
    if let Ok(px) = own.parse::<f32>() {
        return RowShare::Fixed(px);
    }
    let total: f32 = rows.iter().filter_map(|h| star(h)).sum();
    match star(own) {
        // Rows of a fixed or automatic height next to the star ones: their share is unknown.
        Some(s) if total > 0.0 && rows.iter().all(|h| star(h).is_some()) => RowShare::Star(s / total),
        _ => RowShare::Auto,
    }
}
