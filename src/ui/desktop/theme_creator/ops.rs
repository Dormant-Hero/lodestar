//! Editing operations on a theme's elements: the palette of element kinds with good starting
//! values, finding nested elements, adding / duplicating / deleting / reordering, and changes
//! that apply to one variant, screen shape or console only (patches by element name).

use crate::theme::*;
use egui::Rect;

/// Where an element sits: its index in the view, then (for elements inside a stack or a list's
/// item template) the way down.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ElPath {
    pub top: usize,
    pub sub: Vec<(Nest, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Nest {
    /// A stack's `children`.
    Children,
    /// A list's `item_elements` (drawn over every item).
    Items,
}

impl ElPath {
    pub fn top(i: usize) -> ElPath {
        ElPath { top: i, sub: vec![] }
    }

    pub fn child(&self, n: Nest, i: usize) -> ElPath {
        let mut p = self.clone();
        p.sub.push((n, i));
        p
    }

    pub fn parent(&self) -> Option<ElPath> {
        let mut p = self.clone();
        p.sub.pop()?;
        Some(p)
    }
}

fn nested(el: &Element, n: Nest) -> Option<&Vec<Element>> {
    match (n, el) {
        (Nest::Children, Element::Stack { children, .. }) => Some(children),
        (Nest::Items, e) => Some(&e.common().item_elements),
        _ => None,
    }
}

fn nested_mut(el: &mut Element, n: Nest) -> Option<&mut Vec<Element>> {
    match (n, el) {
        (Nest::Children, Element::Stack { children, .. }) => Some(children),
        (Nest::Items, e) => Some(&mut e.common_mut().item_elements),
        _ => None,
    }
}

pub fn get<'a>(els: &'a [Element], p: &ElPath) -> Option<&'a Element> {
    let mut e = els.get(p.top)?;
    for (n, i) in &p.sub {
        e = nested(e, *n)?.get(*i)?;
    }
    Some(e)
}

pub fn get_mut<'a>(els: &'a mut [Element], p: &ElPath) -> Option<&'a mut Element> {
    let mut e = els.get_mut(p.top)?;
    for (n, i) in &p.sub {
        e = nested_mut(e, *n)?.get_mut(*i)?;
    }
    Some(e)
}

/// The list an element is in (the view's elements, or its parent's children / items).
fn siblings_mut<'a>(els: &'a mut Vec<Element>, p: &ElPath) -> Option<(&'a mut Vec<Element>, usize)> {
    match p.sub.split_last() {
        None => Some((els, p.top)),
        Some(((n, i), _)) => {
            let parent = get_mut(els, &p.parent()?)?;
            Some((nested_mut(parent, *n)?, *i))
        }
    }
}

/// What the palette offers to add.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Backdrop,
    Picture,
    GameArt,
    Logo,
    Text,
    Title,
    Description,
    Panel,
    Fade,
    ProgressBar,
    Carousel,
    Wheel,
    Grid,
    List,
    CoverRows,
    Rail,
    Stack,
    Chips,
    Rating,
    Clock,
    Date,
    Battery,
    Badges,
    HelpBar,
    Video,
}

