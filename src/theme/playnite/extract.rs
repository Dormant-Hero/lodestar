//! Reading a [`PlayniteLook`] out of a resolved theme ([`Tree`]). Everything is tied to
//! Playnite's own structure — PART_ names, the default theme's style keys
//! (ListGameItemStyle, GridViewItemTemplate …), implicit view styles (Main, GameDetails …) and
//! Playnite's audio folder — never to one theme's private element names, so it keeps working
//! as themes update. Where a theme leaves something out, the default theme underneath answers.

mod desktop;
mod fonts;
mod item;
mod main_view;
mod plugins;
mod rows;

use super::Mode;

use super::fonts as pfonts;

use super::look::*;
use super::options::Choices;

use super::source::Source;

use super::tree::{Layer, Markup, Texts, Tree, Value, content, markup, name_of, prop_elem, tag, theme_file};

use super::xaml::{self, Resources, attr};

use crate::theme::Color;

use roxmltree::Node;

use std::collections::{BTreeMap, BTreeSet};

/// A Fullscreen view's design size.
pub(crate) const DESIGN: [f32; 2] = [1920.0, 1080.0];

const VIDEO_EXTS: [&str; 7] = ["mp4", "wmv", "avi", "webm", "mkv", "mov", "m4v"];

const SOUND_EXTS: [&str; 6] = ["wav", "mp3", "ogg", "flac", "wma", "m4a"];

/// A details overlay whose average alpha is under this only tints the picture (see details_look).
const FAINT_OVERLAY: u8 = 40;

/// What the importer gets from one theme.
pub(crate) struct Extracted {
    pub look: PlayniteLook,
    /// The flat keyed resources (theme over default) the palette mapping reads.
    pub resources: Resources,
    /// Whether the cover template shows the ThemeExtras banner, and where (extras.rs).
    pub banner: Option<(bool, super::extras::Placement)>,
}

fn ext(rel: &str) -> String {
    rel.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

fn stem(rel: &str) -> &str {
    let base = rel.rsplit('/').next().unwrap_or(rel);
    base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base)
}

/// Read a theme: its XAML over Playnite's default theme, `thememodifier.yaml`,
/// `themeExtras.yaml`, its audio folder and videos.
pub(crate) fn extract(source: &Source, mode: Mode) -> Extracted {
    extract_with(source, mode, &Default::default())
}

