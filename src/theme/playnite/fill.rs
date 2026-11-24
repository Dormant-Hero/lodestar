//! A Playnite theme's look (`PlayniteLook`) laid onto one of Lodestar's layouts: the theme's
//! bars, gradients, background picture, cover focus look, fonts and font sizes, logos — instead
//! of only swapping the layout's colours.
//!
//! Everything is written as TOML tables, by element name: the layouts mark the parts that
//! follow a Playnite theme with names starting with `pn_` (top-bar.toml,
//! single-row.toml). Fields of the newer theme model (N-stop / angled / radial gradients,
//! focus border width / radius / glow, uninstalled scrim, caption background, the desktop
//! style) are written next to a two-colour `gradient` that every renderer draws, so a theme
//! reads the same with or without them.

use super::look::*;
use super::map;
use crate::theme::model::*;
use toml::{Table, Value};

/// Playnite Fullscreen's design height: every fullscreen length in a `PlayniteLook` is in
/// pixels of a 1920 × 1080 screen.
const H: f32 = 1080.0;

/// Which Lodestar layout a fullscreen theme is laid onto.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    /// The theme's own main screen and details page, translated element by element
    /// (compile.rs); its other screens are Top Bar's in the theme's style.
    Translated,
    /// Playnite's own arrangement: top bar, a grid of covers, bottom bar ("Top Bar").
    #[default]
    Classic,
    /// Big artwork above one row of covers, console style ("Single Row").
    Row,
    /// Lodestar's Fullscreen layout: navigation rail, artwork and rows of covers.
    Rail,
}

impl Family {
    pub const ALL: [Family; 4] = [Family::Translated, Family::Classic, Family::Row, Family::Rail];

    /// The button / report wording.
    pub fn label(self) -> &'static str {
        match self {
            Family::Translated => "Translated",
            Family::Classic => "Classic grid",
            Family::Row => "Single row",
            Family::Rail => "Rail and rows",
        }
    }

    /// The built-in layout it uses.
    pub fn builtin(self) -> &'static str {
        match self {
            Family::Translated | Family::Classic => "Top Bar",
            Family::Row => "Single Row",
            Family::Rail => "Fullscreen",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Family::Translated => "translated",
            Family::Classic => "classic",
            Family::Row => "row",
            Family::Rail => "rail",
        }
    }

    pub fn parse(s: &str) -> Option<Family> {
        let s = s.trim().to_ascii_lowercase();
        Family::ALL
            .into_iter()
            .find(|f| f.key() == s || f.label().eq_ignore_ascii_case(&s) || f.builtin().eq_ignore_ascii_case(&s))
            .or(match s.as_str() {
                "grid" => Some(Family::Classic),
                "translate" | "compiled" | "theme" => Some(Family::Translated),
                "single" | "single-row" | "carousel" => Some(Family::Row),
                "rows" | "fullscreen" => Some(Family::Rail),
                _ => None,
            })
    }

    /// The family the theme's game list shape points to.
    pub fn for_shape(shape: ListShape) -> Family {
        match shape {
            ListShape::Grid => Family::Classic,
            ListShape::Row | ListShape::Carousel => Family::Row,
        }
    }
}

/// The fonts the theme gets (values for `[fonts]`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FontSet {
    pub regular: Option<String>,
    pub semibold: Option<String>,
    pub bold: Option<String>,
    pub title: Option<String>,
}

impl FontSet {
    /// Playnite's Titillium Web (compiled in; the system font stands in without it).
    pub fn titillium(body_weight: u16) -> FontSet {
        use crate::theme::fonts::bundled::titillium_for_weight as tw;
        FontSet {
            regular: Some(tw(body_weight).into()),
            semibold: Some(tw(600).into()),
            bold: Some(tw(700).into()),
            title: Some(tw(700).into()),
        }
    }

    /// Lodestar's own (system) fonts.
    pub fn system() -> FontSet {
        FontSet {
            regular: Some("system:sans".into()),
            semibold: Some("system:sans-semibold".into()),
            bold: Some("system:sans-bold".into()),
            title: Some("system:sans-bold".into()),
        }
    }

    fn apply(&self, fonts: &mut Fonts) {
        let Some(regular) = self.regular.clone() else { return };
        let strong = self.bold.clone().or(self.semibold.clone()).unwrap_or_else(|| regular.clone());
        let semi = self.semibold.clone().unwrap_or_else(|| strong.clone());
        fonts.regular = Some(regular.clone());
        fonts.bold = Some(strong.clone());
        fonts.title = Some(self.title.clone().unwrap_or_else(|| strong.clone()));
        for (k, val) in fonts.extra.iter_mut() {
            if k.contains("mono") {
                continue;
            }
            *val = if k.contains("semi") {
                semi.clone()
            } else if k.contains("medium") {
                // Playnite writes buttons and hints in the regular weight.
                regular.clone()
            } else if k.contains("bold") || k.contains("title") || k.contains("display") {
                strong.clone()
            } else {
                regular.clone()
            };
        }
    }
}

/// What the importer hands over besides the look.
#[derive(Clone, Debug, Default)]
pub(crate) struct Inputs {
    pub name: String,
    pub author: String,
    pub version: String,
    pub description: String,
    pub palette: Palette,
    pub fonts: Option<FontSet>,
    /// The theme's own background picture, copied ("images/x.png").
    pub picture: Option<String>,
    /// Theme videos copied (main view, details view) and their fit.
    pub main_video: Option<(String, Fit)>,
    pub details_video: Option<(String, Fit)>,
    pub sounds: std::collections::BTreeMap<String, String>,
    pub music: Vec<String>,
    /// The theme's own words for Playnite's standard labels (loc.rs; LOC key → text).
    pub strings: super::loc::Strings,
}

/// Labels of the details page → the Playnite LOC key a theme renames them with.
const LABELS: [(&str, &str); 4] = [
    ("pn_label_playtime", "LOCTimePlayed"),
    ("pn_label_lastplayed", "LOCGameLastActivityTitle"),
    ("pn_label_release", "LOCGameReleaseDateTitle"),
    ("pn_label_platform", "LOCGamePlatformTitle"),
];

