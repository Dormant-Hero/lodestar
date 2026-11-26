//! The resolved resource scope of a Playnite theme: its XAML documents laid over Playnite's
//! default theme of the same mode (`defaults.rs`), as Playnite's theme loader does, with WPF's
//! lookup rules where they matter for reading a look:
//!
//! - dictionary-level resources by `x:Key` (inline MergedDictionaries included); a theme's
//!   definition wins over the default's, Constants.xaml over its other files;
//! - implicit styles (`<Style TargetType="{x:Type Main}">` without a key) per type name, the
//!   last one loaded, except that a variant of the type's own file never replaces it
//!   (`Views/GameDetails_Plex.xaml` next to `Views/GameDetails.xaml`: Playnite loads a theme's
//!   files by the default theme's file names, so a variant only counts when another file
//!   merges it in);
//! - `BasedOn="{StaticResource {x:Type Button}}"`: the implicit style defined before it;
//! - local scopes (`<ControlTemplate.Resources>`, `<Style.Resources>` …) before the global one;
//! - `StaticResource` / `DynamicResource` chains (markup and `<StaticResource ResourceKey>`
//!   elements) with a depth / loop guard; `BasedOn` style chains; `TemplateBinding` to the
//!   owning style's setters;
//! - typed values: numbers, booleans, strings, Thickness, CornerRadius, colours and brushes
//!   (every gradient stop, the direction, radial centre / radius, ImageBrush stretch / opacity).
//!
//! Nothing is executed and nothing is laid out: triggers and storyboards are only *read* by the
//! extractor. Limits: 4 MB per file, 400 files, 1.5 million XML nodes in all (Aniki ReMake has
//! 14 MB of XAML); localisation dictionaries are skipped.

use super::Mode;
use super::defaults;
use super::look::{Coverage, LookBrush, Stretch};
use super::source::Source;
use super::xaml::{self, attr, parse_color, parse_point};
use crate::theme::Color;
use roxmltree::{Document, Node, NodeId};
use std::borrow::Cow;
use std::collections::HashMap;

pub(crate) const FILE_LIMIT: u64 = 4 * 1024 * 1024;
pub(crate) const MAX_FILES: usize = 400;
pub(crate) const MAX_NODES: usize = 1_500_000;
const MAX_DEPTH: u8 = 16;

/// Where a document comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layer {
    Default,
    Theme,
}

pub(crate) struct XamlText {
    pub rel: String,
    pub text: Cow<'static, str>,
    pub layer: Layer,
}

/// The XAML texts of a theme and of the default theme underneath, in load order (a later
/// definition wins).
#[derive(Default)]
pub(crate) struct Texts {
    pub files: Vec<XamlText>,
    pub skipped: Vec<String>,
    pub mode: Option<Mode>,
}

fn ext(rel: &str) -> String {
    rel.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

fn base_name(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

/// Load order inside one layer: other files, then Media.xaml, then Constants.xaml (shallowest
/// last), so Constants wins as it does for Playnite's colour constants.
fn rank(rel: &str) -> (u8, std::cmp::Reverse<usize>) {
    let r = match base_name(rel).to_ascii_lowercase().as_str() {
        "constants.xaml" => 2,
        "media.xaml" => 1,
        _ => 0,
    };
    (r, std::cmp::Reverse(rel.matches('/').count()))
}

impl Texts {
    /// The default theme's files for `mode`, then the theme's (read from `source`).
    pub fn load(source: &Source, files: &[(String, u64)], mode: Mode) -> Texts {
        let mut theme = vec![];
        let mut skipped = vec![];
        let mut xaml: Vec<&(String, u64)> = files.iter().filter(|(f, _)| ext(f) == "xaml").collect();
        xaml.retain(|(f, _)| {
            let localisation = f.to_ascii_lowercase().starts_with("localization/");
            !localisation
        });
        for (i, (f, size)) in xaml.into_iter().enumerate() {
            if i >= MAX_FILES {
                skipped.push(format!("{f}: more than {MAX_FILES} XAML files"));
                continue;
            }
            if *size > FILE_LIMIT {
                skipped.push(format!("{f}: bigger than 4 MB"));
                continue;
            }
            match source.read_text(f, FILE_LIMIT) {
                Some(t) => theme.push((f.clone(), t)),
                None => skipped.push(format!("{f}: couldn't be read")),
            }
        }
        let mut t = Texts::with_theme(mode, theme);
        t.skipped.extend(skipped);
        t
    }

    /// The default theme's files plus the given theme files (tests, `load`).
    pub fn with_theme(mode: Mode, mut theme: Vec<(String, String)>) -> Texts {
        let mut files: Vec<XamlText> = defaults::files(mode)
            .map(|f| XamlText { rel: f.rel.to_string(), text: Cow::Borrowed(f.text), layer: Layer::Default })
            .collect();
        if mode == Mode::Fullscreen {
            files.push(XamlText {
                rel: "App.xaml".into(),
                text: fullscreen_app_resources().into(),
                layer: Layer::Default,
            });
        }
        files.sort_by_key(|f| rank(&f.rel));
        theme.sort_by_key(|(r, _)| rank(r));
        files.extend(theme.into_iter().map(|(rel, text)| XamlText {
            rel,
            text: Cow::Owned(text.trim_start_matches('\u{feff}').to_string()),
            layer: Layer::Theme,
        }));
        Texts { files, skipped: vec![], mode: Some(mode) }
    }
}

/// Resources Playnite's fullscreen app defines itself (its App.xaml, not a theme file) that
/// themes read: the size its views are designed for (`{StaticResource DesignWidth}` on a
/// full-screen overlay), which Playnite then scales to the screen.
fn fullscreen_app_resources() -> String {
    let [w, h] = super::layout::SCREEN;
    format!(
        r#"<ResourceDictionary xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" xmlns:sys="clr-namespace:System;assembly=mscorlib">
  <sys:Double x:Key="DesignWidth">{w}</sys:Double>
  <sys:Double x:Key="DesignHeight">{h}</sys:Double>
</ResourceDictionary>"#
    )
}

impl Texts {
    /// One more theme dictionary after all the others (it wins): the theme options' values.
    pub fn push_theme(&mut self, rel: &str, text: String) {
        self.files.push(XamlText { rel: rel.to_string(), text: Cow::Owned(text), layer: Layer::Theme });
    }
}

pub(crate) struct Doc<'i> {
    pub rel: &'i str,
    pub layer: Layer,
    pub doc: Document<'i>,
}

/// Where a resource lives.
#[derive(Clone, Copy, Debug)]
struct At {
    doc: usize,
    node: NodeId,
}

/// The resolved scope. Borrows the texts it was built from.
pub(crate) struct Tree<'i> {
    pub mode: Mode,
    pub docs: Vec<Doc<'i>>,
    keyed: HashMap<String, At>,
    /// The implicit style that applies, per type name.
    implicit: HashMap<String, At>,
    /// Every implicit style per type name, in load order (for `BasedOn` a type).
    implicit_all: HashMap<String, Vec<At>>,
    /// The theme's files (for `{ThemeFile …}` paths).
    pub files: Vec<(String, u64)>,
    pub coverage: Coverage,
    /// Fixed bindings being resolved inside each other (a property bound to another element's
    /// property whose style reads the first one again ends here).
    resolving: std::cell::Cell<u8>,
    /// A toggle's `IsChecked` set by the theme is its value (false: unknown, the user flips
    /// it; one bound to a setting stays known).
    pub toggles_at_rest: std::cell::Cell<bool>,
    /// The theme's ThemeOptions variables at their defaults (its options.yaml).
    pub add_on_options: super::settings::AddOnOptions,
    /// The Playnite settings the player chose for this theme (the rest read their defaults).
    pub settings: super::settings::PlayerSettings,
}

