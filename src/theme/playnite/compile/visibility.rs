//! When an element shows: Visibility as written, `{Settings …}` values, DataTriggers on game
//! properties (→ `visible_if` / `hidden_if`), the resting state of triggers on the
//! controller's state, and the storyboards that fade elements in or play them away.

use super::bind::{game_false, game_key, image_art, parse_binding};
use super::build::{OTHER_VIEWS, is_pending_button, later_list_item};
use super::*;

impl Cond {
    /// Both must hold: shown when both rules show it (`&&` of the conditions, `||` of the
    /// hiding ones; a plain pair of bindings is just written one after the other).
    pub(super) fn and(&self, o: &Cond) -> Cond {
        let ops = |s: &str| s.contains("&&") || s.contains("||") || s.contains('!') || s.contains("==");
        Cond {
            visible_if: match (&self.visible_if, &o.visible_if) {
                (Some(a), Some(b)) if a != b => Some(format!("{a} && {b}")),
                (a, b) => a.clone().or_else(|| b.clone()),
            },
            hidden_if: match (&self.hidden_if, &o.hidden_if) {
                (Some(a), Some(b)) if a != b && (ops(a) || ops(b)) => Some(format!("{a} || {b}")),
                (Some(a), Some(b)) if a != b => Some(format!("{a}{b}")),
                (a, b) => a.clone().or_else(|| b.clone()),
            },
        }
    }
}

pub(super) fn is_null(v: &str) -> bool {
    let v = v.trim();
    v.is_empty() || v == "{x:Null}"
}

/// A Visibility (or a boolean bound to one) as written.
pub(super) fn vis_of(v: &str) -> Vis {
    match v.trim().to_ascii_lowercase().as_str() {
        "collapsed" | "false" | "" => Vis::Collapsed,
        "hidden" => Vis::Hidden,
        _ => Vis::Visible,
    }
}

pub(super) fn truthy(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "true" | "visible" => Some(true),
        "false" | "collapsed" | "hidden" | "0" => Some(false),
        _ => None,
    }
}

/// Triggers of a ControlTemplate that set properties of named elements: name → triggers.
pub(super) fn template_triggers<'t, 'i>(t: &'t Tree<'i>, tpl: X<'t, 'i>) -> HashMap<String, Vec<X<'t, 'i>>> {
    let mut m: HashMap<String, Vec<X>> = HashMap::new();
    for trig in t.triggers(tpl) {
        for s in trig.descendants().filter(|s| s.is_element() && tag(*s) == "Setter") {
            if let Some(n) = attr(s, "TargetName") {
                let v = m.entry(n.to_string()).or_default();
                if !v.contains(&trig) {
                    v.push(trig);
                }
            }
        }
    }
    m
}

/// A trigger that reads Playnite's game list layout settings (horizontal layout, rows,
/// columns, item spacing): only those, or those and other conditions on an element that holds
/// no list of its own (a note such as "disable horizontal scrolling for this theme").
pub(super) fn list_settings_trigger(el: X, tr: X) -> bool {
    let bindings: Vec<&str> = std::iter::once(tr)
        .chain(prop_elem(tr, "Conditions").into_iter().flat_map(content))
        .filter_map(|c| attr(c, "Binding"))
        .collect();
    let list_setting = |b: &&&str| match tree::markup(b) {
        Markup::Settings(s) => {
            let s = s.split(',').next().unwrap_or("").trim();
            let s = s.strip_prefix("Fullscreen.").unwrap_or(s);
            matches!(
                s,
                "HorizontalLayout"
                    | "Rows"
                    | "Columns"
                    | "FullscreenItemSpacing"
                    | "FullscreenItemSpacingMargin"
                    | "CoverAspectRatio"
                    | "GridItemHeightRatio"
                    | "GridItemWidthRatio"
            )
        }
        _ => false,
    };

    match bindings.iter().filter(list_setting).count() {
        0 => false,
        n if n == bindings.len() => true,
        _ => !holds_list(el),
    }
}

/// A trigger reads one of the numbers Playnite's list is sized by (not only the horizontal
/// layout switch).
fn reads_list_size(tr: X) -> bool {
    std::iter::once(tr)
        .chain(prop_elem(tr, "Conditions").into_iter().flat_map(content))
        .filter_map(|c| attr(c, "Binding"))
        .any(|b| match tree::markup(b) {
            Markup::Settings(s) => !s.contains("HorizontalLayout"),
            _ => false,
        })
}

/// Triggers evaluated inside each other (`trigger_setter`, `settings_warning`): deep enough for
/// a state set by one trigger that another one reads.
const MAX_NESTED_TRIGGERS: u8 = 3;

/// The element holds a list of its own (it isn't just a note).
fn holds_list(el: X) -> bool {
    el.descendants().any(|d| {
        d.is_element() && matches!(tag(d), "ListBox" | "ListBoxEx" | "ItemsControl" | "ItemsControlEx" | "ListView")
    })
}

