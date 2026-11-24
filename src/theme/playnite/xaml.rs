//! Just enough XAML for Playnite themes: the top-level keyed resources of resource dictionaries
//! (colours, brushes, fonts, numbers, image sources, simple styles). No XAML engine: templates,
//! triggers, bindings and controls are never evaluated, only counted for the report.

use crate::theme::Color;
use std::collections::BTreeMap;

/// One keyed resource.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Res {
    /// `<Color x:Key>` (value: literal or `{DynamicResource …}`).
    Color(Val),
    /// `<SolidColorBrush Color=… Opacity=…>`.
    Solid { color: Val, opacity: f32 },
    /// Linear / radial gradient: (colour, offset) stops and where they run.
    Gradient { stops: Vec<(Val, f32)>, radial: bool, opacity: f32, geom: Geom },
    /// `<StaticResource x:Key="A" ResourceKey="B" />`: another name for a resource.
    Alias(String),
    /// `<ImageBrush ImageSource=…>` / `<BitmapImage UriSource=…>`.
    Image(String),
    /// `<FontFamily x:Key>Name</FontFamily>`.
    Font(Val),
    /// `<sys:Double>`.
    Number(f64),
    /// `<Style x:Key>`: its simple setters (property → value).
    Style(Vec<(String, String)>),
    /// Anything else we can't use (VisualBrush, DrawingBrush, templates …): the element name.
    Other(String),
}

/// Where a gradient runs (WPF defaults: linear 0,0 → 1,1; radial centred, radius 0.5).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Geom {
    pub start: [f32; 2],
    pub end: [f32; 2],
    pub center: [f32; 2],
    pub origin: [f32; 2],
    pub radius: [f32; 2],
    pub absolute: bool,
}

impl Default for Geom {
    fn default() -> Geom {
        Geom {
            start: [0.0, 0.0],
            end: [1.0, 1.0],
            center: [0.5, 0.5],
            origin: [0.5, 0.5],
            radius: [0.5, 0.5],
            absolute: false,
        }
    }
}

/// `"x,y"` (also space separated).
pub(crate) fn parse_point(s: &str) -> Option<[f32; 2]> {
    let v: Vec<f32> = s
        .split([',', ' '])
        .filter(|p| !p.trim().is_empty())
        .map(|p| p.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .ok()?;
    match v.as_slice() {
        [x, y] if x.is_finite() && y.is_finite() => Some([*x, *y]),
        _ => None,
    }
}

impl Geom {
    /// Read a Linear/RadialGradientBrush element's geometry attributes.
    pub fn of(n: roxmltree::Node) -> Geom {
        let mut g = Geom::default();
        let pt = |name: &str| prop(n, name).as_deref().and_then(parse_point);
        let num = |name: &str| prop(n, name).and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| v.is_finite());
        if let Some(p) = pt("StartPoint") {
            g.start = p;
        }
        if let Some(p) = pt("EndPoint") {
            g.end = p;
        }
        if let Some(p) = pt("Center") {
            g.center = p;
            g.origin = p;
        }
        if let Some(p) = pt("GradientOrigin") {
            g.origin = p;
        }
        if let Some(r) = num("RadiusX") {
            g.radius[0] = r;
        }
        if let Some(r) = num("RadiusY") {
            g.radius[1] = r;
        }
        g.absolute = prop(n, "MappingMode").is_some_and(|m| m.trim().eq_ignore_ascii_case("Absolute"));
        g
    }
}

/// A literal value or a reference to another resource.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Val {
    Lit(String),
    Ref(String),
}

impl Val {
    pub fn parse(s: &str) -> Val {
        let t = s.trim();
        if let Some(inner) = t.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
            let mut parts = inner.split_whitespace();
            let kind = parts.next().unwrap_or("");
            if kind == "DynamicResource" || kind == "StaticResource" {
                let key: String = parts.collect::<Vec<_>>().join(" ");
                let key = key.trim_start_matches("ResourceKey=").trim().to_string();
                return Val::Ref(key);
            }
        }
        Val::Lit(t.to_string())
    }
}

const XAML_NS: &str = "http://schemas.microsoft.com/winfx/2006/xaml";