impl Kind {
    /// (group, kinds) in the order the palette lists them.
    pub const GROUPS: [(&'static str, &'static [Kind]); 5] = [
        ("Pictures", &[Kind::Backdrop, Kind::GameArt, Kind::Logo, Kind::Picture, Kind::Video]),
        ("Text", &[Kind::Title, Kind::Text, Kind::Description, Kind::Chips, Kind::Date]),
        ("Shapes", &[Kind::Panel, Kind::Fade, Kind::ProgressBar, Kind::Stack]),
        ("Lists", &[Kind::Carousel, Kind::Wheel, Kind::Grid, Kind::List, Kind::CoverRows, Kind::Rail]),
        ("Extras", &[Kind::Rating, Kind::Badges, Kind::Clock, Kind::Battery, Kind::HelpBar]),
    ];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Backdrop => "Background art",
            Kind::Picture => "Picture from a file",
            Kind::GameArt => "Cover",
            Kind::Logo => "Game logo",
            Kind::Text => "Text",
            Kind::Title => "Game title",
            Kind::Description => "Description",
            Kind::Panel => "Panel",
            Kind::Fade => "Fade",
            Kind::ProgressBar => "Progress bar",
            Kind::Carousel => "Carousel",
            Kind::Wheel => "Wheel",
            Kind::Grid => "Grid",
            Kind::List => "Text list",
            Kind::CoverRows => "Cover rows",
            Kind::Rail => "Navigation rail",
            Kind::Stack => "Stack (flows its parts)",
            Kind::Chips => "Pills (genres …)",
            Kind::Rating => "Star rating",
            Kind::Clock => "Clock",
            Kind::Date => "Last played date",
            Kind::Battery => "Battery level",
            Kind::Badges => "Badges",
            Kind::HelpBar => "Button hints",
            Kind::Video => "Video",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Kind::Backdrop => "The selected game's background, filling the screen",
            Kind::Picture => "A picture you add to the theme",
            Kind::GameArt => "The selected game's cover",
            Kind::Logo => "The selected game's logo",
            Kind::Text => "Any text, with values like the game's name",
            Kind::Title => "The selected game's name",
            Kind::Description => "The game's description, scrolling when long",
            Kind::Panel => "A coloured box with rounded corners",
            Kind::Fade => "A gradient that keeps text readable over art",
            Kind::ProgressBar => "A bar filled by a value (achievements)",
            Kind::Carousel => "A row of items that scrolls, the selection in the middle",
            Kind::Wheel => "Items on a curve, like a wheel",
            Kind::Grid => "Covers in rows and columns",
            Kind::List => "A list of names",
            Kind::CoverRows => "Rows of covers, one per collection (start page)",
            Kind::Rail => "Home, All games, Platforms … down the side",
            Kind::Stack => "Parts placed one after the other; empty ones take no room",
            Kind::Chips => "Genres or facts as small pills",
            Kind::Rating => "0 to 5 stars",
            Kind::Clock => "The time",
            Kind::Date => "When the game was last played",
            Kind::Battery => "Battery level on handhelds and laptops",
            Kind::Badges => "Small icons: favourite, completed, controller",
            Kind::HelpBar => "Which button does what",
            Kind::Video => "A video file played in a box (needs ffmpeg)",
        }
    }

    pub fn base_name(self) -> &'static str {
        match self {
            Kind::Backdrop => "backdrop",
            Kind::Picture => "picture",
            Kind::GameArt => "cover",
            Kind::Logo => "logo",
            Kind::Text => "text",
            Kind::Title => "title",
            Kind::Description => "description",
            Kind::Panel => "panel",
            Kind::Fade => "fade",
            Kind::ProgressBar => "progress",
            Kind::Carousel => "carousel",
            Kind::Wheel => "wheel",
            Kind::Grid => "grid",
            Kind::List => "list",
            Kind::CoverRows => "rows",
            Kind::Rail => "rail",
            Kind::Stack => "stack",
            Kind::Chips => "chips",
            Kind::Rating => "rating",
            Kind::Clock => "clock",
            Kind::Date => "lastplayed",
            Kind::Battery => "battery",
            Kind::Badges => "badges",
            Kind::HelpBar => "help",
            Kind::Video => "video",
        }
    }
}

