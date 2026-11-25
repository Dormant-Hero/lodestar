//! A button's face as its own ControlTemplate draws it. Playnite themes draw their buttons'
//! background, outline, corners and label in the template, and change them with triggers on
//! the button's state: the details page opens with the focus on its Play button, which is why
//! the theme's screenshots show that button in its focused look (a white pill, a coloured
//! ring), and many templates paint Play and Install differently (`IsInstalled` triggers).

use super::bind::{game_false, game_path, parse_binding};
use super::build::icon_font;
use super::visibility::{truthy, vis_of};
use super::*;

/// What a button's template draws behind its label, and the label's look.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Face {
    /// The fill (transparent when the template draws none).
    pub fill: Option<LookBrush>,
    pub border: Option<(Color, f32)>,
    pub radius: f32,
    /// The label colour the template gives (its label part's, else the button's own).
    pub fg: Option<LookBrush>,
    /// The label part's own font size, when the template sets one.
    pub size: Option<f32>,
    /// The label the template writes itself (a TextBlock whose text its triggers set), when it
    /// shows no content of the button's.
    pub text: Option<String>,
    /// The filled part's height and its offset from the button's top, px, when it is a band
    /// narrower than the button's box (a pill inside a taller button); None: it fills the box.
    pub band: Option<(f32, f32)>,
}

/// The state a button is drawn in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ButtonState {
    pub focused: bool,
    /// Whether the game is installed (None: the template's per-game triggers don't hold).
    pub installed: Option<bool>,
}

/// The setters of a template's triggers that hold in one state, by (TargetName — empty for
/// the button itself, property).
type StateSetters<'t, 'i> = HashMap<(String, String), X<'t, 'i>>;

/// Template parts that paint a background.
const FILLED_PARTS: [&str; 7] = ["Border", "Rectangle", "Grid", "StackPanel", "DockPanel", "WrapPanel", "Canvas"];

impl<'b, 't, 'i> Builder<'b, 't, 'i> {
    /// The face of button `el` in `state`; None when it has no template of its own to read.
    pub(super) fn button_face(&self, el: X<'t, 'i>, state: ButtonState) -> Option<Face> {
        let Some(Value::Node(tpl)) = self.get(el, "Template") else { return None };
        if tag(tpl) != "ControlTemplate" {
            return None;
        }
        let setters = self.state_setters(el, tpl, state);
        let root = self.t.template_root(tpl)?;
        let mut face = Face { fill: Some(LookBrush::solid(Color::TRANSPARENT)), ..Face::default() };
        for part in root.descendants().filter(|d| d.is_element() && FILLED_PARTS.contains(&tag(*d))) {
            if self.part_shown(el, part, root, &setters) {
                self.paint_part(el, part, root, &setters, &mut face);
            }
        }
        let label = self.label_part(el, root, &setters);
        face.fg = label
            .and_then(|l| self.inherited_part_value(el, l, root, "Foreground", &setters))
            .or_else(|| self.owner_value(el, "Foreground", &setters))
            .and_then(|v| v.brush());
        face.size = label
            .and_then(|l| self.inherited_part_value(el, l, root, "FontSize", &setters))
            .and_then(|v| v.num())
            .map(|s| s as f32)
            .filter(|s| *s > 0.0);
        face.text = label
            .filter(|l| tag(*l) == "TextBlock" && !holds_content(*l))
            .and_then(|l| self.part_value(el, l, "Text", &setters))
            .and_then(|v| v.text().and_then(|t| self.literal_or_loc(t)))
            .filter(|t| !t.trim().is_empty());
        Some(face)
    }