/// The theme's own label texts (only where it renames one: Lodestar's words stay otherwise).
fn relabel(root: &mut Table, strings: &super::loc::Strings) -> usize {
    let mut n = 0;
    let Some(view) = root.get_mut("details") else { return 0 };
    let Some(els) = elements_mut(view) else { return 0 };
    for e in els.iter_mut() {
        let Value::Table(t) = e else { continue };
        let Some((_, key)) = LABELS.iter().find(|(el, _)| *el == name_of(t)) else { continue };
        if let Some(text) = strings.own(key).map(str::trim).filter(|s| !s.is_empty() && s.len() <= 60) {
            t.insert("text".into(), Value::String(text.to_string()));
            n += 1;
        }
    }
    n
}

// ---------------------------------------------------------------------------- paints

/// A colour with straight alpha as the theme writes it.
fn hex(c: Color) -> Value {
    Value::String(c.to_hex())
}

/// Playnite brushes are drawn over the view background; a fully transparent colour stays so.
fn stops_value(stops: &[(Color, f32)]) -> Value {
    Value::Array(
        stops
            .iter()
            .map(|(c, o)| Value::Array(vec![hex(*c), Value::Float(((*o * 1000.0).round() / 1000.0) as f64)]))
            .collect(),
    )
}

/// A brush as the theme model's `Brush` (desktop style fills, caption background): a colour,
/// or `{ stops, angle }` / `{ stops, radial }`.
pub(crate) fn brush_value(b: &LookBrush) -> Option<Value> {
    match b {
        LookBrush::Solid { color } => Some(hex(*color)),
        LookBrush::Linear { .. } => {
            let mut t = Table::new();
            t.insert("stops".into(), stops_value(&b.box_stops()));
            t.insert("angle".into(), Value::Float(round(b.angle().unwrap_or(90.0)) as f64));
            Some(Value::Table(t))
        }
        LookBrush::Radial { stops, .. } => {
            let mut t = Table::new();
            t.insert("stops".into(), stops_value(stops));
            t.insert("radial".into(), floats(&b.radial().unwrap_or([0.5, 0.5, 0.5, 0.5])));
            Some(Value::Table(t))
        }
        LookBrush::Image { .. } => None,
    }
}

fn round(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

fn floats(v: &[f32]) -> Value {
    Value::Array(v.iter().map(|x| Value::Float(round(*x) as f64)).collect())
}

/// Paint a rect / image table with a brush: `color` (solid or average), a two-colour
/// `gradient` along the nearest axis, and the full gradient (`gradient_stops`,
/// `gradient_angle` / `gradient_radial`). Returns false for brushes a rect can't draw
/// (pictures).
pub(crate) fn paint(t: &mut Table, b: &LookBrush) -> bool {
    for k in ["gradient", "gradient_horizontal", "gradient_stops", "gradient_angle", "gradient_radial"] {
        t.remove(k);
    }
    match b {
        LookBrush::Solid { color } => {
            t.insert("color".into(), hex(*color));
            true
        }
        LookBrush::Linear { .. } => {
            let stops = b.box_stops();
            let angle = b.angle().unwrap_or(90.0).rem_euclid(360.0);
            let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
                return false;
            };
            // Two colours along the nearest axis (what an older renderer draws).
            let horizontal = !(45.0..135.0).contains(&angle) && !(225.0..315.0).contains(&angle);
            let reversed = (135.0..315.0).contains(&angle);
            let (a, z) = if reversed { (last.0, first.0) } else { (first.0, last.0) };
            t.insert("color".into(), hex(b.average().unwrap_or(a)));
            t.insert("gradient".into(), Value::Array(vec![hex(a), hex(z)]));
            if horizontal {
                t.insert("gradient_horizontal".into(), Value::Boolean(true));
            }
            let off_axis = (angle % 90.0).min(90.0 - angle % 90.0) > 0.5;
            if stops.len() > 2 || off_axis {
                t.insert("gradient_stops".into(), stops_value(&stops));
                t.insert("gradient_angle".into(), Value::Float(round(angle) as f64));
            }
            true
        }
        LookBrush::Radial { stops, .. } => {
            let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
                return false;
            };
            t.insert("color".into(), hex(b.average().unwrap_or(first.0)));
            t.insert("gradient".into(), Value::Array(vec![hex(first.0), hex(last.0)]));
            t.insert("gradient_stops".into(), stops_value(stops));
            t.insert("gradient_radial".into(), floats(&b.radial().unwrap_or([0.5, 0.5, 0.5, 0.5])));
            true
        }
        LookBrush::Image { .. } => false,
    }
}