/// A short word for an element's kind (the elements panel).
pub fn kind_word(e: &Element) -> &'static str {
    match e {
        Element::Image { video: Some(_), .. } => "Video",
        Element::Image { .. } => "Picture",
        Element::Text { common, .. } if common.chips => "Pills",
        Element::Text { .. } => "Text",
        Element::Rect { fill: Some(_), .. } => "Bar",
        Element::Rect { .. } => "Shape",
        Element::Carousel { orientation, .. } if *orientation != Orientation::Horizontal => "Wheel",
        Element::Carousel { .. } => "Carousel",
        Element::Grid { .. } => "Grid",
        Element::TextList { .. } => "List",
        Element::Rating { .. } => "Rating",
        Element::DateTime { clock: true, .. } => "Clock",
        Element::DateTime { .. } => "Date",
        Element::Rail { .. } => "Rail",
        Element::Rows { .. } => "Rows",
        Element::Badges { .. } => "Badges",
        Element::Stack { .. } => "Stack",
        Element::HelpBar { .. } => "Hints",
    }
}

/// What an element shows, for the elements panel ("{game.name}", "{art.hero}").
pub fn summary(e: &Element) -> String {
    let s = match e {
        Element::Image { path, video, .. } => video.as_deref().unwrap_or(path),
        Element::Text { text, .. } => text,
        Element::Carousel { item_image, item_text, .. }
        | Element::Grid { item_image, item_text, .. }
        | Element::Rows { item_image, item_text, .. } => {
            if item_image.is_empty() {
                item_text
            } else {
                item_image
            }
        }
        Element::TextList { item_text, .. } => item_text,
        Element::Rating { value, .. } => value,
        Element::DateTime { value, clock, format, .. } => {
            if *clock {
                format
            } else {
                value
            }
        }
        Element::Rect { fill: Some(f), .. } => f,
        Element::Stack { children, .. } => return format!("{} parts", children.len()),
        _ => "",
    };
    let s = s.replace('\n', " ");
    if s.chars().count() > 40 { format!("{}…", s.chars().take(39).collect::<String>()) } else { s }
}

/// A name no other element of `els` (nested ones too) has: `base`, `base2`, `base3` …
pub fn unique_name(els: &[Element], base: &str) -> String {
    let mut taken = std::collections::HashSet::new();
    for e in els {
        e.for_each_nested(&mut |x| {
            taken.insert(x.common().name.clone());
        });
    }
    if !taken.contains(base) {
        return base.to_string();
    }
    (2..).map(|n| format!("{base}{n}")).find(|n| !taken.contains(n)).expect("a free name")
}

fn style(font: Option<&str>, size: f32, color: Color) -> TextStyle {
    TextStyle { font: font.map(str::to_string), font_size: size, color, ..Default::default() }
}