/// Whether a storyboard leaves the element `own` shown: the end value of the last of its
/// Opacity / Visibility animations on that element (or naming no target). None: it animates
/// neither.
pub(super) fn storyboard_end<'a, 'i>(sb: X<'a, 'i>, own: Option<&str>) -> Option<bool> {
    let mut last: Option<(u32, bool)> = None;
    let animation =
        |a: &X| a.is_element() && (tag(*a).ends_with("Animation") || tag(*a).ends_with("AnimationUsingKeyFrames"));
    let last_key = |a: X<'a, 'i>| -> Option<X<'a, 'i>> {
        a.descendants().rfind(|k| k.is_element() && tag(*k).ends_with("KeyFrame"))
    };
    for a in sb.descendants().filter(animation) {
        let target = attr(a, "Storyboard.TargetName").map(str::trim);
        if target.is_some() && target != own {
            continue;
        }
        let prop = attr(a, "Storyboard.TargetProperty").unwrap_or("");
        let shown = if prop.contains("Opacity") {
            let to = attr(a, "To").or_else(|| last_key(a).and_then(|k| attr(k, "Value")));
            to.and_then(|v| v.trim().parse::<f32>().ok()).map(|v| v > 0.01)
        } else if prop.contains("Visibility") {
            last_key(a).and_then(|k| attr(k, "Value")).map(|v| {
                let v = v.to_ascii_lowercase();
                !v.contains("collapsed") && !v.contains("hidden")
            })
        } else {
            None
        };
        let Some(shown) = shown else { continue };
        let begin = attr(a, "BeginTime").and_then(duration_ms).unwrap_or(0);
        let length = attr(a, "Duration").and_then(duration_ms).unwrap_or(0);
        let end = begin + length;
        if last.is_none_or(|(t, _)| end >= t) {
            last = Some((end, shown));
        }
    }
    last.map(|(_, shown)| shown)
}

/// The Opacity a storyboard leaves the element `own` at, with when its last Opacity
/// animation ends (ms).
fn final_opacity(sb: X, own: Option<&str>) -> Option<(u32, f32)> {
    sb.descendants()
        .filter(|a| a.is_element() && tag(*a) == "DoubleAnimation")
        .filter(|a| {
            let target = attr(*a, "Storyboard.TargetName").map(str::trim);
            (target.is_none() || target == own)
                && attr(*a, "Storyboard.TargetProperty").is_some_and(|p| p.contains("Opacity"))
        })
        .filter_map(|a| {
            let to = attr(a, "To")?.trim().parse::<f32>().ok()?;
            let begin = attr(a, "BeginTime").and_then(duration_ms).unwrap_or(0);
            let length = attr(a, "Duration").and_then(duration_ms).unwrap_or(0);
            Some((begin + length, to.clamp(0.0, 1.0)))
        })
        .max_by_key(|(end, _)| *end)
}

/// One of Playnite's own views or state markers, which it shows only on request: the game
/// status and filter panels, the open filter list, "search active" / "filter active" notes,
/// the library update progress.
fn is_playnite_view(el: X) -> bool {
    OTHER_VIEWS.contains(&tag(el))
        || matches!(
            name_of(el).unwrap_or(""),
            "PART_ContentFilterItems"
                | "PART_ElemSearchActive"
                | "PART_ElemExtraFilterActive"
                | "PART_ElemProgressIndicator"
        )
}

/// A part whose Visibility Playnite sets from the game's data (a field the game lacks is
/// collapsed): its named parts other than the screen's own hosts and the game list.
fn shown_by_data(el: X) -> bool {
    name_of(el).is_some_and(|n| {
        n.starts_with("PART_") && !matches!(n, "PART_ListGameItems" | "PART_ViewHost" | "PART_MainHost")
    })
}

/// A test that holds always or never.
fn decided(holds: bool) -> Test {
    if holds { Test::Always } else { Test::Never }
}

/// A Playnite setting's value (as the player chose it, else its default) against `value`.
fn settings_test(setting: Option<SVal>, value: &str) -> Test {
    match setting {
        Some(SVal::Bool(b)) => match truthy(value) {
            Some(v) => decided(v == b),
            None => Test::Never,
        },
        Some(SVal::Num(n)) => decided(value.trim().parse::<f32>().ok() == Some(n)),
        Some(SVal::Text(t)) => decided(t.eq_ignore_ascii_case(value.trim())),
        None => Test::Ui,
    }
}

/// A control the user checks and unchecks (unchecked unless the theme sets it).
fn is_toggle(el: X) -> bool {
    matches!(tag(el), "CheckBox" | "CheckBoxEx" | "ToggleButton" | "ToggleButtonEx" | "RadioButton")
}

pub(super) fn duration_ms(s: &str) -> Option<u32> {
    // h:m:s(.fff)
    let parts: Vec<&str> = s.trim().split(':').collect();
    let secs = match parts.as_slice() {
        [h, m, s] => h.parse::<f32>().ok()? * 3600.0 + m.parse::<f32>().ok()? * 60.0 + s.parse::<f32>().ok()?,
        [s] => s.parse::<f32>().ok()?,
        _ => return None,
    };
    Some((secs * 1000.0).round() as u32)
}

impl<'b, 't, 'i> Builder<'b, 't, 'i> {
    /// `Binding` (raw attribute) equals `value`?
    pub(super) fn test(&self, el: X<'t, 'i>, binding: &str, value: &str) -> Test {
        let value = self.trigger_value(el, value);
        let value = value.as_str();
        match tree::markup(binding) {
            Markup::Settings(s) => settings_test(self.t.settings.get(&s), value),
            Markup::Plugin(p) => self.plugin_test(&p, value),
            Markup::Binding(raw) => self.binding_test(el, binding, &raw, value),
            // Nothing is pending in Lodestar: no notifications, no Playnite update.
            Markup::Other(o) if o.trim() == "Api Notifications.Count" => decided(value.trim() == "0"),
            _ => Test::Ui,
        }
    }

    /// An add-on's setting. The features Lodestar provides (logos, theme options) are there,
    /// every other add-on is missing (its settings unset, its status "not installed").
    fn plugin_test(&self, p: &str, value: &str) -> Test {
        match settings::plugin(p, &self.t.add_on_options) {
            settings::Plugin::Value(v) => {
                decided(settings::plugin_text(p, &v).is_some_and(|got| tree::same_value(&got, value)))
            }
            settings::Plugin::Fallback(f) => decided(tree::same_value(&f, value)),
            settings::Plugin::PerGame(k) => match truthy(value) {
                Some(true) => Test::Set(k.into()),
                Some(false) => Test::Unset(k.into()),
                None if is_null(value) => Test::Unset(k.into()),
                None => Test::Ui,
            },
            settings::Plugin::Missing if p.starts_with("PluginStatus") => decided(truthy(value) == Some(false)),
            settings::Plugin::Missing => decided(is_null(value)),
        }
    }