/// A rect painted with an axis-aligned linear gradient of more than two colours, as one rect
/// per pair of stops (each a plain two-colour gradient, which every renderer draws exactly).
/// `t` becomes the first part; the others are returned (same z, drawn after it).
pub(crate) fn split_axis(t: &mut Table, b: &LookBrush) -> Vec<Table> {
    let LookBrush::Linear { .. } = b else { return vec![] };
    let angle = b.angle().unwrap_or(90.0).rem_euclid(360.0);
    let near = |a: f32| (angle - a).abs() < 0.5 || (angle - a - 360.0).abs() < 0.5;
    let (axis, reversed) = if near(90.0) {
        (1, false)
    } else if near(270.0) {
        (1, true)
    } else if near(0.0) {
        (0, false)
    } else if near(180.0) {
        (0, true)
    } else {
        return vec![];
    };
    let mut stops = b.box_stops();
    if reversed {
        stops = stops.into_iter().rev().map(|(c, o)| (c, 1.0 - o)).collect();
    }
    stops.dedup_by(|x, y| (x.1 - y.1).abs() < 1e-3 && x.0 == y.0);
    if stops.len() <= 2 {
        return vec![];
    }
    let (pos, size, origin) = (pair(t, "pos"), pair(t, "size"), pair(t, "origin"));
    let top_left = [pos[0] - origin[0] * size[0], pos[1] - origin[1] * size[1]];
    let mut parts: Vec<Table> = vec![];
    let name = name_of(t).to_string();
    for (i, w) in stops.windows(2).enumerate() {
        let ((a, o0), (z, o1)) = (w[0], w[1]);
        if o1 - o0 < 1e-3 {
            continue;
        }
        let mut part = t.clone();
        for k in ["gradient_stops", "gradient_angle", "gradient_radial", "gradient_horizontal", "origin"] {
            part.remove(k);
        }
        let mut p = top_left;
        let mut sz = size;
        p[axis] += o0 * size[axis];
        sz[axis] = (o1 - o0) * size[axis];
        set_pair(&mut part, "pos", p);
        set_pair(&mut part, "size", sz);
        part.insert("color".into(), hex(a));
        if a != z {
            part.insert("gradient".into(), Value::Array(vec![hex(a), hex(z)]));
            if axis == 0 {
                part.insert("gradient_horizontal".into(), Value::Boolean(true));
            }
        } else {
            part.remove("gradient");
        }
        if i > 0 {
            part.insert("name".into(), Value::String(format!("{name}_{i}")));
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return vec![];
    }
    *t = parts.remove(0);
    parts
}

/// An opacity mask as the image `mask` (how far each edge fades: [left, top, right, bottom]).
fn mask_edges(m: &Mask) -> [f32; 4] {
    // edge_alpha is [top, right, bottom, left]; a faded edge fades over about half the box.
    let fade = |a: f32| if a < 0.5 { round((1.0 - a) * 0.5) } else { 0.0 };
    [fade(m.edge_alpha[3]), fade(m.edge_alpha[0]), fade(m.edge_alpha[1]), fade(m.edge_alpha[2])]
}

fn fit_of(s: Option<Stretch>) -> Fit {
    match s {
        Some(Stretch::Uniform) => Fit::Contain,
        Some(Stretch::Fill) => Fit::Stretch,
        _ => Fit::Cover,
    }
}

fn fit_str(f: Fit) -> &'static str {
    match f {
        Fit::Contain => "contain",
        Fit::Cover => "cover",
        Fit::Stretch => "stretch",
    }
}

// ---------------------------------------------------------------------------- tables

fn elements_mut(view: &mut Value) -> Option<&mut Vec<Value>> {
    match view {
        Value::Table(vt) => match vt.get_mut("elements") {
            Some(Value::Array(a)) => Some(a),
            _ => None,
        },
        _ => None,
    }
}

fn name_of(t: &Table) -> &str {
    t.get("name").and_then(Value::as_str).unwrap_or("")
}

fn kind_of(t: &Table) -> &str {
    t.get("type").and_then(Value::as_str).unwrap_or("")
}

fn set_f(t: &mut Table, k: &str, v: f32) {
    t.insert(k.into(), Value::Float(round(v) as f64));
}

fn pair(t: &Table, k: &str) -> [f32; 2] {
    match t.get(k) {
        Some(Value::Array(a)) if a.len() == 2 => {
            let f = |v: &Value| v.as_float().or(v.as_integer().map(|i| i as f64)).unwrap_or(0.0) as f32;
            [f(&a[0]), f(&a[1])]
        }
        _ => [0.0, 0.0],
    }
}

fn set_pair(t: &mut Table, k: &str, v: [f32; 2]) {
    t.insert(k.into(), floats(&v));
}

/// The view tables of a theme table.
const VIEWS: [&str; 4] = ["collections", "games", "details", "home"];

fn rect(name: &str, z: f32, pos: [f32; 2], size: [f32; 2]) -> Table {
    let mut t = Table::new();
    t.insert("type".into(), Value::String("rect".into()));
    t.insert("name".into(), Value::String(name.into()));
    set_f(&mut t, "z", z);
    set_pair(&mut t, "pos", pos);
    set_pair(&mut t, "size", size);
    t
}

fn image(name: &str, z: f32, path: &str) -> Table {
    let mut t = Table::new();
    t.insert("type".into(), Value::String("image".into()));
    t.insert("name".into(), Value::String(name.into()));
    set_f(&mut t, "z", z);
    t.insert("path".into(), Value::String(path.into()));
    set_pair(&mut t, "size", [1.0, 1.0]);
    t.insert("fit".into(), Value::String("cover".into()));
    t
}

// ---------------------------------------------------------------------------- report

/// Lines for the report while filling (area, carried?, text).
#[derive(Default)]
pub(crate) struct Notes {
    pub lines: Vec<(super::report::Area, bool, String)>,
}

impl Notes {
    fn ok(&mut self, a: super::report::Area, s: impl Into<String>) {
        self.lines.push((a, true, s.into()));
    }
}

// ---------------------------------------------------------------------------- controller

