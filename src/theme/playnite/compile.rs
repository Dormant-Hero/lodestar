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
//!   HtmlDescription → scrolling text, ButtonContext → a Play / Install button.
//! - Visibility: as written, `{Settings …}` at Playnite's defaults, DataTrigger /
//!   MultiDataTrigger on game properties → `visible_if` / `hidden_if`; triggers on the
//!   controller's state (ElementName helpers, focus, selected index …) are left out and the
//!   element keeps its resting state (counted for the report).
//! - Elements that fade in when the selection changes get an `appear` transition.
//!
//! What can't be drawn (add-on panels, mirrored lists, videos other than the background,
//! custom controls) is dropped and measured: when too much of the screen is dropped or the
//! game list isn't a visible PART_ListGameItems, the importer lays the look onto one of
//! Lodestar's layout families instead (fill.rs).
//!
//! The stages, one submodule each: `build` turns the XAML tree into layout nodes (placement,
//! paint, what each element draws), `visibility` decides when they show (Visibility,
//! triggers, storyboards) and `bind` maps `{Binding}` paths onto Lodestar's bindings; layout.rs
//! measures and arranges the nodes (text measured by `measure`); `emit` writes them as
//! element tables and `stack` turns panels of per-game texts into `stack` containers. The
//! types the stages share are defined here.

mod bind;
mod build;
mod emit;
mod measure;
mod stack;
mod visibility;

use super::Mode;
use super::layout::{self, Al, Content, Def, Dock, Len, Measurer, Node, Panel, Rect, Scale, TextSpec, Vis};
use super::loc::ViewStrings;
use super::look::{LookBrush, PlayniteLook, Stretch};
use super::settings::{self, SVal, setting};
use super::source::Source;
use super::tree::{self, Markup, Texts, Tree, Value, content, name_of, prop_elem, tag};
use super::xaml::attr;
use crate::theme::model::Color;
use roxmltree::Node as X;
use std::collections::{BTreeMap, HashMap};
use toml::{Table, Value as T};

use emit::is_hint_bar;
pub(crate) use measure::FontMeasurer;
use visibility::template_triggers;

const W: f32 = layout::SCREEN[0];

const H: f32 = layout::SCREEN[1];

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

/// Compile the theme's fullscreen Main and GameDetails views. `look` is the extracted look
/// (list shape, cover look); `measurer` measures text in Lodestar's fonts.
pub(crate) fn compile(source: &Source, look: &PlayniteLook, measurer: &mut dyn Measurer) -> Compiled {
    let files = source.files();
    let texts = Texts::load(source, &files, Mode::Fullscreen);
    let tree = Tree::build(&texts, &files);
    let loc = ViewStrings::load(source, &files);
    let dims = |rel: &str| -> Option<[f32; 2]> {
        let bytes = source.read(rel, PICTURE_SIZE_LIMIT)?;
        let r = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
        let (w, h) = r.into_dimensions().ok()?;
        Some([w as f32, h as f32])
    };
    compile_tree(&tree, look, &loc, &dims, measurer)
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
    for (ty, main) in [("Main", true), ("GameDetails", false)] {
        let mut kept = images.clone();
        let Some(mut view) = compile_view(&input, measurer, &mut kept, ty, main) else { continue };
        let usable = |v: &ViewOut| v.stats.list && !list_covered(&v.elements);
        if main && !usable(&view) {
            // The list shows once one of the theme's toggles is flipped (a "more games" or a
            // log-in button): the view with its toggles as the user leaves them.
            tree.toggles_at_rest.set(false);
            let mut open_images = images.clone();
            let open = compile_view(&input, measurer, &mut open_images, ty, main);
            tree.toggles_at_rest.set(true);
            if let Some(v) = open.filter(|v| usable(v)) {
                view = v;
                kept = open_images;
            }
        }
        images = kept;
        if main {
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
        } else {
            out.details = Some(view);
        }
    }
    out.images = images.into_iter().collect();
    out
}

/// What every view is compiled from.
struct Input<'a, 'i> {
    tree: &'a Tree<'i>,
    look: &'a PlayniteLook,
    loc: &'a ViewStrings,
    dims: &'a dyn Fn(&str) -> Option<[f32; 2]>,
}

