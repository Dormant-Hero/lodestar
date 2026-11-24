//! The cover look: the game list's item style and template (`ListGameItemStyle` /
//! `GridViewItemTemplate` …), its focus triggers, frame, glow, caption and badges.

use super::*;

/// What an item's focus / selection, hover and "show game titles" triggers switch on.
#[derive(Default)]
struct FocusTriggers {
    /// Elements a focus trigger shows (Visibility, Opacity, an Opacity animation).
    shown: BTreeSet<String>,
    /// Border brushes and thicknesses a focus trigger sets, by element name.
    brushes: BTreeMap<String, LookBrush>,
    thick: BTreeMap<String, f32>,
    /// The ScaleTransform a focus storyboard grows the most, and to what scale.
    scale_to: Option<(String, f32)>,
    hover_shown: BTreeSet<String>,
    /// What Playnite's "show game titles" setting switches on, with an Opacity it sets.
    setting_shown: BTreeMap<String, Option<f32>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TriggerKind {
    None,
    Focus,
    Hover,
}

impl<'a, 'i> X<'a, 'i> {
    /// The cover look from an item container style and an item template.
    pub(super) fn item_look(&self, style_key: &str, template_key: &str, focus_props: &[&str]) -> ItemLook {
        let mut look = ItemLook::default();
        let style = self.t.resource(style_key).filter(|n| tag(*n) == "Style");
        let tpl = style.and_then(|s| self.t.template(s));
        let item_tpl = self.t.template_of(template_key);
        let root = tpl.and_then(|t| self.t.template_root(t));
        let owners: Vec<N> = [style, tpl, item_tpl].into_iter().flatten().collect();
        let ft = self.focus_triggers(&owners, focus_props);
        let scopes: Vec<N> = [tpl, item_tpl].into_iter().flatten().collect();

        if let Some((frame, glow)) = self.focus_frame(&ft, &scopes) {
            look.focus_glow = glow;
            look.focus_frame = Some(frame);
        }
        look.frame = self.resting_frame(&ft, &scopes);
        // Focus scale, relative to the ScaleTransform's resting scale.
        if let Some((target, to)) = &ft.scale_to {
            let base = self
                .named_in(&scopes, target)
                .and_then(|st| attr(st, "ScaleX"))
                .and_then(|v| v.trim().parse::<f32>().ok())
                .filter(|v| *v > 0.0)
                .unwrap_or(1.0);
            let s = to / base;
            if (0.5..=3.0).contains(&s) && (s - 1.0).abs() > 0.001 {
                look.focus_scale = Some(s);
            }
        }
        // Resting opacity, spacing, background and size of an item.
        if let Some(r) = root {
            look.unfocused_opacity = self.num(r, "Opacity").filter(|o| *o < 0.99 && *o > 0.0);
            look.spacing = match self.t.get(r, "Margin") {
                Some(v @ (Value::Thickness(_) | Value::Num(_))) => v.sides(),
                _ => None,
            };
            look.background = self.brush(r, "Background");
            let w = self.t.get(r, "Width").and_then(|v| v.num());
            let h = self.t.get(r, "Height").and_then(|v| v.num());
            if let (Some(w), Some(h)) = (w, h) {
                look.size = Some([w as f32, h as f32]);
            }
        }
        if look.background.is_none()
            && let Some(s) = style
        {
            look.background = self.t.setter(s, "Background").and_then(|v| v.brush()).filter(|b| !b.is_clear());
        }
        let cover = self.named_in(&scopes, "PART_ImageCover");
        if let Some(cover) = cover {
            let (radius, mask) = self.cover_shape(cover, tpl, &scopes);
            look.cover_radius = (radius > 0.0).then_some(radius);
            look.cover_mask = mask;
        }
        look.caption = self.caption(&scopes, cover, &ft.shown, &ft.hover_shown, &ft.setting_shown);
        self.scrims(&mut look, &owners, &scopes, cover, focus_props);
        look
    }

    /// The first element named `name` in one of `scopes`.
    fn named_in(&self, scopes: &[N<'a, 'i>], name: &str) -> Option<N<'a, 'i>> {
        scopes.iter().find_map(|s| self.t.named(*s, name))
    }