/// A parsed `{…}` markup extension (or a literal).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Markup<'s> {
    Lit(&'s str),
    Resource(String),
    Null,
    Settings(String),
    Binding(String),
    TemplateBinding(String),
    ThemeFile(String),
    /// `PluginSettings …` / `PluginStatus …` / `PluginConverter …` (with its keyword).
    Plugin(String),
    Type(String),
    Other(String),
}

pub(crate) fn markup(s: &str) -> Markup<'_> {
    let t = s.trim();
    if let Some(lit) = t.strip_prefix("{}") {
        return Markup::Lit(lit);
    }
    let Some(inner) = t.strip_prefix('{').and_then(|r| r.strip_suffix('}')) else {
        return Markup::Lit(t);
    };
    let inner = inner.trim();
    let (kind, rest) = inner.split_once(char::is_whitespace).unwrap_or((inner, ""));
    let rest = rest.trim();
    match kind {
        "DynamicResource" | "StaticResource" => {
            let k = rest.trim_start_matches("ResourceKey=").trim();
            Markup::Resource(k.to_string())
        }
        "x:Null" => Markup::Null,
        "Settings" => Markup::Settings(rest.trim_start_matches("Path=").to_string()),
        "Binding" | "MultiBinding" => Markup::Binding(rest.to_string()),
        "TemplateBinding" => Markup::TemplateBinding(rest.trim_start_matches("Property=").trim().to_string()),
        "ThemeFile" => Markup::ThemeFile(rest.trim_matches(|c| c == '\'' || c == '"').to_string()),
        "ThemeFileBinding" => Markup::Binding(rest.to_string()),
        "PluginSettings" | "PluginStatus" | "PluginConverter" => Markup::Plugin(inner.to_string()),
        "x:Type" => Markup::Type(local_type(rest).to_string()),
        _ => Markup::Other(inner.to_string()),
    }
}

/// `local:Main` / `{x:Type Main}` / `controls:GameListItem` → `Main` / `GameListItem`.
pub(crate) fn local_type(s: &str) -> &str {
    let s = s.trim();
    let s = s.strip_prefix("{x:Type").and_then(|r| r.strip_suffix('}')).map(str::trim).unwrap_or(s);
    s.rsplit(':').next().unwrap_or(s).trim()
}

/// The type a resource key names when it is a type (`{x:Type Button}`, the key of an implicit
/// style).
fn type_key(key: &str) -> Option<&str> {
    match markup(key) {
        Markup::Type(_) => Some(local_type(key)),
        _ => None,
    }
}

/// The document is the theme's file for type `ty` (`Views/GameDetails.xaml` for GameDetails).
fn file_is_for(doc: &Doc, ty: &str) -> bool {
    doc.layer == Layer::Theme && stem(doc.rel).eq_ignore_ascii_case(ty)
}

/// The document is named as a variant of the file for type `ty` (`GameDetails_Plex.xaml`,
/// `GameDetails-old.xaml`; not `ButtonEx.xaml` for Button, another type's file).
fn file_is_variant_of(doc: &Doc, ty: &str) -> bool {
    let s = stem(doc.rel);
    s.get(..ty.len()).is_some_and(|head| head.eq_ignore_ascii_case(ty))
        && s[ty.len()..].starts_with(['_', '-', ' ', '.'])
}

/// A file's name without folder and extension.
fn stem(rel: &str) -> &str {
    let name = base_name(rel);
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

/// A typed value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value<'t, 'i> {
    Num(f64),
    Bool(bool),
    Text(String),
    Color(Color),
    Brush(LookBrush),
    Thickness([f32; 4]),
    Corner([f32; 4]),
    Null,
    Setting(String),
    Binding(String),
    ThemeFile(String),
    Plugin(String),
    /// An element that is a value of its own (Style, ControlTemplate, Storyboard, an effect,
    /// VisualBrush …).
    Node(Node<'t, 'i>),
}

impl<'t, 'i> Value<'t, 'i> {
    pub fn num(&self) -> Option<f64> {
        match self {
            Value::Num(v) => Some(*v),
            Value::Text(t) => t.trim().parse().ok().filter(|v: &f64| v.is_finite()),
            Value::Bool(b) => Some(*b as u8 as f64),
            _ => None,
        }
    }