/// The controller theme for `family`, filled from `look`.
pub(crate) fn controller(
    family: Family,
    look: &PlayniteLook,
    inp: &Inputs,
    notes: &mut Notes,
) -> anyhow::Result<Value> {
    use super::report::Area;
    let all = crate::theme::builtin::all();
    let find = |k: &str| all.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, f)| f.clone());
    let base = find(family.builtin()).or_else(|| find("Top Bar")).unwrap_or_else(|| all[0].1.clone());
    let classic = find("Top Bar").unwrap_or_else(|| base.clone());
    let file = controller_file(family, &base, &classic, inp);

    let mut v: Value = toml::from_str(&file.to_toml()?)?;
    let Value::Table(root) = &mut v else {
        anyhow::bail!("theme isn't a table");
    };
    // The layout's own colours → the theme's (details: from Top Bar's palette).
    let base_pairs = map::pairs(&base.palette, &inp.palette);
    let classic_pairs = map::pairs(&classic.palette, &inp.palette);
    for k in VIEWS {
        if let Some(view) = root.get_mut(k) {
            let pairs = if k == "details" && family != Family::Classic { &classic_pairs } else { &base_pairs };
            map::recolor(view, pairs);
        }
    }

    let restyle = Restyle::new(family, look, inp);
    for k in VIEWS {
        if let Some(view) = root.get_mut(k) {
            restyle.view(k, view);
        }
    }

    // Report lines.
    if look.main.background.as_ref().is_some_and(LookBrush::is_gradient) {
        notes.ok(Area::Background, "The background gradient, with all its colours");
    }
    if family == Family::Classic {
        let (top, bottom) = (restyle.top, restyle.bottom);
        let bar = |b: Option<&Bar>| b.and_then(|b| b.background.as_ref()).is_some_and(|x| !x.is_clear());
        if bar(top) || bar(bottom) {
            notes.ok(
                Area::Layout,
                format!(
                    "The top and bottom bars ({} px and {} px high, in the theme's colours)",
                    top.map_or(0.0, |b| b.height).round(),
                    bottom.map_or(0.0, |b| b.height).round()
                ),
            );
        }
    }
    let cover = cover_look_words(&look.item);
    if !cover.is_empty() {
        notes.ok(Area::Covers, format!("The cover look: {}", cover.join(", ")));
    }
    if restyle.logos {
        notes.ok(Area::Covers, "Game logos instead of names where a game has one (Lodestar finds them itself)");
    }
    if look.details.overlay.is_some() {
        notes.ok(Area::Layout, "The game details page: the theme's panel over the game's artwork");
    }
    if restyle.scales_text() {
        notes.ok(Area::Fonts, format!("Text sizes ({} px text)", look.font_sizes.normal.unwrap_or(22.0).round()));
    }
    if let Value::Table(root) = &mut v
        && relabel(root, &inp.strings) > 0
    {
        notes.ok(Area::Layout, "The theme's own words for the game facts (Time played, Last played …)");
    }
    Ok(v)
}

/// The layout family's built-in theme with the import's name, fonts, palette and sounds.
fn controller_file(family: Family, base: &ThemeFile, classic: &ThemeFile, inp: &Inputs) -> ThemeFile {
    let mut file = base.clone();
    file.theme.name = inp.name.clone();
    file.theme.author = inp.author.clone();
    file.theme.version = inp.version.clone();
    file.theme.description = inp.description.clone();
    file.theme.origin = "playnite".into();
    file.theme.controller = true;
    // Playnite Fullscreen opens on its game list (filter presets are a menu away): so does the
    // Classic grid of an import. (The built-in Top Bar keeps Lodestar's collections.)
    if family == Family::Classic {
        file.theme.start = "games".into();
    }
    if let Some(f) = &inp.fonts {
        f.apply(&mut file.fonts);
    }
    // Every family gets Playnite's details page (the theme's GameDetails look).
    if family != Family::Classic {
        file.details = classic.details.clone();
        for (k, v) in &classic.fonts.extra {
            file.fonts.extra.entry(k.clone()).or_insert_with(|| v.clone());
        }
        if let Some(f) = &inp.fonts {
            f.apply(&mut file.fonts);
        }
    }
    file.palette = inp.palette.clone();
    file.sounds = inp.sounds.clone();
    file.music = inp.music.clone();
    file
}

/// The cover look's parts, in words for the report.
fn cover_look_words(item: &ItemLook) -> Vec<&'static str> {
    let mut cover = vec![];
    if item.focus_frame.as_ref().is_some_and(|f| f.width > 0.0) {
        cover.push("focus border");
    }
    if item.focus_glow.is_some() {
        cover.push("glow");
    }
    if item.focus_scale.is_some_and(|s| (s - 1.0).abs() > 0.005) {
        cover.push("growing focused cover");
    }
    if item.cover_radius.is_some_and(|r| r > 0.0) {
        cover.push("rounded corners");
    }
    if item.caption.is_some() {
        cover.push("title strip");
    }
    if item.uninstalled_scrim.is_some() {
        cover.push("darkened games that aren't installed");
    }
    cover
}

/// Lays the theme's look onto a layout family's views.
struct Restyle<'a> {
    family: Family,
    look: &'a PlayniteLook,
    inp: &'a Inputs,
    /// Playnite's bars belong to its own arrangement; the console-style layout keeps its soft
    /// scrims (in the theme's colours).
    top: Option<&'a Bar>,
    bottom: Option<&'a Bar>,
    /// Text sizes follow the theme's FontSize.
    text_scale: f32,
    /// Game logos instead of names.
    logos: bool,
    /// Where the theme's one row of covers sits (Single Row home page).
    row_at: Option<RowPlace>,
}