/// [`extract`] with the theme options' values laid over the theme (see options.rs).
pub(crate) fn extract_with(source: &Source, mode: Mode, chosen: &Choices) -> Extracted {
    let files = source.files();
    let texts = chosen_texts(source, &files, mode, chosen);
    let mut tree = Tree::build(&texts, &files);
    tree.settings = chosen.settings.clone();
    let find = |name: &str| {
        files
            .iter()
            .filter(|(f, _)| f.rsplit('/').next().is_some_and(|b| b.eq_ignore_ascii_case(name)))
            .min_by_key(|(f, _)| f.matches('/').count())
            .map(|(f, _)| f.clone())
    };
    let modifier = find("thememodifier.yaml").and_then(|f| source.read_text(&f, super::source::YAML_LIMIT));
    let mut look = look_of(&tree, modifier.as_deref());
    look.plugins.banners |= find("themeExtras.yaml").is_some();
    look.coverage.api_version = source.read_text("theme.yaml", super::source::YAML_LIMIT).and_then(|t| {
        t.lines()
            .filter_map(|l| l.split_once(':'))
            .find(|(k, _)| k.trim().eq_ignore_ascii_case("ThemeApiVersion"))
            .map(|(_, v)| v.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
            .filter(|v| !v.is_empty())
    });
    let resources = resources_of(&tree);
    let banner = look.plugins.banners.then(|| super::extras::banner_look(&tree, mode == Mode::Fullscreen));
    Extracted { look, resources, banner }
}

/// The theme's XAML over Playnite's default theme, with the chosen constants laid over it.
pub(crate) fn chosen_texts(source: &Source, files: &[(String, u64)], mode: Mode, chosen: &Choices) -> Texts {
    let mut texts = Texts::load(source, files, mode);
    if let Some(x) = &chosen.constants {
        texts.push_theme("thememodifier.xaml", x.clone());
    }
    texts
}

/// `--playnite-look`: the look of a theme as JSON (with how long reading it took).
pub(crate) fn report(src: &std::path::Path) -> anyhow::Result<String> {
    let t0 = std::time::Instant::now();
    let source = Source::open(src)?;
    let manifest = source.read_text("theme.yaml", super::source::YAML_LIMIT).unwrap_or_default();
    let fullscreen = manifest.lines().any(|l| {
        l.split_once(':')
            .is_some_and(|(k, v)| k.trim().eq_ignore_ascii_case("Mode") && v.trim().eq_ignore_ascii_case("Fullscreen"))
    });
    let mode = if fullscreen { Mode::Fullscreen } else { Mode::Desktop };
    let x = extract(&source, mode);
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    // The palette background as the renderer would get it (gradients pass through).
    let files = source.files();
    let mapped = super::map::palette(&x.resources, mode, None, None, None);
    let bg = mapped.background_look(&files);
    let key = match mode {
        Mode::Fullscreen => "MainBackgourndBrush",
        Mode::Desktop => "WindowBackgourndBrush",
    };
    let same = super::map::brush_look(&x.resources, key, &files) == bg;
    let mut v = serde_json::to_value(&x.look)?;
    if let serde_json::Value::Object(o) = &mut v {
        o.insert("read_ms".into(), serde_json::json!((ms * 10.0).round() / 10.0));
        o.insert("theme_resources".into(), serde_json::json!(x.resources.theme.len()));
        o.insert(
            "palette_background".into(),
            serde_json::json!({
                "brush": bg,
                "from_key": same,
                "stops": bg.as_ref().map(|b| b.stops().len()),
                "angle": bg.as_ref().and_then(LookBrush::angle),
                "box_stops": bg.as_ref().map(LookBrush::box_stops),
                "radial": bg.as_ref().and_then(LookBrush::radial),
            }),
        );
    }
    Ok(serde_json::to_string_pretty(&v)?)
}

/// The flat resource maps of the tree (what `xaml::Resources` users read).
pub(crate) fn resources_of(tree: &Tree) -> Resources {
    let mut r = Resources::default();
    for d in &tree.docs {
        let dict = xaml::parse_doc(&d.doc);
        match d.layer {
            Layer::Default => r.defaults.extend(dict.res),
            Layer::Theme => r.theme.extend(dict.res),
        }
    }
    r
}

/// Everything the extractor reads, from a built tree.
pub(crate) fn look_of(tree: &Tree, modifier: Option<&str>) -> PlayniteLook {
    let x = X { t: tree };
    let mut look = PlayniteLook { mode: tree.mode, coverage: tree.coverage.clone(), ..Default::default() };
    look.font_sizes = FontSizes {
        small: x.res_num("FontSizeSmall"),
        normal: x.res_num("FontSize"),
        large: x.res_num("FontSizeLarge"),
        larger: x.res_num("FontSizeLarger"),
        largest: x.res_num("FontSizeLargest"),
    };
    look.fonts.body = x.body_font();
    match tree.mode {
        Mode::Fullscreen => {
            look.item = x.item_look("ListGameItemStyle", "ListGameItemTemplate", &["IsFocused", "IsSelected"]);
            look.main = x.main_look(&look.item);
            look.details = x.details_look();
            look.fonts.title = look.details.title_font.clone();
        }
        Mode::Desktop => {
            look.desktop = x.desktop_look();
            look.fonts.title = x.desktop_title_font();
        }
    }
    look.plugins = x.plugins();
    look.options = modifier.map(|m| x.options(m)).unwrap_or_default();
    look.media = x.media();
    look
}

/// The cover look of an item container style other than the game list's own (the items of a
/// row the theme lays out itself).
pub(crate) fn item_look_of<'a, 'i>(tree: &'a Tree<'i>, style: Node<'a, 'i>) -> ItemLook {
    X { t: tree }.item_look_from(Some(style), None, &["IsFocused", "IsSelected"])
}

/// The extractor's view of the tree.
struct X<'a, 'i> {
    t: &'a Tree<'i>,
}

type N<'a, 'i> = Node<'a, 'i>;

/// The property an element is filled with (a Rectangle's Fill, anything else's Background).
fn fill_prop(n: Node) -> &'static str {
    if tag(n) == "Rectangle" { "Fill" } else { "Background" }
}

/// Grid.Row / Grid.Column / spans of an element.
fn grid_attr(n: Node, name: &str, default: usize) -> usize {
    attr(n, name).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(default)
}

/// Does a Text value bind the game's name?
fn binds_name(v: &Value) -> bool {
    let Value::Binding(b) = v else { return false };
    let path = b.split(',').next().unwrap_or("").trim().trim_start_matches("Path=").trim();
    let mut parts = path.rsplit('.');
    // The game's own name, not that of one of its properties (`CompletionStatus.Name`,
    // `Platforms[0].Name`, `Source.Name`).
    matches!(parts.next(), Some("DisplayName" | "Name")) && parts.all(|p| matches!(p, "Game" | "SelectedGame"))
}

fn edge_alpha(b: &LookBrush) -> [f32; 4] {
    [b.alpha_at(0.5, 0.0), b.alpha_at(1.0, 0.5), b.alpha_at(0.5, 1.0), b.alpha_at(0.0, 0.5)]
}

fn mask_of(b: LookBrush) -> Option<Mask> {
    matches!(b, LookBrush::Linear { .. } | LookBrush::Radial { .. })
        .then(|| Mask { edge_alpha: edge_alpha(&b), brush: b })
}

impl<'a, 'i> X<'a, 'i> {
    fn res_val(&self, key: &str) -> Option<Value<'a, 'i>> {
        let n = self.t.resource(key)?;
        self.t.eval_node(n, 0)
    }

