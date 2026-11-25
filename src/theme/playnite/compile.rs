//! The Playnite view compiler: a fullscreen theme's Main and GameDetails views (the theme's
//! own file, else Playnite's default underneath) laid out once with WPF's rules (layout.rs) on
//! a 1920 × 1080 screen and written as Lodestar elements.
//!
//! - Styles are applied as Playnite does (implicit and keyed, BasedOn, the resolver in tree.rs);
//!   Foreground / FontSize / FontFamily / FontWeight are inherited down the tree.
//! - Border / Panel backgrounds and Rectangles → `rect` (brushes with every stop, corner radius,
//!   border; a solid fill under a gradient OpacityMask becomes a gradient); ImageBrush → `image`.
//! - TextBlock with a literal, a `LOC…` string or a game binding (through Playnite's value
//!   converters) → `text`; Image with a theme file → `image` (copied into the theme), bound to
//!   the game's art → `image {art.*}`.
//! - PART_ elements → Lodestar's own: ListGameItems → `grid` / `carousel` (the cover look comes
//!   from the extracted look), TextClock → `datetime` clock, battery → text, bottom buttons →
//!   `helpbar`, ImageBackground → the game's picture (mask, blur), logo hooks → `{art.logo}`,
//!   HtmlDescription → scrolling text, ButtonContext → a Play / Install button in the face its
//!   template gives it (focused: the details page opens with the focus on it).
//! - Visibility: as written, `{Settings …}` at Playnite's defaults, DataTrigger /
//!   MultiDataTrigger on game properties → `visible_if` / `hidden_if`; triggers on the
//!   controller's state (ElementName helpers, focus, selected index …) are left out and the
//!   element keeps its resting state (counted for the report).
//! - Elements that fade in when the selection changes get an `appear` transition.
//! - The GameDetails page is laid out where Playnite shows it: inside the main screen's
//!   PART_ElemGameDetails, over what the main screen still shows with the page open.
//!
//! What can't be drawn (add-on panels, mirrored lists, videos other than the background,
//! custom controls) is dropped and measured: when too much of the screen is dropped or the
//! game list isn't a visible PART_ListGameItems, the importer lays the look onto one of
//! Lodestar's layout families instead (fill.rs).
//!
//! The stages, one submodule each: `build` turns the XAML tree into layout nodes (placement,
//! paint, what each element draws; `button` reads a button's template), `visibility` decides
//! when they show (Visibility, triggers, storyboards) and `bind` maps `{Binding}` paths onto
//! Lodestar's bindings; layout.rs
//! measures and arranges the nodes (text measured by `measure`); `emit` writes them as
//! element tables and `stack` turns panels of per-game texts into `stack` containers. The
//! types the stages share are defined here.

mod bind;
mod build;
mod button;
mod emit;
mod measure;
mod stack;
mod strip;
mod tabs;
mod visibility;

use super::Mode;
use super::layout::{self, Al, Content, Def, Dock, Len, Measurer, Node, Panel, Rect, Scale, TextSpec, Vis};
use super::loc::ViewStrings;
use super::look::{ItemLook, LookBrush, PlayniteLook, Stretch};
use super::options::Choices;
use super::settings::{self, SVal, setting};
use super::source::Source;
use super::tree::{self, Markup, Tree, Value, content, name_of, prop_elem, tag};
use super::xaml::attr;
use crate::theme::model::Color;
use roxmltree::Node as X;
use std::collections::{BTreeMap, HashMap};
use toml::{Table, Value as T};

use emit::{hint_rows, shoulder_prompts};
pub(crate) use measure::FontMeasurer;
use visibility::template_triggers;

const W: f32 = layout::SCREEN[0];

const H: f32 = layout::SCREEN[1];

/// Placeholders of the main screen's and the details page's video, filled in by `apply`.
const MAIN_VIDEO: &str = "@main_video";
const DETAILS_VIDEO: &str = "@details_video";

/// Below this share of the drawn area translated, the view isn't used.
pub(crate) const MIN_COVERAGE: f32 = 0.6;

/// Biggest theme picture read to learn its size for the layout.
const PICTURE_SIZE_LIMIT: u64 = 64 << 20;

/// What one view's compile did (report + fallback rule).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Stats {
    /// Elements written.
    pub elements: usize,
    /// Parts that couldn't be drawn, by kind ("add-on panel" → 3).
    pub dropped: BTreeMap<&'static str, usize>,
    /// Screen area drawn / dropped, px².
    pub area_drawn: f32,
    pub area_dropped: f32,
    /// Visibility triggers turned into show / hide rules, and the ones that react to the
    /// controller's state (left out).
    pub triggers_mapped: usize,
    pub triggers_dropped: usize,
    /// Parts that fade in when the selection changes.
    pub transitions: usize,
    /// PART_ListGameItems was found, shown and on the screen.
    pub list: bool,
    /// Panels written as `stack`s.
    pub stacks: usize,
    /// The Play / Install button (PART_ButtonContext) is drawn.
    pub play_button: bool,
}

