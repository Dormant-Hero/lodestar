//! The desktop game list's rows (DetailsViewItemStyle): how the selected row is marked (a
//! fill, a bar along its left edge, its text) and the text of installed and uninstalled games.

use super::*;

/// A selection element at most this wide, along the row's left edge, is a bar, not a fill.
const BAR_MAX_WIDTH: f32 = 8.0;

impl<'a, 'i> X<'a, 'i> {
    /// The rows as the item style draws them at rest and selected. Playnite's own style marks
    /// the selected row with a 5 px GlyphBrush bar and GlyphBrush text.
    pub(super) fn list_rows(&self) -> Option<ListRows> {
        let style = self.t.resource("DetailsViewItemStyle").filter(|n| tag(*n) == "Style")?;
        let tpl = self.t.template(style)?;
        // WPF ranks a Style's triggers above its template's, and a later trigger above an
        // earlier one: with the template's first, the last trigger setting a value wins.
        let triggers: Vec<N> = self.t.triggers(tpl).into_iter().chain(self.t.triggers(style)).collect();
        let last_foreground = |matches: &dyn Fn(N) -> bool| {
            triggers.iter().rev().filter(|t| matches(**t)).find_map(|t| self.trigger_foreground(*t))
        };
        let installed = |want: &str| {
            last_foreground(&|t| tag(t) == "DataTrigger" && binds_path(t, "IsInstalled") && value_is(t, want))
        };
        let base = self.t.setter(style, "Foreground").and_then(|v| v.brush());
        let mut rows = ListRows {
            text: installed("True").or_else(|| base.clone()),
            text_uninstalled: installed("False").or(base),
            selected_text: last_foreground(&is_selected),
            ..Default::default()
        };
        // A theme may show the plate in its template's trigger and colour the text in its
        // style's: every IsSelected trigger counts.
        let selected: Vec<N> = triggers.iter().copied().filter(|t| is_selected(*t)).collect();
        for name in self.shown_by(tpl, &selected) {
            if let Some(el) = self.t.named(tpl, &name) {
                self.selection_mark(el, &selected, &mut rows);
            }
        }
        Some(rows)
    }

    /// The Foreground a trigger sets on the item itself.
    fn trigger_foreground(&self, trig: N<'a, 'i>) -> Option<LookBrush> {
        content(trig)
            .find(|s| tag(*s) == "Setter" && attr(*s, "TargetName").is_none() && prop_is(*s, "Foreground"))
            .and_then(|s| self.t.setter_value(s, "Foreground"))
            .and_then(|v| v.brush())
    }

    /// The template parts the triggers bring out, once each in the order they are met: made
    /// visible, faded in or scaled up (a setter or an enter storyboard), or given a fill or border.
    fn shown_by(&self, tpl: N<'a, 'i>, triggers: &[N<'a, 'i>]) -> Vec<String> {
        let mut names = vec![];
        for trig in triggers {
            names.extend(
                content(*trig)
                    .filter(|s| tag(*s) == "Setter" && shows(*s))
                    .filter_map(|s| attr(s, "TargetName").map(|n| n.trim().to_string())),
            );
            let animated = self.t.enter_storyboards(*trig).into_iter().flat_map(|sb| sb.descendants()).filter(|a| {
                attr(*a, "Storyboard.TargetProperty").is_some_and(|p| p.contains("Opacity") || p.contains("Scale"))
                    && attr(*a, "To").and_then(|v| v.trim().parse::<f32>().ok()).is_some_and(|v| v > 0.0)
            });
            names.extend(animated.filter_map(|a| self.animated_part(tpl, attr(a, "Storyboard.TargetName")?.trim())));
        }
        let mut seen = BTreeSet::new();
        names.retain(|n| seen.insert(n.clone()));
        names
    }

    /// The part an animation's target belongs to: a named transform (`ScaleX` 0 → 1 reveals a
    /// plate) stands for the element it transforms.
    fn animated_part(&self, tpl: N<'a, 'i>, target: &str) -> Option<String> {
        let node = self.t.named(tpl, target)?;
        if !tag(node).ends_with("Transform") {
            return Some(target.to_string());
        }
        let owner = node
            .ancestors()
            .skip(1)
            .find(|a| a.is_element() && !tag(*a).contains('.') && !tag(*a).ends_with("Transform"))?;
        name_of(owner).map(str::to_string)
    }