    /// A `{Binding …}`: a constant in a Tag, another element's state, else the game's data.
    fn binding_test(&self, el: X<'t, 'i>, binding: &str, raw: &str, value: &str) -> Test {
        let b = parse_binding(raw);
        if let Some(t) = self.tag_test(el, &b, value) {
            return t;
        }
        if let Some(name) = &b.element {
            return self.element_test(el, binding, &b, name, value);
        }
        if b.relative || b.converter.is_some() && !b.converter.as_deref().unwrap_or("").contains("Null") {
            return Test::Ui;
        }
        if game_false(&b.path) {
            return decided(truthy(value) == Some(false));
        }
        if later_list_item(binding) {
            return decided(is_null(value) || truthy(value) == Some(false));
        }
        if self.main && !b.path.contains("SelectedGame") && !b.path.starts_with("Game.") {
            return Test::Ui;
        }
        let Some(key) = game_key(&b.path) else { return Test::Ui };
        let k = format!("{{{key}}}");
        match truthy(value) {
            Some(true) => Test::Set(k),
            Some(false) => Test::Unset(k),
            None if is_null(value) => Test::Unset(k),
            None => Test::Ui,
        }
    }

    /// A constant the theme keeps in a Tag (theme options): `{Binding Tag, RelativeSource=
    /// {RelativeSource Self}}` / `ElementName=X, Path=Tag`. None: not such a binding.
    fn tag_test(&self, el: X<'t, 'i>, b: &Bind, value: &str) -> Option<Test> {
        if b.path.trim().trim_start_matches("Path=") != "Tag" || b.converter.is_some() {
            return None;
        }
        let owner = if b.on_self() {
            Some(el)
        } else {
            b.element.as_deref().and_then(|n| self.root.and_then(|r| self.t.named(r, n)))
        }?;
        let Some(v) = self.get(owner, "Tag") else { return Some(decided(is_null(value))) };
        let same = match v {
            Value::Bool(x) => truthy(value) == Some(x),
            Value::Num(x) => value.trim().parse::<f64>().ok() == Some(x),
            Value::Text(t) => t.trim().eq_ignore_ascii_case(value.trim()),
            Value::Null => is_null(value),
            Value::Binding(inner) if !inner.contains("Tag") => {
                return Some(self.test(el, &format!("{{Binding {inner}}}"), value));
            }
            Value::Setting(s) => match self.t.settings.get(&s) {
                Some(SVal::Bool(x)) => truthy(value) == Some(x),
                _ => return Some(Test::Ui),
            },
            Value::Plugin(p) => return Some(self.test(el, &format!("{{{p}}}"), value)),
            _ => return Some(Test::Ui),
        };
        Some(decided(same))
    }

    /// A property of the element named `name` (`ElementName=`).
    fn element_test(&self, el: X<'t, 'i>, binding: &str, b: &Bind, name: &str, value: &str) -> Test {
        let path = b.path.trim().trim_start_matches("Path=");
        if let Some(t) = self.element_state_test(el, name, path, value) {
            return t;
        }
        // Playnite's view host fills the screen.
        if name == "PART_ViewHost" && b.converter.is_none() && matches!(path, "ActualWidth" | "ActualHeight") {
            let size = if path == "ActualWidth" { W } else { H };
            return decided(value.trim().parse::<f32>().ok() == Some(size));
        }
        let owner = self.root.and_then(|r| self.t.named(r, name));
        if matches!(path, "ActualHeight" | "ActualWidth" | "IsVisible")
            && let Some(owner) = owner
        {
            return self.size_test(owner, path, value);
        }
        // Another element's property the theme sets to a constant or a setting
        // (`ChangeViewButton.IsChecked = {Settings Fullscreen.HorizontalLayout}`).
        match self.t.static_binding(el, binding) {
            Some(got) => decided(tree::same_value(&got, value)),
            None if b.converter.is_none() => {
                owner.and_then(|o| self.unset_property_test(o, path, value)).unwrap_or(Test::Ui)
            }
            None => Test::Ui,
        }
    }

    /// The shown state of another element that Lodestar knows at rest: the details view,
    /// warnings, holders, pending buttons, the game list, the focus. None: not one of those.
    fn element_state_test(&self, el: X<'t, 'i>, name: &str, path: &str, value: &str) -> Option<Test> {
        if path == "Visibility" {
            // The details view is open while GameDetails is shown and closed on Main: its
            // entrance (slide-ins, fades) has played, its exit hasn't.
            if name == "PART_ElemGameDetails" {
                return Some(match truthy(value) {
                    Some(v) => decided(v == self.details_open),
                    None => Test::Ui,
                });
            }
            // A warning about Playnite's list settings never shows (`settings_warning`).
            if self.root.and_then(|r| self.t.named(r, name)).is_some_and(|o| self.settings_warning(o)) {
                return Some(decided(truthy(value) == Some(false)));
            }
            // An element holding this one is shown whenever this one is (a Play button's
            // label reading the button's own Visibility).
            if el.ancestors().any(|a| a.is_element() && name_of(a) == Some(name)) {
                return Some(decided(truthy(value) == Some(true)));
            }
            if is_pending_button(name) {
                return Some(decided(vis_of(value) == Vis::Collapsed));
            }
        }
        if name == "PART_ListGameItems" && path == "Items.Count" {
            // There are games.
            return Some(if value.trim() == "0" { Test::Never } else { Test::Ui });
        }
        if name == "PART_ListGameItems" && path == "SelectedIndex" && self.placing_list.get() {
            // The list rests where the screen opens it, on its first game: themes nudge it on
            // index 0 so the first tile clears the screen's edge. Lodestar's list scrolls by
            // itself from there. Anything else keyed on the index stays undecided, as a fixed
            // choice would hold for every game.
            return Some(decided(value.trim().parse::<i64>().ok() == Some(0)));
        }
        self.focused_at_rest(name, path).map(|focused| decided(truthy(value) == Some(focused)))
    }