impl Stats {
    pub fn coverage(&self) -> f32 {
        let all = self.area_drawn + self.area_dropped;
        if all <= 0.0 { 0.0 } else { self.area_drawn / all }
    }

    /// Something Lodestar can't draw. Add-on content counts but takes no area: a Playnite
    /// without that add-on shows nothing there either (what themes are made to handle).
    fn drop(&mut self, what: &'static str, area: f32) {
        *self.dropped.entry(what).or_default() += 1;
        if !matches!(what, "add-on panel" | "add-on text" | "video of the theme's add-ons") {
            self.area_dropped += area;
        }
    }
}

/// A compiled view.
#[derive(Clone, Debug, Default)]
pub(crate) struct ViewOut {
    pub elements: Vec<Table>,
    pub stats: Stats,
    /// The theme defines this view itself (not Playnite's default underneath).
    pub own: bool,
    /// The cover look of the game list when it is a row the theme lays out itself (None: the
    /// extracted look's).
    pub list_item: Option<ItemLook>,
}

/// The compiler's result.
#[derive(Clone, Debug, Default)]
pub(crate) struct Compiled {
    pub main: Option<ViewOut>,
    pub details: Option<ViewOut>,
    /// Theme files the elements use: (theme path, path in the Lodestar theme).
    pub images: Vec<(String, String)>,
    /// Why the main view can't be used, in words that follow "The theme's own main screen"
    /// (None: it can).
    pub main_problem: Option<String>,
}

/// Compile the theme's fullscreen Main and GameDetails views with the theme options' values
/// (`chosen`). `look` is the extracted look (list shape, cover look); `measurer` measures text
/// in Lodestar's fonts.
pub(crate) fn compile(source: &Source, look: &PlayniteLook, chosen: &Choices, measurer: &mut dyn Measurer) -> Compiled {
    let files = source.files();
    let texts = super::extract::chosen_texts(source, &files, Mode::Fullscreen, chosen);
    let mut tree = Tree::build(&texts, &files);
    tree.settings = chosen.settings.clone();
    tree.add_on_options = add_on_options(source, &files);
    let loc = ViewStrings::load(source, &files);
    let dims = |rel: &str| -> Option<[f32; 2]> {
        let bytes = source.read(rel, PICTURE_SIZE_LIMIT)?;
        let r = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
        let (w, h) = r.into_dimensions().ok()?;
        Some([w as f32, h as f32])
    };
    compile_tree(&tree, look, &loc, &dims, measurer)
}

/// The theme's own screens as one comparable text (options.rs: whether an option changes
/// them). Pictures keep no size of their own: enough to compare two settings of one theme.
pub(crate) fn views_print(tree: &Tree, source: &Source, measurer: &mut dyn Measurer) -> String {
    let look = super::extract::look_of(tree, None);
    let loc = ViewStrings::load(source, &source.files());
    let c = compile_tree(tree, &look, &loc, &|_| None, measurer);
    let elements = |v: Option<ViewOut>| v.map(|v| v.elements);
    format!("{:?}|{:?}", elements(c.main), elements(c.details))
}

/// The theme's ThemeOptions variables (its options.yaml) at their defaults.
fn add_on_options(source: &Source, files: &[(String, u64)]) -> settings::AddOnOptions {
    files
        .iter()
        .find(|(f, _)| f.eq_ignore_ascii_case("options.yaml"))
        .and_then(|(f, _)| source.read_text(f, tree::FILE_LIMIT))
        .map(|y| settings::AddOnOptions::parse(&y))
        .unwrap_or_default()
}

pub(crate) fn compile_tree(
    tree: &Tree,
    look: &PlayniteLook,
    loc: &ViewStrings,
    dims: &dyn Fn(&str) -> Option<[f32; 2]>,
    measurer: &mut dyn Measurer,
) -> Compiled {
    let mut out = Compiled::default();
    let mut images = BTreeMap::new();
    let input = Input { tree, look, loc, dims };
    if let Some(mut view) = compile_view(&input, measurer, &mut images, View::Main) {
        let usable = |v: &ViewOut| v.stats.list && !list_covered(&v.elements);
        if !usable(&view) {
            // The list shows once one of the theme's toggles is flipped (a "more games" or a
            // log-in button): the view with its toggles as the user leaves them.
            tree.toggles_at_rest.set(false);
            let mut open_images = BTreeMap::new();
            let open = compile_view(&input, measurer, &mut open_images, View::Main);
            if let Some(v) = open.filter(|v| usable(v)) {
                view = v;
                images = open_images;
            } else {
                tree.toggles_at_rest.set(true);
            }
        }
        // Why, as the end of "The theme's own main screen …" in the report.
        out.main_problem = if !view.stats.list {
            Some("needs Playnite add-ons".into())
        } else if list_covered(&view.elements) {
            Some("opens behind a start screen or menu that only Playnite can take away".into())
        } else if view.stats.coverage() < MIN_COVERAGE {
            Some("needs Playnite add-ons for most of what it shows".into())
        } else {
            None
        };
        out.main = Some(view);
    }
    out.details = compile_details(&input, measurer, &mut images, out.main_ok());
    tree.toggles_at_rest.set(true);
    out.images = images.into_iter().collect();
    out
}

