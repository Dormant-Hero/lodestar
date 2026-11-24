//! The fullscreen Main and GameDetails views: bars, the game list's shape and place, the
//! background picture, the details overlay.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum RowH {
    Px(f32),
    Auto,
    Star(f32),
}

impl<'a, 'i> X<'a, 'i> {
    /// The fullscreen Main view: its background, which status parts it shows, its bars, the
    /// game list's place and shape, the background picture.
    pub(super) fn main_look(&self, item: &ItemLook) -> MainLook {
        let mut m = MainLook::default();
        let Some(tpl) = self.t.template_of("Main") else {
            return m;
        };
        let Some(root) = self.t.template_root(tpl) else {
            return m;
        };
        let host = self.t.named(tpl, "PART_ViewHost").unwrap_or(root);
        m.background = self.brush(host, "Background").or_else(|| self.brush(root, "Background"));
        let present = |name: &str| self.t.named(tpl, name).is_some_and(|n| self.hidden_up_to(n, tpl).is_none());
        m.clock = present("PART_TextClock");
        m.battery = present("PART_ElemBatteryStatus");
        m.battery_percent = present("PART_TextBatteryPercentage");
        m.filter_presets = present("PART_FilterPresetSelector");

        let list = self.t.named(tpl, "PART_ListGameItems");
        // The view's row grid: PART_ViewHost's rows (Playnite's top bar / list / bottom bar),
        // else the nearest one around the list.
        let grid = self.rows_of(host).map(|r| (host, r)).or_else(|| list.and_then(|l| self.row_grid(l)));
        let picture_el = self.t.named(tpl, "PART_ImageBackground");
        m.picture = picture_el.map(|p| self.picture(p, grid.as_ref().map(|(g, r)| (*g, r.len()))));
        if let (Some(list), Some((grid, rows))) = (list, grid) {
            self.place_in_rows(&mut m, tpl, list, grid, &rows, picture_el);
        }
        if let Some(list) = list {
            m.list.margin = self.t.get(list, "Margin").and_then(|v| v.sides());
            m.list.width = self.num(list, "Width");
            m.list.height = self.num(list, "Height");
            m.list.valign = self.align(list, "VerticalAlignment");
        }
        let (shape, evidence) = self.list_shape(tpl, list, item);
        m.list.shape = shape;
        m.list.evidence = evidence;
        m
    }