pub(crate) fn key_of(n: roxmltree::Node) -> Option<String> {
    n.attributes().find(|a| a.name() == "Key" && a.namespace() == Some(XAML_NS)).map(|a| a.value().trim().to_string())
}

pub(crate) fn attr<'a>(n: roxmltree::Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.attributes().find(|a| a.name() == name && a.namespace().is_none()).map(|a| a.value())
}

/// A property given as attribute or as a property element (`<SolidColorBrush.Color>#fff</…>`).
pub(crate) fn prop(n: roxmltree::Node, name: &str) -> Option<String> {
    if let Some(v) = attr(n, name) {
        return Some(v.to_string());
    }
    let pe = format!("{}.{name}", n.tag_name().name());
    n.children().find(|c| c.is_element() && c.tag_name().name() == pe).map(|c| {
        // Either text or a nested element (<Color>…</Color>).
        c.children()
            .find(|x| x.is_element())
            .and_then(|x| x.text())
            .or_else(|| c.text())
            .unwrap_or("")
            .trim()
            .to_string()
    })
}

fn opacity(n: roxmltree::Node) -> f32 {
    prop(n, "Opacity").and_then(|v| v.trim().parse().ok()).unwrap_or(1.0f32).clamp(0.0, 1.0)
}

/// What one dictionary file holds.
#[derive(Debug, Default)]
pub(crate) struct Dict {
    pub res: BTreeMap<String, Res>,
    /// Templates, triggers, animations … (things only a XAML engine could show).
    pub templates: usize,
    pub animations: usize,
    /// Video / sound element uses (MediaElement, SoundPlayer …).
    pub media: usize,
}