/// The GameDetails page. Playnite opens it inside the main screen (its PART_ElemGameDetails),
/// so whatever the main screen still shows with the page open is part of it: the page over
/// the main screen when that translates, else the page on its own.
fn compile_details(
    input: &Input,
    measurer: &mut dyn Measurer,
    images: &mut BTreeMap<String, String>,
    main_ok: bool,
) -> Option<ViewOut> {
    if main_ok {
        let mut kept = images.clone();
        let over = compile_view(input, measurer, &mut kept, View::DetailsOverMain)
            .filter(|v| v.stats.play_button && v.stats.coverage() >= MIN_COVERAGE);
        if over.is_some() {
            *images = kept;
            return over;
        }
    }
    input.tree.toggles_at_rest.set(true);
    compile_view(input, measurer, images, View::Details)
}

/// Which screen `compile_view` lays out.
#[derive(Clone, Copy, Debug, PartialEq)]
enum View {
    Main,
    /// The GameDetails page on its own.
    Details,
    /// The main screen with the GameDetails page open in its PART_ElemGameDetails.
    DetailsOverMain,
}

/// What every view is compiled from.
struct Input<'a, 'i> {
    tree: &'a Tree<'i>,
    look: &'a PlayniteLook,
    loc: &'a ViewStrings,
    dims: &'a dyn Fn(&str) -> Option<[f32; 2]>,
}

/// One view laid out and written as elements; the theme files it uses are added to `images`.
fn compile_view(
    input: &Input,
    measurer: &mut dyn Measurer,
    images: &mut BTreeMap<String, String>,
    view: View,
) -> Option<ViewOut> {
    let Input { tree, look, loc, dims } = *input;
    let ty = if view == View::Details { "GameDetails" } else { "Main" };
    let style = tree.implicit_style(ty)?;
    let page = match view {
        View::Main => None,
        _ => Some(tree.implicit_style("GameDetails")?),
    };
    let own = tree.layer_of(page.unwrap_or(style)) == tree::Layer::Theme;
    let tpl = tree.template(style)?;
    let root = tree.template_root(tpl)?;
    let mut b = Builder {
        t: tree,
        look,
        loc,
        dims,
        main: view != View::Details,
        details_open: view != View::Main,
        details_page: if view == View::DetailsOverMain { page.and_then(|p| tree.template(p)) } else { None },
        stats: Stats::default(),
        images,
        tpl_triggers: template_triggers(tree, tpl),
        names: 0,
        root: Some(tpl),
        stacks: stack::enabled(),
        strip: None,
        trigger_depth: std::cell::Cell::new(0),
        placing_list: std::cell::Cell::new(false),
        shoulder_prompts: vec![],
        hint_rows: vec![],
    };
    let ty = match view {
        View::Main => "Main",
        View::Details => "GameDetails",
        View::DetailsOverMain => "GameDetailsOverMain",
    };
    let inh = Inherit::root(tree);
    let mut node = b.build(root, &inh, 0)?;
    if b.details_page.is_some() {
        // The main screen has no place for the page.
        return None;
    }
    layout::layout(&mut node, measurer);
    widen_slots(&mut node);
    b.strip = strip::pick_list(&mut node);
    b.shoulder_prompts = shoulder_prompts(&node);
    b.hint_rows = hint_rows(&node);
    let dump_to = std::env::var_os("LODESTAR_PN_COMPILE_DUMP").map(|d| d.to_string_lossy().to_string());
    if let Some(d) = &dump_to {
        dump(&node, &format!("{d}-{ty}.txt"));
    }
    let mut els = vec![];
    let screen = Rect::new(0.0, 0.0, W, H);
    b.emit(&node, &Ctx { clip: screen, ..Ctx::default() }, &mut els);
    if view == View::DetailsOverMain {
        drop_hidden(&mut els);
    }
    for (i, e) in els.iter_mut().enumerate() {
        e.insert("z".into(), T::Float(round(0.5 + i as f32 * 0.01) as f64));
    }
    if let Some(d) = &dump_to {
        let text: Vec<String> =
            els.iter().map(|e| toml::to_string(e).unwrap_or_default().replace('\n', " | ")).collect();
        let _ = std::fs::write(format!("{d}-{ty}-elements.txt"), text.join("\n"));
        let _ = std::fs::write(format!("{d}-{ty}-stats.txt"), format!("{:#?}", b.stats));
    }
    let mut stats = b.stats;
    stats.elements = els.len();
    let list_item = b.strip.map(|s| ItemLook { cover_radius: (s.radius > 0.0).then_some(s.radius), ..s.look });
    Some(ViewOut { elements: els, stats, own, list_item })
}