    /// Main view parts by the view's row grid (`grid` with `rows`, on a 1080 px screen): the
    /// top and bottom bars around the game list's row, the list's area, the background
    /// picture's band.
    fn place_in_rows(
        &self,
        m: &mut MainLook,
        tpl: N<'a, 'i>,
        list: N<'a, 'i>,
        grid: N<'a, 'i>,
        rows: &[RowH],
        picture_el: Option<N<'a, 'i>>,
    ) {
        let child_of = |el: N<'a, 'i>| std::iter::once(el).chain(el.ancestors()).find(|x| x.parent() == Some(grid));
        let row_of = |el: N<'a, 'i>| -> Option<(usize, usize)> {
            let c = child_of(el)?;
            Some((grid_attr(c, "Grid.Row", 0), grid_attr(c, "Grid.RowSpan", 1)))
        };
        let (list_row, list_span) = row_of(list).unwrap_or((0, 1));
        let bar = |names: &[&str], above: bool| -> Option<Bar> {
            for name in names {
                let Some(el) = self.t.named(tpl, name) else { continue };
                if self.hidden_up_to(el, tpl).is_some() {
                    continue;
                }
                let c = child_of(el)?;
                let (r, _) = row_of(el)?;
                if (above && r >= list_row) || (!above && r <= list_row) {
                    continue;
                }
                let height = self
                    .num(c, "Height")
                    .or_else(|| match rows.get(r) {
                        Some(RowH::Px(px)) => Some(*px),
                        _ => None,
                    })
                    .or_else(|| self.fixed_height(c))?;
                let background = self.brush(c, "Background").or_else(|| {
                    content(c)
                        .filter(|k| matches!(tag(*k), "Border" | "Grid" | "Rectangle"))
                        .find_map(|k| self.brush(k, "Background").or_else(|| self.brush(k, "Fill")))
                });
                return Some(Bar { height, background });
            }
            None
        };
        m.top_bar = bar(
            &[
                "PART_TextClock",
                "PART_ButtonMainMenu",
                "PART_FilterPresetSelector",
                "PART_ElemBatteryStatus",
                "PART_TextBatteryPercentage",
                "PART_ButtonNotifications",
            ],
            true,
        );
        m.bottom_bar = bar(
            &[
                "PART_ButtonPlay",
                "PART_ButtonDetails",
                "PART_ButtonGameOptions",
                "PART_ButtonInstall",
                "PART_ButtonSearch",
                "PART_ButtonFilter",
            ],
            false,
        );
        // Row heights on a 1080 px screen (Auto rows: the bars found in them, else 0).
        let auto = |r: usize| {
            if r < list_row {
                m.top_bar.as_ref().map(|b| b.height).unwrap_or(0.0)
            } else if r > list_row {
                m.bottom_bar.as_ref().map(|b| b.height).unwrap_or(0.0)
            } else {
                0.0
            }
        };
        let fixed: f32 = rows
            .iter()
            .enumerate()
            .map(|(r, h)| match h {
                RowH::Px(p) => *p,
                RowH::Auto => auto(r),
                RowH::Star(_) => 0.0,
            })
            .sum();
        let stars: f32 = rows.iter().map(|h| if let RowH::Star(s) = h { *s } else { 0.0 }).sum();
        let free = (DESIGN[1] - fixed).max(0.0);
        let heights: Vec<f32> = rows
            .iter()
            .enumerate()
            .map(|(r, h)| match h {
                RowH::Px(p) => *p,
                RowH::Star(s) => free * s / stars.max(1e-6),
                RowH::Auto => auto(r),
            })
            .collect();
        let band = |row: usize, span: usize| -> (f32, f32) {
            if heights.is_empty() {
                return (0.0, DESIGN[1]);
            }
            let y: f32 = heights.iter().take(row).sum();
            let h: f32 = heights.iter().skip(row).take(span.max(1)).sum();
            (y, h)
        };
        let (y, h) = band(list_row, list_span);
        // The list's own margin when it sits right in the row; nested lists get the row.
        let margin = if list.parent() == Some(grid) {
            self.t.get(list, "Margin").and_then(|v| v.sides()).unwrap_or([0.0; 4])
        } else {
            [0.0; 4]
        };
        // A fixed size sits in its band by its alignment.
        let place = |start: f32, band: f32, size: Option<f32>, m0: f32, m1: f32, al: Option<Align>| {
            let room = (band - m0 - m1).max(0.0);
            match size {
                Some(s) if s < room => match al {
                    Some(Align::Start) => (start + m0, s),
                    Some(Align::End) => (start + band - m1 - s, s),
                    _ => (start + m0 + (room - s) / 2.0, s),
                },
                _ => (start + m0, room),
            }
        };
        let fixed_h = if list.parent() == Some(grid) { self.num(list, "Height") } else { None };
        let fixed_w = if list.parent() == Some(grid) { self.num(list, "Width") } else { None };
        let (ly, lh) = place(y, h, fixed_h, margin[1], margin[3], self.align(list, "VerticalAlignment"));
        let (lx, lw) = place(0.0, DESIGN[0], fixed_w, margin[0], margin[2], self.align(list, "HorizontalAlignment"));
        m.list.area = Some([lx / DESIGN[0], ly / DESIGN[1], lw / DESIGN[0], lh / DESIGN[1]]);
        if let (Some(pic), Some(p)) = (picture_el, m.picture.as_mut())
            && !p.full
            && let Some((r, span)) = row_of(pic)
        {
            let (y, h) = band(r, span);
            p.area = Some([0.0, y / DESIGN[1], 1.0, h / DESIGN[1]]);
        }
    }