    /// Whether element `name` has the keyboard focus (`IsFocused`, `IsKeyboardFocused`) or
    /// holds it (`IsKeyboardFocusWithin`) at rest: Playnite puts the focus on the details
    /// page's Play / Install button (PART_ButtonContext), and on the main screen on an item
    /// of the game list. None: not a focus property, or no such element in the template being
    /// laid out.
    pub(super) fn focused_at_rest(&self, name: &str, path: &str) -> Option<bool> {
        let owner = self.root.and_then(|r| self.t.named(r, name))?;
        self.focus_state_at_rest(owner, path)
    }

    /// `focused_at_rest` for an element itself, named or not: whether it is or holds the
    /// focused part is read from what it contains.
    fn focus_state_at_rest(&self, el: X<'t, 'i>, path: &str) -> Option<bool> {
        let holder = if self.details_open { "PART_ButtonContext" } else { "PART_ListGameItems" };
        let is_holder = name_of(el) == Some(holder);
        match path {
            // A list's focus is on one of its items, not on the list.
            "IsFocused" | "IsKeyboardFocused" => Some(self.details_open && is_holder),
            "IsKeyboardFocusWithin" => Some(
                is_holder
                    || self.t.named(el, holder).is_some()
                    || self.details_open && self.t.named(el, "PART_ElemGameDetails").is_some(),
            ),
            "IsMouseOver" => Some(false),
            _ => None,
        }
    }

    /// A trigger's `Value` as the theme means it: `{StaticResource True}` is the resource's
    /// value (as seen from `el`). Playnite's own resources define `True` and `False` as the
    /// booleans, so themes use those keys without defining them.
    fn trigger_value(&self, el: X<'t, 'i>, raw: &str) -> String {
        let Markup::Resource(k) = tree::markup(raw) else { return raw.to_string() };
        match self.t.resource_from(el, &k).and_then(|n| self.t.eval_node(n, 0)) {
            Some(Value::Bool(b)) => if b { "True" } else { "False" }.to_string(),
            Some(Value::Num(n)) => n.to_string(),
            Some(Value::Text(t)) => t,
            _ if matches!(k.trim(), "True" | "False") => k.trim().to_string(),
            _ => raw.to_string(),
        }
    }

    /// A test on a property of another element that the theme leaves at WPF's default or sets
    /// only at runtime: an Opacity rests at 1 or where its own storyboards leave it, an unset
    /// toggle is unchecked, Playnite's own panels are closed, an untriggered Visibility is what
    /// is written, and a text bound to the game is empty when the game lacks that field. None:
    /// still undecided.
    fn unset_property_test(&self, owner: X<'t, 'i>, path: &str, value: &str) -> Option<Test> {
        let depth = self.trigger_depth.get();
        if depth >= MAX_NESTED_TRIGGERS {
            return None;
        }
        self.trigger_depth.set(depth + 1);
        let out = match path {
            "Opacity" => value
                .trim()
                .parse::<f32>()
                .ok()
                .and_then(|want| self.opacity_at_rest(owner).map(|o| decided((o - want).abs() < 0.005))),
            "IsChecked"
                if is_toggle(owner) && self.t.toggles_at_rest.get() && self.get(owner, "IsChecked").is_none() =>
            {
                Some(decided(tree::same_value("False", value)))
            }
            "Visibility" if is_playnite_view(owner) => Some(decided(tree::same_value("Collapsed", value))),
            "Visibility"
                if !shown_by_data(owner)
                    && self.get(owner, "Visibility").is_none()
                    && !self.visibility_triggered(owner) =>
            {
                Some(decided(tree::same_value("Visible", value)))
            }
            "Text" | "ItemsSource" if is_null(value) => match self.get(owner, path) {
                Some(Value::Binding(raw)) => {
                    let b = parse_binding(&raw);
                    let ours = !self.main || b.path.contains("SelectedGame");
                    let key = (b.element.is_none() && !b.relative && ours).then(|| game_key(&b.path)).flatten();
                    key.map(|k| Test::Unset(format!("{{{k}}}")))
                }
                _ => None,
            },
            _ => None,
        };
        self.trigger_depth.set(depth);
        out
    }

    /// The opacity an element rests at: where the storyboards its own events start (loaded, a
    /// new game selected) leave it, else a fade holding at rest, else as written, else 1.
    /// None: a fade whose trigger is undecided may move it.
    fn opacity_at_rest(&self, el: X<'t, 'i>) -> Option<f32> {
        let own = name_of(el);
        let mut trigs = self.t.triggers(el);
        if let Some(s) = self.t.style_of(el) {
            trigs.extend(self.t.triggers(s));
        }
        let (events, states): (Vec<X>, Vec<X>) = trigs.into_iter().partition(|tr| tag(*tr) == "EventTrigger");
        let played = events
            .into_iter()
            .flat_map(|tr| self.t.enter_storyboards(tr))
            .filter_map(|sb| final_opacity(sb, own))
            .max_by_key(|(end, _)| *end);
        if let Some((_, o)) = played {
            return Some(o);
        }
        if let Some(o) = self.resting_opacity(el) {
            return Some(o);
        }
        let undecided = states.into_iter().any(|tr| {
            self.t.enter_storyboards(tr).into_iter().any(|sb| final_opacity(sb, own).is_some())
                && match tag(tr) {
                    "DataTrigger" | "MultiDataTrigger" => self.trigger_test(el, tr) != Test::Never,
                    _ => self.own_state_at_rest(el, tr) != Some(false),
                }
        });
        (!undecided).then(|| self.num(el, "Opacity").unwrap_or(1.0))
    }