impl Compiled {
    /// The theme's own main screen can be used.
    pub fn main_ok(&self) -> bool {
        self.main.is_some() && self.main_problem.is_none()
    }

    /// The theme's own details page goes with `family`: always with its translated main
    /// screen, and with a layout family when that stands in for a main screen Lodestar can't
    /// translate (a family the user picks over a working translation keeps its own page).
    pub fn details_with(&self, family: super::fill::Family) -> bool {
        self.details_used() && (family == super::fill::Family::Translated || !self.main_ok())
    }

    /// The details page is used: it translated well enough and shows its Play button (a page
    /// whose panel only an add-on brings in shows nothing else either).
    pub fn details_used(&self) -> bool {
        self.details.as_ref().is_some_and(|d| d.stats.play_button && d.stats.coverage() >= MIN_COVERAGE)
    }
}

/// Put the compiled views into a theme filled from Top Bar (`fill::controller`): its
/// games view becomes the theme's main screen (opened first), its details page the theme's
/// GameDetails when that translated well. `videos` = the copied main / details videos.
pub(crate) fn apply(theme: &mut T, c: &Compiled, look: &PlayniteLook, videos: [Option<&str>; 2]) -> anyhow::Result<()> {
    let root = theme.as_table_mut().ok_or_else(|| anyhow::anyhow!("theme isn't a table"))?;
    if let Some(T::Table(meta)) = root.get_mut("theme") {
        meta.insert("start".into(), T::String("games".into()));
    }
    root.remove("home");
    if let Some(main) = &c.main {
        put_view(root, "games", main, look, videos);
    }
    apply_details(theme, c, look, videos)
}

/// Put the compiled GameDetails page (when it translated well) into a theme filled by any of
/// the layout families: a theme whose main screen Lodestar can't translate still gets its own
/// details page. `videos` = the copied main / details videos.
pub(crate) fn apply_details(
    theme: &mut T,
    c: &Compiled,
    look: &PlayniteLook,
    videos: [Option<&str>; 2],
) -> anyhow::Result<()> {
    let root = theme.as_table_mut().ok_or_else(|| anyhow::anyhow!("theme isn't a table"))?;
    if let Some(details) = c.details.as_ref().filter(|_| c.details_used()) {
        put_view(root, "details", details, look, videos);
    }
    Ok(())
}

/// Replace `view`'s elements with the compiled ones, over the theme's background brush.
fn put_view(root: &mut Table, view: &str, out: &ViewOut, look: &PlayniteLook, videos: [Option<&str>; 2]) {
    let find = |root: &Table, pick: &dyn Fn(&Table) -> bool| -> Option<Table> {
        root.get(view)?.get("elements")?.as_array()?.iter().filter_map(T::as_table).find(|t| pick(t)).cloned()
    };
    // The theme's background brush under everything (Playnite paints the window with it).
    let mut els: Vec<T> = vec![];
    if let Some(mut bg) = find(root, &|t: &Table| t.get("name").and_then(T::as_str) == Some("pn_background")) {
        bg.insert("z".into(), T::Float(0.0));
        els.push(T::Table(bg));
    }
    for e in &out.elements {
        let mut e = e.clone();
        let placeholder = e.get("video").and_then(T::as_str);
        if let Some(p) = placeholder.filter(|p| [MAIN_VIDEO, DETAILS_VIDEO].contains(p)) {
            match videos[usize::from(p == DETAILS_VIDEO)] {
                Some(v) => {
                    e.insert("video".into(), T::String(v.to_string()));
                }
                None => continue,
            }
        }
        if e.get("name").and_then(T::as_str) == Some("pn_list") {
            super::fill::item_look(&mut e, out.list_item.as_ref().unwrap_or(&look.item), false);
        }
        els.push(T::Table(e));
    }
    // Lodestar's button hints on the details page (Playnite shows none there; B goes back),
    // unless the theme draws its own hints (a hint bar or button glyphs) or its own row of
    // texts (buttons, stats) where they go.
    if view == "details"
        && !out.elements.iter().any(|e| e.get("type").and_then(T::as_str) == Some("helpbar"))
        && let Some(mut help) = find(root, &|t: &Table| t.get("type").and_then(T::as_str) == Some("helpbar"))
        && !element_box(&help).is_some_and(|band| texts_in(&out.elements, band))
    {
        help.insert("z".into(), T::Float(90.0));
        els.push(T::Table(help));
    }
    if let Some(T::Table(v)) = root.get_mut(view) {
        v.insert("elements".into(), T::Array(els));
        // Playnite draws these views on a 1920 × 1080 canvas scaled as one piece: on a
        // 16:10 or 4:3 screen texts keep the room their boxes were measured with.
        v.insert("viewbox".into(), T::Array(vec![T::Float(W as f64), T::Float(H as f64)]));
    }
}