    /// The nearest ancestor Grid of `el` with RowDefinitions, and its row heights.
    pub(super) fn row_grid(&self, el: N<'a, 'i>) -> Option<(N<'a, 'i>, Vec<RowH>)> {
        let g = el
            .ancestors()
            .skip(1)
            .find(|a| a.is_element() && tag(*a) == "Grid" && prop_elem(*a, "RowDefinitions").is_some())?;
        Some((g, self.rows_of(g)?))
    }

    /// A Grid's row heights (None: it has no RowDefinitions).
    pub(super) fn rows_of(&self, g: N<'a, 'i>) -> Option<Vec<RowH>> {
        if tag(g) != "Grid" {
            return None;
        }
        let rows = prop_elem(g, "RowDefinitions")?;
        let hs = content(rows)
            .filter(|r| tag(*r) == "RowDefinition")
            .map(|r| {
                let h = match self.t.get(r, "Height") {
                    Some(Value::Num(v)) => return RowH::Px(v as f32),
                    Some(Value::Text(t)) => t,
                    None => "*".into(),
                    _ => "Auto".into(),
                };
                let h = h.trim();
                if h.eq_ignore_ascii_case("auto") {
                    RowH::Auto
                } else if let Some(s) = h.strip_suffix('*') {
                    RowH::Star(if s.is_empty() { 1.0 } else { s.parse().unwrap_or(1.0) })
                } else {
                    h.parse().map(RowH::Px).unwrap_or(RowH::Auto)
                }
            })
            .collect();
        Some(hs)
    }

    /// A fixed Height somewhere at the top of an element's content.
    pub(super) fn fixed_height(&self, el: N<'a, 'i>) -> Option<f32> {
        let mut cur = el;
        for _ in 0..4 {
            if let Some(h) = self.num(cur, "Height") {
                return Some(h);
            }
            let mut kids = content(cur);
            let first = kids.next()?;
            if kids.next().is_some() {
                return None;
            }
            cur = first;
        }
        None
    }

    /// PART_ImageBackground's placement (`grid`: the view's row grid and its row count).
    pub(super) fn picture(&self, p: N<'a, 'i>, grid: Option<(N<'a, 'i>, usize)>) -> BackgroundPicture {
        let span_all = match grid {
            Some((g, rows)) => {
                let c = std::iter::once(p).chain(p.ancestors()).find(|x| x.parent() == Some(g));
                c.is_none_or(|c| grid_attr(c, "Grid.Row", 0) == 0 && grid_attr(c, "Grid.RowSpan", 1) >= rows.max(1))
            }
            None => true,
        };
        let col_span_ok = p
            .parent()
            .and_then(|par| prop_elem(par, "ColumnDefinitions"))
            .is_none_or(|cols| grid_attr(p, "Grid.ColumnSpan", 1) >= content(cols).count().max(1));
        let mask = self
            .t
            .get(p, "ImageOpacityMask")
            .or_else(|| self.t.get(p, "OpacityMask"))
            .and_then(|v| v.brush())
            .and_then(mask_of);
        // A plain translucent layer drawn right after the picture darkens it.
        let darken = p
            .next_siblings()
            .filter(|s| s.is_element() && !tag(*s).contains('.'))
            .take(2)
            .find(|s| matches!(tag(*s), "Border" | "Rectangle" | "Grid") && content(*s).next().is_none())
            .and_then(|s| self.brush(s, "Background").or_else(|| self.brush(s, "Fill")))
            .and_then(|b| match b {
                LookBrush::Solid { color } if color.0[3] < 255 => Some(color),
                _ => None,
            });
        BackgroundPicture {
            full: span_all && col_span_ok,
            halign: self.align(p, "HorizontalAlignment"),
            valign: self.align(p, "VerticalAlignment"),
            stretch: self.text(p, "Stretch").and_then(|s| Stretch::parse(&s)),
            opacity: self.num(p, "Opacity").filter(|o| *o < 0.999),
            mask,
            blur: self.effect_blur(p),
            darken,
            area: None,
        }
    }