    pub fn boolean(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            Value::Text(t) => match t.trim().to_ascii_lowercase().as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn text(&self) -> Option<&str> {
        match self {
            Value::Text(t) => Some(t),
            _ => None,
        }
    }

    pub fn brush(&self) -> Option<LookBrush> {
        match self {
            Value::Brush(b) => Some(b.clone()),
            Value::Color(c) => Some(LookBrush::solid(*c)),
            Value::Text(t) => parse_color(t).map(LookBrush::solid),
            _ => None,
        }
    }

    pub fn color(&self) -> Option<Color> {
        match self {
            Value::Color(c) => Some(*c),
            Value::Brush(b) => b.average(),
            Value::Text(t) => parse_color(t),
            _ => None,
        }
    }

    /// Thickness / CornerRadius / a single number as 4 sides.
    pub fn sides(&self) -> Option<[f32; 4]> {
        match self {
            Value::Thickness(t) | Value::Corner(t) => Some(*t),
            Value::Num(v) => Some([*v as f32; 4]),
            Value::Text(t) => parse_sides(t),
            _ => None,
        }
    }

    /// The largest side of a Thickness / CornerRadius / number.
    pub fn max_side(&self) -> Option<f32> {
        self.sides().map(|s| s.iter().copied().fold(0.0f32, f32::max))
    }

    pub fn node(&self) -> Option<Node<'t, 'i>> {
        match self {
            Value::Node(n) => Some(*n),
            _ => None,
        }
    }
}

/// `"5"`, `"5,10"`, `"1,2,3,4"` (WPF Thickness / CornerRadius text).
pub(crate) fn parse_sides(s: &str) -> Option<[f32; 4]> {
    let v: Vec<f32> = s
        .split([',', ' '])
        .filter(|p| !p.trim().is_empty())
        .map(|p| p.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .ok()?;
    if v.iter().any(|x| !x.is_finite()) {
        return None;
    }
    match v.as_slice() {
        [a] => Some([*a; 4]),
        [h, w] => Some([*h, *w, *h, *w]),
        [a, b, c, d] => Some([*a, *b, *c, *d]),
        _ => None,
    }
}

const BRUSH_PROPS: [&str; 10] = [
    "Background",
    "Foreground",
    "BorderBrush",
    "Fill",
    "Stroke",
    "OpacityMask",
    "ImageOpacityMask",
    "SelectionBrush",
    "CaretBrush",
    "HighlightBrush",
];
const SIDE_PROPS: [&str; 4] = ["Margin", "Padding", "BorderThickness", "CornerRadius"];

/// A literal attribute value typed by the property it's for.
fn literal<'t, 'i>(prop: &str, s: &str) -> Value<'t, 'i> {
    let p = prop.rsplit('.').next().unwrap_or(prop);
    let t = s.trim();
    if BRUSH_PROPS.contains(&p) {
        if let Some(c) = parse_color(t) {
            return Value::Brush(LookBrush::solid(c));
        }
    } else if p.ends_with("Color") {
        if let Some(c) = parse_color(t) {
            return Value::Color(c);
        }
    } else if SIDE_PROPS.contains(&p)
        && let Some(v) = parse_sides(t)
    {
        return if p == "CornerRadius" { Value::Corner(v) } else { Value::Thickness(v) };
    }
    if let Ok(v) = t.parse::<f64>()
        && v.is_finite()
    {
        return Value::Num(v);
    }
    match t.to_ascii_lowercase().as_str() {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => Value::Text(t.to_string()),
    }
}

/// x:Name / Name of an element.
pub(crate) fn name_of<'a>(n: Node<'a, '_>) -> Option<&'a str> {
    n.attributes()
        .find(|a| a.name() == "Name" && (a.namespace().is_none() || a.namespace() == Some(XAML_NS)))
        .map(|a| a.value())
}

const XAML_NS: &str = "http://schemas.microsoft.com/winfx/2006/xaml";

/// The tag's local name.
pub(crate) fn tag<'a>(n: Node<'a, '_>) -> &'a str {
    n.tag_name().name()
}

/// The element children that are content (not property elements).
pub(crate) fn content<'t, 'i>(n: Node<'t, 'i>) -> impl Iterator<Item = Node<'t, 'i>> {
    n.children().filter(|c| c.is_element() && !tag(*c).contains('.'))
}

/// The property element `<X.prop>` of `n` (any owner type prefix).
/// A trigger's Setters: its own children, or inside `<X.Setters>` (both are valid XAML).
pub(crate) fn setters<'t, 'i>(trigger: Node<'t, 'i>) -> impl Iterator<Item = Node<'t, 'i>> {
    content(trigger).chain(prop_elem(trigger, "Setters").into_iter().flat_map(content)).filter(|c| tag(*c) == "Setter")
}

pub(crate) fn prop_elem<'t, 'i>(n: Node<'t, 'i>, prop: &str) -> Option<Node<'t, 'i>> {
    n.children().find(|c| {
        c.is_element()
            && tag(*c).split_once('.').is_some_and(|(owner, p)| p == prop && !owner.is_empty() && !prop.contains('.'))
    })
}

/// A theme file a XAML path points at: `{ThemeFile 'Images/bg.jpg'}`, `../Images/bg.jpg`,
/// `pack://siteoforigin:,,,/Themes/Fullscreen/X/Images/bg.jpg`, `Images\bg.jpg` … (matched
/// against the theme's file list, case-insensitively; a unique file name as the last resort).
pub(crate) fn theme_file(files: &[(String, u64)], raw: &str) -> Option<String> {
    let mut s = raw.trim().trim_matches(|c| c == '{' || c == '}').trim().to_string();
    if let Some(r) = s.strip_prefix("ThemeFile") {
        s = r.trim().trim_matches('\'').trim_matches('"').to_string();
    }
    if let Some((_, r)) = s.split_once(",,,") {
        s = r.to_string();
    }
    if let Some((_, r)) = s.split_once(";component/") {
        s = r.to_string();
    }
    let s = s.replace('\\', "/");
    let parts: Vec<&str> = s.split('/').filter(|c| !c.is_empty() && *c != "." && *c != "..").collect();
    if parts.is_empty() {
        return None;
    }
    let lower: Vec<String> = files.iter().map(|(f, _)| f.to_ascii_lowercase()).collect();
    for start in 0..parts.len() {
        let want = parts[start..].join("/").to_ascii_lowercase();
        if let Some(i) = lower.iter().position(|f| *f == want) {
            return Some(files[i].0.clone());
        }
    }
    let name = parts.last()?.to_ascii_lowercase();
    let mut by_name = files.iter().filter(|(f, _)| base_name(f).eq_ignore_ascii_case(&name));
    let first = by_name.next()?;
    by_name.next().is_none().then(|| first.0.clone())
}

impl<'i> Tree<'i> {
    /// Parse every text (broken files are reported and skipped) and index the scope.
    pub fn build(texts: &'i Texts, files: &[(String, u64)]) -> Tree<'i> {
        let mode = texts.mode.unwrap_or_default();
        let mut cov = Coverage { skipped: texts.skipped.clone(), ..Default::default() };
        let opts =
            || roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: MAX_NODES as u32, ..Default::default() };
        let mut docs: Vec<Doc<'i>> = vec![];
        let mut nodes = 0usize;
        for t in &texts.files {
            if nodes >= MAX_NODES {
                cov.skipped.push(format!("{}: over the size limit for one theme", t.rel));
                continue;
            }
            match Document::parse_with_options(&t.text, opts()) {
                Ok(doc) => {
                    let n = doc.descendants().count();
                    if nodes + n > MAX_NODES && t.layer == Layer::Theme {
                        cov.skipped.push(format!("{}: over the size limit for one theme", t.rel));
                        continue;
                    }
                    nodes += n;
                    docs.push(Doc { rel: &t.rel, layer: t.layer, doc });
                }
                Err(e) => {
                    log::info!("playnite theme: {}: {e}", t.rel);
                    if t.layer == Layer::Theme {
                        cov.broken.push(t.rel.clone());
                    }
                }
            }
        }
        let mut keyed = HashMap::new();
        let mut implicit: HashMap<String, At> = HashMap::new();
        let mut implicit_all: HashMap<String, Vec<At>> = HashMap::new();
        for (i, d) in docs.iter().enumerate() {
            match d.layer {
                Layer::Default => cov.default_files += 1,
                Layer::Theme => cov.theme_files += 1,
            }
            for n in xaml::dictionary_children(&d.doc) {
                let at = At { doc: i, node: n.id() };
                if let Some(k) = xaml::key_of(n) {
                    keyed.insert(k, at);
                } else if tag(n) == "Style"
                    && let Some(tt) = attr(n, "TargetType")
                {
                    let ty = local_type(tt);
                    let own_file = |at: &At| docs.get(at.doc).is_some_and(|d| file_is_for(d, ty));
                    if implicit.get(ty).is_none_or(|old| !own_file(old) || !file_is_variant_of(d, ty)) {
                        implicit.insert(ty.to_string(), at);
                    }
                    implicit_all.entry(ty.to_string()).or_default().push(at);
                }
            }
            if d.layer == Layer::Theme {
                for n in d.doc.descendants().filter(|n| n.is_element()) {
                    match tag(n) {
                        "ControlTemplate" | "DataTemplate" => cov.templates += 1,
                        "Trigger" | "EventTrigger" => cov.triggers += 1,
                        "DataTrigger" | "MultiDataTrigger" | "MultiTrigger" => cov.data_triggers += 1,
                        "Storyboard" => cov.storyboards += 1,
                        "MediaElement" => cov.media_elements += 1,
                        "VisualBrush" => cov.visual_brushes += 1,
                        "DropShadowEffect" | "BlurEffect" => cov.effects += 1,
                        t if t.ends_with(".OpacityMask") || t.ends_with(".ImageOpacityMask") => cov.opacity_masks += 1,
                        _ => {}
                    }
                    if attr(n, "OpacityMask").is_some() {
                        cov.opacity_masks += 1;
                    }
                }
            }
        }
        cov.nodes = nodes;
        cov.keyed_resources = keyed.len();
        cov.implicit_styles = implicit.len();
        Tree {
            mode,
            docs,
            keyed,
            implicit,
            implicit_all,
            files: files.to_vec(),
            coverage: cov,
            resolving: std::cell::Cell::new(0),
            toggles_at_rest: std::cell::Cell::new(true),
            add_on_options: Default::default(),
            settings: Default::default(),
        }
    }

    fn node(&self, at: At) -> Option<Node<'_, 'i>> {
        self.docs.get(at.doc)?.doc.get_node(at.node)
    }

    /// Which document a node is in.
    pub fn doc_of(&self, n: Node) -> Option<&Doc<'i>> {
        let d = n.document() as *const Document;
        self.docs.iter().find(|x| std::ptr::eq(&x.doc as *const Document, d))
    }

    pub fn layer_of(&self, n: Node) -> Layer {
        self.doc_of(n).map(|d| d.layer).unwrap_or(Layer::Default)
    }

    /// A dictionary-level resource by key.
    pub fn resource(&self, key: &str) -> Option<Node<'_, 'i>> {
        self.node(*self.keyed.get(key)?)
    }

    /// Whether the theme itself (not the default underneath) defines the key.
    pub fn theme_defines(&self, key: &str) -> bool {
        self.keyed.get(key).is_some_and(|at| self.docs.get(at.doc).is_some_and(|d| d.layer == Layer::Theme))
    }

    /// The implicit style for a type name (`Main`, `GameDetails`, `TextBlock` …).
    pub fn implicit_style(&self, type_name: &str) -> Option<Node<'_, 'i>> {
        self.node(*self.implicit.get(local_type(type_name))?)
    }

    /// The implicit style for a type name defined before `style` (what a StaticResource to
    /// `{x:Type T}` finds while `style` is being loaded).
    fn implicit_before<'t>(&'t self, type_name: &str, style: Node<'t, 'i>) -> Option<Node<'t, 'i>> {
        let doc = self.docs.iter().position(|d| std::ptr::eq(&d.doc, style.document()))?;
        let before = |at: &&At| at.doc < doc || (at.doc == doc && at.node.get() < style.id().get());
        self.node(*self.implicit_all.get(local_type(type_name))?.iter().rev().find(before)?)
    }