/// An element's box on the 1920 × 1080 screen (`pos`, `size`, `origin`; a text without a
/// height is one line of its font).
fn element_box(e: &Table) -> Option<Rect> {
    let pair = |k: &str| -> Option<[f32; 2]> {
        let a = e.get(k)?.as_array()?;
        Some([a.first()?.as_float()? as f32, a.get(1)?.as_float()? as f32])
    };
    let pos = pair("pos")?;
    let mut size = pair("size").unwrap_or([0.0, 0.0]);
    if size[1] <= 0.0 {
        let font = e.get("style").and_then(|s| s.get("font_size")).and_then(T::as_float).unwrap_or(0.0) as f32;
        size[1] = font * 1.344;
    }
    let origin = pair("origin").unwrap_or([0.0, 0.0]);
    Some(Rect::new((pos[0] - origin[0] * size[0]) * W, (pos[1] - origin[1] * size[1]) * H, size[0] * W, size[1] * H))
}

/// A one-line text of `els` (a label, a button, a hint, a stat — not a description running
/// down the page) sits in `band`.
fn texts_in(els: &[Table], band: Rect) -> bool {
    els.iter().any(|e| {
        matches!(e.get("type").and_then(T::as_str), Some("text" | "datetime"))
            && e.get("visible").and_then(T::as_bool) != Some(false)
            && e.get("wrap").and_then(T::as_bool) != Some(true)
            && element_box(e).is_some_and(|b| b.w > 0.0 && b.intersect(&band).area() > b.area().min(band.area()) * 0.2)
    })
}

/// Report lines for a translated import (area, carried?, text); `with_main`: the theme's own
/// main screen is used too (else only its details page is).
pub(crate) fn report_lines(c: &Compiled, with_main: bool) -> Vec<(super::report::Area, bool, String)> {
    use super::report::Area;
    let mut out = vec![];
    let main = c.main.as_ref().filter(|_| with_main);
    if let Some(m) = main {
        out.push((
            Area::Layout,
            true,
            format!(
                "Main screen: {} parts in the theme's places, sizes and colours ({:.0} % of what it draws)",
                m.stats.elements,
                m.stats.coverage() * 100.0
            ),
        ));
    }
    match &c.details {
        Some(d) if c.details_used() => out.push((
            Area::Layout,
            true,
            format!("Game details page: {} parts, as the theme lays it out", d.stats.elements),
        )),
        Some(d) if d.own => out.push((
            Area::Layout,
            false,
            "The theme's game details page couldn't be translated — Playnite's standard one is used in the theme's colours"
                .to_string(),
        )),
        _ => {}
    }
    let mut dropped: BTreeMap<&str, usize> = BTreeMap::new();
    let mut ui = 0;
    let mut fades = 0;
    for v in [main, c.details.as_ref().filter(|_| c.details_used())].into_iter().flatten() {
        for (k, n) in &v.stats.dropped {
            *dropped.entry(k).or_default() += n;
        }
        ui += v.stats.triggers_dropped;
        fades += v.stats.transitions;
    }
    if fades > 0 {
        out.push((Area::Layout, true, format!("Parts that fade in when you move to another game ({fades})")));
    }
    if !dropped.is_empty() {
        let list: Vec<String> = dropped
            .iter()
            .map(|(k, n)| match (*k, *n) {
                (k, 1) => format!("1 {}", singular(k)),
                (k, n) => format!("{n} {}", plural(k)),
            })
            .collect();
        out.push((Area::NotPossible, false, format!("Parts of its screens Lodestar can't draw: {}", list.join(", "))));
    }
    if ui > 0 {
        let (what, rest) = if ui == 1 {
            ("1 effect that reacts".to_string(), "that part keeps its")
        } else {
            (format!("{ui} effects that react"), "those parts keep their")
        };
        out.push((
            Area::NotPossible,
            false,
            format!("{what} to the controller (menus sliding in, helpers that move the list) — {rest} resting look"),
        ));
    }
    out
}

fn singular(k: &str) -> String {
    k.to_string()
}