impl<'a> Restyle<'a> {
    fn new(family: Family, look: &'a PlayniteLook, inp: &'a Inputs) -> Restyle<'a> {
        Restyle {
            family,
            look,
            inp,
            top: look.main.top_bar.as_ref().filter(|_| family == Family::Classic),
            bottom: look.main.bottom_bar.as_ref().filter(|_| family == Family::Classic),
            text_scale: look.font_sizes.normal.map(|n| (n / 22.0).clamp(0.75, 1.4)).unwrap_or(1.0),
            logos: look.plugins.logos || look.details.logo,
            row_at: (family == Family::Row).then(|| row_place(&look.main.list)).flatten(),
        }
    }

    fn scales_text(&self) -> bool {
        (self.text_scale - 1.0).abs() > 0.02 && self.family == Family::Classic
    }

    /// One view (`k`): its background, every element, and the theme's pictures under them.
    fn view(&self, k: &str, view: &mut Value) {
        if let Value::Table(vt) = view {
            vt.insert("background".into(), hex(self.inp.palette.background));
        }
        let details = k == "details";
        let Some(els) = elements_mut(view) else { return };
        let mut after: Vec<Table> = vec![];
        for e in els.iter_mut() {
            let Value::Table(t) = e else { continue };
            let name = name_of(t).to_string();
            self.element(t, &name, details, &mut after);
            if !details && self.family == Family::Classic {
                self.bar_item(t, &name, k);
            }
            // The row of covers (and the game's name / facts around it) where the theme has it.
            if k == "home"
                && let Some(r) = &self.row_at
            {
                r.apply(t, &name);
            }
            // The cover look on every list of games (and the focus look on collection lists).
            if matches!(kind_of(t), "grid" | "rows" | "carousel") {
                item_look(t, &self.look.item, k == "collections");
            }
            if self.scales_text() {
                scale_text(t, self.text_scale);
            }
        }
        let extra = self.underlays(k, details);
        els.extend(after.into_iter().map(Value::Table));
        for (i, t) in extra.into_iter().enumerate() {
            els.insert(i, Value::Table(t));
        }
    }

    /// The layout's own `pn_` elements in the theme's look; parts of split gradients go to
    /// `after`.
    fn element(&self, t: &mut Table, name: &str, details: bool, after: &mut Vec<Table>) {
        let look = self.look;
        let classic = self.family == Family::Classic;
        match name {
            "pn_background" => {
                let b = if details { look.details.background.as_ref() } else { None }.or(look.main.background.as_ref());
                if let Some(b) = b
                    && paint(t, b)
                {
                    after.extend(split_axis(t, b));
                }
            }
            "pn_topbar" if !details => match self.top {
                Some(bar) => {
                    let h = (bar.height / H).clamp(0.0, 0.3);
                    set_pair(t, "size", [1.0, h]);
                    match &bar.background {
                        Some(b) if !b.is_clear() && paint(t, b) => after.extend(split_axis(t, b)),
                        _ => {
                            t.insert("visible".into(), Value::Boolean(false));
                        }
                    }
                }
                None if classic => {
                    t.insert("visible".into(), Value::Boolean(false));
                }
                None => {}
            },
            "pn_bottombar" if !details => match self.bottom {
                Some(bar) => {
                    let h = (bar.height / H).clamp(0.0, 0.3);
                    set_pair(t, "pos", [0.0, 1.0 - h]);
                    set_pair(t, "size", [1.0, h]);
                    match &bar.background {
                        Some(b) if !b.is_clear() && paint(t, b) => after.extend(split_axis(t, b)),
                        _ => {
                            t.insert("visible".into(), Value::Boolean(false));
                        }
                    }
                }
                None if classic => {
                    t.insert("visible".into(), Value::Boolean(false));
                }
                None => {}
            },
            "pn_band_fade" if details => {
                // GameDetailsBackgroundBrush over the band; the fade rect goes, the band
                // takes the whole brush.
                if look.details.overlay.as_ref().is_some_and(|b| !b.is_clear() && b.average().is_some()) {
                    t.insert("visible".into(), Value::Boolean(false));
                }
            }
            "pn_band" if details => {
                // A clear or picture overlay: Top Bar's own band (in the theme's
                // colours) keeps the text readable.
                if let Some(b) = look.details.overlay.as_ref().filter(|b| !b.is_clear() && b.average().is_some()) {
                    set_pair(t, "pos", [0.0, 3.0 / 7.0]);
                    set_pair(t, "size", [1.0, 4.0 / 7.0]);
                    if paint(t, b) {
                        after.extend(split_axis(t, b));
                    }
                }
            }
            "pn_clock" if !look.main.clock && !details => {
                t.insert("visible".into(), Value::Boolean(false));
            }
            "pn_battery" if !look.main.battery && !details => {
                t.insert("visible".into(), Value::Boolean(false));
            }
            "pn_logo" => {
                t.insert("visible".into(), Value::Boolean(self.logos));
            }
            "pn_title" => {
                if self.logos {
                    t.insert("hidden_if".into(), Value::String("{art.logo}".into()));
                } else {
                    t.remove("hidden_if");
                }
                if details && let Some(sz) = look.details.title_size {
                    style_size(t, (sz / H).clamp(0.02, 0.08));
                }
            }
            _ => {}
        }
    }

    /// Top Bar: items of the top / bottom bar stay centred in it, and the game list
    /// fills what the bars leave.
    fn bar_item(&self, t: &mut Table, name: &str, k: &str) {
        let (top, bottom) = (self.top, self.bottom);
        let top_items = ["pn_filter", "pn_battery", "pn_clock", "pn_menu"];
        if let Some(bar) = top
            && top_items.contains(&name)
        {
            let h = (bar.height / H).clamp(0.04, 0.3);
            let p = pair(t, "pos");
            set_pair(t, "pos", [p[0], h * 0.5]);
        }
        if name == "pn_count"
            && let Some(bar) = top
        {
            let h = (bar.height / H).clamp(0.04, 0.3);
            let p = pair(t, "pos");
            set_pair(t, "pos", [p[0], h * 0.82]);
        }
        if name == "pn_help"
            && let Some(bar) = bottom
        {
            let h = (bar.height / H).clamp(0.03, 0.3);
            let p = pair(t, "pos");
            set_pair(t, "pos", [p[0], 1.0 - h * 0.5]);
        }
        if name == "pn_list" && k == "games" {
            let th = top.map_or(0.1019, |b| b.height / H).clamp(0.0, 0.3);
            let bh = bottom.map_or(0.0926, |b| b.height / H).clamp(0.0, 0.3);
            let p = pair(t, "pos");
            set_pair(t, "pos", [p[0], th + 0.016]);
            let s = pair(t, "size");
            set_pair(t, "size", [s[0], (1.0 - th - bh - 0.032).max(0.3)]);
        }
    }

    /// What goes under the view's elements: the theme's own background picture / video, the
    /// selected game's picture (Classic grid) and the background gradient (Rail).
    fn underlays(&self, k: &str, details: bool) -> Vec<Table> {
        let (look, inp) = (self.look, self.inp);
        let mut extra: Vec<Table> = vec![];
        let picture = if details { None } else { inp.picture.as_deref() };
        let video = if details { inp.details_video.as_ref() } else { inp.main_video.as_ref() };
        if picture.is_some() || video.is_some() {
            let mut t = image("pn_theme_picture", -1.5, picture.unwrap_or(""));
            if let Some(p) = &look.main.picture.as_ref().filter(|_| !details) {
                if let Some(m) = &p.mask {
                    t.insert("mask".into(), floats(&mask_edges(m)));
                }
                if let Some(o) = p.opacity.filter(|o| *o < 0.99) {
                    set_f(&mut t, "opacity", o);
                }
                t.insert("fit".into(), Value::String(fit_str(fit_of(p.stretch)).into()));
            }
            if let Some((vid, fit)) = video {
                t.insert("video".into(), Value::String(vid.clone()));
                if picture.is_none() {
                    t.insert("fit".into(), Value::String(fit_str(*fit).into()));
                }
            }
            extra.push(t);
        }
        // Playnite's main screen shows the selected game's background picture
        // (PART_ImageBackground, darkened by Playnite's "dark amount"): behind the Classic grid
        // too, when the theme's main view has one.
        if self.family == Family::Classic
            && k == "games"
            && let Some(p) = &look.main.picture
        {
            extra.push(game_picture(p));
            let mut scrim = rect("pn_game_picture_dim", -1.1, [0.0, 0.0], [1.0, 1.0]);
            scrim.insert("color".into(), hex(p.darken.unwrap_or(Color([0, 0, 0, MAIN_DARKEN]))));
            extra.push(scrim);
        }
        // The Rail layout has no pn_ background: the theme's background gradient under it.
        if self.family == Family::Rail
            && !details
            && let Some(b) = &look.main.background
            && b.is_gradient()
        {
            let mut t = rect("pn_background", -2.0, [0.0, 0.0], [1.0, 1.0]);
            if paint(&mut t, b) {
                let parts = split_axis(&mut t, b);
                extra.push(t);
                extra.extend(parts);
            }
        }
        extra
    }
}