/// A new element of `kind` for a view, placed and styled to look right straight away
/// (colours from the palette, a sensible size near the middle of the screen).
pub fn new_element(kind: Kind, view: ViewKind, pal: &Palette, name: String) -> Element {
    let common = |pos: [f32; 2], size: [f32; 2]| Common { name: name.clone(), pos, size, z: 3.0, ..Default::default() };
    let games = view != ViewKind::Collections;
    let (art, cover, logo) = if games {
        ("{art.hero}", "{art.cover}", "{art.logo}")
    } else {
        ("{collection.hero}", "{collection.cover}", "{collection.logo}")
    };
    let title_text = if games { "{game.name}" } else { "{collection.name}" };
    match kind {
        Kind::Backdrop => Element::Image {
            common: Common { z: 0.0, blur: 0.02, opacity: 0.6, fade: Some(0.35), ..common([0.0, 0.0], [1.0, 1.0]) },
            path: art.into(),
            fallbacks: vec![cover.into()],
            fit: Fit::Cover,
            tint: None,
            tile: false,
            corner_radius: 0.0,
            gradient: None,
            tint_end: None,
            fallback_text: None,
            fallback_style: None,
            video: None,
        },
        Kind::Picture | Kind::Video => Element::Image {
            common: common([0.4, 0.35], [0.2, 0.0]),
            path: String::new(),
            fallbacks: vec![],
            fit: Fit::Contain,
            tint: None,
            tile: false,
            corner_radius: 0.0,
            gradient: None,
            tint_end: None,
            fallback_text: None,
            fallback_style: None,
            video: (kind == Kind::Video).then(String::new),
        },
        Kind::GameArt => Element::Image {
            common: Common { placeholder: true, ..common([0.06, 0.25], [0.18, 0.4]) },
            path: cover.into(),
            fallbacks: vec![],
            fit: Fit::Contain,
            tint: None,
            tile: false,
            corner_radius: 0.012,
            gradient: None,
            tint_end: None,
            fallback_text: None,
            fallback_style: None,
            video: None,
        },
        Kind::Logo => Element::Image {
            common: Common { origin: [0.0, 1.0], ..common([0.05, 0.4], [0.3, 0.16]) },
            path: logo.into(),
            fallbacks: vec![],
            fit: Fit::Contain,
            tint: None,
            tile: false,
            corner_radius: 0.0,
            gradient: None,
            tint_end: None,
            fallback_text: Some(title_text.into()),
            fallback_style: Some(style(Some("title"), 0.06, pal.text)),
            video: None,
        },
        Kind::Text | Kind::Title | Kind::Description => {
            let (text, st, pos, size, wrap, scroll) = match kind {
                Kind::Title => (
                    title_text,
                    TextStyle { shadow: Some(Color([0, 0, 0, 128])), ..style(Some("title"), 0.06, pal.text) },
                    [0.05, 0.12],
                    [0.6, 0.0],
                    false,
                    false,
                ),
                Kind::Description => {
                    ("{game.description}", style(None, 0.024, pal.text_dim), [0.05, 0.5], [0.4, 0.2], true, true)
                }
                _ => ("Text", style(None, 0.03, pal.text), [0.05, 0.3], [0.0, 0.0], false, false),
            };
            Element::Text {
                common: common(pos, size),
                text: text.into(),
                style: st,
                background: None,
                wrap,
                max_lines: None,
                scroll,
            }
        }
        Kind::Chips => Element::Text {
            common: Common {
                chips: true,
                radius: Some(0.012),
                padding: Some([0.012, 0.005]),
                ..common([0.05, 0.3], [0.5, 0.0])
            },
            text: "{game.genrelist}|{game.year}".into(),
            style: style(None, 0.02, pal.text),
            background: Some(pal.surface_alt.with_alpha(200)),
            wrap: false,
            max_lines: None,
            scroll: false,
        },
        Kind::Panel => Element::Rect {
            common: Common { z: 1.0, ..common([0.05, 0.6], [0.4, 0.3]) },
            color: pal.surface.with_alpha(220),
            corner_radius: 0.016,
            border_color: None,
            border_width: 0.0,
            gradient: None,
            fill: None,
            track: None,
        },
        Kind::Fade => Element::Rect {
            common: Common { z: 1.0, ..common([0.0, 0.5], [1.0, 0.5]) },
            color: pal.background.with_alpha(0),
            corner_radius: 0.0,
            border_color: None,
            border_width: 0.0,
            gradient: Some([pal.background.with_alpha(0), pal.background.with_alpha(240)]),
            fill: None,
            track: None,
        },
        Kind::ProgressBar => Element::Rect {
            common: Common { visible_if: Some("{game.achievements}".into()), ..common([0.05, 0.45], [0.2, 0.008]) },
            color: pal.accent,
            corner_radius: 0.004,
            border_color: None,
            border_width: 0.0,
            gradient: None,
            fill: Some("{game.achievementspercent}".into()),
            track: Some(pal.surface_alt),
        },
        Kind::Carousel | Kind::Wheel => {
            let wheel = kind == Kind::Wheel;
            let (image, fallbacks, text) = if games {
                ("{art.cover}", vec![], "{game.name}")
            } else {
                ("{collection.hero}", vec!["{collection.cover}".to_string()], "{collection.name}")
            };
            Element::Carousel {
                common: Common {
                    item_fit: Some(Fit::Cover),
                    radius: Some(0.014),
                    item_fill: Some(pal.surface_alt),
                    controller_badge: games,
                    favorite_badge: games,
                    item_caption: (!games).then(|| "{collection.name}".to_string()),
                    ..if wheel { common([0.62, 0.0], [0.38, 1.0]) } else { common([0.0, 0.62], [1.0, 0.32]) }
                },
                orientation: if wheel { Orientation::Wheel } else { Orientation::Horizontal },
                item_size: if wheel {
                    [0.3, 0.16]
                } else if games {
                    [0.12, 0.27]
                } else {
                    [0.24, 0.24]
                },
                visible_items: if wheel { 7.0 } else { 5.0 },
                item_image: image.into(),
                item_fallbacks: fallbacks,
                item_text: text.into(),
                item_text_style: style(Some("bold"), 0.028, pal.text),
                selected_scale: 1.15,
                unselected_opacity: 0.6,
                background: None,
                item_align: 0.5,
                selected_position: 0.5,
                wheel_angle: 12.0,
                selected_border: Some(pal.selection),
                wheel_origin: None,
                wheel_align: None,
                cross_offset: 0.0,
                items_around: None,
                item_tint: None,
                selected_margins: None,
            }
        }
        Kind::Grid => Element::Grid {
            common: Common {
                item_fit: Some(Fit::Cover),
                controller_badge: games,
                favorite_badge: games,
                ..common([0.05, 0.2], [0.9, 0.72])
            },
            columns: 6,
            item_aspect: 1.5,
            spacing: 0.02,
            item_image: if games { "{art.cover}" } else { "{collection.cover}" }.into(),
            item_fallbacks: vec![],
            item_text: title_text.into(),
            item_text_style: style(None, 0.022, pal.text),
            show_labels: false,
            selected_scale: 1.06,
            selected_border: Some(pal.selection),
            item_background: Some(pal.surface_alt),
            corner_radius: 0.01,
            unselected_opacity: 0.85,
            tiles: None,
        },
        Kind::List => Element::TextList {
            common: Common { controller_badge: games, favorite_badge: games, ..common([0.05, 0.2], [0.4, 0.7]) },
            item_text: title_text.into(),
            style: style(None, 0.03, pal.text),
            selected_color: Some(pal.accent_text),
            selector_color: Some(pal.accent),
            selector_image: None,
            row_height: 1.6,
            margin: 0.01,
            favorite_color: None,
            corner_radius: 0.008,
            selector_width: None,
            selector_offset: 0.0,
            selected_background: None,
            selected_background_margins: None,
        },
        Kind::CoverRows => Element::Rows {
            common: Common { item_fit: Some(Fit::Cover), radius: Some(0.01), ..common([0.08, 0.42], [0.9, 0.55]) },
            rows: vec!["recent".into(), "favorites".into(), "platforms".into()],
            max_rows: 6,
            row_items: 30,
            item_size: [0.1, 0.24],
            spacing: 0.012,
            row_spacing: 0.03,
            title: "{collection.name}".into(),
            title_style: style(Some("bold"), 0.026, pal.text),
            item_image: "{art.cover}".into(),
            item_fallbacks: vec![],
            item_text: "{game.name}".into(),
            item_text_style: style(None, 0.022, pal.text),
            selected_scale: 1.06,
            selected_border: Some(pal.selection),
            unselected_opacity: 0.6,
        },
        Kind::Rail => Element::Rail {
            common: Common { z: 6.0, ..common([0.0, 0.0], [0.06, 1.0]) },
            entries: vec![],
            style: style(None, 0.022, pal.text),
            row_height: 2.4,
            current_color: Some(pal.accent),
            selector_color: Some(pal.accent.with_alpha(60)),
            background: Some(pal.surface.with_alpha(235)),
            compact: true,
        },
        Kind::Stack => Element::Stack {
            common: common([0.05, 0.3], [0.4, 0.0]),
            direction: Direction::Vertical,
            gap: 0.012,
            align: StackAlign::Start,
            justify: StackAlign::Start,
            children: vec![
                Element::Text {
                    common: Common { name: format!("{name}_title"), ..Default::default() },
                    text: title_text.into(),
                    style: style(Some("title"), 0.05, pal.text),
                    background: None,
                    wrap: true,
                    max_lines: Some(2),
                    scroll: false,
                },
                Element::Text {
                    common: Common { name: format!("{name}_info"), ..Default::default() },
                    text: if games { "{game.developer}" } else { "{collection.countlabel}" }.into(),
                    style: style(None, 0.024, pal.text_dim),
                    background: None,
                    wrap: false,
                    max_lines: None,
                    scroll: false,
                },
            ],
            background: None,
            corner_radius: 0.0,
            border_color: None,
            border_width: 0.0,
        },
        Kind::Rating => Element::Rating {
            common: common([0.05, 0.4], [0.0, 0.03]),
            value: "{game.rating}".into(),
            filled_path: None,
            unfilled_path: None,
            color: pal.favorite,
        },
        Kind::Clock => Element::DateTime {
            common: Common { origin: [1.0, 0.5], ..common([0.955, 0.065], [0.0, 0.0]) },
            value: String::new(),
            style: TextStyle { align: Align::Right, ..style(Some("bold"), 0.026, pal.text) },
            clock: true,
            format: "%H:%M".into(),
            background: None,
        },
        Kind::Date => Element::DateTime {
            common: common([0.05, 0.45], [0.0, 0.0]),
            value: "{game.lastplayed}".into(),
            style: style(None, 0.022, pal.text_dim),
            clock: false,
            format: "relative".into(),
            background: None,
        },
        Kind::Battery => Element::Text {
            common: Common {
                origin: [1.0, 0.5],
                visible_if: Some("{system.battery}".into()),
                ..common([0.9, 0.065], [0.0, 0.0])
            },
            text: "{system.battery}".into(),
            style: TextStyle { align: Align::Right, ..style(None, 0.021, pal.text_dim) },
            background: None,
            wrap: false,
            max_lines: None,
            scroll: false,
        },
        Kind::Badges => Element::Badges {
            common: common([0.05, 0.5], [0.12, 0.03]),
            slots: vec!["favorite".into(), "completed".into(), "controller".into()],
            column: false,
            lines: 1,
            per_line: 4,
            margin: [0.006, 0.0],
            align: Align::Left,
            icons: Default::default(),
            color: None,
        },
        Kind::HelpBar => Element::HelpBar {
            common: Common { z: 6.0, origin: [1.0, 0.5], ..common([0.955, 0.94], [0.9, 0.05]) },
            style: TextStyle { align: Align::Right, ..style(None, 0.021, pal.text_dim) },
            icon_color: None,
            background: None,
            buttons: vec![],
            entry_spacing: None,
            label_spacing: None,
            button_scale: None,
            glyphs: false,
        },
    }
}