/// Parse one resource dictionary. Errors are reported, not fatal (a broken file is skipped).
pub(crate) fn parse(text: &str) -> anyhow::Result<Dict> {
    let opts = roxmltree::ParsingOptions { allow_dtd: false, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(text, opts)?;
    Ok(parse_doc(&doc))
}

/// Gradient stops of a brush element (direct or inside `<X.GradientStops>`).
pub(crate) fn stops_of(n: roxmltree::Node) -> Vec<(Val, f32)> {
    n.descendants()
        .filter(|s| s.is_element() && s.tag_name().name() == "GradientStop")
        .map(|s| {
            let c = Val::parse(&prop(s, "Color").unwrap_or_default());
            let o = prop(s, "Offset").and_then(|v| v.trim().parse().ok()).unwrap_or(0.0f32);
            (c, o)
        })
        .collect()
}

/// The dictionary-level resource elements of a document: the root ResourceDictionary's
/// children and those of inline `ResourceDictionary.MergedDictionaries` dictionaries (in
/// document order; a later one wins, as in WPF).
pub(crate) fn dictionary_children<'a, 'i>(doc: &'a roxmltree::Document<'i>) -> Vec<roxmltree::Node<'a, 'i>> {
    let mut out = vec![];
    let root = doc.root_element();
    if root.tag_name().name() != "ResourceDictionary" {
        return out;
    }
    fn walk<'a, 'i>(d: roxmltree::Node<'a, 'i>, depth: u8, out: &mut Vec<roxmltree::Node<'a, 'i>>) {
        for n in d.children().filter(|n| n.is_element()) {
            if n.tag_name().name() == "ResourceDictionary.MergedDictionaries" {
                if depth < 4 {
                    for m in n.children().filter(|m| m.is_element() && m.tag_name().name() == "ResourceDictionary") {
                        walk(m, depth + 1, out);
                    }
                }
                continue;
            }
            out.push(n);
        }
    }
    walk(root, 0, &mut out);
    // Merged dictionaries come first in WPF's lookup order: the dictionary's own keys win.
    out.sort_by_key(|n| n.parent() == Some(root));
    out
}

/// [`parse`] for an already parsed document.
pub(crate) fn parse_doc(doc: &roxmltree::Document) -> Dict {
    let mut d = Dict::default();
    for n in doc.descendants().filter(|n| n.is_element()) {
        match n.tag_name().name() {
            "ControlTemplate" | "DataTemplate" => d.templates += 1,
            "Storyboard" => d.animations += 1,
            "MediaElement" => d.media += 1,
            _ => {}
        }
    }
    for n in dictionary_children(doc) {
        let Some(key) = key_of(n) else { continue };
        let res = match n.tag_name().name() {
            "Color" => Res::Color(Val::parse(n.text().unwrap_or(""))),
            "SolidColorBrush" => {
                Res::Solid { color: Val::parse(&prop(n, "Color").unwrap_or_default()), opacity: opacity(n) }
            }
            name @ ("LinearGradientBrush" | "RadialGradientBrush") => Res::Gradient {
                stops: stops_of(n),
                radial: name.starts_with("Radial"),
                opacity: opacity(n),
                geom: Geom::of(n),
            },
            "StaticResource" | "DynamicResource" => match attr(n, "ResourceKey") {
                Some(k) => Res::Alias(k.trim().to_string()),
                None => Res::Other(n.tag_name().name().to_string()),
            },
            "ImageBrush" => match prop(n, "ImageSource") {
                Some(s) => Res::Image(s),
                None => Res::Other("ImageBrush".into()),
            },
            "BitmapImage" => match prop(n, "UriSource") {
                Some(s) => Res::Image(s),
                None => Res::Other("BitmapImage".into()),
            },
            "FontFamily" => Res::Font(Val::parse(n.text().unwrap_or(""))),
            "Double" => match n.text().and_then(|t| t.trim().parse().ok()) {
                Some(v) => Res::Number(v),
                None => Res::Other("Double".into()),
            },
            "Style" => Res::Style(
                n.children()
                    .filter(|s| s.is_element() && s.tag_name().name() == "Setter")
                    .filter_map(|s| Some((attr(s, "Property")?.to_string(), attr(s, "Value")?.to_string())))
                    .collect(),
            ),
            other => Res::Other(other.to_string()),
        };
        d.res.insert(key, res);
    }
    d
}

/// A WPF colour: `#RGB`, `#ARGB`, `#RRGGBB`, `#AARRGGBB` (alpha first!) or a named colour.
pub(crate) fn parse_color(s: &str) -> Option<Color> {
    let t = s.trim();
    if let Some(h) = t.strip_prefix('#') {
        let hex = |i: usize, n: usize| -> Option<u8> {
            let v = u8::from_str_radix(h.get(i..i + n)?, 16).ok()?;
            Some(if n == 1 { v * 17 } else { v })
        };
        return match h.len() {
            3 => Some(Color([hex(0, 1)?, hex(1, 1)?, hex(2, 1)?, 255])),
            4 => Some(Color([hex(1, 1)?, hex(2, 1)?, hex(3, 1)?, hex(0, 1)?])),
            6 => Some(Color([hex(0, 2)?, hex(2, 2)?, hex(4, 2)?, 255])),
            8 => Some(Color([hex(2, 2)?, hex(4, 2)?, hex(6, 2)?, hex(0, 2)?])),
            _ => None,
        };
    }
    named(t)
}

/// WPF's named colours (`System.Windows.Media.Colors`), the ones themes use.
fn named(name: &str) -> Option<Color> {
    let lower = name.to_ascii_lowercase();
    if lower == "transparent" {
        return Some(Color([255, 255, 255, 0]));
    }
    let rgb: u32 = match lower.as_str() {
        "black" => 0x000000,
        "white" => 0xffffff,
        "red" => 0xff0000,
        "green" => 0x008000,
        "lime" => 0x00ff00,
        "blue" => 0x0000ff,
        "yellow" => 0xffff00,
        "orange" => 0xffa500,
        "orangered" => 0xff4500,
        "gray" | "grey" => 0x808080,
        "darkgray" | "darkgrey" => 0xa9a9a9,
        "lightgray" | "lightgrey" => 0xd3d3d3,
        "dimgray" | "dimgrey" => 0x696969,
        "silver" => 0xc0c0c0,
        "gainsboro" => 0xdcdcdc,
        "whitesmoke" => 0xf5f5f5,
        "gold" => 0xffd700,
        "aqua" | "cyan" => 0x00ffff,
        "magenta" | "fuchsia" => 0xff00ff,
        "purple" => 0x800080,
        "navy" => 0x000080,
        "teal" => 0x008080,
        "maroon" => 0x800000,
        "olive" => 0x808000,
        "dodgerblue" => 0x1e90ff,
        "deepskyblue" => 0x00bfff,
        "skyblue" => 0x87ceeb,
        "lightblue" => 0xadd8e6,
        "steelblue" => 0x4682b4,
        "royalblue" => 0x4169e1,
        "cornflowerblue" => 0x6495ed,
        "midnightblue" => 0x191970,
        "darkblue" => 0x00008b,
        "crimson" => 0xdc143c,
        "darkred" => 0x8b0000,
        "tomato" => 0xff6347,
        "coral" => 0xff7f50,
        "salmon" => 0xfa8072,
        "hotpink" => 0xff69b4,
        "deeppink" => 0xff1493,
        "pink" => 0xffc0cb,
        "violet" => 0xee82ee,
        "orchid" => 0xda70d6,
        "plum" => 0xdda0dd,
        "indigo" => 0x4b0082,
        "slategray" | "slategrey" => 0x708090,
        "darkslategray" | "darkslategrey" => 0x2f4f4f,
        "ivory" => 0xfffff0,
        "beige" => 0xf5f5dc,
        "khaki" => 0xf0e68c,
        "chartreuse" => 0x7fff00,
        "limegreen" => 0x32cd32,
        "lightgreen" => 0x90ee90,
        "forestgreen" => 0x228b22,
        "seagreen" => 0x2e8b57,
        "darkgreen" => 0x006400,
        "turquoise" => 0x40e0d0,
        "brown" => 0xa52a2a,
        "chocolate" => 0xd2691e,
        "tan" => 0xd2b48c,
        "darkorange" => 0xff8c00,
        "ghostwhite" => 0xf8f8ff,
        "aliceblue" => 0xf0f8ff,
        _ => return None,
    };
    Some(Color([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255]))
}

/// A brush as far as Lodestar can show it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Brush {
    Solid(Color),
    /// Stops sorted by offset.
    Gradient {
        stops: Vec<(Color, f32)>,
        radial: bool,
        geom: Geom,
    },
    Image(String),
}