/// Single Row's home page laid out after the theme's list: the row of covers at the theme's
/// height, the game's logo / name / facts moved out of its way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RowPlace {
    /// Top of the rows element (row title included) and the covers' height, screen fractions.
    pub top: f32,
    pub item_h: f32,
    /// How far the logo / name / facts / description move (screen fractions, + = down).
    pub text_shift: f32,
    /// The description has no room (the row is in the middle of the screen).
    pub hide_description: bool,
}

/// Single Row's own geometry (single-row.toml): 0.235 high covers under a row title; the text
/// block runs from the logo's bottom at 0.4 to the description's end at 0.565.
const ROW_ITEM_H: f32 = 0.235;
const ROW_TITLE: f32 = 0.05;
const TOP_BAR: f32 = 0.105;

/// Where the theme's list sits, from PART_ListGameItems' area, margin, height and vertical
/// alignment. None: no fixed height (the layout's own place stays).
pub(crate) fn row_place(l: &ListLook) -> Option<RowPlace> {
    let area = l.area?;
    let h = (l.height? / H).clamp(0.0, 1.0);
    let m = l.margin.unwrap_or([0.0; 4]).map(|v| v / H);
    let top = match l.valign {
        Some(super::look::Align::Start) => area[1] + m[1],
        Some(super::look::Align::End) => area[1] + area[3] - m[3] - h,
        _ => area[1] + (area[3] - h) * 0.5 + (m[1] - m[3]) * 0.5,
    }
    .clamp(0.0, (1.0 - h).max(0.0));
    if top + h * 0.5 > 0.62 {
        return None; // Low on the screen already: Single Row's own place.
    }
    // Small covers keep a readable size; big lists don't outgrow the layout's covers.
    let top_row = top.max(TOP_BAR) < 0.3;
    // A row along the top is a strip of small covers (its box holds the enlarged focused one
    // and the game's name under it); a row across the middle fills most of its box.
    let item_h = if top_row { (h * 0.45).clamp(0.13, 0.2) } else { (h * 0.75).clamp(0.13, ROW_ITEM_H) };
    let top = top.max(TOP_BAR);
    let bottom = top + ROW_TITLE + item_h;
    if top_row {
        // A row along the top (PlayStation 5 style): the game's logo and facts below it, low
        // on the left.
        let logo_bottom = (bottom + 0.3).clamp(0.62, 0.72);
        Some(RowPlace { top, item_h, text_shift: logo_bottom - 0.4, hide_description: false })
    } else {
        // A row across the middle (Xbox style): the logo and facts above it.
        Some(RowPlace { top, item_h, text_shift: (top - 0.03) - 0.425 - 0.04, hide_description: true })
    }
}

impl RowPlace {
    fn apply(&self, t: &mut Table, name: &str) {
        match name {
            "pn_list" if kind_of(t) == "rows" => {
                let p = pair(t, "pos");
                set_pair(t, "pos", [p[0], self.top]);
                let s = pair(t, "size");
                set_pair(t, "size", [s[0], self.item_h + 0.075]);
                let w = self.item_h * (0.088 / ROW_ITEM_H);
                set_pair(t, "item_size", [w, self.item_h]);
            }
            // The rail's icons go under a row along the top (they would sit on its covers).
            "rail" if self.text_shift > 0.0 => {
                let p = pair(t, "pos");
                set_pair(t, "pos", [p[0], self.top + ROW_TITLE + self.item_h + 0.04]);
            }
            "pn_logo" | "pn_title" | "pn_facts" | "pn_description" => {
                let p = pair(t, "pos");
                set_pair(t, "pos", [p[0], (p[1] + self.text_shift).clamp(0.12, 0.9)]);
                if name == "pn_description" && self.hide_description {
                    t.insert("visible".into(), Value::Boolean(false));
                }
            }
            _ => {}
        }
    }
}

/// Playnite's default darkening of the main screen's game picture (its MainBackgroundImage
/// DarkAmount setting), as an alpha.
const MAIN_DARKEN: u8 = 102;