/// Indices of `els` in drawing order (by `z`, ties in file order), back to front.
pub fn draw_order(els: &[Element]) -> Vec<usize> {
    let mut o: Vec<usize> = (0..els.len()).collect();
    o.sort_by(|&a, &b| els[a].common().z.partial_cmp(&els[b].common().z).unwrap_or(std::cmp::Ordering::Equal));
    o
}

/// Move the element at file index `from` so that it is drawn at position `to` of the drawing
/// order (0 = at the back). Only that element changes: it takes the `z` of its new neighbour
/// and moves next to it in the file. Returns its new file index.
pub fn move_in_draw_order(els: &mut Vec<Element>, from: usize, to: usize) -> usize {
    let order = draw_order(els);
    let Some(cur) = order.iter().position(|&i| i == from) else { return from };
    let mut rest: Vec<usize> = order.iter().copied().filter(|&i| i != from).collect();
    let to = to.min(rest.len());
    if to == cur {
        return from;
    }
    // The neighbour below (drawn just before it), else the one above.
    let below = to.checked_sub(1).map(|k| rest[k]);
    let above = rest.get(to).copied();
    let mut el = els.remove(from);
    let fix = |i: usize| if i > from { i - 1 } else { i };
    let at = match (below, above) {
        (Some(b), _) => {
            el.common_mut().z = els[fix(b)].common().z;
            fix(b) + 1
        }
        (None, Some(a)) => {
            el.common_mut().z = els[fix(a)].common().z;
            fix(a)
        }
        (None, None) => 0,
    };
    els.insert(at, el);
    rest.clear();
    at
}