impl Brush {
    /// One colour standing for the brush: a gradient's average over the box it paints (see
    /// `LookBrush::average`).
    pub fn average(&self) -> Option<Color> {
        match self {
            Brush::Solid(c) => Some(*c),
            Brush::Gradient { .. } => self.to_look(&[]).average(),
            Brush::Image(_) => None,
        }
    }

    /// The brush with its full geometry (gradients keep every stop, the direction and the
    /// radial centre / radius). `files`: the theme's files, to resolve a picture's path.
    pub fn to_look(&self, files: &[(String, u64)]) -> super::look::LookBrush {
        use super::look::LookBrush;
        match self {
            Brush::Solid(c) => LookBrush::solid(*c),
            Brush::Gradient { stops, radial: false, geom } => {
                LookBrush::Linear { stops: stops.clone(), start: geom.start, end: geom.end, absolute: geom.absolute }
            }
            Brush::Gradient { stops, radial: true, geom } => LookBrush::Radial {
                stops: stops.clone(),
                center: geom.center,
                origin: geom.origin,
                radius: geom.radius,
            },
            Brush::Image(src) => LookBrush::Image {
                file: super::tree::theme_file(files, src),
                source: src.clone(),
                stretch: Default::default(),
                opacity: 1.0,
            },
        }
    }
}

/// Area-weighted average of a piecewise-linear gradient over [0, 1] (WPF pads the ends).
pub(crate) fn gradient_average(stops: &[(Color, f32)]) -> Option<Color> {
    let first = stops.first()?;
    if stops.len() == 1 {
        return Some(first.0);
    }
    let at = |t: f32| -> [f32; 4] {
        let f = |c: Color| c.0.map(|v| v as f32);
        if t <= stops[0].1 {
            return f(stops[0].0);
        }
        for w in stops.windows(2) {
            let (a, b) = (w[0], w[1]);
            if t <= b.1 {
                let span = (b.1 - a.1).max(1e-6);
                let k = ((t - a.1) / span).clamp(0.0, 1.0);
                let (ca, cb) = (f(a.0), f(b.0));
                return [0, 1, 2, 3].map(|i| ca[i] + (cb[i] - ca[i]) * k);
            }
        }
        f(stops[stops.len() - 1].0)
    };
    const N: usize = 64;
    let mut sum = [0f32; 4];
    for i in 0..N {
        let c = at((i as f32 + 0.5) / N as f32);
        for k in 0..4 {
            sum[k] += c[k];
        }
    }
    Some(Color(sum.map(|v| (v / N as f32).round().clamp(0.0, 255.0) as u8)))
}