    /// One shown template part drawn over the face so far: later parts are drawn over earlier
    /// ones, so the last filled part is the face and the last stroked part its outline.
    fn paint_part(
        &self,
        el: X<'t, 'i>,
        part: X<'t, 'i>,
        root: X<'t, 'i>,
        setters: &StateSetters<'t, 'i>,
        face: &mut Face,
    ) {
        let rect = tag(part) == "Rectangle";
        let prop = |p: &str| self.part_value(el, part, p, setters);
        let fill = prop(if rect { "Fill" } else { "Background" }).and_then(|v| v.brush()).filter(|b| !b.is_clear());
        let radius = if rect {
            prop("RadiusX").and_then(|v| v.num()).map(|v| v as f32)
        } else {
            prop("CornerRadius").and_then(|v| v.max_side())
        };
        if let Some(f) = fill {
            face.fill = Some(f);
            face.radius = radius.unwrap_or(0.0);
            face.band = self.num(part, "Height").map(|h| {
                let above = |p: X<'t, 'i>| self.sides(p, "Margin")[1];
                (if part == root { above(root) } else { above(root) + above(part) }, h)
            });
        }
        let thick = prop(if rect { "StrokeThickness" } else { "BorderThickness" }).and_then(|v| v.max_side());
        let stroke = prop(if rect { "Stroke" } else { "BorderBrush" })
            .and_then(|v| v.brush())
            .and_then(|b| b.average())
            .filter(|c| c.0[3] > 0);
        if let (Some(t), Some(c)) = (thick.filter(|t| *t > 0.0), stroke) {
            face.border = Some((c, t));
            if face.radius == 0.0 {
                face.radius = radius.unwrap_or(0.0);
            }
        }
    }

    /// The part that draws the button's label: the TextBlock holding its ContentPresenter (or
    /// the presenter itself), else a TextBlock showing the button's Content, else a TextBlock
    /// whose text the template's triggers write (icon-font glyphs aside).
    fn label_part(&self, el: X<'t, 'i>, root: X<'t, 'i>, setters: &StateSetters<'t, 'i>) -> Option<X<'t, 'i>> {
        let parts: Vec<X> =
            root.descendants().filter(|d| d.is_element() && self.part_shown(el, *d, root, setters)).collect();
        let presenter = parts
            .iter()
            .find(|d| tag(**d) == "ContentPresenter")
            .map(|p| p.ancestors().take_while(|a| *a != root).find(|a| tag(*a) == "TextBlock").unwrap_or(*p));
        let shows_content = |d: &&X| {
            tag(**d) == "TextBlock"
                && attr(**d, "Text")
                    .is_some_and(|t| matches!(tree::markup(t), Markup::TemplateBinding(p) if p.trim() == "Content"))
        };
        let written = |d: &&X| {
            tag(**d) == "TextBlock"
                && name_of(**d).is_some_and(|n| setters.contains_key(&(n.to_string(), "Text".to_string())))
                && !self
                    .part_value(el, **d, "FontFamily", setters)
                    .and_then(|v| v.text().map(icon_font))
                    .unwrap_or(false)
        };
        presenter.or_else(|| parts.iter().find(shows_content).or_else(|| parts.iter().find(written)).copied())
    }

    /// A text property of a template part as WPF inherits it: the part's own value, else the
    /// nearest part holding it that sets it (as `TextElement.…` on presenters and panels).
    fn inherited_part_value(
        &self,
        el: X<'t, 'i>,
        part: X<'t, 'i>,
        root: X<'t, 'i>,
        prop: &str,
        setters: &StateSetters<'t, 'i>,
    ) -> Option<Value<'t, 'i>> {
        let attached = format!("TextElement.{prop}");
        let mut cur = Some(part);
        while let Some(p) = cur {
            let own = if tag(p) == "TextBlock" { prop } else { attached.as_str() };
            if let Some(v) = self.part_value(el, p, own, setters) {
                return Some(v);
            }
            if p == root {
                return None;
            }
            cur = p.parent_element();
        }
        None
    }

    /// The setters of the button's triggers (its template's and its style's) that hold in
    /// `state`, later ones over earlier ones as in WPF.
    fn state_setters(&self, el: X<'t, 'i>, tpl: X<'t, 'i>, state: ButtonState) -> StateSetters<'t, 'i> {
        let mut trigs = self.t.triggers(tpl);
        if let Some(s) = self.t.style_of(el) {
            trigs.extend(self.t.triggers(s));
        }
        let mut out = HashMap::new();
        for tr in trigs.into_iter().filter(|tr| button_trigger_holds(*tr, state)) {
            for set in tree::setters(tr) {
                let Some(p) = attr(set, "Property") else { continue };
                let p = p.trim().rsplit('.').next().unwrap_or("").to_string();
                out.insert((attr(set, "TargetName").unwrap_or("").to_string(), p), set);
            }
        }
        out
    }