fn plural(k: &str) -> String {
    match k {
        "add-on panel" => "add-on panels".into(),
        "button" => "buttons".into(),
        "control" => "controls".into(),
        "custom control" => "custom controls".into(),
        "drawn shape" => "drawn shapes".into(),
        "icon glyph" => "icon glyphs".into(),
        "list driven by the theme's helpers" => "lists driven by the theme's helpers".into(),
        "picture" => "pictures".into(),
        "picture bound to something Lodestar doesn't have" => "pictures of things Lodestar doesn't have".into(),
        "text Lodestar has no data for" => "texts Lodestar has no data for".into(),
        "add-on text" => "add-on texts".into(),
        "video of the theme's add-ons" => "videos of the theme's add-ons".into(),
        other => format!("{other} ×"),
    }
}

/// A `{Binding …}` taken apart.
#[derive(Clone, Debug, Default, PartialEq)]
struct Bind {
    path: String,
    element: Option<String>,
    /// A `RelativeSource` or `Source` instead of the data context.
    relative: bool,
    /// The `RelativeSource` mode (`Self`, `TemplatedParent`, `FindAncestor` …).
    relative_mode: Option<String>,
    converter: Option<String>,
    param: Option<String>,
    format: Option<String>,
}

impl Bind {
    /// It reads the element it is set on (`RelativeSource Self`).
    fn on_self(&self) -> bool {
        self.relative_mode.as_deref() == Some("Self")
    }

    /// It reads the control a template part belongs to: the part itself or the templated
    /// parent.
    fn on_control(&self) -> bool {
        matches!(self.relative_mode.as_deref(), Some("Self" | "TemplatedParent"))
    }
}

/// When something is shown, in Lodestar terms.
#[derive(Clone, Debug, Default, PartialEq)]
struct Cond {
    visible_if: Option<String>,
    hidden_if: Option<String>,
}

/// One trigger condition, evaluated statically.
#[derive(Clone, Debug, PartialEq)]
enum Test {
    Always,
    Never,
    /// True when the binding resolves non-empty / empty.
    Set(String),
    Unset(String),
    /// Several game properties at once: a `visible_if` condition (`{a} && !{b}`).
    Expr(String),
    /// Depends on the controller's state (focus, helpers, selected index …).
    Ui,
}

/// Inherited text properties.
#[derive(Clone, Debug)]
struct Inherit {
    fg: Option<LookBrush>,
    size: f32,
    family: Option<String>,
    weight: Option<String>,
}