    /// What a part shown on selection draws: a thin part on the left edge (or a left-only
    /// border) is the bar, anything else with a fill is the row's fill.
    fn selection_mark(&self, el: N<'a, 'i>, sel: &[N<'a, 'i>], rows: &mut ListRows) {
        let name = name_of(el);
        let set = |prop: &str| {
            sel.iter()
                .rev()
                .flat_map(|t| content(*t))
                .find(|s| tag(*s) == "Setter" && attr(*s, "TargetName").map(str::trim) == name && prop_is(*s, prop))
                .and_then(|s| self.t.setter_value(s, prop))
                .and_then(|v| v.brush())
                .filter(|b| !b.is_clear())
        };
        // A Grid holding a plain Border paints with that Border.
        let painter = std::iter::once(el).chain(content(el).filter(|c| tag(*c) == "Border")).collect::<Vec<_>>();
        let fill = set(fill_prop(el))
            .or_else(|| painter.iter().find_map(|p| self.brush(*p, fill_prop(*p))))
            .map(|b| b.with_opacity(self.selected_opacity(el, sel)));
        let sides = self.t.get(el, "BorderThickness").and_then(|v| v.sides()).unwrap_or([0.0; 4]);
        let left_only = sides[0] > 0.0 && sides[1..].iter().all(|s| *s == 0.0);
        let thin = self.num(el, "Width").is_some_and(|w| w <= BAR_MAX_WIDTH)
            && self.align(el, "HorizontalAlignment") == Some(Align::Start);
        if left_only && rows.selected_bar.is_none() {
            let brush = set("BorderBrush").or_else(|| self.brush(el, "BorderBrush"));
            rows.selected_bar = brush.map(|b| (b, sides[0]));
        } else if thin && rows.selected_bar.is_none() {
            rows.selected_bar = fill.zip(self.num(el, "Width"));
            return;
        }
        if rows.selected_fill.is_none() && fill.is_some() {
            rows.selected_fill = fill;
            rows.radius = painter.iter().find_map(|p| self.t.get(*p, "CornerRadius").and_then(|v| v.max_side()));
        }
    }

    /// A part's opacity while selected: what the triggers set or fade it to, else its own
    /// (a part resting invisible and only shown by a trigger is drawn fully).
    fn selected_opacity(&self, el: N<'a, 'i>, sel: &[N<'a, 'i>]) -> f32 {
        let name = name_of(el);
        let targets = |n: N, name_attr: &str, prop_attr: &str| {
            attr(n, name_attr).map(str::trim) == name && attr(n, prop_attr).is_some_and(|p| p.contains("Opacity"))
        };
        let set = sel
            .iter()
            .rev()
            .flat_map(|t| content(*t))
            .filter(|s| tag(*s) == "Setter" && targets(*s, "TargetName", "Property"))
            .find_map(|s| attr(s, "Value")?.trim().parse::<f32>().ok());
        let faded = || {
            sel.iter().rev().flat_map(|t| self.t.enter_storyboards(*t)).flat_map(|sb| sb.descendants()).find_map(|a| {
                let to = || attr(a, "To")?.trim().parse::<f32>().ok();
                targets(a, "Storyboard.TargetName", "Storyboard.TargetProperty").then(to).flatten()
            })
        };
        let own = || self.num(el, "Opacity").filter(|o| *o > 0.01);
        set.or_else(faded).or_else(own).unwrap_or(1.0).clamp(0.0, 1.0)
    }
}

/// A `Trigger` on the item's `IsSelected` (also spelt `Selector.IsSelected`) being true.
fn is_selected(t: N) -> bool {
    tag(t) == "Trigger" && attr(t, "Property").is_some_and(|p| p.trim().ends_with("IsSelected")) && value_is(t, "True")
}

/// A setter that brings a part out: makes it visible or opaque, or gives it a fill or border.
fn shows(s: N) -> bool {
    let value = attr(s, "Value").unwrap_or("").trim();
    (prop_is(s, "Visibility") && value.eq_ignore_ascii_case("visible"))
        || (prop_is(s, "Opacity") && value.parse::<f32>().is_ok_and(|o| o > 0.0))
        || prop_is(s, "Background")
        || prop_is(s, "Fill")
        || prop_is(s, "BorderBrush")
}

/// A setter / trigger names this property (`Border.Background` counts as `Background`).
fn prop_is(n: N, prop: &str) -> bool {
    attr(n, "Property").is_some_and(|p| p.trim().rsplit('.').next() == Some(prop))
}

fn value_is(n: N, want: &str) -> bool {
    attr(n, "Value").is_some_and(|v| v.trim().eq_ignore_ascii_case(want))
}

/// A DataTrigger bound to the game's `path` (`{Binding IsInstalled}`).
fn binds_path(n: N, path: &str) -> bool {
    attr(n, "Binding").is_some_and(|b| match markup(b) {
        Markup::Binding(b) => b.split(',').next().unwrap_or("").trim().trim_start_matches("Path=").trim() == path,
        _ => false,
    })
}