    /// A resource as seen from `at`: local `<X.Resources>` scopes up the tree first.
    pub fn resource_from<'t>(&'t self, at: Node<'t, 'i>, key: &str) -> Option<Node<'t, 'i>> {
        for a in at.ancestors().filter(|a| a.is_element()) {
            if let Some(res) = prop_elem(a, "Resources") {
                let mut holders = vec![res];
                holders.extend(res.children().filter(|c| c.is_element() && tag(*c) == "ResourceDictionary"));
                for h in holders {
                    if let Some(n) = content(h).filter(|c| xaml::key_of(*c).as_deref() == Some(key)).last() {
                        return Some(n);
                    }
                }
            }
            if a.parent().is_none_or(|p| p.is_root()) {
                break;
            }
        }
        self.resource(key).or_else(|| self.implicit_style(type_key(key)?))
    }

    /// The raw value of a property set on the element itself: an attribute or the content of
    /// a property element.
    pub fn local<'t>(&'t self, el: Node<'t, 'i>, prop: &str) -> Option<Value<'t, 'i>> {
        self.local_depth(el, prop, 0)
    }

    fn local_depth<'t>(&'t self, el: Node<'t, 'i>, prop: &str, depth: u8) -> Option<Value<'t, 'i>> {
        // Set twice (an attribute and a property element, which themes do with `Style`): the
        // property element is read last and wins.
        if let Some(pe) = prop_elem(el, prop) {
            match content(pe).next() {
                Some(child) => return self.eval_node(child, depth),
                None => {
                    if let Some(t) = pe.text().filter(|t| !t.trim().is_empty()) {
                        return Some(literal(prop, t));
                    }
                }
            }
        }
        let a = el.attributes().find(|a| a.name() == prop && a.namespace().is_none())?;
        self.eval_str(el, prop, a.value(), depth)
    }

    /// A property's value: set on the element, else from its style (explicit, else implicit
    /// for its type), following BasedOn. `TemplateBinding`s go to the owning style.
    pub fn get<'t>(&'t self, el: Node<'t, 'i>, prop: &str) -> Option<Value<'t, 'i>> {
        let v = self.local(el, prop).or_else(|| {
            let style = self.style_of(el)?;
            self.static_trigger_setter(el, style, prop).or_else(|| self.setter(style, prop))
        })?;
        if let Value::Binding(b) = &v
            && let Some(p) = b.strip_prefix("TemplateBinding:")
        {
            return self.template_owner_setter(el, p);
        }
        Some(v)
    }

    /// `TemplateBinding X`: the setter X of the style whose template holds `el`.
    fn template_owner_setter<'t>(&'t self, el: Node<'t, 'i>, prop: &str) -> Option<Value<'t, 'i>> {
        let tpl = el.ancestors().find(|a| a.is_element() && tag(*a) == "ControlTemplate")?;
        let style = tpl.ancestors().find(|a| a.is_element() && tag(*a) == "Style")?;
        self.setter(style, prop)
    }

    /// The style applied to an element: its `Style` property, else the implicit style of its
    /// type (never a style for itself: a Style element has none).
    pub fn style_of<'t>(&'t self, el: Node<'t, 'i>) -> Option<Node<'t, 'i>> {
        if tag(el) == "Style" || tag(el) == "Setter" {
            return None;
        }
        if let Some(Value::Node(s)) = self.local(el, "Style")
            && tag(s) == "Style"
        {
            return Some(s);
        }
        self.implicit_style(tag(el))
    }

    /// A style's setter for `prop` (its own setters, then BasedOn's).
    pub fn setter<'t>(&'t self, style: Node<'t, 'i>, prop: &str) -> Option<Value<'t, 'i>> {
        let mut s = style;
        for _ in 0..MAX_DEPTH {
            for set in content(s).filter(|c| tag(*c) == "Setter") {
                if attr(set, "TargetName").is_some() {
                    continue;
                }
                let Some(p) = attr(set, "Property") else { continue };
                let p = p.trim();
                if p == prop || (!prop.contains('.') && p.rsplit('.').next() == Some(prop)) {
                    return self.setter_value(set, prop);
                }
            }
            s = self.based_on(s)?;
        }
        None
    }

    /// A Setter's value (attribute or `<Setter.Value>` element).
    pub fn setter_value<'t>(&'t self, set: Node<'t, 'i>, prop: &str) -> Option<Value<'t, 'i>> {
        if let Some(v) = attr(set, "Value") {
            return self.eval_str(set, prop, v, 0);
        }
        let pe = prop_elem(set, "Value")?;
        match content(pe).next() {
            Some(c) => self.eval_node(c, 0),
            None => pe.text().map(|t| literal(prop, t)),
        }
    }

    /// The style a style is BasedOn.
    pub fn based_on<'t>(&'t self, style: Node<'t, 'i>) -> Option<Node<'t, 'i>> {
        let b = attr(style, "BasedOn")?;
        let n = match markup(b) {
            Markup::Resource(k) => match type_key(&k) {
                Some(ty) => self.implicit_before(ty, style)?,
                None => self.resource_from(style, &k)?,
            },
            Markup::Type(t) => self.implicit_style(&t)?,
            _ => return None,
        };
        (tag(n) == "Style" && n != style).then_some(n)
    }

    /// The ControlTemplate a style sets (following BasedOn).
    pub fn template<'t>(&'t self, style: Node<'t, 'i>) -> Option<Node<'t, 'i>> {
        match self.setter(style, "Template")? {
            Value::Node(n) if tag(n) == "ControlTemplate" => Some(n),
            _ => None,
        }
    }

    /// The template of a keyed or implicit style.
    pub fn template_of(&self, key_or_type: &str) -> Option<Node<'_, 'i>> {
        let style =
            self.resource(key_or_type).filter(|n| tag(*n) == "Style").or_else(|| self.implicit_style(key_or_type))?;
        self.template(style)
    }

    /// Evaluate an attribute / setter string as seen from `at`.
    pub fn eval_str<'t>(&'t self, at: Node<'t, 'i>, prop: &str, s: &str, depth: u8) -> Option<Value<'t, 'i>> {
        if depth > MAX_DEPTH {
            return None;
        }
        Some(match markup(s) {
            Markup::Lit(l) => literal(prop, l),
            Markup::Resource(k) => {
                let n = self.resource_from(at, &k)?;
                return self.eval_node(n, depth + 1);
            }
            Markup::Null => Value::Null,
            Markup::Settings(x) => Value::Setting(x),
            Markup::Binding(b) => Value::Binding(b),
            Markup::TemplateBinding(p) => Value::Binding(format!("TemplateBinding:{p}")),
            Markup::ThemeFile(f) => Value::ThemeFile(f),
            Markup::Plugin(p) => Value::Plugin(p),
            Markup::Type(t) => Value::Text(t),
            Markup::Other(o) => Value::Text(format!("{{{o}}}")),
        })
    }

    /// The value an element stands for (a resource definition or a property element's child).
    pub fn eval_node<'t>(&'t self, n: Node<'t, 'i>, depth: u8) -> Option<Value<'t, 'i>> {
        if depth > MAX_DEPTH {
            return None;
        }
        let text = || n.text().unwrap_or("").trim();
        let num = |p: &str| -> Option<f32> {
            self.local_depth(n, p, depth + 1).and_then(|v| v.num()).map(|v| v as f32).filter(|v| v.is_finite())
        };
        Some(match tag(n) {
            "StaticResource" | "DynamicResource" => {
                let k = attr(n, "ResourceKey")?;
                let target = self.resource_from(n, k.trim())?;
                if target == n {
                    return None;
                }
                return self.eval_node(target, depth + 1);
            }
            "Color" => match self.eval_str(n, "Color", text(), depth + 1)? {
                Value::Color(c) => Value::Color(c),
                other => Value::Color(other.color()?),
            },
            "SolidColorBrush" => {
                let c = self.local_depth(n, "Color", depth + 1)?.color()?;
                let o = num("Opacity").unwrap_or(1.0).clamp(0.0, 1.0);
                Value::Brush(LookBrush::solid(scale_alpha(c, o)))
            }
            name @ ("LinearGradientBrush" | "RadialGradientBrush") => {
                let o = num("Opacity").unwrap_or(1.0).clamp(0.0, 1.0);
                let mut stops: Vec<(Color, f32)> = n
                    .descendants()
                    .filter(|s| s.is_element() && tag(*s) == "GradientStop")
                    .filter_map(|s| {
                        let c = self.local_depth(s, "Color", depth + 1)?.color()?;
                        let off = self.local_depth(s, "Offset", depth + 1).and_then(|v| v.num()).unwrap_or(0.0) as f32;
                        Some((scale_alpha(c, o), off))
                    })
                    .collect();
                if stops.is_empty() {
                    return None;
                }
                stops.sort_by(|a, b| a.1.total_cmp(&b.1));
                let g = xaml::Geom::of(n);
                Value::Brush(if name.starts_with("Radial") {
                    LookBrush::Radial { stops, center: g.center, origin: g.origin, radius: g.radius }
                } else {
                    LookBrush::Linear { stops, start: g.start, end: g.end, absolute: g.absolute }
                })
            }
            "ImageBrush" | "BitmapImage" => {
                let src_prop = if tag(n) == "ImageBrush" { "ImageSource" } else { "UriSource" };
                let source = match self.local_depth(n, src_prop, depth + 1) {
                    Some(Value::ThemeFile(f)) => format!("{{ThemeFile '{f}'}}"),
                    Some(Value::Text(t)) => t,
                    Some(Value::Brush(LookBrush::Image { source, .. })) => source,
                    Some(Value::Binding(b)) => format!("{{Binding {b}}}"),
                    _ => return Some(Value::Node(n)),
                };
                let stretch = attr(n, "Stretch").and_then(Stretch::parse).unwrap_or(Stretch::Fill);
                Value::Brush(LookBrush::Image {
                    file: theme_file(&self.files, &source),
                    source,
                    stretch,
                    opacity: num("Opacity").unwrap_or(1.0).clamp(0.0, 1.0),
                })
            }
            "Double" | "Single" | "Int32" | "Int64" => Value::Num(text().parse().ok().filter(|v: &f64| v.is_finite())?),
            "Boolean" => Value::Bool(text().eq_ignore_ascii_case("true")),
            "String" | "FontFamily" | "FontWeight" | "GridLength" | "Visibility" => Value::Text(text().to_string()),
            "Thickness" => Value::Thickness(parse_sides(text())?),
            "CornerRadius" => Value::Corner(parse_sides(text())?),
            "Point" => {
                let p = parse_point(text())?;
                Value::Thickness([p[0], p[1], p[0], p[1]])
            }
            _ => Value::Node(n),
        })
    }

    /// Element descendants of `scope` (itself included) named `name`.
    pub fn named<'t>(&'t self, scope: Node<'t, 'i>, name: &str) -> Option<Node<'t, 'i>> {
        scope.descendants().find(|n| n.is_element() && name_of(*n) == Some(name))
    }

    /// The first content element of a ControlTemplate (its visual root).
    pub fn template_root<'t>(&'t self, tpl: Node<'t, 'i>) -> Option<Node<'t, 'i>> {
        content(tpl).next()
    }

    /// The Storyboards a trigger starts (`<BeginStoryboard Storyboard="{StaticResource X}">`
    /// or an inline Storyboard) on enter.
    pub fn enter_storyboards<'t>(&'t self, trigger: Node<'t, 'i>) -> Vec<Node<'t, 'i>> {
        let mut out = vec![];
        let holders: Vec<Node> = match tag(trigger) {
            "EventTrigger" => prop_elem(trigger, "Actions").into_iter().chain([trigger]).collect(),
            _ => prop_elem(trigger, "EnterActions").into_iter().collect(),
        };
        for h in holders {
            for b in h.children().filter(|c| c.is_element() && tag(*c) == "BeginStoryboard") {
                if let Some(Value::Node(sb)) = self.local(b, "Storyboard") {
                    out.push(sb);
                } else if let Some(sb) = content(b).find(|c| tag(*c) == "Storyboard") {
                    out.push(sb);
                }
            }
        }
        out.retain(|s| tag(*s) == "Storyboard");
        out
    }

    /// Triggers of a ControlTemplate or Style (`X.Triggers`, BasedOn styles' included).
    pub fn triggers<'t>(&'t self, owner: Node<'t, 'i>) -> Vec<Node<'t, 'i>> {
        let mut out = vec![];
        let mut o = Some(owner);
        for _ in 0..MAX_DEPTH {
            let Some(cur) = o else { break };
            if let Some(t) = prop_elem(cur, "Triggers") {
                out.extend(content(t));
            }
            o = if tag(cur) == "Style" { self.based_on(cur) } else { None };
        }
        out
    }

    /// Every theme-layer element (for plugin / media scans), with its document.
    pub fn theme_elements(&self) -> impl Iterator<Item = (&Doc<'i>, Node<'_, 'i>)> {
        self.docs
            .iter()
            .filter(|d| d.layer == Layer::Theme)
            .flat_map(|d| d.doc.descendants().filter(|n| n.is_element()).map(move |n| (d, n)))
    }
}