/// One view (`Main` / `GameDetails`) laid out and written as elements; the theme files it uses
/// are added to `images`.
fn compile_view(
    input: &Input,
    measurer: &mut dyn Measurer,
    images: &mut BTreeMap<String, String>,
    ty: &str,
    main: bool,
) -> Option<ViewOut> {
    let Input { tree, look, loc, dims } = *input;
    let style = tree.implicit_style(ty)?;
    let own = tree.layer_of(style) == tree::Layer::Theme;
    let tpl = tree.template(style)?;
    let root = tree.template_root(tpl)?;
    let mut b = Builder {
        t: tree,
        look,
        loc,
        dims,
        main,
        stats: Stats::default(),
        images,
        tpl_triggers: template_triggers(tree, tpl),
        names: 0,
        root: Some(tpl),
        stacks: stack::enabled(),
    };
    let inh = Inherit::root(tree);
    let mut node = b.build(root, &inh, 0)?;
    layout::layout(&mut node, measurer);
    widen_slots(&mut node);
    let dump_to = std::env::var_os("LODESTAR_PN_COMPILE_DUMP").map(|d| d.to_string_lossy().to_string());
    if let Some(d) = &dump_to {
        dump(&node, &format!("{d}-{ty}.txt"));
    }
    let mut els = vec![];
    let screen = Rect::new(0.0, 0.0, W, H);
    b.emit(&node, &Ctx { clip: screen, ..Ctx::default() }, &mut els);
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
    Some(ViewOut { elements: els, stats, own })
}

impl Compiled {
    /// The details page is used (translated well enough).
    pub fn details_used(&self) -> bool {
        self.details.as_ref().is_some_and(|d| !d.elements.is_empty() && d.stats.coverage() >= MIN_COVERAGE)
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
    let find = |root: &Table, view: &str, pick: &dyn Fn(&Table) -> bool| -> Option<Table> {
        root.get(view)?.get("elements")?.as_array()?.iter().filter_map(T::as_table).find(|t| pick(t)).cloned()
    };
    let named = |n: &'static str| move |t: &Table| t.get("name").and_then(T::as_str) == Some(n);
    let views: [(&str, Option<&ViewOut>, Option<&str>); 2] = [
        ("games", c.main.as_ref(), videos[0]),
        ("details", c.details.as_ref().filter(|_| c.details_used()), videos[1]),
    ];
    for (view, out, video) in views {
        let Some(out) = out else { continue };
        // The theme's background brush under everything (Playnite paints the window with it).
        let mut els: Vec<T> = vec![];
        if let Some(mut bg) = find(root, view, &named("pn_background")) {
            bg.insert("z".into(), T::Float(0.0));
            els.push(T::Table(bg));
        }
        for e in &out.elements {
            let mut e = e.clone();
            if e.get("video").and_then(T::as_str) == Some("@main_video") {
                match video {
                    Some(v) => {
                        e.insert("video".into(), T::String(v.to_string()));
                    }
                    None => continue,
                }
            }
            if e.get("name").and_then(T::as_str) == Some("pn_list") {
                super::fill::item_look(&mut e, &look.item, false);
            }
            els.push(T::Table(e));
        }
        // Lodestar's button hints on the details page (Playnite shows none there; B goes back),
        // unless the theme draws its own row of texts (buttons, hints, stats) where they go.
        if view == "details"
            && !out.elements.iter().any(is_hint_bar)
            && let Some(mut help) = find(root, view, &|t: &Table| t.get("type").and_then(T::as_str) == Some("helpbar"))
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
    Ok(())
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

/// Report lines for a translated import (area, carried?, text).
pub(crate) fn report_lines(c: &Compiled) -> Vec<(super::report::Area, bool, String)> {
    use super::report::Area;
    let mut out = vec![];
    if let Some(m) = &c.main {
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
    for v in [c.main.as_ref(), c.details.as_ref().filter(|_| c.details_used())].into_iter().flatten() {
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
        out.push((
            Area::NotPossible,
            false,
            format!(
                "{ui} effect{} that react to the controller (menus sliding in, helpers that move the list) — those parts keep their resting look",
                if ui == 1 { "" } else { "s" }
            ),
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
    relative: bool,
    converter: Option<String>,
    param: Option<String>,
    format: Option<String>,
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
    Help(TextOut),
    Button(ButtonOut),
    /// The theme's background video.
    Video,
    /// The filter preset tabs (PART_FilterPresetSelector): Lodestar's main collections, the
    /// open one highlighted.
    Tabs(TabsOut),
    /// A button prompt the theme draws next to its own text (`ButtonPromptA` …): that
    /// button's glyph in the user's controller style (a help bar `buttons` name).
    Prompt(&'static str, Option<Color>),
}

#[derive(Clone, Debug, Default)]
struct TabsOut {
    label: TextOut,
    /// Colour of the open collection's tab.
    current: Option<Color>,
}

/// The tabs a filter preset strip shows: Lodestar's collections that match Playnite's default
/// presets (key, label). Any other open collection gets a tab of its own after them.
const TABS: [(&str, &str); 4] =
    [("all", "All Games"), ("recent", "Continue Playing"), ("favorites", "Favourites"), ("installed", "Installed")];

#[derive(Clone, Debug, Default)]
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
    /// Play / Install by the game's state.
    context: bool,
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
    main: bool,
    stats: Stats,
    images: &'b mut BTreeMap<String, String>,
    tpl_triggers: HashMap<String, Vec<X<'t, 'i>>>,
    names: usize,
    /// The view's visual root (ElementName lookups).
    root: Option<X<'t, 'i>>,
    /// Flowing panels become `stack` containers (the placement tests turn this off).
    stacks: bool,
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