    fn res_num(&self, key: &str) -> Option<f32> {
        self.res_val(key)?.num().map(|v| v as f32)
    }

    fn res_brush(&self, key: &str) -> Option<LookBrush> {
        self.res_val(key)?.brush()
    }

    fn num(&self, el: N<'a, 'i>, prop: &str) -> Option<f32> {
        self.t.get(el, prop)?.num().map(|v| v as f32).filter(|v| v.is_finite())
    }

    /// A non-transparent brush.
    fn brush(&self, el: N<'a, 'i>, prop: &str) -> Option<LookBrush> {
        self.t.get(el, prop)?.brush().filter(|b| !b.is_clear())
    }

    fn text(&self, el: N<'a, 'i>, prop: &str) -> Option<String> {
        match self.t.get(el, prop)? {
            Value::Text(t) => Some(t),
            Value::Bool(b) => Some(b.to_string()),
            _ => None,
        }
    }

    fn align(&self, el: N<'a, 'i>, prop: &str) -> Option<Align> {
        Align::parse(&self.text(el, prop)?)
    }

    /// Hidden as written (Visibility Collapsed / Hidden or Opacity 0); a `{Settings …}` or a
    /// binding counts as shown.
    fn hidden(&self, el: N<'a, 'i>) -> bool {
        let vis = self.text(el, "Visibility").unwrap_or_default().to_ascii_lowercase();
        vis == "collapsed"
            || vis == "hidden"
            || (self.num(el, "Opacity").is_some_and(|o| o <= 0.01) && !self.fades_in(el))
    }

    /// Opacity 0 that a trigger of the element (its own or its style's) animates up: the
    /// common "fade the view in once it's shown / loaded" pattern, so it is shown.
    fn fades_in(&self, el: N<'a, 'i>) -> bool {
        let own = name_of(el);
        let mut trigs = self.t.triggers(el);
        if let Some(s) = self.t.style_of(el) {
            trigs.extend(self.t.triggers(s));
        }
        trigs.into_iter().flat_map(|t| self.t.enter_storyboards(t)).any(|sb| {
            sb.descendants().filter(|a| a.is_element() && tag(*a).ends_with("Animation")).any(|a| {
                let target = attr(a, "Storyboard.TargetName").map(str::trim);
                attr(a, "Storyboard.TargetProperty").is_some_and(|p| p.contains("Opacity"))
                    && (target.is_none() || target == own)
                    && attr(a, "To").and_then(|v| v.trim().parse::<f32>().ok()).is_some_and(|v| v > 0.01)
            })
        })
    }

    /// Hidden itself or through an ancestor up to (not including) `stop`.
    fn hidden_up_to(&self, el: N<'a, 'i>, stop: N<'a, 'i>) -> Option<N<'a, 'i>> {
        el.ancestors()
            .filter(|a| a.is_element() && !tag(*a).contains('.'))
            .take_while(|a| *a != stop)
            .find(|a| self.hidden(*a))
    }

    fn effect_glow(&self, el: N<'a, 'i>) -> Option<Glow> {
        let e = self.t.get(el, "Effect")?.node()?;
        if tag(e) != "DropShadowEffect" {
            return None;
        }
        let n = |p: &str| self.num(e, p);
        Some(Glow {
            color: self.t.get(e, "Color").and_then(|v| v.color()).unwrap_or(Color::BLACK),
            blur: n("BlurRadius").unwrap_or(5.0),
            opacity: n("Opacity").unwrap_or(1.0).clamp(0.0, 1.0),
            depth: n("ShadowDepth").unwrap_or(5.0),
        })
    }

    fn effect_blur(&self, el: N<'a, 'i>) -> Option<f32> {
        let e = self.t.get(el, "Effect")?.node()?;
        (tag(e) == "BlurEffect").then(|| self.num(e, "Radius").unwrap_or(5.0))
    }

    /// The frame an element draws (Border BorderThickness / Rectangle StrokeThickness).
    fn frame_of(&self, el: N<'a, 'i>, brush_override: Option<LookBrush>, thick_override: Option<f32>) -> Option<Frame> {
        let (thick_prop, brush_prop) = match tag(el) {
            "Rectangle" => ("StrokeThickness", "Stroke"),
            _ => ("BorderThickness", "BorderBrush"),
        };
        let width = thick_override.or_else(|| self.t.get(el, thick_prop)?.max_side())?;
        if width <= 0.0 {
            return None;
        }
        let brush = brush_override.or_else(|| self.brush(el, brush_prop));
        let radius = self
            .t
            .get(el, "CornerRadius")
            .and_then(|v| v.max_side())
            .or_else(|| self.num(el, "RadiusX"))
            .unwrap_or(0.0);
        let margin = self.t.get(el, "Margin").and_then(|v| v.sides()).unwrap_or([0.0; 4]);
        let outset = (-margin.iter().copied().fold(f32::INFINITY, f32::min)).max(0.0);
        Some(Frame { width, brush, radius, outset })
    }
}

#[cfg(test)]
mod tests;