/// What a node draws.
#[derive(Clone, Debug, Default)]
enum Kind {
    #[default]
    None,
    /// Can't be drawn (counted, with its area).
    Drop(&'static str),
    Text(TextOut),
    Image(ImageOut),
    List,
    Clock(TextOut),
    /// A bottom-bar button (the help bar takes the place of its row). `unread`: a button
    /// drawing its InputHint's prompt and a label Lodestar can't read (set at runtime), so its
    /// measured box is too small for what it shows.
    Help {
        look: TextOut,
        unread: bool,
    },
    Button(ButtonOut),
    /// One of the theme's own videos, by the placeholder of the view it belongs to
    /// (`MAIN_VIDEO` / `DETAILS_VIDEO`; the importer copies one video per view).
    Video(&'static str),
    /// A row of the game list's items the theme lays out itself (strip.rs); the game list when
    /// PART_ListGameItems itself doesn't show.
    Mirror(Box<strip::Strip>),
    /// The filter preset tabs (PART_FilterPresetSelector): Lodestar's main collections, the
    /// open one highlighted.
    Tabs(Box<TabsOut>),
    /// A button prompt the theme draws next to its own text (`ButtonPromptA` …): that
    /// button's glyph in the user's controller style (a help bar `buttons` name).
    Prompt(&'static str, Option<Color>),
}

#[derive(Clone, Debug, Default)]
struct TabsOut {
    /// The names in Lodestar's own look (when the theme's preset template isn't read).
    label: TextOut,
    /// Colour of the open collection's tab in that look.
    current: Option<Color>,
    /// The strip's direction and the theme's preset buttons (tabs.rs).
    look: tabs::PresetLook,
}

/// The tabs a filter preset strip shows: Lodestar's collections that match Playnite's default
/// presets (key, label). Any other open collection gets a tab of its own after them.
const TABS: [(&str, &str); 4] =
    [("all", "All Games"), ("recent", "Continue Playing"), ("favorites", "Favourites"), ("installed", "Installed")];

#[derive(Clone, Debug, Default, PartialEq)]
struct TextOut {
    text: String,
    color: Option<Color>,
    size: f32,
    font: Option<&'static str>,
    align: Option<&'static str>,
    wrap: bool,
    upper: bool,
    shadow: Option<Color>,
    /// The game's description (scrolls in its box).
    scroll: bool,
    /// Bound to something that changes per game (its box may grow within the slot).
    bound: bool,
    /// Where the element sits in its slot (HorizontalAlignment): "left", "center", "right".
    anchor: &'static str,
}

#[derive(Clone, Debug, Default)]
struct ImageOut {
    path: String,
    fallbacks: Vec<String>,
    fit: &'static str,
    mask: Option<[f32; 4]>,
    blur: f32,
    placeholder: bool,
    /// A picture that is a background (drawn first, may cover the screen).
    picture: bool,
}

#[derive(Clone, Debug, Default)]
struct ButtonOut {
    label: TextOut,
    fill: Option<LookBrush>,
    border: Option<(Color, f32)>,
    radius: f32,
    /// The face's height and its offset from the button's top, when the button's template
    /// paints a band narrower than its box (button.rs `Face::band`).
    band: Option<(f32, f32)>,
    /// Play / Install by the game's state.
    context: bool,
    /// The label is the one the template writes itself (not Play / Install by the game's state).
    own_text: bool,
    /// The look for installed games when the template draws them differently (this one is then
    /// the other games').
    installed: Option<Box<ButtonOut>>,
}

/// A node's drawing.
#[derive(Clone, Debug, Default)]
struct Info {
    name: String,
    tag: String,
    kind: Kind,
    /// Background / Fill.
    fill: Option<LookBrush>,
    stroke: Option<(Color, f32)>,
    radius: f32,
    opacity: f32,
    cond: Cond,
    /// Clips its children (ScrollViewer, ClipToBounds).
    clip: bool,
    /// Fades in after the selection changes (ms, delay ms).
    appear_ms: Option<(u32, u32)>,
}

type LNode = Node<Info>;

struct Builder<'b, 't, 'i> {
    t: &'t Tree<'i>,
    look: &'b PlayniteLook,
    loc: &'b ViewStrings,
    dims: &'b dyn Fn(&str) -> Option<[f32; 2]>,
    /// Bindings are the main screen's (`SelectedGame.…`), not the details page's (`Game.…`).
    main: bool,
    /// The game details page is open (PART_ElemGameDetails shows, the focus is on its Play
    /// button).
    details_open: bool,
    /// The GameDetails template to lay out in the main screen's PART_ElemGameDetails.
    details_page: Option<X<'t, 'i>>,
    stats: Stats,
    images: &'b mut BTreeMap<String, String>,
    tpl_triggers: HashMap<String, Vec<X<'t, 'i>>>,
    names: usize,
    /// The view's visual root (ElementName lookups).
    root: Option<X<'t, 'i>>,
    /// Flowing panels become `stack` containers (the placement tests turn this off).
    stacks: bool,
    /// The row the theme lays out itself that stands in for the game list.
    strip: Option<strip::Strip>,
    /// Triggers being evaluated inside each other (`trigger_setter`, `settings_warning`): a
    /// trigger whose condition reads what it sets ends here.
    trigger_depth: std::cell::Cell<u8>,
    /// The game list's own size and margins are being read (`place`): its SelectedIndex is
    /// the first game's.
    placing_list: std::cell::Cell<bool>,
    /// Where the view's LB / RB prompts sit (the tab strip keeps clear of them).
    shoulder_prompts: Vec<Rect>,
    /// Where the view's bottom-bar buttons sit (the help bar spans those level with it).
    hint_rows: Vec<emit::HintRow>,
}

fn round(v: f32) -> f32 {
    (v * 10000.0).round() / 10000.0
}

/// What `decorate` needs of the node being written.
struct Deco<'a> {
    n: &'a LNode,
    opacity: f32,
    cond: &'a Cond,
}

/// Emit context.
#[derive(Clone, Debug)]
struct Ctx {
    opacity: f32,
    clip: Rect,
    cond: Cond,
    /// The scroll viewer the element is in (its visible box).
    scroll: Option<Rect>,
    /// The row of bottom buttons the help bar takes the place of.
    row: Option<Rect>,
    /// What WPF's layout clips the element to: the boxes of its ancestors with a set width
    /// (Width / MaxWidth), where content wider than that is cut off. The tab strip keeps to it.
    bounds: Rect,
}

/// How Playnite's FullscreenTilePanel fills a game list's box: horizontal layout (Playnite's
/// default) makes items `width ÷ (Columns + ½)` wide, as tall as the cover aspect says (at most
/// the box), and as many rows as fit; the vertical layout makes them `height ÷ (Rows + ½)`
/// tall with as many columns as fit.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Tiles {
    /// One item's slot [w, h], px.
    item: [f32; 2],
    rows: usize,
    columns: usize,
}

