//! The XAML element tree → layout nodes: placement properties, paint, inherited text
//! properties and what each element draws (`Kind`).

use super::bind::{bind_art, bind_text, game_path, parse_binding};
use super::*;

impl Inherit {
    pub(super) fn root(t: &Tree) -> Inherit {
        let res = |k: &str| t.resource(k).and_then(|n| t.eval_node(n, 0));
        Inherit {
            fg: res("TextBrush").and_then(|v| v.brush()),
            size: res("FontSize").and_then(|v| v.num()).map(|v| v as f32).unwrap_or(22.0),
            family: None,
            weight: None,
        }
    }
}

/// Elements that are other views or Lodestar's own pages: never part of this view.
pub(super) const OTHER_VIEWS: [&str; 14] = [
    "GameDetails",
    "GameStatus",
    "Filters",
    "FiltersAdditional",
    "FiltersView",
    "GameMenu",
    "MainMenu",
    "SettingsMenus",
    "Popup",
    "ToolTip",
    "ContextMenu",
    "MessageBox",
    "NotificationsMenu",
    "SearchView",
];

pub(super) const MAX_DEPTH: usize = 64;

pub(super) const MAX_NODES: usize = 6000;

/// A setter's raw string value (`{DynamicResource X}` kept as written).
pub(super) fn raw_setter(t: &Tree, style: X, prop: &str) -> Option<String> {
    let mut s = Some(style);
    for _ in 0..16 {
        let cur = s?;
        for set in content(cur).filter(|c| tag(*c) == "Setter") {
            if attr(set, "TargetName").is_some() {
                continue;
            }
            if attr(set, "Property").is_some_and(|p| p.trim() == prop || p.trim().rsplit('.').next() == Some(prop)) {
                return attr(set, "Value").map(str::to_string);
            }
        }
        s = t.based_on(cur);
    }
    None
}

/// A binding to a later item of one of the game's lists (`SelectedGame.Genres[2]`).
pub(super) fn later_list_item(raw: &str) -> bool {
    let Markup::Binding(inner) = tree::markup(raw) else { return false };
    let p = game_path(&parse_binding(&inner).path);
    let Some((_, rest)) = p.split_once('[') else { return false };
    rest.split(']').next().and_then(|i| i.trim().parse::<usize>().ok()).is_some_and(|i| i > 0)
}

/// A sound file (by its extension).
pub(super) fn is_audio(path: &str) -> bool {
    let l = path.trim().to_ascii_lowercase();
    [".mp3", ".wav", ".ogg", ".wma", ".flac", ".m4a", ".aac"].iter().any(|e| l.ends_with(e))
}

/// Windows' Marlett: window and list symbols on letter codes.
pub(super) fn marlett(f: &str) -> bool {
    f.to_ascii_lowercase().contains("marlett")
}

/// The Unicode symbol for a Marlett letter (None: one Lodestar has no match for).
pub(super) fn symbol_glyph(text: &str) -> Option<char> {
    Some(match text.trim() {
        "h" | "n" | "=" => '•',
        "a" => '✓',
        "r" => '✕',
        "3" => '◀',
        "4" => '▶',
        "5" => '▲',
        "6" => '▼',
        _ => return None,
    })
}

/// A label ("Sorting:") whose value next to it is a text Lodestar can't fill isn't drawn
/// either: alone it reads as a value that is missing. It keeps its room in the layout.
pub(super) fn drop_orphan_labels(kids: &mut [LNode]) {
    for i in 0..kids.len() {
        let label = matches!(&kids[i].data.kind, Kind::Text(t) if !t.bound && t.text.trim_end().ends_with(':'));
        let next = kids[i + 1..].iter().find(|k| k.vis != Vis::Collapsed);
        let value_dropped =
            next.is_some_and(|k| matches!(k.data.kind, Kind::Drop("text Lodestar has no data for" | "add-on text")));
        if label && value_dropped {
            kids[i].data.kind = Kind::None;
        }
    }
}

/// Playnite's prompt template key → help bar button name.
pub(super) fn prompt_of(key: &str) -> Option<&'static str> {
    let b = key.trim().strip_prefix("ButtonPrompt")?;
    Some(match b.to_ascii_lowercase().as_str() {
        "a" => "a",
        "b" => "b",
        "x" => "x",
        "y" => "y",
        "lb" | "l1" => "lb",
        "rb" | "r1" => "rb",
        "start" | "menu" | "options" => "start",
        "back" | "view" | "select" | "share" => "select",
        _ => return None,
    })
}

/// The symbol for a Segoe MDL2 / Fluent icon (private-use code points) on a button: the menu
/// bars for the "hamburger" glyphs, else the "more" dots (what such buttons open: options).
pub(super) fn icon_symbol(text: &str) -> &'static str {
    match text.trim().chars().next() {
        Some('\u{e700}') | Some('\u{e8fd}') | Some('\u{e14c}') => "☰",
        _ => "…",
    }
}

/// Fonts of symbols (icons, controller buttons), not letters.
pub(super) fn icon_font(f: &str) -> bool {
    let l = f.to_ascii_lowercase();
    [
        "icofont",
        "icon",
        "mdl2",
        "fluent",
        "controller",
        "glyph",
        "symbol",
        "awesome",
        "material",
        "buttons",
        "webdings",
        "wingdings",
    ]
    .iter()
    .any(|w| l.contains(w))
}

/// Room between the filter preset tabs as measured.
pub(super) const TAB_GAP: &str = "    ";