    /// One row, a grid or a carousel? Evidence from Playnite's structure only.
    pub(super) fn list_shape(
        &self,
        tpl: N<'a, 'i>,
        list: Option<N<'a, 'i>>,
        item: &ItemLook,
    ) -> (ListShape, Vec<String>) {
        let mut ev = vec![];
        let (mut row, mut carousel, mut grid) = (0i32, 0i32, 0i32);
        let theme_main = self.t.layer_of(tpl) == Layer::Theme;
        if !theme_main {
            ev.push("Playnite's own Main view (a cover grid)".into());
            return (ListShape::Grid, ev);
        }
        let Some(list) = list else {
            ev.push("no PART_ListGameItems: the theme draws its own list".into());
            return (ListShape::Grid, ev);
        };
        let h = self.num(list, "Height");
        if let Some(h) = h
            && h <= 700.0
        {
            row += 2;
            ev.push(format!("the list is {h:.0} px high"));
        }
        if h.is_none_or(|h| h > 700.0)
            && let Some(hh) = self.horizontal_layout_height(tpl, list)
        {
            row += 2;
            ev.push(format!("in Playnite's horizontal layout the list is {hh:.0} px high"));
        }
        if self.num(list, "Width").is_some_and(|w| w > DESIGN[0]) {
            row += 1;
            ev.push("the list is wider than the screen".into());
        }
        if let (Some(h), Some([_, ih])) = (h, item.size)
            && h < ih * 2.0
        {
            row += 1;
            ev.push("the list fits one item".into());
        }
        if let Some(panel) = self.t.get(list, "ItemsPanel").and_then(|v| v.node()) {
            let p = panel.descendants().find(|n| n.is_element() && tag(*n).ends_with("Panel"));
            match p.map(|p| (tag(p), attr(p, "Orientation").unwrap_or(""))) {
                Some(("StackPanel" | "VirtualizingStackPanel", o)) if o.eq_ignore_ascii_case("Horizontal") => {
                    row += 3;
                    ev.push("its items panel is one horizontal line".into());
                }
                Some(("WrapPanel", _)) => {
                    grid += 2;
                    ev.push("its items panel wraps".into());
                }
                _ => {}
            }
        }
        if attr(list, "ScrollViewer.HorizontalScrollBarVisibility").is_some_and(|v| v.trim() == "Hidden")
            && attr(list, "ScrollViewer.VerticalScrollBarVisibility").is_none()
        {
            row += 1;
            ev.push("it scrolls sideways".into());
        }
        // A display-only copy of the list moved by the selected index (carousel).
        let hidden_list = self.hidden(list);
        let mut follows_index = 0;
        let mut carousel_named = false;
        for el in tpl.descendants().filter(|n| n.is_element()) {
            if name_of(el).is_some_and(|n| n.to_ascii_lowercase().contains("carousel")) {
                carousel_named = true;
            }
            for a in el.attributes() {
                let v = a.value();
                if v.contains("PART_ListGameItems")
                    && v.contains("SelectedIndex")
                    && (v.contains("Math") || a.name().ends_with("X"))
                {
                    follows_index += 1;
                }
            }
        }
        if carousel_named {
            carousel += 3;
            ev.push("it has a carousel".into());
        }
        if follows_index > 0 && (hidden_list || carousel_named) {
            carousel += 2;
            ev.push("a copy of the list moves with the selected game".into());
        }
        let text_of = |d: &str| self.t.doc_of(tpl).is_some_and(|doc| doc.doc.input_text().contains(d));
        if text_of("Fullscreen.Rows") || text_of("Fullscreen.Columns") {
            grid += 1;
            ev.push("it follows Playnite's rows / columns setting".into());
        }
        let shape = if carousel >= 3 && carousel >= row {
            ListShape::Carousel
        } else if row >= 2 && row > grid {
            ListShape::Row
        } else {
            ListShape::Grid
        };
        if ev.is_empty() {
            ev.push("nothing says otherwise: a cover grid".into());
        }
        (shape, ev)
    }