/// The selected game's background picture where the theme's PART_ImageBackground sits.
fn game_picture(p: &super::look::BackgroundPicture) -> Table {
    let mut t = image("pn_game_picture", -1.2, "{art.hero}");
    t.insert("fallbacks".into(), Value::Array(vec![Value::String("{art.banner}".into())]));
    if let Some(a) = p.area.filter(|a| !p.full && a[2] > 0.05 && a[3] > 0.05) {
        set_pair(&mut t, "pos", [a[0], a[1]]);
        set_pair(&mut t, "size", [a[2], a[3]]);
    }
    if let Some(m) = &p.mask {
        t.insert("mask".into(), floats(&mask_edges(m)));
    }
    if let Some(o) = p.opacity.filter(|o| *o < 0.99) {
        set_f(&mut t, "opacity", o);
    }
    if let Some(b) = p.blur.filter(|b| *b > 0.5) {
        set_f(&mut t, "blur", (b / H).clamp(0.0, 0.05));
    }
    t.insert("fit".into(), Value::String(fit_str(fit_of(p.stretch.or(Some(Stretch::UniformToFill)))).into()));
    t
}

fn style_size(t: &mut Table, size: f32) {
    if let Some(Value::Table(s)) = t.get_mut("style") {
        set_f(s, "font_size", size);
    }
}

fn scale_text(t: &mut Table, k: f32) {
    for key in ["style", "item_text_style", "title_style"] {
        if let Some(Value::Table(s)) = t.get_mut(key)
            && let Some(v) = s.get("font_size").and_then(Value::as_float)
        {
            set_f(s, "font_size", v as f32 * k);
        }
    }
}

/// The focused cover's look on a grid / rows / carousel table.
pub(crate) fn item_look(t: &mut Table, item: &ItemLook, collections: bool) {
    let kind = kind_of(t).to_string();
    if let Some(f) = &item.focus_frame {
        if let Some(c) = f.brush.as_ref().and_then(LookBrush::average) {
            t.insert("selected_border".into(), hex(c.with_alpha(255)));
        }
        if f.width > 0.0 {
            set_f(t, "selected_border_width", (f.width / H).clamp(0.001, 0.02));
        }
        if f.radius > 0.0 {
            set_f(t, "selected_radius", (f.radius / H).clamp(0.0, 0.05));
        }
    }
    match &item.focus_glow {
        Some(g) => {
            let mut gt = Table::new();
            let a = (g.color.0[3] as f32 * g.opacity.clamp(0.0, 1.0)).round() as u8;
            gt.insert("color".into(), hex(g.color.with_alpha(a)));
            set_f(&mut gt, "blur", (g.blur / H).clamp(0.002, 0.06));
            t.insert("selected_glow".into(), Value::Table(gt));
        }
        None => {
            // No glow in the theme: none in Lodestar either.
            let mut gt = Table::new();
            gt.insert("color".into(), hex(Color::TRANSPARENT));
            t.insert("selected_glow".into(), Value::Table(gt));
        }
    }
    if let Some(s) = item.focus_scale.filter(|s| s.is_finite()) {
        set_f(t, "selected_scale", s.clamp(1.0, 1.5));
    }
    if let Some(o) = item.unfocused_opacity.filter(|o| o.is_finite()) {
        set_f(t, "unselected_opacity", o.clamp(0.2, 1.0));
    }
    if let Some(r) = item.cover_radius {
        let key = if kind == "grid" { "corner_radius" } else { "radius" };
        set_f(t, key, (r / H).clamp(0.0, 0.05));
    }
    if collections {
        return;
    }
    if let Some(c) = item.uninstalled_scrim {
        t.insert("uninstalled_scrim".into(), hex(c));
    }
    match &item.caption {
        Some(c) if c.overlay => {
            t.insert("item_caption".into(), Value::String("{game.name}".into()));
            if let Some(b) = c.background.as_ref().and_then(brush_value) {
                t.insert("caption_background".into(), b);
            }
            if let (Some(sz), Some(Value::Table(s))) = (c.font_size, t.get_mut("item_text_style")) {
                set_f(s, "font_size", (sz / H).clamp(0.01, 0.05));
            }
        }
        Some(_) => {
            t.remove("item_caption");
            if kind == "grid" {
                t.insert("show_labels".into(), Value::Boolean(true));
            }
        }
        None => {
            t.remove("item_caption");
        }
    }
}

// ---------------------------------------------------------------------------- desktop