/// All dictionaries of a theme merged over Playnite's defaults, with reference resolution.
#[derive(Debug, Default)]
pub(crate) struct Resources {
    /// Resources the theme itself defines.
    pub theme: BTreeMap<String, Res>,
    /// Playnite's own defaults for the mode (themes only ship the files they change).
    pub defaults: BTreeMap<String, Res>,
}

impl Resources {
    pub fn get(&self, key: &str) -> Option<&Res> {
        let mut k = key;
        for _ in 0..8 {
            match self.theme.get(k).or_else(|| self.defaults.get(k))? {
                Res::Alias(next) => k = next,
                r => return Some(r),
            }
        }
        None
    }

    /// Whether the theme decides this resource: it defines it, or a default it inherits
    /// refers to something the theme defines (a default brush of the theme's colour).
    pub fn theme_defines(&self, key: &str) -> bool {
        self.decided(key, 0)
    }

    fn decided(&self, key: &str, depth: u8) -> bool {
        if self.theme.contains_key(key) {
            return true;
        }
        if depth > 8 {
            return false;
        }
        let refs = |v: &Val| matches!(v, Val::Ref(k) if self.decided(k, depth + 1));
        match self.defaults.get(key) {
            Some(Res::Color(v)) | Some(Res::Font(v)) => refs(v),
            Some(Res::Solid { color, .. }) => refs(color),
            Some(Res::Gradient { stops, .. }) => stops.iter().any(|(v, _)| refs(v)),
            Some(Res::Alias(k)) => self.decided(k, depth + 1),
            _ => false,
        }
    }

    /// A colour value (literal or reference to a Color / brush resource).
    pub fn color_val(&self, v: &Val, depth: u8) -> Option<Color> {
        match v {
            Val::Lit(s) => parse_color(s),
            Val::Ref(k) => self.color_depth(k, depth + 1),
        }
    }

    /// The colour of a Color or brush resource (a gradient's average).
    pub fn color(&self, key: &str) -> Option<Color> {
        self.color_depth(key, 0)
    }

    fn color_depth(&self, key: &str, depth: u8) -> Option<Color> {
        if depth > 8 {
            return None;
        }
        self.brush_depth(key, depth)?.average()
    }

    pub fn brush(&self, key: &str) -> Option<Brush> {
        self.brush_depth(key, 0)
    }

    fn brush_depth(&self, key: &str, depth: u8) -> Option<Brush> {
        if depth > 8 {
            return None;
        }
        match self.get(key)? {
            Res::Color(v) => self.color_val(v, depth).map(Brush::Solid),
            Res::Solid { color, opacity } => {
                let c = self.color_val(color, depth)?;
                Some(Brush::Solid(scale_alpha(c, *opacity)))
            }
            Res::Gradient { stops, radial, opacity, geom } => {
                let mut s: Vec<(Color, f32)> = stops
                    .iter()
                    .filter_map(|(v, o)| Some((scale_alpha(self.color_val(v, depth)?, *opacity), *o)))
                    .collect();
                if s.is_empty() {
                    return None;
                }
                s.sort_by(|a, b| a.1.total_cmp(&b.1));
                Some(Brush::Gradient { stops: s, radial: *radial, geom: *geom })
            }
            Res::Image(src) => Some(Brush::Image(src.clone())),
            _ => None,
        }
    }

    /// A FontFamily resource's text, following references.
    pub fn font(&self, key: &str) -> Option<String> {
        let mut k = key.to_string();
        for _ in 0..8 {
            match self.get(&k)? {
                Res::Font(Val::Lit(s)) => return Some(s.clone()),
                Res::Font(Val::Ref(r)) => k = r.clone(),
                _ => return None,
            }
        }
        None
    }