    /// One of the element's style triggers or the view's triggers naming it sets its
    /// Visibility.
    fn visibility_triggered(&self, el: X<'t, 'i>) -> bool {
        let sets_visibility = |tr: &X<'t, 'i>, target: Option<&str>| {
            tree::setters(*tr).any(|s| {
                attr(s, "Property").is_some_and(|p| p.trim().ends_with("Visibility")) && attr(s, "TargetName") == target
            })
        };
        let styled = self.t.style_of(el).is_some_and(|s| self.t.triggers(s).iter().any(|tr| sets_visibility(tr, None)));
        let named = name_of(el)
            .and_then(|n| self.tpl_triggers.get(n).map(|v| v.iter().any(|tr| sets_visibility(tr, Some(n)))))
            .unwrap_or(false);
        styled || named
    }

    /// A test on another element's size or `IsVisible`: nothing of it shows when it (or what
    /// holds it) is collapsed at rest; a picture of the game's art shows when the game has it
    /// (a theme showing the name where the logo has no height).
    pub(super) fn size_test(&self, owner: X<'t, 'i>, path: &str, value: &str) -> Test {
        let hidden = if path == "IsVisible" { "False" } else { "0" };
        let collapsed = owner.ancestors().filter(|a| a.is_element()).any(|a| {
            self.t.get(a, "Visibility").is_some_and(|v| match v {
                Value::Text(t) => matches!(t.trim().to_ascii_lowercase().as_str(), "collapsed" | "hidden"),
                _ => false,
            })
        });
        if collapsed {
            return if tree::same_value(hidden, value) { Test::Always } else { Test::Never };
        }
        let art = match self.get(owner, "Source") {
            Some(Value::Binding(raw)) => image_art(name_of(owner).unwrap_or(""), &parse_binding(&raw)),
            _ => None,
        };
        match art {
            Some(k) if tree::same_value(hidden, value) => Test::Unset(k.into()),
            Some(k) if path == "IsVisible" && truthy(value) == Some(true) => Test::Set(k.into()),
            _ => Test::Ui,
        }
    }

