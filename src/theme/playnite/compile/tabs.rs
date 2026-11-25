//! The filter preset strip (PART_FilterPresetSelector) as Lodestar's tabs. Playnite draws one
//! button per preset with the theme's `ItemFilterQuickPreset` template: often the theme's
//! picture for the preset's name (`{ThemeFileBinding Name, PathFormat='Icons/Filter/{0}.png'}`)
//! or the name itself, on a plate that changes on the open preset; some themes stack the
//! presets down the screen (an icon rail). Each of Lodestar's tabs is drawn the way that
//! template draws a preset of the same name, at rest and when open. Themes without such a
//! template get Lodestar's names, the open one bold.

mod read;

use super::bind::parse_binding;
use super::build::{raw_setter, spec};
use super::visibility::vis_of;
use super::*;
use read::{Item, Reading};

/// The preset names a tab's picture is looked up by, per tab (`TABS` order): its own label,
/// then the name of Playnite's own default preset for the same games (Playnite creates "All",
/// "Recently Played" and "Favorites"; a theme's pictures for those are what a Playnite user
/// with the default presets sees). Playnite matches a picture by the exact name.
const PRESET_NAMES: [&[&str]; 4] =
    [&["All Games", "All"], &["Continue Playing", "Recently Played"], &["Favourites", "Favorites"], &["Installed"]];

/// Room between Lodestar's tab names as the layout measures the strip.
const TAB_GAP: &str = "    ";

/// A picture without a size of its own or of its holders and of unknown natural size, px.
const DEFAULT_PICTURE: f32 = 40.0;

/// The most a picture with no size anywhere takes (WPF would draw it at its natural size,
/// which for preset icons can be 512 px), px.
const MAX_PICTURE: f32 = 200.0;

/// Rough average glyph width of the strip's names, em (only to size the strip's box: the
/// renderer measures the real text).
const GLYPH_EM: f32 = 0.55;

/// The strip as the theme's template draws it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct PresetLook {
    /// The presets stack down the screen.
    pub vertical: bool,
    /// Per tab (`TABS` order): its button at rest and when it is the open preset; None when
    /// the theme hides that preset. Empty: no template of the theme's to read (Lodestar's
    /// names are drawn).
    pub items: Vec<Option<[PresetItem; 2]>>,
    /// The open collection when it is none of the tabs: its name in the open look.
    pub other: Option<PresetItem>,
    /// Where the buttons sit across the strip ("start", "center", "end").
    pub align: &'static str,
    /// The strip's size as the buttons add up, px (a rough measure for the layout).
    pub size: [f32; 2],
    /// Where the selector's template puts its presets across the selector's own box (an items
    /// control centred in it): Stretch when nothing in the template aligns them.
    pub across: Al,
}

/// How the selector's template lays out its presets.
#[derive(Default)]
struct PanelLayout {
    /// The presets stack down the screen.
    vertical: bool,
    /// The template numbers its items with `AlternationCount`.
    alternation: bool,
    /// See `PresetLook::across`.
    across: Al,
}