/// A copy of the element with a new name, placed a little lower right.
pub fn duplicate(els: &mut Vec<Element>, p: &ElPath) -> Option<ElPath> {
    let mut copy = get(els, p)?.clone();
    let name = unique_name(els, &format!("{}_copy", copy.common().name.trim().trim_end_matches("_copy")));
    {
        let c = copy.common_mut();
        c.name = name;
        if p.sub.is_empty() {
            c.pos = [super::geom::tidy(c.pos[0] + 0.02), super::geom::tidy(c.pos[1] + 0.02)];
        }
    }
    let (list, i) = siblings_mut(els, p)?;
    list.insert(i + 1, copy);
    let mut np = p.clone();
    match np.sub.last_mut() {
        Some((_, j)) => *j = i + 1,
        None => np.top = i + 1,
    }
    Some(np)
}

pub fn delete(els: &mut Vec<Element>, p: &ElPath) -> Option<Element> {
    let (list, i) = siblings_mut(els, p)?;
    (i < list.len()).then(|| list.remove(i))
}

/// Move a nested element one place earlier (`-1`) or later (`1`) in its parent's flow.
pub fn shift_nested(els: &mut Vec<Element>, p: &ElPath, by: isize) -> Option<ElPath> {
    let (list, i) = siblings_mut(els, p)?;
    let j = i.checked_add_signed(by)?;
    if j >= list.len() {
        return None;
    }
    list.swap(i, j);
    let mut np = p.clone();
    if let Some((_, k)) = np.sub.last_mut() {
        *k = j;
    }
    Some(np)
}