    /// A template part's property in this state: a trigger's setter, else as written (a
    /// TemplateBinding is the button's own value), else its style's.
    fn part_value(
        &self,
        el: X<'t, 'i>,
        part: X<'t, 'i>,
        prop: &str,
        setters: &StateSetters<'t, 'i>,
    ) -> Option<Value<'t, 'i>> {
        let short = prop.rsplit('.').next().unwrap_or(prop);
        if let Some(n) = name_of(part)
            && let Some(set) = setters.get(&(n.to_string(), short.to_string()))
        {
            return self.t.setter_value(*set, short);
        }
        match self.t.local(part, prop) {
            Some(Value::Binding(b)) if b.starts_with("TemplateBinding:") => {
                self.owner_value(el, b.trim_start_matches("TemplateBinding:").trim(), setters)
            }
            Some(v) => Some(v),
            None => self.t.style_of(part).and_then(|s| self.t.setter(s, prop)),
        }
    }

    /// The button's own property in this state (a trigger's setter, else its value).
    fn owner_value(&self, el: X<'t, 'i>, prop: &str, setters: &StateSetters<'t, 'i>) -> Option<Value<'t, 'i>> {
        match setters.get(&(String::new(), prop.to_string())) {
            Some(set) => self.t.setter_value(*set, prop),
            None => self.get(el, prop),
        }
    }

    /// A template part and every part holding it are shown (not Collapsed, Hidden or fully
    /// transparent) in this state.
    fn part_shown(&self, el: X<'t, 'i>, part: X<'t, 'i>, root: X<'t, 'i>, setters: &StateSetters<'t, 'i>) -> bool {
        let mut cur = Some(part);
        while let Some(p) = cur {
            let hidden = match self.part_value(el, p, "Visibility", setters) {
                Some(Value::Text(t)) => vis_of(&t) != Vis::Visible,
                _ => false,
            };
            let clear = self.part_value(el, p, "Opacity", setters).and_then(|v| v.num()).is_some_and(|o| o <= 0.01);
            if hidden || clear {
                return false;
            }
            if p == root {
                break;
            }
            cur = p.parent_element();
        }
        true
    }
}

/// A TextBlock that shows the button's content through a ContentPresenter inside it.
fn holds_content(tb: X) -> bool {
    tb.descendants().any(|d| d.is_element() && tag(d) == "ContentPresenter")
}

/// Every condition of a button trigger holds in `state`. Conditions on anything but the
/// button's focus, pointer, enabled state and its game's install state never hold here.
fn button_trigger_holds(tr: X, state: ButtonState) -> bool {
    let conditions: Vec<X> = match tag(tr) {
        "Trigger" | "DataTrigger" => vec![tr],
        "MultiTrigger" | "MultiDataTrigger" => {
            prop_elem(tr, "Conditions").map(|c| content(c).collect()).unwrap_or_default()
        }
        _ => vec![],
    };
    !conditions.is_empty()
        && conditions.iter().all(|c| {
            let want = attr(*c, "Value").and_then(truthy);
            let got = match (attr(*c, "Property"), attr(*c, "Binding")) {
                (Some(p), _) => state_of(p, state),
                (None, Some(b)) => match tree::markup(b) {
                    Markup::Binding(raw) => {
                        let b = parse_binding(&raw);
                        let path = b.path.trim().trim_start_matches("Path=");
                        if b.on_control() {
                            state_of(path, state)
                        } else if b.element.is_none() && !b.relative {
                            game_state(path, state)
                        } else {
                            None
                        }
                    }
                    _ => None,
                },
                _ => None,
            };
            got.is_some() && got == want
        })
}

/// A state property of the button itself.
fn state_of(prop: &str, state: ButtonState) -> Option<bool> {
    match prop.trim() {
        "IsFocused" | "IsKeyboardFocused" | "IsKeyboardFocusWithin" => Some(state.focused),
        "IsMouseOver" | "IsPressed" => Some(false),
        "IsEnabled" => Some(true),
        _ => None,
    }
}

/// A state of the button's game.
fn game_state(path: &str, state: ButtonState) -> Option<bool> {
    if game_false(path) {
        return Some(false);
    }
    (game_path(path) == "IsInstalled").then_some(state.installed).flatten()
}