    /// What the item's focus / selection, hover and "show game titles" triggers switch on.
    fn focus_triggers(&self, owners: &[N<'a, 'i>], focus_props: &[&str]) -> FocusTriggers {
        let mut ft = FocusTriggers::default();
        for owner in owners {
            for trig in self.t.triggers(*owner) {
                let kind = self.trigger_kind(trig, focus_props);
                if self.shows_titles(trig) {
                    for set in content(trig).filter(|s| tag(*s) == "Setter") {
                        let (Some(target), Some(p)) = (attr(set, "TargetName"), attr(set, "Property")) else {
                            continue;
                        };
                        let v = self.t.setter_value(set, p);
                        match p.rsplit('.').next().unwrap_or(p) {
                            "Visibility" if v.as_ref().and_then(|v| v.text()) == Some("Visible") => {
                                ft.setting_shown.entry(target.trim().to_string()).or_default();
                            }
                            "Opacity" => {
                                if let Some(o) = v.and_then(|v| v.num()) {
                                    *ft.setting_shown.entry(target.trim().to_string()).or_default() = Some(o as f32);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                if kind == TriggerKind::None {
                    continue;
                }
                for set in content(trig).filter(|s| tag(*s) == "Setter") {
                    let (Some(target), Some(p)) = (attr(set, "TargetName"), attr(set, "Property")) else {
                        continue;
                    };
                    let target = target.trim().to_string();
                    let v = self.t.setter_value(set, p);
                    let p = p.rsplit('.').next().unwrap_or(p);
                    match (kind, p) {
                        (TriggerKind::Focus, "Visibility") if v.as_ref().and_then(|v| v.text()) == Some("Visible") => {
                            ft.shown.insert(target);
                        }
                        (TriggerKind::Focus, "Opacity")
                            if v.as_ref().and_then(|v| v.num()).is_some_and(|o| o > 0.0) =>
                        {
                            ft.shown.insert(target);
                        }
                        (TriggerKind::Focus, "BorderBrush" | "Stroke") => {
                            if let Some(b) = v.and_then(|v| v.brush()).filter(|b| !b.is_clear()) {
                                ft.brushes.insert(target, b);
                            }
                        }
                        (TriggerKind::Focus, "BorderThickness" | "StrokeThickness") => {
                            if let Some(w) = v.and_then(|v| v.max_side()) {
                                ft.thick.insert(target, w);
                            }
                        }
                        (TriggerKind::Hover, "Visibility") if v.as_ref().and_then(|v| v.text()) == Some("Visible") => {
                            ft.hover_shown.insert(target);
                        }
                        _ => {}
                    }
                }
                if kind == TriggerKind::Focus {
                    for sb in self.t.enter_storyboards(trig) {
                        for anim in sb.descendants().filter(|a| a.is_element() && tag(*a).ends_with("Animation")) {
                            let target = attr(anim, "Storyboard.TargetName").unwrap_or("").trim().to_string();
                            let prop = attr(anim, "Storyboard.TargetProperty").unwrap_or("");
                            let to = attr(anim, "To").and_then(|v| v.trim().parse::<f32>().ok());
                            let Some(to) = to else { continue };
                            if prop.contains("ScaleX") {
                                if ft.scale_to.as_ref().is_none_or(|(_, s)| to > *s) {
                                    ft.scale_to = Some((target, to));
                                }
                            } else if prop.trim() == "Opacity" && to > 0.0 && !target.is_empty() {
                                ft.shown.insert(target);
                            }
                        }
                    }
                }
            }
        }
        ft
    }

    /// The focus frame: a frame the focus trigger shows or recolours (best candidate by name
    /// and by being recoloured), with its glow.
    fn focus_frame(&self, ft: &FocusTriggers, scopes: &[N<'a, 'i>]) -> Option<(Frame, Option<Glow>)> {
        let mut cands: Vec<(u8, N)> = vec![];
        for name in ft.shown.iter().chain(ft.brushes.keys()).chain(ft.thick.keys()) {
            let Some(el) = self.named_in(scopes, name) else { continue };
            if !matches!(tag(el), "Border" | "Rectangle") {
                continue;
            }
            let lname = name.to_ascii_lowercase();
            let score = ft.brushes.contains_key(name) as u8 * 4
                + (lname.contains("border") || lname.contains("focus") || lname.contains("select")) as u8 * 2
                + (!lname.contains("flash") && !lname.contains("shine")) as u8;
            cands.push((score, el));
        }
        cands.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
        for (_, el) in &cands {
            let name = name_of(*el).unwrap_or("");
            if let Some(f) = self.frame_of(*el, ft.brushes.get(name).cloned(), ft.thick.get(name).copied()) {
                return Some((f, self.effect_glow(*el).filter(|g| g.opacity > 0.0)));
            }
        }
        None
    }

    /// A frame every item has (an empty, painted Border / Rectangle the focus triggers leave
    /// alone).
    fn resting_frame(&self, ft: &FocusTriggers, scopes: &[N<'a, 'i>]) -> Option<Frame> {
        for scope in scopes {
            for el in scope.descendants().filter(|n| n.is_element() && matches!(tag(*n), "Border" | "Rectangle")) {
                let name = name_of(el).unwrap_or("");
                if ft.shown.contains(name) || ft.brushes.contains_key(name) || self.hidden_up_to(el, *scope).is_some() {
                    continue;
                }
                if content(el).next().is_some() {
                    continue;
                }
                if let Some(f) = self.frame_of(el, None, None)
                    && f.brush.is_some()
                {
                    return Some(f);
                }
            }
        }
        None
    }

    /// The cover's rounding (Borders, VisualBrush masks and clips around it, else a mask on
    /// the item container) and the first other opacity mask on the way.
    fn cover_shape(&self, cover: N<'a, 'i>, tpl: Option<N<'a, 'i>>, scopes: &[N<'a, 'i>]) -> (f32, Option<Mask>) {
        let mut radius: f32 = 0.0;
        let mut mask = None;
        for a in cover.ancestors().filter(|a| a.is_element() && !tag(*a).contains('.')) {
            if tag(a) == "ControlTemplate" {
                break;
            }
            if a != cover
                && tag(a) == "Border"
                && let Some(r) = self.t.get(a, "CornerRadius").and_then(|v| v.max_side())
            {
                radius = radius.max(r);
            }
            if let Some(m) = self.t.get(a, "OpacityMask") {
                match m {
                    Value::Node(vb) if tag(vb) == "VisualBrush" => {
                        if let Some(r) = self.visual_brush_radius(vb, scopes) {
                            radius = radius.max(r);
                        }
                    }
                    v => {
                        if mask.is_none() {
                            mask = v.brush().and_then(mask_of);
                        }
                    }
                }
            }
            if let Some(Value::Node(g)) = self.t.get(a, "Clip")
                && tag(g) == "RectangleGeometry"
                && let Some(r) = self.num(g, "RadiusX")
            {
                radius = radius.max(r);
            }
        }
        // A mask on the item container (around the ContentPresenter that holds the cover).
        if radius == 0.0
            && let Some(t) = tpl
        {
            for el in t.descendants().filter(|n| n.is_element() && !tag(*n).contains('.')) {
                if let Some(Value::Node(vb)) = self.t.local(el, "OpacityMask")
                    && tag(vb) == "VisualBrush"
                    && el.descendants().any(|d| d.is_element() && tag(d) == "ContentPresenter")
                    && let Some(r) = self.visual_brush_radius(vb, scopes)
                {
                    radius = radius.max(r);
                }
            }
        }
        (radius, mask)
    }

    /// Scrims: what darkens a game that isn't installed (IsInstalled = False) and a hovered
    /// one.
    fn scrims(
        &self,
        look: &mut ItemLook,
        owners: &[N<'a, 'i>],
        scopes: &[N<'a, 'i>],
        cover: Option<N<'a, 'i>>,
        focus_props: &[&str],
    ) {
        for owner in owners {
            for trig in self.t.triggers(*owner) {
                let installed_false = self.condition_is(trig, "IsInstalled", "False");
                let hover = self.trigger_kind(trig, focus_props) == TriggerKind::Hover;
                if !installed_false && !hover {
                    continue;
                }
                for set in content(trig).filter(|s| tag(*s) == "Setter") {
                    let (Some(target), Some(p)) = (attr(set, "TargetName"), attr(set, "Property")) else {
                        continue;
                    };
                    let el = self.named_in(scopes, target.trim());
                    let v = self.t.setter_value(set, p);
                    let colour = match p {
                        "Visibility" if v.as_ref().and_then(|v| v.text()) == Some("Visible") => el.and_then(|e| {
                            // The layer's own Opacity dims its colour.
                            let c = self.brush(e, "Background")?.average()?;
                            let o = self.num(e, "Opacity").unwrap_or(1.0).clamp(0.0, 1.0);
                            Some(Color([c.0[0], c.0[1], c.0[2], (c.0[3] as f32 * o).round() as u8]))
                        }),
                        "Opacity" if el == cover && cover.is_some() => v
                            .and_then(|v| v.num())
                            .filter(|o| *o < 1.0)
                            .map(|o| Color([0, 0, 0, ((1.0 - o) * 255.0).round() as u8])),
                        _ => None,
                    };
                    if let Some(c) = colour.filter(|c| c.0[3] > 0) {
                        if installed_false && look.uninstalled_scrim.is_none() {
                            look.uninstalled_scrim = Some(c);
                        } else if hover && look.hover_scrim.is_none() {
                            look.hover_scrim = Some(c);
                        }
                    }
                }
            }
        }
    }

    /// The corner radius of the Border a VisualBrush mask draws (`Visual="{Binding
    /// ElementName=Mask}"` or an inline `<VisualBrush.Visual><Border CornerRadius=…>`).
    pub(super) fn visual_brush_radius(&self, vb: N<'a, 'i>, scopes: &[N<'a, 'i>]) -> Option<f32> {
        let visual = match self.t.local(vb, "Visual") {
            Some(Value::Binding(b)) => {
                let name = b.split(',').find_map(|p| p.trim().strip_prefix("ElementName="))?.trim().to_string();
                scopes.iter().find_map(|s| self.t.named(*s, &name))?
            }
            Some(Value::Node(n)) => n,
            _ => return None,
        };
        std::iter::once(visual)
            .chain(visual.descendants())
            .filter(|n| n.is_element() && tag(*n) == "Border")
            .find_map(|b| self.t.get(b, "CornerRadius")?.max_side())
            .filter(|r| *r > 0.0)
    }

    /// The game-name text of an item and how it's shown.
    pub(super) fn caption(
        &self,
        scopes: &[N<'a, 'i>],
        cover: Option<N<'a, 'i>>,
        shown: &BTreeSet<String>,
        hover: &BTreeSet<String>,
        setting: &BTreeMap<String, Option<f32>>,
    ) -> Option<Caption> {
        let mut best: Option<(u8, Caption)> = None;
        for scope in scopes {
            for el in scope.descendants().filter(|n| n.is_element() && tag(*n) == "TextBlock") {
                if !self.t.local(el, "Text").as_ref().is_some_and(binds_name) {
                    continue;
                }
                let vis = self.t.get(el, "Visibility");
                let hid = self.hidden_up_to(el, *scope);
                let named_up = |set: &BTreeSet<String>| {
                    std::iter::once(el).chain(el.ancestors()).filter_map(|a| name_of(a)).any(|n| set.contains(n))
                };
                let (shown_how, rank) = match (&vis, hid) {
                    (Some(Value::Setting(s)), None) if s.contains("Title") || s.contains("Name") => (Shown::Setting, 3),
                    (_, Some(_))
                        if std::iter::once(el)
                            .chain(el.ancestors())
                            .filter_map(|a| name_of(a))
                            .any(|n| setting.contains_key(n)) =>
                    {
                        (Shown::Setting, 3)
                    }
                    (_, Some(_)) if named_up(shown) => (Shown::Focus, 2),
                    (_, Some(_)) if named_up(hover) => (Shown::Hover, 1),
                    (_, Some(_)) => continue,
                    _ => (Shown::Always, 3),
                };
                // Overlay: the text shares a Grid cell with the cover.
                // (A title in the item container over the cover from the item template: the
                // ContentPresenter that holds the template stands for the cover.)
                let overlay = cover.is_none_or(|c| {
                    if c.ancestors().any(|a| a == *scope) {
                        return self.same_cell(el, c);
                    }
                    scope
                        .descendants()
                        .find(|n| n.is_element() && tag(*n) == "ContentPresenter")
                        .is_some_and(|p| self.same_cell(el, p))
                });
                // The strip behind the text: the text's own background or a Border around it.
                let mut background = self.brush(el, "Background");
                let mut strip = el;
                // Up through single-child wrappers (the strip), not into the item's root.
                for a in el.ancestors().filter(|a| a.is_element() && !tag(*a).contains('.')).skip(1) {
                    if Some(a) == cover
                        || scopes.contains(&a)
                        || cover.is_some_and(|c| a.descendants().any(|d| d == c))
                        || content(a).count() > 1
                    {
                        break;
                    }
                    strip = a;
                    if background.is_none() && matches!(tag(a), "Border" | "Grid") {
                        background = self.brush(a, "Background");
                    }
                }
                // A separate strip layer the same setting shows (a childless Border).
                if background.is_none() {
                    background = setting.iter().find_map(|(name, opacity)| {
                        let layer = scopes.iter().find_map(|s| self.t.named(*s, name))?;
                        if tag(layer) != "Border" || content(layer).next().is_some() {
                            return None;
                        }
                        let b = self.brush(layer, "Background")?;
                        let o = opacity.or_else(|| self.num(layer, "Opacity")).unwrap_or(1.0).clamp(0.0, 1.0);
                        Some(b.with_opacity(o))
                    });
                }
                let size_raw = attr(el, "FontSize").map(markup);
                let cap = Caption {
                    shown: shown_how,
                    overlay,
                    background,
                    foreground: self.brush(el, "Foreground"),
                    align: self
                        .text(el, "TextAlignment")
                        .and_then(|t| Align::parse(&t))
                        .or_else(|| self.align(el, "HorizontalAlignment")),
                    valign: self.align(strip, "VerticalAlignment").or_else(|| self.align(el, "VerticalAlignment")),
                    font_size: self.num(el, "FontSize"),
                    font_size_key: match size_raw {
                        Some(Markup::Resource(k)) => Some(k),
                        _ => None,
                    },
                };
                if best.as_ref().is_none_or(|(r, _)| rank > *r) {
                    best = Some((rank, cap));
                }
            }
        }
        best.map(|(_, c)| c)
    }

    /// Whether two elements sit in the same cell of their nearest common Grid (so one is drawn
    /// over the other) rather than stacked.
    pub(super) fn same_cell(&self, a: N<'a, 'i>, b: N<'a, 'i>) -> bool {
        let anc_b: Vec<N> = b.ancestors().collect();
        let Some(common) = a.ancestors().find(|x| anc_b.contains(x)) else {
            return false;
        };
        if !matches!(tag(common), "Grid" | "Canvas" | "Border" | "ContentControl" | "Viewbox") {
            return false;
        }
        let child = |n: N<'a, 'i>| n.ancestors().find(|x| x.parent() == Some(common));
        let (Some(ca), Some(cb)) = (child(a), child(b)) else {
            return true;
        };
        let cell = |n: N| (grid_attr(n, "Grid.Row", 0), grid_attr(n, "Grid.Column", 0));
        let span = |n: N| grid_attr(n, "Grid.RowSpan", 1).max(grid_attr(n, "Grid.ColumnSpan", 1));
        cell(ca) == cell(cb) || span(ca) > 1 || span(cb) > 1
    }

    /// A trigger on Playnite's "show game titles" setting being on.
    pub(super) fn shows_titles(&self, trig: N<'a, 'i>) -> bool {
        let on = |n: N| {
            attr(n, "Binding").is_some_and(|b| matches!(markup(b), Markup::Settings(s) if s.contains("ShowGameTitles")))
                && attr(n, "Value").is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
        };
        on(trig) || prop_elem(trig, "Conditions").is_some_and(|c| content(c).any(on))
    }

    pub(super) fn trigger_kind(&self, trig: N<'a, 'i>, focus_props: &[&str]) -> TriggerKind {
        let mut props: Vec<(String, String)> = vec![];
        if let (Some(p), Some(v)) = (attr(trig, "Property"), attr(trig, "Value")) {
            props.push((p.to_string(), v.to_string()));
        }
        if let Some(c) = prop_elem(trig, "Conditions") {
            for cond in content(c) {
                if let (Some(p), Some(v)) = (attr(cond, "Property"), attr(cond, "Value")) {
                    props.push((p.to_string(), v.to_string()));
                }
            }
        }
        let is_true = |v: &str| v.trim().eq_ignore_ascii_case("true");
        if props.iter().any(|(p, v)| focus_props.contains(&p.rsplit('.').next().unwrap_or(p)) && is_true(v)) {
            return TriggerKind::Focus;
        }
        if props.iter().any(|(p, v)| p.ends_with("IsMouseOver") && is_true(v)) {
            return TriggerKind::Hover;
        }
        TriggerKind::None
    }

    /// A DataTrigger / MultiDataTrigger condition `{Binding path} = value`.
    pub(super) fn condition_is(&self, trig: N<'a, 'i>, path: &str, value: &str) -> bool {
        let check = |n: N| {
            let binds = attr(n, "Binding").is_some_and(|b| match markup(b) {
                Markup::Binding(b) => {
                    b.split(',').next().unwrap_or("").trim().trim_start_matches("Path=").trim() == path
                }
                _ => false,
            });
            binds && attr(n, "Value").is_some_and(|v| v.trim().eq_ignore_ascii_case(value))
        };
        if check(trig) {
            return true;
        }
        prop_elem(trig, "Conditions").is_some_and(|c| content(c).any(check))
    }
}