// ---- changes for one variant / screen shape / console only ----

/// Where a change goes.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Layer {
    /// The theme itself (every variant, shape and console).
    #[default]
    Base,
    Variant(String),
    /// A screen shape's layout (`aspects` key, "16:10").
    Aspect(String),
    /// A collection's look (`overrides` key, "platform:sony_playstation2").
    Console(String),
}

impl Layer {
    pub fn set<'a>(&self, f: &'a ThemeFile) -> Option<&'a CollectionOverride> {
        match self {
            Layer::Base => None,
            Layer::Variant(k) => f.variants.get(k),
            Layer::Aspect(k) => f.aspects.get(k),
            Layer::Console(k) => f.overrides.get(k),
        }
    }

    pub fn set_mut<'a>(&self, f: &'a mut ThemeFile) -> Option<&'a mut CollectionOverride> {
        Some(match self {
            Layer::Base => return None,
            Layer::Variant(k) => f.variants.entry(k.clone()).or_default(),
            Layer::Aspect(k) => f.aspects.entry(k.clone()).or_default(),
            Layer::Console(k) => f.overrides.entry(k.clone()).or_default(),
        })
    }
}

pub fn tables_mut(o: &mut CollectionOverride, kind: ViewKind) -> &mut std::collections::BTreeMap<String, toml::Table> {
    match kind {
        ViewKind::Collections => &mut o.collections,
        ViewKind::Games => &mut o.games,
        ViewKind::Details => &mut o.details,
        ViewKind::Home => &mut o.home,
    }
}

pub fn tables(o: &CollectionOverride, kind: ViewKind) -> &std::collections::BTreeMap<String, toml::Table> {
    match kind {
        ViewKind::Collections => &o.collections,
        ViewKind::Games => &o.games,
        ViewKind::Details => &o.details,
        ViewKind::Home => &o.home,
    }
}