/// Style triggers whose condition is fixed by the theme itself (its constants, which theme
/// options change), evaluated the way WPF would: `DataTrigger` / `MultiDataTrigger` bound to
/// `{Binding Source={StaticResource K}}`, to the element's own `Tag` (`RelativeSource Self`) or
/// to a named element's `Tag` in the same template, when that Tag is a resource or a literal.
/// Triggers on anything that changes while Playnite runs (the game, focus, add-ons) are left
/// to the extractor / compiler.
impl<'i> Tree<'i> {
    /// The value a fixed trigger of `style` sets for `prop` on `el` (the last true one wins,
    /// a derived style's before its BasedOn's).
    pub fn static_trigger_setter<'t>(
        &'t self,
        el: Node<'t, 'i>,
        style: Node<'t, 'i>,
        prop: &str,
    ) -> Option<Value<'t, 'i>> {
        let mut s = Some(style);
        for _ in 0..MAX_DEPTH {
            let cur = s?;
            if let Some(t) = prop_elem(cur, "Triggers") {
                let mut hit = None;
                for trig in content(t) {
                    let setters: Vec<Node> = setters(trig)
                        .filter(|c| attr(*c, "TargetName").is_none())
                        .filter(|set| {
                            attr(*set, "Property").map(str::trim).is_some_and(|p| {
                                p == prop || (!prop.contains('.') && p.rsplit('.').next() == Some(prop))
                            })
                        })
                        .collect();
                    if setters.is_empty() {
                        continue;
                    }
                    let ok = self.static_trigger(el, trig);
                    if ok != Some(true) {
                        continue;
                    }
                    for set in setters {
                        hit = self.setter_value(set, prop).or(hit);
                    }
                }
                if hit.is_some() {
                    return hit;
                }
            }
            s = self.based_on(cur);
        }
        None
    }

    /// Whether a DataTrigger / MultiDataTrigger holds by what the theme fixes (None: it
    /// depends on something that changes at runtime). A MultiDataTrigger is false as soon as
    /// one condition is, unknown when one can't be told.
    pub(crate) fn static_trigger(&self, el: Node, trig: Node) -> Option<bool> {
        match tag(trig) {
            "DataTrigger" => self.static_condition(el, trig),
            "MultiDataTrigger" => prop_elem(trig, "Conditions").and_then(|c| {
                let mut known = true;
                for cond in content(c).filter(|c| tag(*c) == "Condition") {
                    match self.static_condition(el, cond) {
                        Some(false) => return Some(false),
                        Some(true) => {}
                        None => known = false,
                    }
                }
                known.then_some(true)
            }),
            _ => None,
        }
    }

    /// A trigger / condition's `Binding` against its `Value`: Some(true / false) when the
    /// theme fixes it, None when it depends on something that changes at runtime.
    pub(crate) fn static_condition(&self, el: Node, cond: Node) -> Option<bool> {
        let binding = attr(cond, "Binding")?;
        let want = attr(cond, "Value")?.trim();
        let got = self.static_binding(el, binding)?;
        Some(same_value(&got, want))
    }

    /// The text a fixed `{Binding …}` gives (see above), through its Converter: a constant the
    /// theme keeps in a resource or a Tag, a `{Settings …}` at Playnite's default, or a property
    /// of another named element (`ElementName=X, Path=P`) that is set to one of those — the
    /// element's resting value (a ToggleButton's `IsChecked`, a Border's `Opacity` …). None
    /// when it depends on something that changes at runtime (focus, the selection, the list).
    pub(crate) fn static_binding(&self, el: Node, binding: &str) -> Option<String> {
        // Deep enough for a setting held by a toggle that another element's trigger reads.
        const MAX_NESTED: u8 = 4;
        let depth = self.resolving.get();
        if depth >= MAX_NESTED {
            return None;
        }
        self.resolving.set(depth + 1);
        let out = self.resolve_binding(el, binding);
        self.resolving.set(depth);
        out
    }

    fn resolve_binding(&self, el: Node, binding: &str) -> Option<String> {
        let inner = binding.trim().strip_prefix('{')?.strip_suffix('}')?.trim();
        match markup(binding) {
            Markup::Settings(s) => return self.settings.get(&s).map(|v| v.text()),
            Markup::Plugin(p) => return super::settings::plugin_value(&p, &self.add_on_options),
            _ => {}
        }
        let rest = inner.strip_prefix("Binding")?.trim();
        let parts = split_top(rest);
        let mut path = String::new();
        let (mut source, mut relative_self, mut element, mut converter, mut param) = (None, false, None, None, None);
        for (i, p) in parts.iter().enumerate() {
            let p = p.trim();
            match p.split_once('=') {
                Some((k, v)) if !k.contains('{') => {
                    let (k, v) = (k.trim(), v.trim());
                    match k {
                        "Path" => path = v.to_string(),
                        "Source" => source = Some(v.to_string()),
                        "RelativeSource" => relative_self = v.contains("Self"),
                        "ElementName" => element = Some(v.to_string()),
                        "Converter" => converter = Some(v.to_string()),
                        "ConverterParameter" => param = Some(v.trim_matches('\'').to_string()),
                        "FallbackValue" | "Mode" | "TargetNullValue" | "StringFormat" | "UpdateSourceTrigger" => {}
                        _ => return None,
                    }
                }
                _ if i == 0 => path = p.to_string(),
                _ => return None,
            }
        }
        let text_of = |v: Value| -> Option<String> {
            match v {
                Value::Bool(b) => Some(if b { "True".into() } else { "False".into() }),
                Value::Num(n) => Some(n.to_string()),
                Value::Text(t) => Some(t),
                Value::Null => Some(String::new()),
                _ => None,
            }
        };
        let tag_of = |n: Node| -> Option<String> {
            match attr(n, "Tag") {
                Some(t) => match markup(t) {
                    Markup::Resource(k) => {
                        let r = self.resource_from(n, &k)?;
                        text_of(self.eval_node(r, 1)?)
                    }
                    Markup::Lit(l) => Some(l.to_string()),
                    _ => None,
                },
                None => None,
            }
        };
        let raw = if let Some(src) = source {
            let Markup::Resource(k) = markup(&src) else { return None };
            if !path.is_empty() {
                return None;
            }
            let r = self.resource_from(el, &k)?;
            text_of(self.eval_node(r, 1)?)?
        } else if path == "Tag" && relative_self {
            tag_of(el)?
        } else {
            let name = element?;
            let scope = el
                .ancestors()
                .find(|a| a.is_element() && matches!(tag(*a), "ControlTemplate" | "DataTemplate"))
                .or_else(|| el.ancestors().last())?;
            let owner = self.named(scope, &name)?;
            if path == "Tag" { tag_of(owner)? } else { self.resting_value(owner, &path)? }
        };
        match converter {
            None => Some(raw),
            Some(c) => super::convert::apply(&c, &raw, param.as_deref()),
        }
    }

    /// One of the element's own triggers animates `prop` on it (its value then changes at
    /// runtime: a banner fading in once the list shows, say).
    fn animated(&self, el: Node, prop: &str) -> bool {
        let own = name_of(el);
        let mut trigs = self.triggers(el);
        if let Some(s) = self.style_of(el) {
            trigs.extend(self.triggers(s));
        }
        trigs.into_iter().flat_map(|t| self.enter_storyboards(t)).any(|sb| {
            sb.descendants().filter(|a| a.is_element() && tag(*a).contains("Animation")).any(|a| {
                let target = attr(a, "Storyboard.TargetName").map(str::trim);
                (target.is_none() || target == own)
                    && attr(a, "Storyboard.TargetProperty")
                        .is_some_and(|p| p.trim().trim_matches(['(', ')']).rsplit('.').next() == Some(prop))
            })
        })
    }

    /// The value a named element's property rests at, as a binding's raw string: set to a
    /// literal, a resource, a `{Settings …}` or a fixed binding (on the element or by its
    /// style). Properties that only change at runtime (IsFocused, SelectedIndex, ActualWidth,
    /// Items.Count …) aren't set by the theme, so they give None.
    fn resting_value(&self, owner: Node, path: &str) -> Option<String> {
        if path.is_empty() || !path.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        let value = self.get(owner, path)?;
        if path == "IsChecked" && !self.toggles_at_rest.get() && !matches!(value, Value::Setting(_)) {
            return None;
        }
        if self.animated(owner, path) {
            return None;
        }
        match value {
            Value::Bool(b) => Some(if b { "True".into() } else { "False".into() }),
            Value::Num(n) => Some(n.to_string()),
            Value::Text(t) => Some(t),
            Value::Null => Some(String::new()),
            Value::Setting(s) => self.settings.get(&s).map(|v| v.text()),
            Value::Binding(b) if !b.starts_with("TemplateBinding:") => {
                self.static_binding(owner, &format!("{{Binding {b}}}"))
            }
            Value::Plugin(p) => super::settings::plugin_value(&p, &self.add_on_options),
            _ => None,
        }
    }
}