    /// The conditions of a DataTrigger / MultiDataTrigger / (property) Trigger, ANDed.
    pub(super) fn trigger_test(&self, el: X<'t, 'i>, trig: X<'t, 'i>) -> Test {
        let mut tests = vec![];
        match tag(trig) {
            "DataTrigger" => {
                let (Some(b), Some(v)) = (attr(trig, "Binding"), attr(trig, "Value")) else {
                    return Test::Ui;
                };
                tests.push(self.test(el, b, v));
            }
            "MultiDataTrigger" => {
                let Some(cs) = prop_elem(trig, "Conditions") else { return Test::Ui };
                for c in content(cs) {
                    let (Some(b), Some(v)) = (attr(c, "Binding"), attr(c, "Value")) else {
                        return Test::Ui;
                    };
                    tests.push(self.test(el, b, v));
                }
            }
            _ => return Test::Ui,
        }
        // All conditions must hold: one that never does decides, whatever the others.
        if tests.contains(&Test::Never) {
            return Test::Never;
        }
        if tests.contains(&Test::Ui) {
            return Test::Ui;
        }
        let sets: Vec<Test> = tests.into_iter().filter(|t| *t != Test::Always).collect();
        match sets.len() {
            0 => Test::Always,
            1 => sets.into_iter().next().unwrap_or(Test::Always),
            _ => Test::Expr(
                sets.iter()
                    .map(|t| match t {
                        Test::Set(k) => k.clone(),
                        Test::Unset(k) => format!("!{k}"),
                        _ => String::new(),
                    })
                    .collect::<Vec<_>>()
                    .join(" && "),
            ),
        }
    }
    /// Visibility as written + the triggers of its style and of the template.
    pub(super) fn visibility(&mut self, el: X<'t, 'i>) -> (Vis, Cond) {
        let mut cond = Cond::default();
        // As written (on the element, else its style's setter); the style's triggers follow.
        let written = match self.t.local(el, "Visibility") {
            Some(Value::Binding(b)) if b.starts_with("TemplateBinding:") => self.t.get(el, "Visibility"),
            Some(v) => Some(v),
            None => self.t.style_of(el).and_then(|s| self.t.setter(s, "Visibility")),
        };
        let mut shown = match written {
            Some(Value::Text(t)) => match t.to_ascii_lowercase().as_str() {
                "collapsed" => Vis::Collapsed,
                "hidden" => Vis::Hidden,
                _ => Vis::Visible,
            },
            Some(Value::Setting(s)) => match self.t.settings.get(&s) {
                Some(SVal::Bool(false)) => Vis::Collapsed,
                _ => Vis::Visible,
            },
            Some(Value::Binding(raw)) => {
                let b = parse_binding(&raw);
                let conv = b.converter.clone().unwrap_or_default();
                let own_tag = b.path.trim() == "Tag" && b.on_self();
                if own_tag {
                    // A theme option kept in the element's Tag.
                    let on = match self.get(el, "Tag") {
                        Some(Value::Bool(x)) => Some(x),
                        Some(Value::Text(t)) => truthy(&t),
                        _ => None,
                    };
                    match on {
                        Some(on) if on == conv.contains("Inverted") => Vis::Collapsed,
                        _ => Vis::Visible,
                    }
                } else if b.element.is_some() || b.relative {
                    match self.t.static_binding(el, &format!("{{Binding {raw}}}")).as_deref().map(vis_of) {
                        Some(v) => v,
                        None => {
                            self.stats.triggers_dropped += 1;
                            Vis::Visible
                        }
                    }
                } else if game_false(&b.path) {
                    if conv.contains("Inverted") { Vis::Visible } else { Vis::Collapsed }
                } else if (!self.main || b.path.contains("SelectedGame"))
                    && let Some(k) = game_key(&b.path)
                {
                    let k = format!("{{{k}}}");
                    if conv.contains("Inverted") {
                        cond.hidden_if = Some(k);
                    } else {
                        cond.visible_if = Some(k);
                    }
                    self.stats.triggers_mapped += 1;
                    Vis::Visible
                } else {
                    Vis::Visible
                }
            }
            Some(Value::Plugin(p)) => {
                let inverted = p.contains("Inverted");
                match settings::plugin(&p, &self.t.add_on_options) {
                    settings::Plugin::Value(v) => settings::plugin_text(&p, &v).as_deref().map_or(Vis::Visible, vis_of),
                    settings::Plugin::Fallback(f) => vis_of(&f),
                    settings::Plugin::PerGame(k) => {
                        if inverted {
                            cond.hidden_if = Some(k.into());
                        } else {
                            cond.visible_if = Some(k.into());
                        }
                        self.stats.triggers_mapped += 1;
                        Vis::Visible
                    }
                    settings::Plugin::Missing => Vis::Collapsed,
                }
            }
            _ => Vis::Visible,
        };
        // Triggers: the element's style, then the template's (for named elements).
        let mut trigs = vec![];
        if let Some(s) = self.t.style_of(el) {
            for tr in self.t.triggers(s) {
                trigs.push((tr, None));
            }
        }
        if let Some(n) = name_of(el)
            && let Some(v) = self.tpl_triggers.get(n)
        {
            for tr in v.clone() {
                trigs.push((tr, Some(n.to_string())));
            }
        }
        for (tr, target) in trigs {
            let set = tr.descendants().find(|s| {
                s.is_element()
                    && tag(*s) == "Setter"
                    && attr(*s, "Property").is_some_and(|p| p.trim().ends_with("Visibility"))
                    && attr(*s, "TargetName").map(str::to_string) == target
            });
            let Some(set) = set else { continue };
            let to = match attr(set, "Value").map(|v| v.trim().to_ascii_lowercase()) {
                Some(v) if v == "visible" => Vis::Visible,
                Some(v) if v == "collapsed" => Vis::Collapsed,
                Some(v) if v == "hidden" => Vis::Hidden,
                _ => continue,
            };
            let test = match self.trigger_test(el, tr) {
                // A condition only the theme's own constants decide (`Source={StaticResource …}`).
                Test::Ui => match self.t.static_trigger(el, tr) {
                    Some(true) => Test::Always,
                    Some(false) => Test::Never,
                    None => Test::Ui,
                },
                t => t,
            };
            match test {
                // A note shown for Playnite's own list layout settings ("turn off horizontal
                // scrolling"): Lodestar lays the list out itself and has no such settings.
                Test::Always if to == Vis::Visible && list_settings_trigger(el, tr) => {
                    if let Some(Value::Text(base)) = self.t.style_of(el).and_then(|s| self.t.setter(s, "Visibility")) {
                        shown = vis_of(&base);
                    }
                }
                Test::Always => shown = to,
                Test::Never => {}
                Test::Ui => self.stats.triggers_dropped += 1,
                Test::Set(_) | Test::Unset(_) | Test::Expr(_) if to == shown => {}
                Test::Expr(e) => {
                    self.stats.triggers_mapped += 1;
                    if to == Vis::Visible {
                        cond.visible_if = Some(e);
                        shown = Vis::Visible;
                    } else {
                        cond.hidden_if = Some(e);
                    }
                }
                test => {
                    self.stats.triggers_mapped += 1;
                    let on = matches!(test, Test::Set(_));
                    let (Test::Set(k) | Test::Unset(k)) = test else { continue };
                    if to == Vis::Visible {
                        // Shown only when the test holds.
                        if on {
                            cond.visible_if = Some(k);
                        } else {
                            cond.hidden_if = Some(k);
                        }
                        shown = Vis::Visible;
                    } else if on {
                        cond.hidden_if = Some(k);
                    } else {
                        cond.visible_if = Some(k);
                    }
                }
            }
        }
        if shown == Vis::Visible && self.settings_warning(el) {
            shown = Vis::Collapsed;
        }
        (shown, cond)
    }

    /// Hidden only by triggers on the numbers Playnite's list is sized by (columns, rows,
    /// spacing, cover aspect) and shown for any other values: a warning that the user's settings
    /// don't match the theme's. Lodestar sizes the list itself, so it never shows.
    pub(super) fn settings_warning(&self, el: X<'t, 'i>) -> bool {
        let Some(style) = self.t.style_of(el) else { return false };
        // The style's Visibility triggers, split into gates on the list's size settings and the
        // rest (cheap checks first: most elements have no such gate).
        let triggers: Vec<(X, bool)> = self
            .t
            .triggers(style)
            .into_iter()
            .filter_map(|tr| {
                let to = tree::setters(tr)
                    .find(|s| {
                        attr(*s, "TargetName").is_none()
                            && attr(*s, "Property").is_some_and(|p| p.trim().ends_with("Visibility"))
                    })
                    .and_then(|s| attr(s, "Value"))?;
                let gate = vis_of(to) == Vis::Collapsed && list_settings_trigger(el, tr) && reads_list_size(tr);
                Some((tr, gate))
            })
            .collect();
        if !triggers.iter().any(|(_, gate)| *gate)
            || holds_list(el)
            || self.t.local(el, "Visibility").is_some_and(|v| v.text().is_some_and(|t| vis_of(t) != Vis::Visible))
        {
            return false;
        }
        // Warnings whose triggers read other warnings end here.
        let depth = self.trigger_depth.get();
        if depth >= MAX_NESTED_TRIGGERS {
            return false;
        }
        self.trigger_depth.set(depth + 1);
        let others = triggers.iter().any(|(tr, gate)| !gate && self.trigger_test(el, *tr) != Test::Never);
        self.trigger_depth.set(depth);
        !others
    }