/// The keys of `edited` that differ from `base` (tables compared key by key, everything else
/// whole), and the keys `edited` lacks that `base` has (a patch can't take a value away).
pub fn diff(base: &toml::Table, edited: &toml::Table) -> (toml::Table, Vec<String>) {
    let mut out = toml::Table::new();
    let mut lost = vec![];
    for (k, v) in edited {
        match (base.get(k), v) {
            (Some(b), v) if b == v => {}
            (Some(toml::Value::Table(bt)), toml::Value::Table(et)) => {
                let (sub, sub_lost) = diff(bt, et);
                lost.extend(sub_lost.into_iter().map(|s| format!("{k}.{s}")));
                if !sub.is_empty() {
                    out.insert(k.clone(), toml::Value::Table(sub));
                }
            }
            _ => {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    for k in base.keys() {
        if !edited.contains_key(k) {
            lost.push(k.clone());
        }
    }
    (out, lost)
}

fn to_table(e: &Element) -> toml::Table {
    match toml::Value::try_from(e) {
        Ok(toml::Value::Table(t)) => t,
        _ => toml::Table::new(),
    }
}

/// Change the element `p` of view `kind` for `layer` only: `f` edits the element as that
/// layer shows it, and the difference from the theme's own element is stored as the layer's
/// patch for it (by name: an element without one is named first). Returns the settings the
/// layer couldn't take away (plain words for the user), or Err when the element is missing.
pub fn edit_in_layer(
    file: &mut ThemeFile,
    layer: &Layer,
    kind: ViewKind,
    p: &ElPath,
    f: impl FnOnce(&mut Element),
) -> Result<Vec<String>, ()> {
    let view = match kind {
        ViewKind::Collections => &mut file.collections,
        ViewKind::Games => &mut file.games,
        ViewKind::Details => file.details.as_mut().ok_or(())?,
        ViewKind::Home => file.home.as_mut().ok_or(())?,
    };
    if *layer == Layer::Base {
        let e = get_mut(&mut view.elements, p).ok_or(())?;
        f(e);
        return Ok(vec![]);
    }
    // Patches go by the top element's name.
    if view.elements.get(p.top).is_none() {
        return Err(());
    }
    if view.elements[p.top].common().name.trim().is_empty() {
        let n = unique_name(&view.elements, kind_word(&view.elements[p.top]).to_lowercase().as_str());
        view.elements[p.top].common_mut().name = n;
    }
    let base = view.elements[p.top].clone();
    let name = base.common().name.clone();
    let set = layer.set_mut(file).ok_or(())?;
    let tables = tables_mut(set, kind);
    let shown = match tables.get(&name) {
        Some(t) => crate::theme::render::merge_element(&base, t).map_err(|_| ())?,
        None => base.clone(),
    };
    let mut edited = shown;
    let sub = ElPath { top: 0, sub: p.sub.clone() };
    let target = get_mut(std::slice::from_mut(&mut edited), &sub).ok_or(())?;
    f(target);
    let (patch, lost) = diff(&to_table(&base), &to_table(&edited));
    if patch.is_empty() {
        tables.remove(&name);
    } else {
        tables.insert(name, patch);
    }
    Ok(lost)
}

/// Remove the layer's change of one element (back to the theme's own).
pub fn reset_in_layer(file: &mut ThemeFile, layer: &Layer, kind: ViewKind, name: &str) -> bool {
    let Some(set) = layer.set_mut(file) else { return false };
    tables_mut(set, kind).remove(name).is_some()
}

/// The boxes of `els` (top level, drawing order) for snapping and outlines, with `natural`
/// giving the size of elements with an automatic size.
pub fn boxes(els: &[Element], natural: &dyn Fn(usize) -> egui::Vec2) -> Vec<(usize, Rect)> {
    draw_order(els)
        .into_iter()
        .filter(|&i| els[i].common().visible)
        .map(|i| {
            let keep = matches!(els[i], Element::Image { .. } | Element::Rating { .. });
            (i, super::geom::element_box(els[i].common(), natural(i), keep))
        })
        .collect()
}