pub(super) fn spec(t: &TextOut) -> TextSpec {
    TextSpec { text: t.text.clone(), size: t.size, bold: t.font.is_some(), wrap: t.wrap, max_lines: None }
}

/// What a per-game text measures as (a typical game's).
pub(super) fn sample_spec(t: &TextOut) -> TextSpec {
    let sample = |k: &str| -> &'static str {
        match k {
            "game.name" => "Metaphor: ReFantazio",
            "game.description" => {
                "A fantasy adventure about a young hero who sets out on a long journey across a divided kingdom, \
                 meeting companions, uncovering old secrets and facing a tournament that decides the next ruler. \
                 Explore towns and dungeons, fight in turn-based battles and shape the fate of the realm."
            }
            "game.platform" => "PC (Windows)",
            "game.genres" | "game.genre" => "Role-playing, Adventure",
            "game.developer" | "game.publisher" => "Studio Zero",
            "game.year" => "2024",
            "game.playtime" => "12h 30m",
            "game.lastplayed" => "12 Mar 2024",
            "game.source" => "Steam",
            "list.count" | "list.index" => "283",
            "collection.name" => "Continue Playing",
            "system.battery" => "82%",
            _ => "Text",
        }
    };
    let mut s = String::new();
    let mut rest = t.text.as_str();
    while let Some(o) = rest.find('{') {
        s.push_str(&rest[..o]);
        let Some(c) = rest[o..].find('}') else { break };
        s.push_str(sample(&rest[o + 1..o + c]));
        rest = &rest[o + c + 1..];
    }
    s.push_str(rest);
    TextSpec { text: s, ..spec(t) }
}

pub(super) fn fit_name(f: Scale) -> &'static str {
    match f {
        Scale::UniformToFill => "cover",
        Scale::Fill => "stretch",
        Scale::Uniform | Scale::None => "contain",
    }
}

/// An OpacityMask as the image `mask`: how far each edge fades [left, top, right, bottom].
pub(super) fn mask_edges(b: &LookBrush) -> Option<[f32; 4]> {
    if !matches!(b, LookBrush::Linear { .. } | LookBrush::Radial { .. }) {
        return None;
    }
    let fade = |a: f32| if a < 0.5 { round((1.0 - a) * 0.5) } else { 0.0 };
    let e = [b.alpha_at(0.0, 0.5), b.alpha_at(0.5, 0.0), b.alpha_at(1.0, 0.5), b.alpha_at(0.5, 1.0)];
    let m = e.map(fade);
    m.iter().any(|v| *v > 0.0).then_some(m)
}

/// A colour seen through a gradient opacity mask: the mask's geometry with the colour's alpha
/// scaled per stop.
pub(super) fn masked(c: Color, mask: &LookBrush) -> LookBrush {
    let scale = |stops: &[(Color, f32)]| -> Vec<(Color, f32)> {
        stops
            .iter()
            .map(|(m, o)| (c.with_alpha(((c.0[3] as f32) * (m.0[3] as f32 / 255.0)).round() as u8), *o))
            .collect()
    };
    match mask {
        LookBrush::Linear { stops, start, end, absolute } => {
            LookBrush::Linear { stops: scale(stops), start: *start, end: *end, absolute: *absolute }
        }
        LookBrush::Radial { stops, center, origin, radius } => {
            LookBrush::Radial { stops: scale(stops), center: *center, origin: *origin, radius: *radius }
        }
        _ => LookBrush::solid(c),
    }
}

impl<'b, 't, 'i> Builder<'b, 't, 'i> {
    pub(super) fn get(&self, el: X<'t, 'i>, prop: &str) -> Option<Value<'t, 'i>> {
        self.t.get(el, prop)
    }

    /// A number property; never NaN or infinite (WPF's `Width="NaN"` means "not set", and
    /// layout clamps would panic on such bounds).
    pub(super) fn num(&self, el: X<'t, 'i>, prop: &str) -> Option<f32> {
        let v: f32 = match self.get(el, prop)? {
            Value::Setting(s) => match setting(&s)? {
                SVal::Num(n) => n,
                _ => return None,
            },
            // Another element's constant (`Opacity="{Binding Opacity, ElementName=X}"`).
            Value::Binding(raw) => self.t.static_binding(el, &format!("{{Binding {raw}}}"))?.trim().parse().ok()?,
            v => v.num()? as f32,
        };
        v.is_finite().then_some(v)
    }

    pub(super) fn text(&self, el: X<'t, 'i>, prop: &str) -> Option<String> {
        match self.get(el, prop)? {
            Value::Text(t) => Some(t),
            Value::Bool(b) => Some(b.to_string()),
            Value::Num(n) => Some(n.to_string()),
            Value::Setting(s) => match setting(&s)? {
                SVal::Text(t) => Some(t),
                SVal::Bool(b) => Some(b.to_string()),
                SVal::Num(n) => Some(n.to_string()),
            },
            _ => None,
        }
    }

    pub(super) fn sides(&self, el: X<'t, 'i>, prop: &str) -> [f32; 4] {
        self.get(el, prop).and_then(|v| v.sides()).unwrap_or([0.0; 4])
    }

    pub(super) fn brush(&self, el: X<'t, 'i>, prop: &str) -> Option<LookBrush> {
        self.get(el, prop)?.brush().filter(|b| !b.is_clear() || matches!(b, LookBrush::Image { .. }))
    }

    pub(super) fn grid_attr(&self, el: X<'t, 'i>, prop: &str, default: usize) -> usize {
        self.num(el, prop).map(|v| v.max(0.0) as usize).unwrap_or(default)
    }