/// A trigger's `Value` matches what its binding gives (WPF compares after converting the
/// Value to the property's type: "True" = true, "0.5" = 0.50, "Visible" = Visibility.Visible).
pub(crate) fn same_value(got: &str, want: &str) -> bool {
    let (a, b) = (got.trim(), want.trim());
    let truth = |s: &str| match s.to_ascii_lowercase().as_str() {
        "true" | "visible" => Some(true),
        "false" | "collapsed" | "hidden" => Some(false),
        _ => None,
    };
    let null = |s: &str| s.is_empty() || s == "{x:Null}";
    a.eq_ignore_ascii_case(b)
        || null(a) && null(b)
        || matches!((a.parse::<f64>(), b.parse::<f64>()), (Ok(x), Ok(y)) if x == y)
        || matches!((truth(a), truth(b)), (Some(x), Some(y)) if x == y && a.parse::<f64>().is_err())
}

/// Split `a, b={X c, d}, e` at the commas outside braces.
fn split_top(s: &str) -> Vec<String> {
    let mut out = vec![];
    let (mut depth, mut cur) = (0i32, String::new());
    for ch in s.chars() {
        match ch {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(ch);
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

fn scale_alpha(c: Color, o: f32) -> Color {
    let [r, g, b, a] = c.0;
    Color([r, g, b, (a as f32 * o).round() as u8])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const NS: &str = r#"xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" xmlns:sys="clr-namespace:System;assembly=mscorlib""#;

    pub(crate) fn dict(body: &str) -> String {
        format!("<ResourceDictionary {NS}>\n{body}\n</ResourceDictionary>")
    }

    fn texts(mode: Mode, files: &[(&str, String)]) -> Texts {
        Texts::with_theme(mode, files.iter().map(|(a, b)| (a.to_string(), b.clone())).collect())
    }

    #[test]
    fn markup_extensions_parse() {
        assert_eq!(markup("{DynamicResource TextBrush}"), Markup::Resource("TextBrush".into()));
        assert_eq!(markup("{StaticResource ResourceKey=A}"), Markup::Resource("A".into()));
        assert_eq!(markup("{x:Null}"), Markup::Null);
        assert_eq!(
            markup("{Settings Fullscreen.ShowGameTitles}"),
            Markup::Settings("Fullscreen.ShowGameTitles".into())
        );
        assert_eq!(markup("{ThemeFile 'Images/bg.jpg'}"), Markup::ThemeFile("Images/bg.jpg".into()));
        assert_eq!(markup("{x:Type local:Main}"), Markup::Type("Main".into()));
        assert_eq!(markup("{}{0} games"), Markup::Lit("{0} games"));
        assert!(matches!(markup("{PluginSettings Plugin=ThemeExtras, Path=X}"), Markup::Plugin(_)));
        assert_eq!(parse_sides("1,2"), Some([1.0, 2.0, 1.0, 2.0]));
        assert_eq!(parse_sides("-5"), Some([-5.0; 4]));
        assert_eq!(local_type("{x:Type GameListItem}"), "GameListItem");
    }

    #[test]
    fn fixed_bindings_follow_other_elements() {
        let view = dict(
            r##"<Grid x:Key="View">
              <CheckBox x:Name="Layout" IsChecked="{Settings Fullscreen.HorizontalLayout}"/>
              <CheckBox x:Name="Flag" IsChecked="True"/>
              <Border x:Name="Shade" Opacity="0"/>
              <Border x:Name="Banner" Opacity="0">
                <Border.Style><Style TargetType="Border"><Style.Triggers>
                  <EventTrigger RoutedEvent="Loaded"><BeginStoryboard><Storyboard>
                    <DoubleAnimation Storyboard.TargetProperty="Opacity" To="1" Duration="0:0:1"/>
                  </Storyboard></BeginStoryboard></EventTrigger>
                </Style.Triggers></Style></Border.Style>
              </Border>
              <TextBlock x:Name="Probe" Style="{DynamicResource Missing}">
                <TextBlock.Style><Style TargetType="TextBlock"><Setter Property="FontSize" Value="40"/></Style></TextBlock.Style>
              </TextBlock>
            </Grid>"##,
        );
        let t = texts(Mode::Fullscreen, &[("Views/Main.xaml", view)]);
        let tree = Tree::build(&t, &[]);
        let root = tree.resource("View").unwrap();
        let probe = tree.named(root, "Probe").unwrap();
        let fixed = |b: &str| tree.static_binding(probe, b);
        assert_eq!(fixed("{Binding IsChecked, ElementName=Layout}").as_deref(), Some("True"));
        assert_eq!(
            fixed("{Binding ElementName=Flag, Path=IsChecked, Converter={StaticResource InvertedBooleanToVisibilityConverter}}").as_deref(),
            Some("Collapsed")
        );
        assert_eq!(fixed("{Binding Opacity, ElementName=Shade}").as_deref(), Some("0"));
        assert_eq!(fixed("{Settings Fullscreen.Columns}").as_deref(), Some("7"));
        // Animated at runtime, not part of the view, or never set: unknown.
        assert_eq!(fixed("{Binding Opacity, ElementName=Banner}"), None);
        assert_eq!(fixed("{Binding IsChecked, ElementName=Nowhere}"), None);
        assert_eq!(fixed("{Binding IsFocused, ElementName=Flag}"), None);
        // A toggle the user flips, when asked to leave toggles open; a setting stays known.
        tree.toggles_at_rest.set(false);
        assert_eq!(fixed("{Binding IsChecked, ElementName=Flag}"), None);
        assert_eq!(fixed("{Binding IsChecked, ElementName=Layout}").as_deref(), Some("True"));
        tree.toggles_at_rest.set(true);
        // A Style set both as an attribute and as a property element: the element wins.
        let style = tree.style_of(probe).unwrap();
        assert_eq!(tree.setter(style, "FontSize").and_then(|v| v.num()), Some(40.0));
    }

    #[test]
    fn theme_wins_over_defaults_and_constants_over_other_files() {
        let t = texts(
            Mode::Fullscreen,
            &[
                ("Constants.xaml", dict(r##"<Color x:Key="GlyphColor">#FF00FF00</Color>"##)),
                ("Views/Other.xaml", dict(r##"<Color x:Key="GlyphColor">#FF0000FF</Color>"##)),
            ],
        );
        let tree = Tree::build(&t, &[]);
        let n = tree.resource("GlyphColor").unwrap();
        assert_eq!(tree.eval_node(n, 0).unwrap().color(), Some(Color([0, 255, 0, 255])));
        assert!(tree.theme_defines("GlyphColor"));
        // Everything else comes from Playnite's default (its SelectionBrush uses GlyphColor).
        assert!(!tree.theme_defines("SelectionBrush"));
        let sel = tree.eval_node(tree.resource("SelectionBrush").unwrap(), 0).unwrap();
        let Value::Brush(LookBrush::Linear { stops, start, end, .. }) = sel else { panic!("{sel:?}") };
        assert_eq!(stops[0].0, Color([0, 255, 0, 255]));
        assert_eq!(start, [0.5, -0.5]);
        assert_eq!(end, [1.0, 1.5]);
        assert!(tree.implicit_style("Main").is_some());
        assert_eq!(tree.coverage.theme_files, 2);
        assert!(tree.coverage.default_files > 10);
    }

    #[test]
    fn brushes_keep_stops_geometry_aliases_and_loops_end() {
        let t = texts(
            Mode::Desktop,
            &[(
                "Constants.xaml",
                dict(
                    r##"<Color x:Key="A">#80FF0000</Color>
                    <RadialGradientBrush x:Key="R" Center="0.5,0.2" GradientOrigin="0.5,0.1" RadiusX="0.6" RadiusY="0.8" Opacity="0.5">
                      <RadialGradientBrush.GradientStops>
                        <GradientStop Color="{DynamicResource A}" Offset="1"/>
                        <GradientStop Color="#FF000000" Offset="0"/>
                        <GradientStop Color="White" Offset="0.5"/>
                      </RadialGradientBrush.GradientStops>
                    </RadialGradientBrush>
                    <StaticResource x:Key="Alias" ResourceKey="R" />
                    <SolidColorBrush x:Key="L1" Color="{DynamicResource L2}" />
                    <SolidColorBrush x:Key="L2" Color="{DynamicResource L1}" />
                    <StaticResource x:Key="Self" ResourceKey="Self" />
                    <ImageBrush x:Key="Pic" ImageSource="{ThemeFile 'Images/bg.png'}" Stretch="UniformToFill" Opacity="0.4" />
                    <sys:Boolean x:Key="Flag">True</sys:Boolean>
                    <CornerRadius x:Key="Round">4,4,0,0</CornerRadius>
                    <Thickness x:Key="Pad">2,4</Thickness>"##,
                ),
            )],
        );
        let tree = Tree::build(&t, &[("Images/bg.png".into(), 10)]);
        let v = |k: &str| tree.eval_node(tree.resource(k).unwrap(), 0);
        let Some(Value::Brush(LookBrush::Radial { stops, center, origin, radius })) = v("Alias") else { panic!() };
        assert_eq!(stops.len(), 3);
        assert_eq!(stops[0], (Color([0, 0, 0, 128]), 0.0));
        assert_eq!(stops[1].1, 0.5);
        assert_eq!(stops[2], (Color([255, 0, 0, 64]), 1.0));
        assert_eq!((center, origin, radius), ([0.5, 0.2], [0.5, 0.1], [0.6, 0.8]));
        assert_eq!(v("L1"), None);
        assert_eq!(v("Self"), None);
        assert_eq!(
            v("Pic"),
            Some(Value::Brush(LookBrush::Image {
                file: Some("Images/bg.png".into()),
                source: "{ThemeFile 'Images/bg.png'}".into(),
                stretch: Stretch::UniformToFill,
                opacity: 0.4
            }))
        );
        assert_eq!(v("Flag"), Some(Value::Bool(true)));
        assert_eq!(v("Round"), Some(Value::Corner([4.0, 4.0, 0.0, 0.0])));
        assert_eq!(v("Pad").unwrap().sides(), Some([2.0, 4.0, 2.0, 4.0]));
    }

    #[test]
    fn styles_follow_based_on_implicit_styles_local_scopes_and_template_bindings() {
        let t = texts(
            Mode::Fullscreen,
            &[(
                "Views/Test.xaml",
                dict(
                    r##"<SolidColorBrush x:Key="Pink" Color="#FFFF00FF" />
                    <Style x:Key="Base" TargetType="Border">
                      <Setter Property="BorderThickness" Value="2" />
                      <Setter Property="Background" Value="{DynamicResource Pink}" />
                    </Style>
                    <Style x:Key="Derived" TargetType="Border" BasedOn="{StaticResource Base}">
                      <Setter Property="BorderThickness" Value="6" />
                    </Style>
                    <Style x:Key="Loop1" BasedOn="{StaticResource Loop2}" />
                    <Style x:Key="Loop2" BasedOn="{StaticResource Loop1}" />
                    <Style TargetType="{x:Type TestView}">
                      <Setter Property="Background" Value="#FF102030" />
                      <Setter Property="Template">
                        <Setter.Value>
                          <ControlTemplate TargetType="{x:Type TestView}">
                            <ControlTemplate.Resources>
                              <sys:Double x:Key="Pink">7</sys:Double>
                            </ControlTemplate.Resources>
                            <Grid x:Name="Root" Background="{TemplateBinding Background}">
                              <Border x:Name="B" Style="{StaticResource Derived}" Width="{DynamicResource Pink}" />
                              <Border x:Name="C">
                                <Border.Style><Style TargetType="Border" BasedOn="{StaticResource Base}" /></Border.Style>
                              </Border>
                            </Grid>
                          </ControlTemplate>
                        </Setter.Value>
                      </Setter>
                    </Style>"##,
                ),
            )],
        );
        let tree = Tree::build(&t, &[]);
        let tpl = tree.template_of("TestView").unwrap();
        let root = tree.template_root(tpl).unwrap();
        assert_eq!(tree.get(root, "Background").unwrap().color(), Some(Color([0x10, 0x20, 0x30, 255])));
        let b = tree.named(tpl, "B").unwrap();
        assert_eq!(tree.get(b, "BorderThickness").unwrap().max_side(), Some(6.0));
        assert_eq!(tree.get(b, "Background").unwrap().color(), Some(Color([255, 0, 255, 255])));
        // The template's own resources shadow the global key.
        assert_eq!(tree.get(b, "Width").unwrap().num(), Some(7.0));
        let c = tree.named(tpl, "C").unwrap();
        assert_eq!(tree.get(c, "BorderThickness").unwrap().max_side(), Some(2.0));
        let l = tree.resource("Loop1").unwrap();
        assert_eq!(tree.setter(l, "Anything"), None);
    }

    #[test]
    fn a_style_based_on_a_type_takes_the_implicit_style_before_it() {
        let buttons = dict(
            r##"<Style TargetType="{x:Type Button}">
              <Setter Property="FontSize" Value="30" />
              <Setter Property="Foreground" Value="#FF00FF00" />
            </Style>"##,
        );
        let derived = dict(
            r##"<Style TargetType="{x:Type ButtonEx}" BasedOn="{StaticResource {x:Type Button}}">
              <Setter Property="FontSize" Value="40" />
            </Style>
            <Style x:Key="Plain" TargetType="Button" BasedOn="{StaticResource {x:Type Button}}" />
            <Style TargetType="{x:Type Button}" BasedOn="{StaticResource {x:Type Button}}">
              <Setter Property="FontSize" Value="50" />
            </Style>"##,
        );
        let t = texts(
            Mode::Fullscreen,
            &[("DefaultControls/Button.xaml", buttons), ("DerivedStyles/ButtonEx.xaml", derived)],
        );
        let tree = Tree::build(&t, &[]);
        let ex = tree.implicit_style("ButtonEx").unwrap();
        assert_eq!(tree.setter(ex, "FontSize").and_then(|v| v.num()), Some(40.0));
        assert_eq!(tree.setter(ex, "Foreground").and_then(|v| v.color()), Some(Color([0, 255, 0, 255])));
        let plain = tree.resource("Plain").unwrap();
        assert_eq!(tree.setter(plain, "FontSize").and_then(|v| v.num()), Some(30.0));
        // The later Button style restyles the earlier one and is the one Buttons get.
        let button = tree.implicit_style("Button").unwrap();
        assert_eq!(tree.setter(button, "FontSize").and_then(|v| v.num()), Some(50.0));
        assert_eq!(tree.setter(button, "Foreground").and_then(|v| v.color()), Some(Color([0, 255, 0, 255])));
    }

    #[test]
    fn a_views_own_file_wins_over_a_variant_and_design_size_resolves() {
        let view = |bg: &str| {
            dict(&format!(
                r##"<Style TargetType="{{x:Type GameDetails}}">
                  <Setter Property="Background" Value="{bg}" />
                  <Setter Property="Width" Value="{{StaticResource DesignWidth}}" />
                </Style>"##
            ))
        };
        let t = texts(
            Mode::Fullscreen,
            &[("Views/GameDetails.xaml", view("#FF0000FF")), ("Views/GameDetails_Alt.xaml", view("#FFFF0000"))],
        );
        let tree = Tree::build(&t, &[]);
        let style = tree.implicit_style("GameDetails").unwrap();
        assert_eq!(tree.setter(style, "Background").and_then(|v| v.color()), Some(Color([0, 0, 255, 255])));
        assert_eq!(tree.setter(style, "Width").and_then(|v| v.num()), Some(1920.0));
        assert_eq!(
            tree.resource("DesignHeight").and_then(|n| tree.eval_node(n, 0)).and_then(|v| v.num()),
            Some(1080.0)
        );
    }

    #[test]
    fn caps_and_broken_files_are_reported() {
        let t = texts(
            Mode::Desktop,
            &[("Views/Broken.xaml", "<ResourceDictionary><Unclosed></ResourceDictionary>".into())],
        );
        let tree = Tree::build(&t, &[]);
        assert_eq!(tree.coverage.broken, vec!["Views/Broken.xaml".to_string()]);
        assert_eq!(
            theme_file(&[("Images/BG.png".into(), 1)], "pack://siteoforigin:,,,/Themes/X/images/bg.png"),
            Some("Images/BG.png".into())
        );
    }
}