    /// The list's Height (under half the screen) set by a style trigger for Playnite's "horizontal layout"
    /// setting: `{Settings Fullscreen.HorizontalLayout} = True` directly, or through an element
    /// whose IsChecked is bound to that setting (a hidden view toggle).
    pub(super) fn horizontal_layout_height(&self, tpl: N<'a, 'i>, list: N<'a, 'i>) -> Option<f32> {
        let is_setting = |v: &str| matches!(markup(v), Markup::Settings(s) if s.split(',').next().is_some_and(|p| p.trim() == "Fullscreen.HorizontalLayout"));
        let horizontal_true = |cond: N<'a, 'i>| -> bool {
            let (Some(b), Some(v)) = (attr(cond, "Binding"), attr(cond, "Value")) else {
                return false;
            };
            if !v.trim().eq_ignore_ascii_case("true") {
                return false;
            }
            if is_setting(b) {
                return true;
            }
            let Markup::Binding(b) = markup(b) else { return false };
            let el = b.split(',').find_map(|p| p.trim().strip_prefix("ElementName=")).map(str::trim);
            let path = b
                .split(',')
                .map(str::trim)
                .find_map(|p| p.strip_prefix("Path="))
                .or_else(|| b.split(',').next().map(str::trim).filter(|p| !p.contains('=')));
            el.zip(path).is_some_and(|(el, path)| {
                path.trim() == "IsChecked"
                    && self.t.named(tpl, el).and_then(|e| attr(e, "IsChecked")).is_some_and(is_setting)
            })
        };
        let style = self.t.style_of(list)?;
        for trig in self.t.triggers(style) {
            let on =
                horizontal_true(trig) || prop_elem(trig, "Conditions").is_some_and(|c| content(c).any(horizontal_true));
            if !on {
                continue;
            }
            for set in content(trig).filter(|s| tag(*s) == "Setter" && attr(*s, "TargetName").is_none()) {
                let Some(p) = attr(set, "Property") else { continue };
                if p.trim().rsplit('.').next() != Some("Height") {
                    continue;
                }
                // Under half the screen: room for one row of covers only.
                if let Some(h) = self.t.setter_value(set, "Height").and_then(|v| v.num()).map(|v| v as f32)
                    && h > 0.0
                    && h <= DESIGN[1] / 2.0
                {
                    return Some(h);
                }
            }
        }
        None
    }