/// The desktop theme: Lodestar's layout with the theme's palette, fonts and desktop style.
pub(crate) fn desktop(look: &PlayniteLook, inp: &Inputs, notes: &mut Notes) -> anyhow::Result<Value> {
    use super::report::Area;
    let mut file = crate::theme::builtin::all().into_iter().next().map(|(_, f)| f).unwrap_or_default();
    file.theme.name = inp.name.clone();
    file.theme.author = inp.author.clone();
    file.theme.version = inp.version.clone();
    file.theme.description = inp.description.clone();
    file.theme.origin = "playnite".into();
    file.theme.controller = false;
    file.palette = inp.palette.clone();
    if let Some(f) = &inp.fonts {
        f.apply(&mut file.fonts);
    }
    let mut v: Value = toml::from_str(&file.to_toml()?)?;
    let Value::Table(root) = &mut v else {
        anyhow::bail!("theme isn't a table");
    };
    let d = &look.desktop;
    let mut ds = Table::new();
    let pt = |px: f32| round(px.clamp(0.0, 40.0));
    if let Some(r) = d.control_radius {
        set_f(&mut ds, "control_radius", pt(r));
    }
    if let Some(r) = d.window_radius {
        set_f(&mut ds, "window_radius", pt(r));
    }
    if let Some(r) = d.grid.cover_radius {
        set_f(&mut ds, "cover_radius", pt(r));
    }
    if let Some(s) = d.grid.spacing {
        set_f(&mut ds, "cover_gap", ((s[0] + s[2]).max(s[1] + s[3])).clamp(0.0, 40.0));
    }
    let title = match d.cover_title {
        super::look::CoverTitle::None => None,
        super::look::CoverTitle::Under => Some("under"),
        super::look::CoverTitle::Overlay => Some("overlay"),
        super::look::CoverTitle::Hover => Some("hover"),
    };
    if let Some(t) = title {
        ds.insert("cover_title".into(), Value::String(t.into()));
    }
    if let Some(f) = &d.grid.focus_frame {
        let mut sb = Table::new();
        if let Some(c) = f.brush.as_ref().and_then(LookBrush::average) {
            sb.insert("color".into(), hex(c.with_alpha(255)));
        }
        set_f(&mut sb, "width", f.width.clamp(1.0, 8.0));
        ds.insert("selection_border".into(), Value::Table(sb));
    }
    if let Some(c) = d.grid.hover_scrim {
        set_f(&mut ds, "hover_darken", c.0[3] as f32 / 255.0);
    }
    let mut fills = vec![];
    for (key, b, what) in [
        ("window_gradient", &d.window_background, "window"),
        ("sidebar_fill", &d.sidebar, "side panel"),
        ("topbar_fill", &d.top_panel, "top bar"),
        ("details_fill", &d.details_background, "details panel"),
    ] {
        if let Some(val) = b.as_ref().filter(|b| !b.is_clear()).and_then(brush_value) {
            ds.insert(key.into(), val);
            fills.push(what);
        }
    }
    if let Some(p) = &inp.picture {
        let mut bp = Table::new();
        bp.insert("path".into(), Value::String(p.clone()));
        if let Some(LookBrush::Image { stretch, opacity, .. }) = &d.background_picture {
            bp.insert("fit".into(), Value::String(fit_str(fit_of(Some(*stretch))).into()));
            // A half-transparent picture over the dark window reads as a dimmed one.
            if *opacity < 0.99 {
                set_f(&mut bp, "dim", (1.0 - opacity).clamp(0.0, 0.9));
            }
        }
        // (The importer's own "Background picture: …" line already names it.)
        ds.insert("background_picture".into(), Value::Table(bp));
    }
    if let Some(dp) = &d.details_picture {
        if let Some(c) = dp.darken {
            set_f(&mut ds, "details_background_dim", c.0[3] as f32 / 255.0);
        }
        if let Some(m) = &dp.mask {
            let e = mask_edges(m);
            if e[3] > 0.0 {
                set_f(&mut ds, "details_background_fade", (e[3] * 2.0).clamp(0.0, 1.0));
            }
        }
    }
    let fs = &look.font_sizes;
    if let Some(n) = fs.normal.filter(|n| *n > 4.0) {
        let mut sc = Table::new();
        set_f(&mut sc, "normal", (n / 14.0).clamp(0.6, 2.0));
        for (k, v) in [("small", fs.small), ("large", fs.large), ("larger", fs.larger), ("largest", fs.largest)] {
            if let Some(v) = v {
                set_f(&mut sc, k, (v / n).clamp(0.5, 4.0));
            }
        }
        ds.insert("font_scale".into(), Value::Table(sc));
        notes.ok(Area::Fonts, format!("Text sizes ({} px text)", n.round()));
    }
    if !fills.is_empty() {
        notes.ok(Area::Background, format!("Gradients and fills of the {}", fills.join(", ")));
    }
    let mut shape = vec![];
    if d.control_radius.is_some() || d.window_radius.is_some() {
        shape.push("corner shapes");
    }
    if d.grid.cover_radius.is_some() {
        shape.push("rounded covers");
    }
    if d.grid.focus_frame.is_some() {
        shape.push("selection border");
    }
    if title.is_some() {
        shape.push("where game names go");
    }
    if !shape.is_empty() {
        notes.ok(Area::Covers, format!("The theme's {}", shape.join(", ")));
    }
    if !ds.is_empty() {
        let mut base = match root.remove("desktop") {
            Some(Value::Table(t)) => t,
            _ => Table::new(),
        };
        base.extend(ds);
        root.insert("desktop".into(), Value::Table(base));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(top_px: f32, height: f32, valign: super::super::look::Align) -> ListLook {
        ListLook {
            area: Some([0.0, 0.0, 1.0, 1.0]),
            margin: Some([0.0, top_px, 0.0, 0.0]),
            height: Some(height),
            valign: Some(valign),
            ..Default::default()
        }
    }

    #[test]
    fn single_row_follows_the_themes_list() {
        use super::super::look::Align;
        // A strip of covers along the top (PlayStation 5 style): small covers under the top bar,
        // the game's logo and facts moved down below them.
        let top = row_place(&list(125.0, 120.0, Align::Start)).unwrap();
        assert!((top.top - 125.0 / H).abs() < 1e-4);
        assert!(top.item_h >= 0.13 && top.item_h <= 0.2);
        assert!(top.text_shift > 0.2 && !top.hide_description);
        // Across the middle (Xbox style): the text goes above it, without the description.
        let mid = row_place(&list(390.0, 320.0, Align::Start)).unwrap();
        assert!(mid.text_shift < 0.0 && mid.hide_description);
        assert!(mid.item_h <= ROW_ITEM_H);
        // Low on the screen, or no fixed height: Single Row's own place.
        assert_eq!(row_place(&list(700.0, 320.0, Align::Start)), None);
        assert_eq!(row_place(&ListLook { height: None, ..list(0.0, 0.0, Align::Start) }), None);
        // Applied to the layout's tables.
        let mut rows = Table::new();
        rows.insert("type".into(), Value::String("rows".into()));
        set_pair(&mut rows, "pos", [0.062, 0.6]);
        set_pair(&mut rows, "size", [0.938, 0.31]);
        top.apply(&mut rows, "pn_list");
        assert!((pair(&rows, "pos")[1] - top.top).abs() < 1e-3);
        assert!((pair(&rows, "item_size")[1] - top.item_h).abs() < 1e-3);
        let mut logo = Table::new();
        set_pair(&mut logo, "pos", [0.07, 0.4]);
        top.apply(&mut logo, "pn_logo");
        assert!(pair(&logo, "pos")[1] > 0.6);
        let mut desc = Table::new();
        set_pair(&mut desc, "pos", [0.07, 0.485]);
        mid.apply(&mut desc, "pn_description");
        assert_eq!(desc.get("visible").and_then(Value::as_bool), Some(false));
    }
}