    /// Opacity that a trigger animates up (the view or part fading in once shown / loaded /
    /// when the selection changes): (it shows, ms of a fade on selection change).
    pub(super) fn fade_in(&self, el: X<'t, 'i>) -> (bool, Option<(u32, u32)>) {
        let own = name_of(el);
        let mut trigs = self.t.triggers(el);
        if let Some(s) = self.t.style_of(el) {
            trigs.extend(self.t.triggers(s));
        }
        let mut shows = false;
        for tr in trigs {
            // A data trigger that never holds never starts its storyboard.
            if matches!(tag(tr), "DataTrigger" | "MultiDataTrigger") && self.trigger_test(el, tr) == Test::Never {
                continue;
            }
            // A fade on the element's own focus or pointer plays only when that happens.
            if tag(tr) == "Trigger" && self.own_state_at_rest(el, tr) == Some(false) {
                continue;
            }
            let event = attr(tr, "RoutedEvent").unwrap_or("");
            let selection = tag(tr) == "EventTrigger" && event.contains("TargetUpdated");
            for sb in self.t.enter_storyboards(tr) {
                for a in sb.descendants().filter(|a| a.is_element() && tag(*a).ends_with("Animation")) {
                    let target = attr(a, "Storyboard.TargetName").map(str::trim);
                    let opacity = attr(a, "Storyboard.TargetProperty").is_some_and(|p| p.contains("Opacity"));
                    let up = attr(a, "To").and_then(|v| v.trim().parse::<f32>().ok()).is_some_and(|v| v > 0.01);
                    if opacity && up && (target.is_none() || target == own) {
                        if selection {
                            let d = attr(a, "Duration").and_then(duration_ms).unwrap_or(300);
                            let b = attr(a, "BeginTime").and_then(duration_ms).unwrap_or(0);
                            return (true, Some((d.clamp(1, 3000), b.min(3000))));
                        }
                        shows = true;
                    }
                }
            }
        }
        (shows, None)
    }

    /// Whether a property trigger on the element's own focus or pointer holds at rest (see
    /// `focus_state_at_rest`). None: another property.
    fn own_state_at_rest(&self, el: X<'t, 'i>, tr: X<'t, 'i>) -> Option<bool> {
        let state = self.focus_state_at_rest(el, attr(tr, "Property")?.trim())?;
        Some(attr(tr, "Value").and_then(truthy) == Some(state))
    }

    /// Its OpacityMask is a VisualBrush of an element the view doesn't have, or of one that
    /// shows nothing.
    pub(super) fn masked_by_nothing(&self, el: X<'t, 'i>) -> bool {
        let Some(Value::Node(vb)) = self.get(el, "OpacityMask") else { return false };
        if tag(vb) != "VisualBrush" {
            return false;
        }
        let Some(Markup::Binding(raw)) = attr(vb, "Visual").map(tree::markup) else { return false };
        let Some(name) = parse_binding(&raw).element else { return false };
        match self.root.and_then(|r| self.t.named(r, &name)) {
            None => true,
            // A mask element painted transparent with nothing inside it (a fade the theme
            // switches off, e.g. when there's no trailer) shows nothing through it either.
            Some(mask) => {
                content(mask).next().is_none()
                    && self.get(mask, "Background").and_then(|v| v.brush()).is_some_and(|b| b.is_clear())
            }
        }
    }

    /// The theme takes the element away for good once it has played (an intro video, a start
    /// or lock screen): one of its storyboards ends with it faded out or collapsed and none
    /// brings it back.
    pub(super) fn dismissed(&self, el: X<'t, 'i>) -> bool {
        let own = name_of(el);
        let mut trigs = self.t.triggers(el);
        if let Some(s) = self.t.style_of(el) {
            trigs.extend(self.t.triggers(s));
        }
        let (mut away, mut back) = (false, false);
        for tr in trigs {
            if matches!(tag(tr), "DataTrigger" | "MultiDataTrigger") && self.trigger_test(el, tr) == Test::Never {
                continue;
            }
            // Undone when the trigger stops holding: not for good.
            let undone = prop_elem(tr, "ExitActions").is_some_and(|x| content(x).next().is_some());
            for sb in self.t.enter_storyboards(tr) {
                if undone {
                    back |= storyboard_end(sb, own).is_some();
                    continue;
                }
                match storyboard_end(sb, own) {
                    Some(true) => back = true,
                    Some(false) => away = true,
                    None => {}
                }
            }
        }
        away && !back
    }

    /// The value a trigger that holds at rest sets for `prop` on `el` (see `trigger_setter`).
    pub(super) fn trigger_set(&self, el: X<'t, 'i>, prop: &str) -> Option<Value<'t, 'i>> {
        self.t.setter_value(self.trigger_setter(el, prop)?, prop)
    }