    pub fn style_setter(&self, key: &str, property: &str) -> Option<String> {
        match self.get(key)? {
            Res::Style(s) => s.iter().find(|(p, _)| p == property).map(|(_, v)| v.clone()),
            _ => None,
        }
    }
}

fn scale_alpha(c: Color, o: f32) -> Color {
    let [r, g, b, a] = c.0;
    Color([r, g, b, (a as f32 * o).round() as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wpf_colours_are_argb() {
        assert_eq!(parse_color("#FF2C3A67"), Some(Color([0x2c, 0x3a, 0x67, 0xff])));
        assert_eq!(parse_color("#8855CDFF"), Some(Color([0x55, 0xcd, 0xff, 0x88])));
        assert_eq!(parse_color("#f2f2f2"), Some(Color([0xf2, 0xf2, 0xf2, 0xff])));
        assert_eq!(parse_color("#8fff"), Some(Color([255, 255, 255, 0x88])));
        assert_eq!(parse_color("OrangeRed"), Some(Color([0xff, 0x45, 0, 255])));
        assert_eq!(parse_color("Transparent").map(|c| c.0[3]), Some(0));
        assert_eq!(parse_color("sc#1,0,0,0"), None);
    }

    #[test]
    fn references_resolve_through_colours_and_brushes() {
        let d = parse(
            r##"<ResourceDictionary xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
                xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
                xmlns:sys="clr-namespace:System;assembly=mscorlib">
              <Color x:Key="TextColor">#FFEBEBEB</Color>
              <SolidColorBrush x:Key="TextBrush" Color="{DynamicResource TextColor}" />
              <SolidColorBrush x:Key="Half" Color="{StaticResource TextColor}" Opacity="0.5" />
              <SolidColorBrush x:Key="Nested"><SolidColorBrush.Color>#ff0000</SolidColorBrush.Color></SolidColorBrush>
              <LinearGradientBrush x:Key="Bg"><GradientStop Color="#FF000000" Offset="0"/><GradientStop Color="#FFFFFFFF" Offset="1"/></LinearGradientBrush>
              <sys:Double x:Key="FontSize">22</sys:Double>
              <FontFamily x:Key="FontFamily">./Fonts/#Titillium Web</FontFamily>
              <ImageBrush x:Key="Pic" ImageSource="{ThemeFile 'Images/bg.jpg'}" />
              <Style x:Key="TextBlockBaseStyle" TargetType="TextBlock"><Setter Property="FontFamily" Value="Lato" /></Style>
              <VisualBrush x:Key="Odd" />
            </ResourceDictionary>"##,
        )
        .unwrap();
        let r = Resources { theme: d.res, defaults: Default::default() };
        assert_eq!(r.color("TextBrush"), Some(Color([0xeb, 0xeb, 0xeb, 255])));
        assert_eq!(r.color("Half").unwrap().0[3], 128);
        assert_eq!(r.color("Nested"), Some(Color([255, 0, 0, 255])));
        let avg = r.color("Bg").unwrap();
        assert!((126..=129).contains(&avg.0[0]), "{avg:?}");
        assert_eq!(r.get("FontSize"), Some(&Res::Number(22.0)));
        assert_eq!(r.font("FontFamily").as_deref(), Some("./Fonts/#Titillium Web"));
        assert_eq!(r.brush("Pic"), Some(Brush::Image("{ThemeFile 'Images/bg.jpg'}".into())));
        assert_eq!(r.style_setter("TextBlockBaseStyle", "FontFamily").as_deref(), Some("Lato"));
        assert_eq!(r.get("Odd"), Some(&Res::Other("VisualBrush".into())));
        assert_eq!(r.color("Odd"), None);
    }

    #[test]
    fn reference_loops_end() {
        let d = parse(
            r#"<ResourceDictionary xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
                xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml">
              <SolidColorBrush x:Key="A" Color="{DynamicResource B}" />
              <SolidColorBrush x:Key="B" Color="{DynamicResource A}" />
            </ResourceDictionary>"#,
        )
        .unwrap();
        let r = Resources { theme: d.res, defaults: Default::default() };
        assert_eq!(r.color("A"), None);
    }
}