/// One preset button in one state.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct PresetItem {
    pub picture: Option<Picture>,
    /// The box of a picture the preset lacks, kept empty, px: the template draws a stand-in
    /// there that Lodestar can't (an icon-font glyph), and the strip keeps its rhythm.
    pub blank: Option<[f32; 2]>,
    /// The preset's name and its opacity, when the template shows it.
    pub text: Option<(TextOut, f32)>,
    /// What the template paints behind it (or only the button's fixed box).
    pub plate: Option<Plate>,
    /// Room around the button (its plate when it has one), px [left, top, right, bottom].
    pub margin: [f32; 4],
    pub opacity: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Picture {
    /// The theme's file until the strip is done, then its path in the Lodestar theme.
    pub path: String,
    /// Its box, px (the picture fits inside keeping its aspect).
    pub size: [f32; 2],
    pub opacity: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Plate {
    pub fill: Option<LookBrush>,
    pub border: Option<(Color, f32)>,
    pub radius: f32,
    /// Its size when the template fixes it, px (0: as big as what it holds).
    pub size: [f32; 2],
    /// Room between its edge and what it holds, px [x, y].
    pub padding: [f32; 2],
}

impl PresetItem {
    /// Its rough size with its margin, px.
    fn extent(&self) -> [f32; 2] {
        let text = self.text.as_ref().map(|(t, _)| {
            let bold = if t.font.is_some() { 1.1 } else { 1.0 };
            [t.text.chars().count() as f32 * t.size * GLYPH_EM * bold, t.size * 1.344]
        });
        let pic = self.picture.as_ref().map(|p| p.size).or(self.blank);
        let content = match (pic, text) {
            (Some(p), Some(t)) => [p[0] + t[0] + t[1] * 0.3, p[1].max(t[1])],
            (Some(c), None) | (None, Some(c)) => c,
            (None, None) => [0.0, 0.0],
        };
        let boxed = |a: usize| match &self.plate {
            Some(p) if p.size[a] > 0.0 => p.size[a],
            Some(p) => content[a] + 2.0 * p.padding[a],
            None => content[a],
        };
        // The renderer keeps no negative room (WPF would overlap the neighbours).
        let m = self.margin.map(|v| v.max(0.0));
        [boxed(0) + m[0] + m[2], boxed(1) + m[1] + m[3]]
    }
}

/// Layers painted over each other: `top` over `under` (solid colours blend, anything else
/// covers what is under it).
fn over(top: LookBrush, under: Option<LookBrush>) -> LookBrush {
    let (LookBrush::Solid { color: t }, Some(LookBrush::Solid { color: u })) = (&top, &under) else { return top };
    let (ta, ua) = (t.0[3] as f32 / 255.0, u.0[3] as f32 / 255.0);
    let a = ta + ua * (1.0 - ta);
    if a <= 0.0 {
        return top;
    }
    let ch = |i: usize| ((t.0[i] as f32 * ta + u.0[i] as f32 * ua * (1.0 - ta)) / a).round() as u8;
    LookBrush::solid(Color([ch(0), ch(1), ch(2), (a * 255.0).round() as u8]))
}

/// A brush with an opacity folded into its alpha.
fn faded(b: LookBrush, opacity: f32) -> LookBrush {
    match b {
        LookBrush::Solid { color } if opacity < 1.0 => {
            LookBrush::solid(color.with_alpha((color.0[3] as f32 * opacity).round() as u8))
        }
        other => other,
    }
}

impl<'b, 't, 'i> Builder<'b, 't, 'i> {
    /// The FilterPresetSelector `el` → the tab strip (`n.data.kind`) and the room it wants.
    pub(super) fn preset_strip(&mut self, el: X<'t, 'i>, inh: &Inherit, n: &mut LNode) {
        let (fg, current) = self.tab_colors();
        let mut label = self.text_out(el, inh, String::new(), false);
        label.color = fg.or(label.color);
        let look = self.preset_look(el, inh);
        let names: Vec<&str> = TABS.iter().map(|(_, l)| *l).collect();
        n.content = if !look.items.is_empty() {
            Content::Fixed(look.size)
        } else if look.vertical {
            Content::Fixed([label.size * 8.0, label.size * 1.344 * 1.2 * 4.0])
        } else {
            Content::Text(TextSpec { text: names.join(TAB_GAP), ..spec(&label) })
        };
        // Faded out in this view (a strip that hides while the game's page is open).
        let unseen = |i: &PresetItem| i.opacity <= 0.01;
        if !look.items.is_empty() && look.items.iter().flatten().flatten().all(unseen) {
            n.content = Content::None;
            return;
        }
        n.data.kind = Kind::Tabs(Box::new(TabsOut { label, current, look }));
    }

    /// The strip's direction and, when the theme has a preset template, each tab's button.
    fn preset_look(&mut self, selector: X<'t, 'i>, inh: &Inherit) -> PresetLook {
        let PanelLayout { vertical, alternation, across } = self.presets_panel(selector);
        let mut out = PresetLook { vertical, across, align: "center", ..Default::default() };
        let Some(style) = self.t.resource("ItemFilterQuickPreset") else { return out };
        let fallback_h = self.num(selector, "Height").filter(|h| *h > 0.0);
        let mut items = vec![];
        for (index, names) in PRESET_NAMES.into_iter().enumerate() {
            let mut states = vec![];
            for open in [false, true] {
                let item = Item { names, index, open };
                let Some(r) = self.read_item(style, selector, item, alternation) else { return out };
                // A preset the template collapses (one of the first few only) has no button.
                if !open
                    && self.button_value(&r, "Visibility").and_then(|v| v.text().map(vis_of)) == Some(Vis::Collapsed)
                {
                    break;
                }
                states.push(self.preset_item(&r, inh, fallback_h));
            }
            items.push(<[PresetItem; 2]>::try_from(states).ok());
        }
        // Neither a name nor a picture anywhere: not a template Lodestar can follow.
        let drawn = |i: &PresetItem| i.picture.is_some() || i.text.is_some();
        if !items.iter().flatten().flatten().any(drawn) {
            return out;
        }
        let other = Item { names: &[], index: 0, open: true };
        out.other = self.read_item(style, selector, other, alternation).map(|r| self.preset_item(&r, inh, fallback_h));
        let accent = self.tab_colors().1;
        for item in items.iter_mut().flatten() {
            mark_open(item, accent);
        }
        out.align = match self.t.setter(style, "VerticalAlignment").and_then(|v| v.text().map(str::to_ascii_lowercase))
        {
            Some(a) if a == "top" => "start",
            Some(a) if a == "bottom" => "end",
            _ => "center",
        };
        out.size = strip_size(&items, vertical);
        out.items = items;
        self.copy_pictures(&mut out);
        out
    }

    /// The template's look for one preset in one state.
    fn preset_item(&self, r: &Reading<'t, 'i>, inh: &Inherit, fallback_h: Option<f32>) -> PresetItem {
        let parts: Vec<X> = r.root.descendants().filter(|d| d.is_element()).collect();
        let picture = parts
            .iter()
            .filter(|d| tag(**d) == "Image" && self.item_shown(r, **d))
            .filter_map(|d| Some((*d, self.preset_file(&path_format(*d)?, r.item.names)?)))
            .last()
            .map(|(img, file)| (img, self.picture(r, img, file, fallback_h)));
        let names: Vec<X> = parts.iter().copied().filter(|d| tag(*d) == "TextBlock" && self.shows_name(*d)).collect();
        let shown = names.iter().copied().find(|d| self.item_shown(r, *d));
        // A template that pictures its presets and hides their names here is an icon strip: a
        // preset without a picture of its own shows no name either (Playnite draws the
        // template's stand-in glyph or nothing there). The open collection when it is none of
        // the tabs keeps its name: Lodestar looks up no picture for it.
        let icons_only =
            !r.item.names.is_empty() && parts.iter().any(|d| tag(*d) == "Image" && path_format(*d).is_some());
        // Otherwise, with nothing of the preset drawn (a glyph Lodestar can't draw): its name.
        let text_part = shown.or_else(|| names.first().copied().filter(|_| picture.is_none() && !icons_only));
        let blank = parts
            .iter()
            .copied()
            .find(|d| tag(*d) == "Image" && path_format(*d).is_some())
            .filter(|_| icons_only && picture.is_none() && text_part.is_none());
        let text = text_part.map(|tb| {
            let mut t = self.preset_text(r, tb, inh);
            t.text = r.item.names.first().copied().unwrap_or_default().to_string();
            (t, if shown.is_some() { self.item_opacity(r, tb) } else { 1.0 })
        });
        let content = picture.as_ref().map(|p| p.0).or(text_part).or(blank);
        let mut item = PresetItem {
            picture: picture.map(|p| p.1),
            blank: blank.map(|img| self.picture_box(r, img, None, fallback_h)),
            text,
            opacity: self.button_value(r, "Opacity").and_then(|v| v.num()).map_or(1.0, |o| o.clamp(0.0, 1.0) as f32),
            ..Default::default()
        };
        self.place_item(r, &mut item, content);
        item
    }

    /// The item's box: the template's plate around `content` (what the item shows), else
    /// the button's own fixed box, else just the room around `content`.
    fn place_item(&self, r: &Reading<'t, 'i>, item: &mut PresetItem, content: Option<X<'t, 'i>>) {
        let inner = content.map(|c| self.inset(r, c)).unwrap_or([0.0; 4]);
        if let Some((part, mut plate)) = self.plate(r) {
            let outer = self.inset(r, part);
            let gap = |a: usize, b: usize| ((inner[a] - outer[a]).max(0.0) + (inner[b] - outer[b]).max(0.0)) / 2.0;
            plate.padding = [gap(0, 2), gap(1, 3)];
            plate.size = [self.room(r, part, "Width").unwrap_or(0.0), self.room(r, part, "Height").unwrap_or(0.0)];
            item.margin = outer;
            item.plate = Some(plate);
            return;
        }
        let own = |d: &str| self.button_value(r, d).and_then(|v| v.num()).map_or(0.0, |v| v.max(0.0) as f32);
        let size = [own("Width"), own("Height")];
        if size[0] > 0.0 || size[1] > 0.0 {
            // A button of a fixed size holds what it shows in its middle.
            item.margin = self.button_value(r, "Margin").and_then(|v| v.sides()).unwrap_or([0.0; 4]);
            item.plate = Some(Plate { size, ..Default::default() });
        } else {
            item.margin = inner;
        }
    }

    /// A TextBlock showing the preset's name.
    fn shows_name(&self, tb: X<'t, 'i>) -> bool {
        let raw = attr(tb, "Text")
            .map(str::to_string)
            .or_else(|| self.t.style_of(tb).and_then(|s| raw_setter(self.t, s, "Text")));
        match raw.as_deref().map(tree::markup) {
            Some(Markup::Binding(b)) => parse_binding(&b).path.trim().trim_start_matches("Path=") == "Name",
            _ => false,
        }
    }

    /// The name's text look: each property from the TextBlock, else the parts holding it
    /// (WPF inherits Foreground and the font down the tree), else the button, else the strip.
    fn preset_text(&self, r: &Reading<'t, 'i>, tb: X<'t, 'i>, inh: &Inherit) -> TextOut {
        let chain = self.item_chain(r, tb);
        let first = |prop: &str| {
            chain.iter().find_map(|p| self.item_value(r, *p, prop, 0)).or_else(|| self.button_value(r, prop))
        };
        let own = Inherit {
            fg: first("Foreground").and_then(|v| v.brush()).or_else(|| inh.fg.clone()),
            size: first("FontSize").and_then(|v| v.num()).map(|v| v as f32).filter(|v| *v > 0.0).unwrap_or(inh.size),
            family: first("FontFamily").and_then(|v| v.text().map(str::to_string)).or_else(|| inh.family.clone()),
            weight: first("FontWeight").and_then(|v| v.text().map(str::to_string)).or_else(|| inh.weight.clone()),
        };
        let mut t = self.text_out(tb, &own, String::new(), false);
        let converter = attr(tb, "Text").map(|raw| match tree::markup(raw) {
            Markup::Binding(b) => parse_binding(&b).converter.unwrap_or_default(),
            _ => String::new(),
        });
        t.upper |= converter.is_some_and(|c| c.contains("UpperCase") || c.contains("ToUpper"));
        t
    }

    /// The preset's picture `img` showing `file`, its box sized the way WPF's Uniform stretch
    /// sizes it: its own Width / Height, else the room its holders give it, the other side
    /// from the picture's aspect.
    fn picture(&self, r: &Reading<'t, 'i>, img: X<'t, 'i>, file: String, fallback_h: Option<f32>) -> Picture {
        let natural = (self.dims)(&file).filter(|d| d[0] > 0.0 && d[1] > 0.0);
        let size = self.picture_box(r, img, natural, fallback_h);
        Picture { path: file, size, opacity: self.item_opacity(r, img) }
    }

    /// The box of the picture part `img` for a picture of `natural` size (None: unknown).
    fn picture_box(
        &self,
        r: &Reading<'t, 'i>,
        img: X<'t, 'i>,
        natural: Option<[f32; 2]>,
        fallback_h: Option<f32>,
    ) -> [f32; 2] {
        let [nw, nh] = natural.unwrap_or([DEFAULT_PICTURE, DEFAULT_PICTURE]);
        let own = |p: &str| self.item_num(r, img, p).filter(|v| *v > 0.0);
        let (w, h) = (own("Width"), own("Height"));
        match (w, h) {
            (Some(w), Some(h)) => [w, h],
            _ => {
                let room = [w.or_else(|| self.room(r, img, "Width")), h.or_else(|| self.room(r, img, "Height"))];
                let scale = match room {
                    [Some(rw), Some(rh)] => (rw / nw).min(rh / nh),
                    [Some(rw), None] => rw / nw,
                    [None, Some(rh)] => rh / nh,
                    [None, None] => fallback_h.map_or(1.0, |fh| (fh / nh).min(1.0)).min(MAX_PICTURE / nw.max(nh)),
                };
                [nw * scale, nh * scale]
            }
        }
    }

    /// What the template paints behind the preset: its shown Borders / Rectangles with a
    /// fill or an all-round outline, later ones over earlier ones. The plate's shape is the
    /// first filled part's (else the first outlined one's). Thin parts (bars, bullets) aren't
    /// plates.
    fn plate(&self, r: &Reading<'t, 'i>) -> Option<(X<'t, 'i>, Plate)> {
        const THIN: f32 = 8.0;
        let mut shape: Option<(X, bool)> = None;
        let mut plate = Plate::default();
        for part in r.root.descendants().filter(|d| d.is_element() && matches!(tag(*d), "Border" | "Rectangle")) {
            let thin = ["Width", "Height"].iter().any(|d| self.item_num(r, part, d).is_some_and(|v| v < THIN));
            if thin || !self.item_shown(r, part) {
                continue;
            }
            let rect = tag(part) == "Rectangle";
            let value = |p: &str| self.item_value(r, part, p, 0);
            let opacity = self.item_opacity(r, part);
            let fill = value(if rect { "Fill" } else { "Background" })
                .and_then(|v| v.brush())
                .filter(|b| !b.is_clear() && !matches!(b, LookBrush::Image { .. }))
                .map(|b| faded(b, opacity));
            let sides = value(if rect { "StrokeThickness" } else { "BorderThickness" }).and_then(|v| v.sides());
            let stroke = value(if rect { "Stroke" } else { "BorderBrush" })
                .and_then(|v| v.color())
                .map(|c| c.with_alpha((c.0[3] as f32 * opacity).round() as u8))
                .filter(|c| c.0[3] > 0)
                .zip(sides.filter(|s| s.iter().all(|v| *v > 0.0)).map(|s| s.iter().copied().fold(0.0, f32::max)));
            if fill.is_none() && stroke.is_none() {
                continue;
            }
            let radius = if rect {
                value("RadiusX").and_then(|v| v.num()).map(|v| v as f32)
            } else {
                value("CornerRadius").and_then(|v| v.max_side())
            };
            if shape.is_none_or(|(_, filled)| !filled && fill.is_some()) {
                shape = Some((part, fill.is_some()));
                plate.radius = radius.unwrap_or(0.0);
            }
            if let Some(f) = fill {
                plate.fill = Some(over(f, plate.fill.take()));
            }
            if stroke.is_some() {
                plate.border = stroke;
            }
        }
        shape.map(|(part, _)| (part, plate))
    }

    /// The theme's file for a preset named one of `names` through `format`, matched as
    /// Playnite matches it: the exact name (Windows file names ignore case).
    fn preset_file(&self, format: &str, names: &[&str]) -> Option<String> {
        names.iter().find_map(|name| {
            let rel = format.replace("{0}", name);
            Some(self.t.files.iter().find(|(f, _)| f.eq_ignore_ascii_case(&rel))?.0.clone())
        })
    }

    /// Theme files of the pictures the strip shows → their paths in the Lodestar theme.
    fn copy_pictures(&mut self, look: &mut PresetLook) {
        for item in look.items.iter_mut().flatten().flatten().chain(look.other.iter_mut()) {
            if let Some(p) = &mut item.picture {
                p.path = self.theme_image(&p.path);
            }
        }
    }

    /// The filter preset tab colours when the theme's template isn't read: the theme's
    /// `ItemFilterQuickPreset` style (its text colour, and the one its IsChecked / IsFocused
    /// triggers give), else the accent glyph colour for the open tab.
    fn tab_colors(&self) -> (Option<Color>, Option<Color>) {
        let colour = |v: Option<Value>| v.and_then(|v| v.brush()).and_then(|b| b.average());
        let style = self.t.resource("ItemFilterQuickPreset");
        let normal = style.and_then(|s| colour(self.t.setter(s, "Foreground")));
        let mut current = None;
        if let Some(tpl) = style.and_then(|s| self.t.template(s)) {
            for state in ["IsChecked", "IsFocused"] {
                let set =
                    self.t.triggers(tpl).into_iter().filter(|tr| attr(*tr, "Property") == Some(state)).find_map(|tr| {
                        tree::setters(tr)
                            .find(|c| attr(*c, "Property").is_some_and(|p| p.trim().ends_with("Foreground")))
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

    /// How the selector's template lays out its presets.
    fn presets_panel(&self, selector: X<'t, 'i>) -> PanelLayout {
        let Some(tpl) = self.t.style_of(selector).and_then(|s| self.t.template(s)) else {
            return PanelLayout::default();
        };
        let alternation = tpl.descendants().any(|d| {
            d.is_element() && attr(d, "AlternationCount").is_some_and(|c| c.trim() != "0" && !c.trim().is_empty())
        });
        // Its ItemsPanelTemplate, or an element marked IsItemsHost.
        let panel = tpl.descendants().find_map(|d| {
            if !d.is_element() {
                return None;
            }
            if tag(d) == "ItemsPanelTemplate" {
                return content(d).next();
            }
            attr(d, "IsItemsHost").filter(|v| v.eq_ignore_ascii_case("true")).map(|_| d)
        });
        PanelLayout {
            vertical: panel.is_some_and(|p| self.stacks_down(p)),
            alternation,
            across: panel.map_or(Al::Stretch, |p| self.presets_across(p)),
        }
    }

    /// The presets' panel stacks them down the screen.
    fn stacks_down(&self, panel: X<'t, 'i>) -> bool {
        matches!(tag(panel), "StackPanel" | "VirtualizingStackPanel" | "WrapPanel")
            && self.text(panel, "Orientation").is_some_and(|o| o.eq_ignore_ascii_case("vertical"))
    }

    /// The first HorizontalAlignment other than Stretch from the template's root in to the
    /// presets' panel: an element sized to its content and aligned in the selector's box puts
    /// everything inside it there.
    fn presets_across(&self, panel: X<'t, 'i>) -> Al {
        let chain: Vec<X> = std::iter::once(panel)
            .chain(panel.ancestors())
            .take_while(|a| !matches!(tag(*a), "ControlTemplate" | "Setter.Value"))
            .filter(|a| a.is_element() && !tag(*a).contains('.') && tag(*a) != "ItemsPanelTemplate")
            .collect();
        chain
            .iter()
            .rev()
            .filter_map(|a| self.text(*a, "HorizontalAlignment").and_then(|h| Al::parse(&h)))
            .find(|a| *a != Al::Stretch)
            .unwrap_or(Al::Stretch)
    }
}

/// The open preset looks just like the others (a theme that marks it only while it has the
/// focus): Lodestar's own cue, its name bold in the `accent` colour, else the other presets
/// dimmed.
fn mark_open(item: &mut [PresetItem; 2], accent: Option<Color>) {
    if item[0] != item[1] {
        return;
    }
    match &mut item[1].text {
        Some((t, _)) => {
            t.font = Some("bold");
            t.color = accent.or(t.color);
        }
        None => item[0].opacity *= 0.5,
    }
}

/// The strip's size from its buttons at rest, px.
fn strip_size(items: &[Option<[PresetItem; 2]>], vertical: bool) -> [f32; 2] {
    let sizes: Vec<[f32; 2]> = items.iter().flatten().map(|i| i[0].extent()).collect();
    let (along, across) = if vertical { (1, 0) } else { (0, 1) };
    let mut out = [0.0; 2];
    out[along] = sizes.iter().map(|s| s[along]).sum();
    out[across] = sizes.iter().map(|s| s[across]).fold(0.0, f32::max);
    out
}

/// The `PathFormat` of an Image whose Source is a picture per preset name.
fn path_format(img: X) -> Option<String> {
    let src = attr(img, "Source")?;
    if !src.contains("ThemeFileBinding") || !src.contains("Name") {
        return None;
    }
    let rest = src.split("PathFormat=").nth(1)?.trim_start();
    let quote = rest.chars().next().filter(|c| *c == '\'' || *c == '"')?;
    let body = &rest[1..];
    let end = body.find(quote)?;
    let format = body[..end].replace('\\', "/");
    format.contains("{0}").then_some(format)
}