    pub(super) fn inherit(&self, el: X<'t, 'i>, up: &Inherit) -> Inherit {
        let mut i = up.clone();
        if let Some(b) = self.get(el, "Foreground").and_then(|v| v.brush()) {
            i.fg = Some(b);
        }
        if let Some(s) = self.num(el, "FontSize").filter(|s| *s > 0.0) {
            i.size = s;
        }
        if let Some(raw) = attr(el, "FontFamily")
            .map(str::to_string)
            .or_else(|| self.t.style_of(el).and_then(|s| raw_setter(self.t, s, "FontFamily")))
        {
            i.family = Some(raw);
        }
        if let Some(w) = self.text(el, "FontWeight") {
            i.weight = Some(w);
        }
        i
    }

    /// The Lodestar font role of the inherited family / weight.
    pub(super) fn font(&self, inh: &Inherit) -> Option<&'static str> {
        let w = inh.weight.as_deref().unwrap_or("").to_ascii_lowercase();
        let f = inh.family.as_deref().unwrap_or("").to_ascii_lowercase();
        let resolved = inh
            .family
            .as_deref()
            .and_then(|raw| match tree::markup(raw) {
                Markup::Resource(k) => {
                    self.t.resource(&k).and_then(|n| self.t.eval_node(n, 0)).and_then(|v| v.text().map(str::to_string))
                }
                _ => None,
            })
            .unwrap_or_default()
            .to_ascii_lowercase();
        let all = format!("{w} {f} {resolved}");
        if all.contains("semibold") || all.contains("demibold") {
            Some("semibold")
        } else if all.contains("bold") || all.contains("black") || all.contains("heavy") {
            Some("bold")
        } else {
            None
        }
    }

    pub(super) fn text_out(&self, el: X<'t, 'i>, inh: &Inherit, text: String, bound: bool) -> TextOut {
        let color = inh.fg.as_ref().and_then(LookBrush::average).or(Some(Color::WHITE));
        let align = match self.text(el, "TextAlignment").map(|s| s.to_ascii_lowercase()).as_deref() {
            Some("center") => Some("center"),
            Some("right") => Some("right"),
            _ => None,
        };
        let wrap = self.text(el, "TextWrapping").is_some_and(|w| w.to_ascii_lowercase().starts_with("wrap"));
        let shadow = self.get(el, "Effect").and_then(|v| v.node()).filter(|e| tag(*e) == "DropShadowEffect").map(|e| {
            let c = self.get(e, "Color").and_then(|v| v.color()).unwrap_or(Color::BLACK);
            let o = self.num(e, "Opacity").unwrap_or(1.0).clamp(0.0, 1.0);
            c.with_alpha((c.0[3] as f32 * o) as u8)
        });
        TextOut {
            text,
            color,
            size: inh.size,
            font: self.font(inh),
            align,
            wrap,
            upper: self.text(el, "CharacterCasing").is_some_and(|c| c.eq_ignore_ascii_case("upper")),
            shadow,
            scroll: false,
            bound,
            anchor: match self.text(el, "HorizontalAlignment").and_then(|s| Al::parse(&s)) {
                Some(Al::Center) => "center",
                Some(Al::End) => "right",
                _ => "left",
            },
        }
    }

    /// The text a TextBlock shows (literal, LOC string, binding) → (template, bound, upper).
    pub(super) fn text_of(&mut self, el: X<'t, 'i>) -> Option<(String, bool, bool)> {
        let raw = attr(el, "Text")
            .map(str::to_string)
            .or_else(|| self.t.style_of(el).and_then(|s| raw_setter(self.t, s, "Text")));
        let Some(raw) = raw else {
            // Inline text / Runs.
            let mut s = String::new();
            for c in el.children() {
                if c.is_text() {
                    s.push_str(c.text().unwrap_or(""));
                } else if c.is_element() && tag(c) == "Run" {
                    if let Some(t) = attr(c, "Text") {
                        s.push_str(&self.literal_or_loc(t)?);
                    } else {
                        s.push_str(c.text().unwrap_or(""));
                    }
                } else if c.is_element() && tag(c) == "LineBreak" {
                    s.push('\n');
                }
            }
            let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
            return (!s.is_empty()).then_some((s, false, false));
        };
        match tree::markup(&raw) {
            Markup::Binding(b) => {
                let b = parse_binding(&b);
                let (tpl, upper) = bind_text(&b, self.main)?;
                Some((tpl, true, upper))
            }
            Markup::TemplateBinding(_) | Markup::Plugin(_) | Markup::Settings(_) | Markup::Other(_) => None,
            _ => Some((self.literal_or_loc(&raw)?, false, false)),
        }
    }

    /// The button of a `ContentControl` showing one of Playnite's `ButtonPrompt…` templates.
    /// Triggers and analogue sticks / triggers (which Lodestar doesn't use) give None.
    pub(super) fn prompt_button(&self, el: X<'t, 'i>) -> Option<&'static str> {
        let raw = attr(el, "ContentTemplate")
            .map(str::to_string)
            .or_else(|| self.t.style_of(el).and_then(|s| raw_setter(self.t, s, "ContentTemplate")))?;
        let Markup::Resource(key) = tree::markup(&raw) else { return None };
        prompt_of(&key)
    }

    /// The text of a button whose Content is an element (a TextBlock inside it): the text, or
    /// for an icon-font glyph the symbol Lodestar draws for it.
    pub(super) fn content_label(&self, el: X<'t, 'i>) -> Option<String> {
        el.descendants().filter(|d| d.is_element() && tag(*d) == "TextBlock").find_map(|d| {
            let t = attr(d, "Text").and_then(|t| self.literal_or_loc(t))?;
            let t = t.trim();
            if t.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c)) {
                Some(icon_symbol(t).to_string())
            } else {
                (!t.is_empty()).then(|| t.to_string())
            }
        })
    }

    pub(super) fn literal_or_loc(&self, raw: &str) -> Option<String> {
        match tree::markup(raw) {
            Markup::Lit(l) => Some(l.to_string()),
            Markup::Resource(k) if k.starts_with("LOC") => self.loc.text(&k),
            Markup::Resource(k) => {
                self.t.resource(&k).and_then(|n| self.t.eval_node(n, 0)).and_then(|v| v.text().map(str::to_string))
            }
            _ => None,
        }
    }

    pub(super) fn theme_image(&mut self, file: &str) -> String {
        if let Some(d) = self.images.get(file) {
            return d.clone();
        }
        let flat: String = file
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' })
            .collect();
        let dest = format!("images/view/{flat}");
        self.images.insert(file.to_string(), dest.clone());
        dest
    }

    pub(super) fn fit_of(&self, el: X<'t, 'i>, default: Scale) -> Scale {
        self.text(el, "Stretch").and_then(|s| Scale::parse(&s)).unwrap_or(default)
    }

    /// An Image / FadeImage → what it shows.
    pub(super) fn image_kind(&mut self, el: X<'t, 'i>, name: &str) -> (Kind, Content) {
        let tagname = tag(el);
        let default_fit = if tagname == "FadeImage" { Scale::UniformToFill } else { Scale::Uniform };
        let fill = self.fit_of(el, default_fit);
        let fit = fit_name(fill);
        let mask = self
            .get(el, "ImageOpacityMask")
            .or_else(|| self.get(el, "OpacityMask"))
            .and_then(|v| v.brush())
            .and_then(|b| mask_edges(&b));
        let blur = self
            .get(el, "Effect")
            .and_then(|v| v.node())
            .filter(|e| tag(*e) == "BlurEffect")
            .map(|e| self.num(e, "Radius").unwrap_or(5.0))
            .unwrap_or(0.0)
            .max(self.num(el, "BlurAmount").unwrap_or(0.0));
        let art = |path: &str, natural: [f32; 2], picture: bool, fallbacks: Vec<String>| {
            (
                Kind::Image(ImageOut { path: path.into(), fallbacks, fit, mask, blur, placeholder: picture, picture }),
                Content::Image { natural: Some(natural), stretch: fill },
            )
        };
        match name {
            "PART_ImageBackground" => {
                let (mut k, c) = art("{art.hero}", [1920.0, 1080.0], true, vec!["{art.banner}".into()]);
                if let Kind::Image(i) = &mut k {
                    // The game's picture fills its area (Lodestar's pictures aren't all 16:9).
                    i.fit = "cover";
                }
                return (k, c);
            }
            "PART_ImageCover" => return art("{art.cover}", [600.0, 900.0], false, vec![]),
            "PART_ImageIcon" => return art("{art.icon}", [256.0, 256.0], false, vec![]),
            _ => {}
        }
        let src = self.get(el, "Source");
        match src {
            Some(Value::ThemeFile(f)) => self.file_image(&f, fill, fit, mask, blur),
            Some(Value::Text(t)) => match tree::theme_file(&self.t.files, &t) {
                Some(f) => self.file_image(&f, fill, fit, mask, blur),
                None => (Kind::Drop("picture"), Content::None),
            },
            Some(Value::Brush(LookBrush::Image { file: Some(f), .. })) => self.file_image(&f, fill, fit, mask, blur),
            Some(Value::Binding(raw)) => {
                let b = parse_binding(&raw);
                let logo = name.to_ascii_lowercase().contains("logo") || b.path.to_ascii_lowercase().contains("logo");
                if logo {
                    return art("{art.logo}", [800.0, 310.0], false, vec![]);
                }
                match bind_art(&b) {
                    Some("{art.cover}") => art("{art.cover}", [600.0, 900.0], false, vec![]),
                    Some("{art.hero}") => art("{art.hero}", [1920.0, 1080.0], true, vec!["{art.banner}".into()]),
                    Some("{art.icon}") => art("{art.icon}", [256.0, 256.0], false, vec![]),
                    Some("{art.logo}") => art("{art.logo}", [800.0, 310.0], false, vec![]),
                    _ => (Kind::Drop("picture bound to something Lodestar doesn't have"), Content::None),
                }
            }
            _ => (Kind::None, Content::None),
        }
    }

    pub(super) fn file_image(
        &mut self,
        f: &str,
        fill: Scale,
        fit: &'static str,
        mask: Option<[f32; 4]>,
        blur: f32,
    ) -> (Kind, Content) {
        let natural = (self.dims)(f);
        let path = self.theme_image(f);
        (
            Kind::Image(ImageOut { path, fallbacks: vec![], fit, mask, blur, placeholder: false, picture: false }),
            Content::Image { natural, stretch: fill },
        )
    }

    /// The layout node for `el` and everything under it (None: not part of this view, or the
    /// tree is too deep / too big).
    pub(super) fn build(&mut self, el: X<'t, 'i>, up: &Inherit, depth: usize) -> Option<LNode> {
        self.names += 1;
        if depth > MAX_DEPTH || self.names > MAX_NODES {
            return None;
        }
        let tg = tag(el).to_string();
        let name = name_of(el).unwrap_or("").to_string();
        if OTHER_VIEWS.contains(&tg.as_str()) || name.starts_with("PART_Elem") && name != "PART_ElemBatteryStatus" {
            return None;
        }
        let mut n = LNode::new(Info { name: name.clone(), tag: tg.clone(), opacity: 1.0, ..Default::default() });
        self.place(el, &mut n);
        self.show_and_fade(el, &mut n);
        if n.vis == Vis::Collapsed {
            return Some(n);
        }
        let inh = self.inherit(el, up);
        n.data.clip = self.text(el, "ClipToBounds").is_some_and(|v| v.eq_ignore_ascii_case("true"));
        self.paint(el, &tg, &mut n);
        if self.what_it_draws(el, &tg, &name, &inh, depth, &mut n) {
            return Some(n);
        }
        // Children (a ContentControl's Content element, a Border's child …).
        for c in content(el) {
            if let Some(k) = self.build(c, &inh, depth + 1) {
                n.children.push(k);
            }
        }
        if let Some(c) = prop_elem(el, "Content").and_then(|p| content(p).next())
            && let Some(k) = self.build(c, &inh, depth + 1)
        {
            n.children.push(k);
        }
        drop_orphan_labels(&mut n.children);
        Some(n)
    }

    /// Size, margins, alignment and the panel-attached properties (Grid, DockPanel, Canvas).
    fn place(&self, el: X<'t, 'i>, n: &mut LNode) {
        n.width = self.num(el, "Width").filter(|v| *v >= 0.0);
        n.height = self.num(el, "Height").filter(|v| *v >= 0.0);
        n.min = [self.num(el, "MinWidth").unwrap_or(0.0), self.num(el, "MinHeight").unwrap_or(0.0)];
        n.max = [self.num(el, "MaxWidth").unwrap_or(f32::INFINITY), self.num(el, "MaxHeight").unwrap_or(f32::INFINITY)];
        n.margin = self.sides(el, "Margin");
        n.padding = self.sides(el, "Padding");
        n.ha = self.text(el, "HorizontalAlignment").and_then(|s| Al::parse(&s)).unwrap_or(Al::Stretch);
        n.va = self.text(el, "VerticalAlignment").and_then(|s| Al::parse(&s)).unwrap_or(Al::Stretch);
        n.row = self.grid_attr(el, "Grid.Row", 0);
        n.col = self.grid_attr(el, "Grid.Column", 0);
        n.row_span = self.grid_attr(el, "Grid.RowSpan", 1).max(1);
        n.col_span = self.grid_attr(el, "Grid.ColumnSpan", 1).max(1);
        n.z = self.num(el, "Panel.ZIndex").unwrap_or(0.0) as i32;
        n.dock = match self.text(el, "DockPanel.Dock").map(|s| s.to_ascii_lowercase()).as_deref() {
            Some("top") => Dock::Top,
            Some("right") => Dock::Right,
            Some("bottom") => Dock::Bottom,
            _ => Dock::Left,
        };
        n.canvas = [
            self.num(el, "Canvas.Left"),
            self.num(el, "Canvas.Top"),
            self.num(el, "Canvas.Right"),
            self.num(el, "Canvas.Bottom"),
        ];
        n.translate = self.translate(el);
        if let Some(m) = self.resting_margin(el) {
            n.margin = m;
        }
    }

    /// Visibility and its show / hide rules, opacity and the appear transition.
    fn show_and_fade(&mut self, el: X<'t, 'i>, n: &mut LNode) {
        let (vis, cond) = self.visibility(el);
        n.vis = vis;
        n.data.cond = cond;
        if n.vis == Vis::Visible && self.masked_by_nothing(el) {
            // WPF draws nothing through an opacity mask that shows a missing element.
            n.vis = Vis::Hidden;
        }
        if n.vis == Vis::Visible && self.dismissed(el) {
            // An intro, start or lock screen: gone once it has played.
            n.vis = Vis::Collapsed;
            self.stats.triggers_mapped += 1;
        }
        let opacity = self.num(el, "Opacity").unwrap_or(1.0).clamp(0.0, 1.0);
        let (fades, appear) = self.fade_in(el);
        n.data.opacity = if opacity <= 0.01 && fades { 1.0 } else { opacity };
        if let Some(ms) = appear {
            n.data.appear_ms = Some(ms);
            self.stats.transitions += 1;
        }
    }

    /// Background / fill / border of the elements that paint one.
    fn paint(&self, el: X<'t, 'i>, tg: &str, n: &mut LNode) {
        let (fill_prop, stroke_prop, thick_prop) = match tg {
            "Rectangle" | "Ellipse" => ("Fill", "Stroke", "StrokeThickness"),
            _ => ("Background", "BorderBrush", "BorderThickness"),
        };
        let paints = matches!(
            tg,
            "Border"
                | "Grid"
                | "StackPanel"
                | "DockPanel"
                | "Canvas"
                | "WrapPanel"
                | "Rectangle"
                | "Ellipse"
                | "TextBlock"
        );
        if !paints {
            return;
        }
        let mut fill = self.brush(el, fill_prop);
        // A solid fill faded by a gradient OpacityMask is that gradient.
        if let (Some(LookBrush::Solid { color }), Some(mask)) =
            (fill.clone(), self.get(el, "OpacityMask").and_then(|v| v.brush()))
        {
            fill = Some(masked(color, &mask));
        }
        n.data.fill = fill;
        let thick = self.get(el, thick_prop).and_then(|v| v.max_side()).unwrap_or(0.0);
        if tg != "Rectangle" && tg != "Ellipse" {
            n.border = self.sides(el, thick_prop);
        }
        if thick > 0.0
            && let Some(c) = self.brush(el, stroke_prop).and_then(|b| b.average()).filter(|c| c.0[3] > 0)
        {
            n.data.stroke = Some((c, thick));
        }
        n.data.radius = match tg {
            "Rectangle" => self.num(el, "RadiusX").unwrap_or(0.0),
            "Ellipse" => -1.0,
            _ => self.get(el, "CornerRadius").and_then(|v| v.max_side()).unwrap_or(0.0),
        };
    }

    /// What the element is: its panel, or what it draws (`n.data.kind`, `n.content`). True:
    /// the node is complete (its children aren't built).
    fn what_it_draws(
        &mut self,
        el: X<'t, 'i>,
        tg: &str,
        name: &str,
        inh: &Inherit,
        depth: usize,
        n: &mut LNode,
    ) -> bool {
        match tg {
            "Grid" => {
                let defs = |p: &str, size: &str, min: &str, max: &str| -> Vec<Def> {
                    prop_elem(el, p)
                        .map(|d| {
                            content(d)
                                .map(|c| Def {
                                    len: self.text(c, size).map(|s| Len::parse(&s)).unwrap_or(Len::Star(1.0)),
                                    min: self.num(c, min).unwrap_or(0.0),
                                    max: self.num(c, max).unwrap_or(f32::INFINITY),
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                };
                n.panel = Panel::Grid {
                    rows: defs("RowDefinitions", "Height", "MinHeight", "MaxHeight"),
                    cols: defs("ColumnDefinitions", "Width", "MinWidth", "MaxWidth"),
                };
            }
            "StackPanel" | "VirtualizingStackPanel" => {
                let h = self.text(el, "Orientation").is_some_and(|o| o.eq_ignore_ascii_case("horizontal"));
                n.panel = Panel::Stack { horizontal: h };
            }
            "WrapPanel" => {
                let v = self.text(el, "Orientation").is_some_and(|o| o.eq_ignore_ascii_case("vertical"));
                n.panel = Panel::Wrap { horizontal: !v };
            }
            "DockPanel" => {
                n.panel = Panel::Dock {
                    fill_last: !self.text(el, "LastChildFill").is_some_and(|v| v.eq_ignore_ascii_case("false")),
                };
            }
            "Canvas" => n.panel = Panel::Canvas,
            "Viewbox" => {
                n.panel = Panel::Viewbox { stretch: self.fit_of(el, Scale::Uniform) };
            }
            "ScrollViewer" | "ScrollViewerEx" => {
                let off = |p: &str, d: bool| {
                    self.text(el, p).map(|v| !matches!(v.to_ascii_lowercase().as_str(), "disabled")).unwrap_or(d)
                };
                n.panel = Panel::Scroll {
                    horizontal: off("HorizontalScrollBarVisibility", false),
                    vertical: off("VerticalScrollBarVisibility", true),
                };
                n.data.clip = true;
            }
            "TextBlock" | "Label" => return self.text_block(el, tg, name, inh, n),
            "Image" | "FadeImage" => {
                let (k, c) = self.image_kind(el, name);
                n.data.kind = k;
                n.content = c;
            }
            "MediaElement" => n.data.kind = self.media_kind(el),
            "HtmlTextView" => {
                let size = self.num(el, "HtmlFontSize").unwrap_or(16.0) * 1.25;
                let fg = self.get(el, "HtmlForeground").and_then(|v| v.color());
                let mut t = self.text_out(el, &Inherit { size, ..inh.clone() }, "{game.description}".into(), true);
                if fg.is_some() {
                    t.color = fg;
                }
                t.wrap = true;
                t.scroll = true;
                n.content = Content::Text(sample_spec(&t));
                n.data.kind = Kind::Text(t);
            }
            "ListBoxEx" | "ListBox" | "ItemsControl" | "ListView" if name == "PART_ListGameItems" => {
                n.data.kind = Kind::List;
                n.content = Content::Fixed([W, 400.0]);
                return true;
            }
            "ListBoxEx" | "ListBox" | "ItemsControl" | "ListView" | "ComboBox" => {
                n.data.kind = Kind::Drop("list driven by the theme's helpers");
                return true;
            }
            "FilterPresetSelector" => {
                let (fg, current) = self.tab_colors();
                let mut t = self.text_out(el, inh, String::new(), false);
                t.color = fg.or(t.color);
                let all: Vec<&str> = TABS.iter().map(|(_, l)| *l).collect();
                n.content = Content::Text(TextSpec { text: all.join(TAB_GAP), ..spec(&t) });
                n.data.kind = Kind::Tabs(TabsOut { label: t, current });
                return true;
            }
            "ButtonEx" | "Button" => {
                self.button(el, name, inh, depth, n);
                return true;
            }
            "ContentControl" | "ContentPresenter" => return self.content_control(el, name, inh, n),
            "Border" | "Decorator" | "AdornerDecorator" | "Rectangle" | "Ellipse" => {}
            "Path" | "Polygon" | "Line" | "Polyline" => {
                n.data.kind = Kind::Drop("drawn shape");
                return true;
            }
            "CheckBoxEx" | "CheckBox" | "ToggleButton" | "Slider" | "ProgressBar" | "TextBox" | "TextBoxEx"
            | "SearchBox" => {
                n.data.kind = Kind::Drop("control");
                n.content = Content::Fixed([48.0, 48.0]);
                return true;
            }
            _ => {
                n.data.kind = Kind::Drop("custom control");
                return true;
            }
        }
        false
    }

    /// A TextBlock or Label: the clock, the battery, symbol fonts, or its text (a literal, a
    /// `LOC…` string or a game binding). True: the node is complete.
    fn text_block(&mut self, el: X<'t, 'i>, tg: &str, name: &str, inh: &Inherit, n: &mut LNode) -> bool {
        if name == "PART_TextClock" {
            let t = self.text_out(el, inh, "00:00".into(), true);
            n.content = Content::Text(spec(&t));
            n.data.kind = Kind::Clock(t);
        } else if name == "PART_TextBatteryPercentage" {
            let t = self.text_out(el, inh, "{system.battery}".into(), true);
            n.content = Content::Text(TextSpec { text: "82%".into(), ..spec(&t) });
            n.data.kind = Kind::Text(t);
            n.data.cond = n.data.cond.and(&Cond { visible_if: Some("{system.battery}".into()), hidden_if: None });
        } else if inh.family.as_deref().is_some_and(marlett) {
            // Windows' Marlett symbols: the same symbol in Lodestar's font, else nothing.
            match symbol_glyph(attr(el, "Text").unwrap_or("")) {
                Some(c) => {
                    let t = self.text_out(el, &Inherit { family: None, ..inh.clone() }, c.to_string(), false);
                    n.content = Content::Text(spec(&t));
                    n.data.kind = Kind::Text(t);
                }
                None => {
                    n.data.kind = Kind::Drop("icon glyph");
                    n.content = Content::Fixed([inh.size, inh.size]);
                }
            }
        } else if inh.family.as_deref().is_some_and(icon_font) {
            n.data.kind = Kind::Drop("icon glyph");
            n.content = Content::Fixed([inh.size, inh.size]);
        } else if attr(el, "Text").is_some_and(later_list_item) {
            // `Genres[1]`, `Genres[2]` …: Lodestar's binding gives the whole list at the
            // first item's place.
            n.vis = Vis::Collapsed;
            return true;
        } else {
            let raw_text = if tg == "Label" {
                attr(el, "Content")
                    .map(|c| (self.literal_or_loc(c), false, false))
                    .and_then(|(t, b, u)| t.map(|t| (t, b, u)))
            } else {
                self.text_of(el)
            };
            match raw_text {
                // Private-use characters: an icon font's glyph.
                Some((text, _, _)) if text.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c)) => {
                    n.data.kind = Kind::Drop("icon glyph");
                    n.content = Content::Fixed([inh.size, inh.size]);
                }
                Some((text, bound, upper)) => {
                    let mut t = self.text_out(el, inh, text, bound);
                    t.upper |= upper;
                    n.content = Content::Text(sample_spec(&t));
                    n.data.kind = Kind::Text(t);
                }
                None => {
                    let raw = attr(el, "Text")
                        .map(str::to_string)
                        .or_else(|| self.t.style_of(el).and_then(|s| raw_setter(self.t, s, "Text")));
                    if let Some(raw) = raw {
                        let addon = matches!(tree::markup(&raw), Markup::Plugin(_))
                            || matches!(tree::markup(&raw), Markup::Resource(k) if k.starts_with("LOC"));
                        n.data.kind = Kind::Drop(if addon { "add-on text" } else { "text Lodestar has no data for" });
                        n.content = Content::Fixed([inh.size * 4.0, inh.size * 1.33]);
                    }
                }
            }
        }
        false
    }

    /// A MediaElement: the theme's own background video plays there; trailers and videos the
    /// theme's add-ons find per game don't exist in Lodestar.
    fn media_kind(&self, el: X<'t, 'i>) -> Kind {
        match self.get(el, "Source") {
            // Sounds (a start-up chime, menu clicks) aren't pictures.
            Some(Value::ThemeFile(f)) if is_audio(&f) => Kind::None,
            Some(Value::Text(t)) if is_audio(&t) => Kind::None,
            Some(Value::ThemeFile(_)) => Kind::Video,
            Some(Value::Text(t)) if tree::theme_file(&self.t.files, &t).is_some() => Kind::Video,
            _ => Kind::Drop("video of the theme's add-ons"),
        }
    }

    /// A button: the Play / Install / options button, a bottom-bar prompt (→ the help bar),
    /// the main menu button, else dropped.
    fn button(&mut self, el: X<'t, 'i>, name: &str, inh: &Inherit, depth: usize, n: &mut LNode) {
        let bottom = matches!(
            name,
            "PART_ButtonPlay"
                | "PART_ButtonInstall"
                | "PART_ButtonDetails"
                | "PART_ButtonGameOptions"
                | "PART_ButtonSearch"
                | "PART_ButtonFilter"
        );
        let label = attr(el, "Content").and_then(|c| self.literal_or_loc(c));
        let big = n.height.is_some_and(|h| h >= 60.0);
        if name == "PART_ButtonContext" || name == "PART_ButtonOptions" || (bottom && big && name == "PART_ButtonPlay")
        {
            // The options button often shows only an icon (Segoe's "More" dots): its
            // nearest symbol, never "Play", which would read as a second Play button.
            let label =
                label.or_else(|| self.content_label(el)).or_else(|| (name == "PART_ButtonOptions").then(|| "…".into()));
            let mut t = self.text_out(el, inh, label.clone().unwrap_or_else(|| "Play".into()), false);
            t.align = Some("center");
            let fill = self.brush(el, "Background");
            let border = self
                .get(el, "BorderThickness")
                .and_then(|v| v.max_side())
                .filter(|t| *t > 0.0)
                .and_then(|th| self.brush(el, "BorderBrush").and_then(|b| b.average()).map(|c| (c, th)));
            let radius = self.get(el, "CornerRadius").and_then(|v| v.max_side()).unwrap_or(0.0);
            let pad = self.sides(el, "Padding");
            n.padding = [pad[0].max(20.0), pad[1].max(8.0), pad[2].max(20.0), pad[3].max(8.0)];
            n.content = Content::Text(TextSpec { text: "Install".into(), ..spec(&t) });
            n.data.kind =
                Kind::Button(ButtonOut { label: t, fill, border, radius, context: name == "PART_ButtonContext" });
            return;
        }
        if bottom {
            let t = self.text_out(el, inh, String::new(), false);
            let text = label.unwrap_or_default();
            n.content = Content::Text(TextSpec { text: text.clone(), ..spec(&t) });
            n.padding = [12.0, 6.0, 12.0, 6.0];
            n.data.kind = Kind::Help(TextOut { text, ..t });
            return;
        }
        if name == "PART_ButtonMainMenu" {
            // Its picture, else Lodestar's menu glyph.
            let mut kids = vec![];
            for c in content(el) {
                if let Some(k) = self.build(c, inh, depth + 1) {
                    kids.push(k);
                }
            }
            let draws = kids.iter().any(|k| matches!(k.data.kind, Kind::Image(_)));
            if draws {
                n.children = kids;
            } else {
                let mut t = self.text_out(el, inh, "☰".into(), false);
                t.align = Some("center");
                t.color = self
                    .t
                    .resource("GlyphBrush")
                    .and_then(|r| self.t.eval_node(r, 0))
                    .and_then(|v| v.color())
                    .or(t.color);
                n.content = Content::Text(spec(&t));
                n.data.kind = Kind::Text(t);
            }
            return;
        }
        n.data.kind = Kind::Drop("button");
        n.content = Content::Fixed([48.0, 48.0]);
    }

    /// A ContentControl / ContentPresenter: a button prompt, the battery, a logo hook or an
    /// add-on's panel (true: the node is complete); else a plain container.
    fn content_control(&mut self, el: X<'t, 'i>, name: &str, inh: &Inherit, n: &mut LNode) -> bool {
        if let Some(b) = self.prompt_button(el) {
            // Sized like Playnite's prompt templates (a disc as tall as the control,
            // shoulder and menu buttons wider).
            let h = n.height.unwrap_or(inh.size * 1.15).max(8.0);
            let w = if matches!(b, "a" | "b" | "x" | "y") { h } else { h * 1.5 };
            n.content = Content::Fixed([w, h]);
            n.data.kind = Kind::Prompt(b, inh.fg.as_ref().and_then(LookBrush::average));
            return true;
        }
        let lname = name.to_ascii_lowercase();
        if name == "PART_ElemBatteryStatus" {
            let t = self.text_out(el, &Inherit { size: inh.size * 0.6, ..inh.clone() }, "▮".into(), false);
            n.content = Content::Text(spec(&t));
            n.data.kind = Kind::Text(t);
            n.data.cond = n.data.cond.and(&Cond { visible_if: Some("{system.battery}".into()), hidden_if: None });
            return true;
        }
        if lname.contains("logo") && (lname.contains("extrametadata") || lname.contains("loader")) {
            n.data.kind = Kind::Image(ImageOut { path: "{art.logo}".into(), fit: "contain", ..Default::default() });
            n.content = Content::Fixed([500.0, 190.0]);
            return true;
        }
        if name.contains('_') && !name.starts_with("PART_")
            || self.text(el, "Style").is_some() && content(el).next().is_none()
        {
            n.data.kind = Kind::Drop("add-on panel");
            return true;
        }
        false
    }

    /// The filter preset tab colours: the theme's `ItemFilterQuickPreset` style (its text
    /// colour, and the one its IsChecked / IsFocused triggers give), else the accent glyph colour
    /// for the open tab.
    pub(super) fn tab_colors(&self) -> (Option<Color>, Option<Color>) {
        let colour = |v: Option<Value>| v.and_then(|v| v.brush()).and_then(|b| b.average());
        let style = self.t.resource("ItemFilterQuickPreset");
        let normal = style.and_then(|s| colour(self.t.setter(s, "Foreground")));
        let mut current = None;
        if let Some(tpl) = style.and_then(|s| self.t.template(s)) {
            for state in ["IsChecked", "IsFocused"] {
                let set =
                    self.t.triggers(tpl).into_iter().filter(|tr| attr(*tr, "Property") == Some(state)).find_map(|tr| {
                        content(tr).find(|c| {
                            tag(*c) == "Setter"
                                && attr(*c, "Property").is_some_and(|p| p.trim().ends_with("Foreground"))
                        })
                    });
                if current.is_none()
                    && let Some(set) = set
                {
                    current = colour(self.t.setter_value(set, "Foreground"));
                }
            }
        }
        let glyph = colour(self.t.resource("GlyphBrush").and_then(|r| self.t.eval_node(r, 0)));
        (normal, current.or(glyph))
    }

    /// A static RenderTransform translation.
    pub(super) fn translate(&self, el: X<'t, 'i>) -> [f32; 2] {
        let Some(rt) = prop_elem(el, "RenderTransform") else { return [0.0, 0.0] };
        let mut out = [0.0, 0.0];
        for t in rt.descendants().filter(|d| d.is_element() && tag(*d) == "TranslateTransform") {
            out[0] += self.num(t, "X").unwrap_or(0.0);
            out[1] += self.num(t, "Y").unwrap_or(0.0);
        }
        out
    }
}