    pub(super) fn details_look(&self) -> DetailsLook {
        let mut d = DetailsLook::default();
        let Some(tpl) = self.t.template_of("GameDetails") else {
            return d;
        };
        let Some(root) = self.t.template_root(tpl) else {
            return d;
        };
        let host = self.t.named(tpl, "PART_ViewHost").unwrap_or(root);
        d.background = self.brush(host, "Background").or_else(|| self.brush(root, "Background"));
        let grid = self.row_grid_of_view(host);
        d.picture = self.t.named(tpl, "PART_ImageBackground").map(|p| self.picture(p, grid));
        // The panel over the picture: a childless Border with a gradient / translucent fill.
        // A faint tint (under ~16 % on average) dims the whole picture a touch; the panel that
        // keeps the text readable is the next, stronger one when there is one.
        let pic = self.t.named(tpl, "PART_ImageBackground");
        let overlays: Vec<LookBrush> = tpl
            .descendants()
            .filter(|n| n.is_element() && tag(*n) == "Border" && content(*n).next().is_none())
            .filter(|n| Some(*n) != pic && self.hidden_up_to(*n, tpl).is_none())
            .filter_map(|n| self.brush(n, "Background"))
            .filter(|b| b.is_gradient() || b.average().is_some_and(|c| c.0[3] < 255))
            .collect();
        let faint = |b: &LookBrush| b.average().is_some_and(|c| c.0[3] < FAINT_OVERLAY);
        d.overlay = overlays.iter().find(|b| !faint(b)).or(overlays.first()).cloned();
        let cover = self.t.named(tpl, "PART_ImageCover");
        let title = tpl
            .descendants()
            .filter(|n| n.is_element() && tag(*n) == "TextBlock")
            .filter(|n| self.t.local(*n, "Text").as_ref().is_some_and(binds_name))
            .find(|n| self.hidden_up_to(*n, tpl).is_none())
            .or_else(|| self.t.named(tpl, "PART_TextName"));
        if let Some(t) = title {
            d.title_font = self.font_role_of(t);
            d.title_size = self.num(t, "FontSize");
            d.title_shadow = self.effect_glow(t);
            if let Some(c) = cover {
                d.cover_side = self.side_of(c, t);
            }
        }
        d.description_size = self
            .t
            .named(tpl, "PART_HtmlDescription")
            .and_then(|h| self.num(h, "HtmlFontSize").or_else(|| self.num(h, "FontSize")))
            .or_else(|| self.t.named(tpl, "PART_TextDescription").and_then(|h| self.num(h, "FontSize")));
        d.logo = self.details_has_logo_hook();
        d
    }

    pub(super) fn row_grid_of_view(&self, host: N<'a, 'i>) -> Option<(N<'a, 'i>, usize)> {
        let rows = prop_elem(host, "RowDefinitions")?;
        Some((host, content(rows).count()))
    }

    pub(super) fn details_has_logo_hook(&self) -> bool {
        let Some(tpl) = self.t.template_of("GameDetails") else {
            return false;
        };
        tpl.descendants().filter(|n| n.is_element()).any(|n| {
            name_of(n).is_some_and(|x| x.contains("Logo"))
                || n.attributes().any(|a| {
                    let v = a.value();
                    v.contains("LogoLoader") || (v.contains("Binding") && v.contains("Logo"))
                })
        })
    }

    /// Left / right of `a` against `b`: Grid columns of their common Grid, else document order
    /// in a horizontal panel.
    pub(super) fn side_of(&self, a: N<'a, 'i>, b: N<'a, 'i>) -> Option<Side> {
        let anc_b: Vec<N> = b.ancestors().collect();
        let common = a.ancestors().find(|x| anc_b.contains(x))?;
        let child = |n: N<'a, 'i>| n.ancestors().find(|x| x.parent() == Some(common));
        let (ca, cb) = (child(a)?, child(b)?);
        match tag(common) {
            "Grid" => {
                let (x, y) = (grid_attr(ca, "Grid.Column", 0), grid_attr(cb, "Grid.Column", 0));
                match x.cmp(&y) {
                    std::cmp::Ordering::Less => Some(Side::Left),
                    std::cmp::Ordering::Greater => Some(Side::Right),
                    _ => None,
                }
            }
            "DockPanel" => {
                let dock = |n: N| attr(n, "DockPanel.Dock").unwrap_or("Left").trim().to_ascii_lowercase();
                match (dock(ca).as_str(), dock(cb).as_str()) {
                    ("right", _) => Some(Side::Right),
                    ("left", _) => Some(Side::Left),
                    _ => None,
                }
            }
            "StackPanel" | "VirtualizingStackPanel" | "WrapPanel"
                if attr(common, "Orientation").is_some_and(|o| o.trim() == "Horizontal") =>
            {
                let order = |n: N| common.children().position(|c| c == n);
                (order(ca) < order(cb)).then_some(Side::Left).or(Some(Side::Right))
            }
            _ => None,
        }
    }
}