/// An opaque panel drawn after the game list, shown whatever the game, hides most of it: a
/// start screen or a menu the theme's own state machine (or an add-on) takes away.
fn list_covered(els: &[Table]) -> bool {
    let rect_of = |e: &Table| -> Option<Rect> {
        let p = e.get("pos")?.as_array()?;
        let s = e.get("size")?.as_array()?;
        let f = |v: &T| v.as_float().unwrap_or(0.0) as f32;
        Some(Rect::new(f(&p[0]) * W, f(&p[1]) * H, f(&s[0]) * W, f(&s[1]) * H))
    };
    let Some(i) = els.iter().position(|e| e.get("name").and_then(T::as_str) == Some("pn_list")) else {
        return false;
    };
    let Some(list) = rect_of(&els[i]) else { return false };
    let mut covered = 0.0;
    for e in &els[i + 1..] {
        if e.get("type").and_then(T::as_str) != Some("rect")
            || e.contains_key("visible_if")
            || e.contains_key("hidden_if")
        {
            continue;
        }
        let alpha =
            e.get("color").and_then(T::as_str).and_then(Color::parse).map(|c| c.0[3]).unwrap_or(0) as f32 / 255.0;
        let op = e.get("opacity").and_then(T::as_float).unwrap_or(1.0) as f32;
        if alpha * op < 0.8 || e.contains_key("gradient_stops") {
            continue;
        }
        if let Some(r) = rect_of(e) {
            covered += r.intersect(&list).area();
        }
    }
    covered > list.area() * 0.5
}

/// Elements under a later one that covers the whole screen with nothing showing through it
/// (an opaque rect, or the game's picture filling the screen, whatever the game) are never
/// seen: the main screen under a details page that paints its own background.
fn drop_hidden(els: &mut Vec<Table>) {
    let covers = |e: &Table| -> bool {
        let plain = !["visible_if", "hidden_if", "transitions", "mask", "corner_radius", "gradient_stops", "gradient"]
            .iter()
            .any(|k| e.contains_key(*k))
            && e.get("opacity").and_then(T::as_float).is_none_or(|o| o >= 0.999);
        let full =
            element_box(e).is_some_and(|b| b.x <= 0.5 && b.y <= 0.5 && b.x + b.w >= W - 0.5 && b.y + b.h >= H - 0.5);
        let opaque = match e.get("type").and_then(T::as_str) {
            Some("rect") => e.get("color").and_then(T::as_str).and_then(Color::parse).is_some_and(|c| c.0[3] == 255),
            Some("image") => {
                e.get("path").and_then(T::as_str) == Some("{art.hero}")
                    && e.get("fit").and_then(T::as_str) == Some("cover")
            }
            _ => false,
        };
        plain && full && opaque
    };
    if let Some(i) = els.iter().rposition(covers) {
        els.drain(..i);
    }
}

/// Per-game single-line texts and clocks may grow past the box they were measured in (with a
/// sample value) up to the next thing beside them in their panel, or the panel's edge: a
/// longer date or play time is then drawn whole where Playnite would draw it whole too, and
/// never over its neighbour (a wide grid cell doesn't let it run under the next part).
fn widen_slots(n: &mut LNode) {
    let parent = n.rect;
    let boxes: Vec<(Rect, bool)> = n
        .children
        .iter()
        .map(|c| {
            (
                c.rect,
                c.vis == Vis::Visible
                    && c.rect.area() > 0.0
                    && (!matches!(c.data.kind, Kind::None) || !c.children.is_empty() || c.data.fill.is_some()),
            )
        })
        .collect();
    for (i, c) in n.children.iter_mut().enumerate() {
        let grows = match &c.data.kind {
            Kind::Text(t) => t.bound && !t.wrap,
            Kind::Clock(_) => true,
            _ => false,
        };
        if grows && c.vis == Vis::Visible {
            let r = c.rect;
            let (mut lo, mut hi) = (parent.x.min(r.x), (parent.x + parent.w).max(r.x + r.w));
            for (j, (b, draws)) in boxes.iter().enumerate() {
                let beside = b.y < r.y + r.h && b.y + b.h > r.y;
                if j == i || !draws || !beside {
                    continue;
                }
                if b.x >= r.x + r.w - 0.5 {
                    hi = hi.min(b.x);
                } else if b.x + b.w <= r.x + 0.5 {
                    lo = lo.max(b.x + b.w);
                }
            }
            if hi - lo >= r.w {
                c.slot = Rect::new(lo, c.slot.y, hi - lo, c.slot.h);
            }
        }
        widen_slots(c);
    }
}

/// The laid-out tree as text (developer diagnostics: `LODESTAR_PN_COMPILE_DUMP=<prefix>`).
fn dump(node: &LNode, path: &str) {
    use std::fmt::Write as _;
    let mut s = String::new();
    layout::walk(node, &mut |n, d| {
        let r = n.rect;
        let _ = writeln!(
            s,
            "{:w$}{} {} [{:.0},{:.0} {:.0}x{:.0}] {:?} op={:.2} z={} {:?} {:?}",
            "",
            n.data.tag,
            n.data.name,
            r.x,
            r.y,
            r.w,
            r.h,
            n.vis,
            n.data.opacity,
            n.z,
            n.data.kind,
            n.data.cond,
            w = d * 2
        );
    });
    let _ = std::fs::write(path, s);
}

#[cfg(test)]
mod tests;
