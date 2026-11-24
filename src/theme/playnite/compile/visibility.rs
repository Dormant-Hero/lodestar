//! When an element shows: Visibility as written, `{Settings …}` values, DataTriggers on game
//! properties (→ `visible_if` / `hidden_if`), the resting state of triggers on the
//! controller's state, and the storyboards that fade elements in or play them away.

use super::bind::{bind_art, game_false, game_key, parse_binding};
use super::build::later_list_item;
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

/// A trigger that only reads Playnite's game list layout settings (horizontal layout, rows,
/// columns, item spacing).
pub(super) fn list_settings_trigger(tr: X) -> bool {
    let bindings: Vec<&str> = std::iter::once(tr)
        .chain(prop_elem(tr, "Conditions").into_iter().flat_map(content))
        .filter_map(|c| attr(c, "Binding"))
        .collect();
    !bindings.is_empty()
        && bindings.iter().all(|b| match tree::markup(b) {
            Markup::Settings(s) => {
                let s = s.split(',').next().unwrap_or("").trim();
                let s = s.strip_prefix("Fullscreen.").unwrap_or(s);
                matches!(
                    s,
                    "HorizontalLayout" | "Rows" | "Columns" | "FullscreenItemSpacing" | "FullscreenItemSpacingMargin"
                )
            }
            _ => false,
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
        match tree::markup(binding) {
            Markup::Settings(s) => match setting(&s) {
                Some(SVal::Bool(b)) => match truthy(value) {
                    Some(v) => {
                        if v == b {
                            Test::Always
                        } else {
                            Test::Never
                        }
                    }
                    None => Test::Never,
                },
                Some(SVal::Num(n)) => {
                    if value.trim().parse::<f32>().ok() == Some(n) {
                        Test::Always
                    } else {
                        Test::Never
                    }
                }
                Some(SVal::Text(t)) => {
                    if t.eq_ignore_ascii_case(value.trim()) {
                        Test::Always
                    } else {
                        Test::Never
                    }
                }
                None => Test::Ui,
            },
            // Add-ons: the features Lodestar provides (logos, theme options) are there, every
            // other add-on is missing (its settings unset, its status "not installed").
            Markup::Plugin(p) => match settings::plugin(&p) {
                settings::Plugin::Value(v) => match settings::plugin_text(&p, &v) {
                    Some(got) if tree::same_value(&got, value) => Test::Always,
                    _ => Test::Never,
                },
                settings::Plugin::Fallback(f) => {
                    if tree::same_value(&f, value) {
                        Test::Always
                    } else {
                        Test::Never
                    }
                }
                settings::Plugin::PerGame(k) => match truthy(value) {
                    Some(true) => Test::Set(k.into()),
                    Some(false) => Test::Unset(k.into()),
                    None if is_null(value) => Test::Unset(k.into()),
                    None => Test::Ui,
                },
                settings::Plugin::Missing if p.starts_with("PluginStatus") => {
                    if truthy(value) == Some(false) {
                        Test::Always
                    } else {
                        Test::Never
                    }
                }
                settings::Plugin::Missing => {
                    if is_null(value) {
                        Test::Always
                    } else {
                        Test::Never
                    }
                }
            },
            Markup::Binding(raw) => {
                let b = parse_binding(&raw);
                // A constant the theme keeps in a Tag (theme options): `{Binding Tag,
                // RelativeSource={RelativeSource Self}}` / `ElementName=X, Path=Tag`.
                if b.path.trim().trim_start_matches("Path=") == "Tag" && b.converter.is_none() {
                    let owner = if raw.contains("Self") {
                        Some(el)
                    } else {
                        b.element.as_deref().and_then(|n| self.root.and_then(|r| self.t.named(r, n)))
                    };
                    if let Some(o) = owner
                        && let Some(v) = self.get(o, "Tag")
                    {
                        let same = match v {
                            Value::Bool(x) => truthy(value) == Some(x),
                            Value::Num(x) => value.trim().parse::<f64>().ok() == Some(x),
                            Value::Text(t) => t.trim().eq_ignore_ascii_case(value.trim()),
                            Value::Null => is_null(value),
                            Value::Binding(inner) if !inner.contains("Tag") => {
                                return self.test(el, &format!("{{Binding {inner}}}"), value);
                            }
                            Value::Setting(s) => match setting(&s) {
                                Some(SVal::Bool(x)) => truthy(value) == Some(x),
                                _ => return Test::Ui,
                            },
                            Value::Plugin(p) => return self.test(el, &format!("{{{p}}}"), value),
                            _ => return Test::Ui,
                        };
                        return if same { Test::Always } else { Test::Never };
                    }
                    if owner.is_some() {
                        return if is_null(value) { Test::Always } else { Test::Never };
                    }
                }
                if let Some(name) = &b.element {
                    // The details view is open while GameDetails is shown and closed on Main:
                    // its entrance (slide-ins, fades) has played, its exit hasn't.
                    if name == "PART_ElemGameDetails" && b.path.trim().trim_start_matches("Path=") == "Visibility" {
                        return match truthy(value) {
                            Some(v) if v != self.main => Test::Always,
                            Some(_) => Test::Never,
                            None => Test::Ui,
                        };
                    }
                    if name == "PART_ListGameItems" && b.path.trim() == "Items.Count" {
                        // There are games.
                        return if value.trim() == "0" { Test::Never } else { Test::Ui };
                    }
                    let path = b.path.trim().trim_start_matches("Path=");
                    if matches!(path, "ActualHeight" | "ActualWidth" | "IsVisible")
                        && let Some(owner) = self.root.and_then(|r| self.t.named(r, name))
                    {
                        return self.size_test(owner, path, value);
                    }
                    // Another element's property the theme sets to a constant or a setting
                    // (`ChangeViewButton.IsChecked = {Settings Fullscreen.HorizontalLayout}`).
                    return match self.t.static_binding(el, binding) {
                        Some(got) => {
                            if tree::same_value(&got, value) {
                                Test::Always
                            } else {
                                Test::Never
                            }
                        }
                        None => Test::Ui,
                    };
                }
                if b.relative || b.converter.is_some() && !b.converter.as_deref().unwrap_or("").contains("Null") {
                    return Test::Ui;
                }
                if game_false(&b.path) {
                    return if truthy(value) == Some(false) { Test::Always } else { Test::Never };
                }
                if later_list_item(binding) {
                    return if is_null(value) || truthy(value) == Some(false) { Test::Always } else { Test::Never };
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
            Markup::Other(o) if o.starts_with("Api ") => Test::Ui,
            _ => Test::Ui,
        }
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
            Some(Value::Binding(raw)) => {
                let b = parse_binding(&raw);
                let logo = name_of(owner).is_some_and(|n| n.to_ascii_lowercase().contains("logo"))
                    || b.path.to_ascii_lowercase().contains("logo");
                if logo { Some("{art.logo}") } else { bind_art(&b) }
            }
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
        let mut shown = match self.get(el, "Visibility") {
            Some(Value::Text(t)) => match t.to_ascii_lowercase().as_str() {
                "collapsed" => Vis::Collapsed,
                "hidden" => Vis::Hidden,
                _ => Vis::Visible,
            },
            Some(Value::Setting(s)) => match setting(&s) {
                Some(SVal::Bool(false)) => Vis::Collapsed,
                _ => Vis::Visible,
            },
            Some(Value::Binding(raw)) => {
                let b = parse_binding(&raw);
                let conv = b.converter.clone().unwrap_or_default();
                let own_tag = b.path.trim() == "Tag" && raw.contains("Self");
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
                match settings::plugin(&p) {
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
            match self.trigger_test(el, tr) {
                // A note shown for Playnite's own list layout settings ("turn off horizontal
                // scrolling"): Lodestar lays the list out itself and has no such settings.
                Test::Always if to == Vis::Visible && list_settings_trigger(tr) => {
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
        (shown, cond)
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

    /// Its OpacityMask is a VisualBrush of an element the view doesn't have.
    pub(super) fn masked_by_nothing(&self, el: X<'t, 'i>) -> bool {
        let Some(Value::Node(vb)) = self.get(el, "OpacityMask") else { return false };
        if tag(vb) != "VisualBrush" {
            return false;
        }
        let Some(Markup::Binding(raw)) = attr(vb, "Visual").map(tree::markup) else { return false };
        parse_binding(&raw).element.is_some_and(|n| self.root.and_then(|r| self.t.named(r, &n)).is_none())
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

    /// The Margin a slide-in leaves the element at when its trigger holds whenever the view is
    /// shown (e.g. "the details view is open"): the animation's `To`.
    pub(super) fn resting_margin(&self, el: X<'t, 'i>) -> Option<[f32; 4]> {
        let own = name_of(el);
        let mut trigs = self.t.triggers(el);
        if let Some(s) = self.t.style_of(el) {
            trigs.extend(self.t.triggers(s));
        }
        let mut out = None;
        for tr in trigs {
            if !matches!(tag(tr), "DataTrigger" | "MultiDataTrigger") || self.trigger_test(el, tr) != Test::Always {
                continue;
            }
            for sb in self.t.enter_storyboards(tr) {
                for a in sb.descendants().filter(|a| a.is_element() && tag(*a) == "ThicknessAnimation") {
                    let target = attr(a, "Storyboard.TargetName").map(str::trim);
                    let margin = attr(a, "Storyboard.TargetProperty").is_some_and(|p| p.trim().ends_with("Margin"));
                    if margin
                        && (target.is_none() || target == own)
                        && let Some(to) = attr(a, "To").and_then(tree::parse_sides)
                    {
                        out = Some(to);
                    }
                }
            }
        }
        out
    }
}