    /// The Setter of a trigger that holds at rest for `prop` on `el`: one of the view's own
    /// triggers naming it (`Setter TargetName=…`, a state the view switches the element into,
    /// e.g. `Tag="HorizontalMode"`), else a trigger of its style (a property trigger matching
    /// such a state, `Trigger Property="Tag" Value="HorizontalMode"`, or a data trigger that
    /// always holds). The last one wins, as in WPF. Visibility has its own rules (`visibility`).
    pub(super) fn trigger_setter(&self, el: X<'t, 'i>, prop: &str) -> Option<X<'t, 'i>> {
        let depth = self.trigger_depth.get();
        if prop == "Visibility" || depth >= MAX_NESTED_TRIGGERS {
            return None;
        }
        self.trigger_depth.set(depth + 1);
        let out = self.view_trigger_setter(el, prop).or_else(|| {
            let style = self.t.style_of(el)?;
            let mut out = None;
            for tr in self.t.triggers(style) {
                // The setter first: deciding whether a trigger holds is the costly part.
                let Some(set) = tree::setters(tr)
                    .filter(|c| {
                        attr(*c, "TargetName").is_none() && attr(*c, "Property").is_some_and(|p| same_property(p, prop))
                    })
                    .last()
                else {
                    continue;
                };
                let holds = match tag(tr) {
                    "DataTrigger" | "MultiDataTrigger" => self.trigger_test(el, tr) == Test::Always,
                    _ => self.property_trigger_holds(el, tr),
                };
                if holds {
                    out = Some(set);
                }
            }
            out
        });
        self.trigger_depth.set(depth);
        out
    }

    /// A Setter of the view's own triggers (holding at rest) for `prop` on the named `el`.
    fn view_trigger_setter(&self, el: X<'t, 'i>, prop: &str) -> Option<X<'t, 'i>> {
        let name = name_of(el)?;
        let mut out = None;
        for tr in self.tpl_triggers.get(name)? {
            if !matches!(tag(*tr), "DataTrigger" | "MultiDataTrigger") {
                continue;
            }
            let set = tr.descendants().filter(|c| c.is_element() && tag(*c) == "Setter").rfind(|set| {
                attr(*set, "TargetName").map(str::trim) == Some(name)
                    && attr(*set, "Property").is_some_and(|p| same_property(p, prop))
            });
            if let Some(set) = set
                && self.trigger_test(el, *tr) == Test::Always
            {
                out = Some(set);
            }
        }
        out
    }

    /// A style's `Trigger Property="P" Value="V"` holds at rest: P is set to V on the element
    /// or by one of the view's triggers (runtime properties such as IsFocused have no resting
    /// value, so their triggers don't hold).
    fn property_trigger_holds(&self, el: X<'t, 'i>, tr: X<'t, 'i>) -> bool {
        if tag(tr) != "Trigger" {
            return false;
        }
        let (Some(p), Some(want)) = (attr(tr, "Property"), attr(tr, "Value")) else { return false };
        let p = p.trim();
        if p == "Visibility" {
            return false;
        }
        let got =
            self.t.local(el, p).or_else(|| self.view_trigger_setter(el, p).and_then(|s| self.t.setter_value(s, p)));
        match got {
            Some(Value::Text(t)) => tree::same_value(&t, want),
            Some(Value::Bool(b)) => tree::same_value(if b { "True" } else { "False" }, want),
            Some(Value::Num(n)) => tree::same_value(&n.to_string(), want),
            _ => false,
        }
    }

    /// The Margin a slide-in leaves the element at when its trigger holds whenever the view is
    /// shown (e.g. "the details view is open"): the animation's `To`.
    pub(super) fn resting_margin(&self, el: X<'t, 'i>) -> Option<[f32; 4]> {
        self.resting_to(el, "ThicknessAnimation", "Margin").and_then(tree::parse_sides)
    }

    /// The Opacity a fade leaves the element at when its trigger holds whenever the view is
    /// shown (the main screen's picture brightening, its logo fading out, as the details page
    /// opens).
    pub(super) fn resting_opacity(&self, el: X<'t, 'i>) -> Option<f32> {
        self.resting_to(el, "DoubleAnimation", "Opacity")?.trim().parse::<f32>().ok().map(|o| o.clamp(0.0, 1.0))
    }

    /// The `To` of the last `animation` of the element's `prop` that a trigger holding whenever
    /// the view is shown starts.
    fn resting_to(&self, el: X<'t, 'i>, animation: &str, prop: &str) -> Option<&'t str> {
        let own = name_of(el);
        let mut trigs = self.t.triggers(el);
        if let Some(s) = self.t.style_of(el) {
            trigs.extend(self.t.triggers(s));
        }
        let mut out = None;
        for tr in trigs {
            let holds = match tag(tr) {
                "DataTrigger" | "MultiDataTrigger" => self.trigger_test(el, tr) == Test::Always,
                _ => self.property_trigger_holds(el, tr),
            };
            if !holds {
                continue;
            }
            for sb in self.t.enter_storyboards(tr) {
                for a in sb.descendants().filter(|a| a.is_element() && tag(*a) == animation) {
                    let target = attr(a, "Storyboard.TargetName").map(str::trim);
                    let animates = attr(a, "Storyboard.TargetProperty")
                        .is_some_and(|p| p.trim().trim_end_matches(')').ends_with(prop));
                    if animates
                        && (target.is_none() || target == own)
                        && let Some(to) = attr(a, "To")
                    {
                        out = Some(to);
                    }
                }
            }
        }
        out
    }
}

/// A Setter's `Property` names `prop` (`Grid.Row`, or `FrameworkElement.Margin` for `Margin`).
fn same_property(p: &str, prop: &str) -> bool {
    let p = p.trim();
    p == prop || (!prop.contains('.') && p.rsplit('.').next() == Some(prop))
}
