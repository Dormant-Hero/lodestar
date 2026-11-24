//! Theme model: paints (gradients, brushes), the item focus look and the desktop style.
//! (Re-exported from model.rs.)
//!
//! Gradients (`Gradient`, element fields `gradient_stops` / `gradient_angle` / `gradient_radial`):
//!   * stops are `[colour, offset]` pairs, offset 0..1 along the gradient (values outside 0..1
//!     are allowed: the colour before the first / after the last stop is padded, as WPF does);
//!   * `angle` (linear) is the direction the gradient runs, in degrees clockwise from
//!     "left → right", in the element box's own (stretched) coordinates: 0 = left → right,
//!     90 = top → bottom (the default), 45 = top-left corner → bottom-right corner. Offset 0 is
//!     the box corner the direction starts from, 1 the opposite one.
//!   * `radial = [cx, cy, rx, ry]` (fractions of the box; WPF Center / RadiusX / RadiusY with
//!     the GradientOrigin at the centre): offset 0 at the centre, 1 on the ellipse, the last
//!     colour beyond it. Replaces `angle`.

use super::{Color, Fit};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// One colour stop: `[colour, offset]` in TOML.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    pub color: Color,
    pub offset: f32,
}

impl GradientStop {
    pub fn new(color: Color, offset: f32) -> GradientStop {
        GradientStop { color, offset }
    }
}

impl Serialize for GradientStop {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        (self.color, self.offset).serialize(s)
    }
}

impl<'de> Deserialize<'de> for GradientStop {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (color, offset) = <(Color, f32)>::deserialize(d)?;
        Ok(GradientStop { color, offset })
    }
}

/// A linear (any angle) or radial gradient with any number of stops.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Gradient {
    pub stops: Vec<GradientStop>,
    /// Linear direction, degrees clockwise from left → right (None = 90: top → bottom).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub angle: Option<f32>,
    /// Radial: [cx, cy, rx, ry] as fractions of the box.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radial: Option<[f32; 4]>,
}

impl Gradient {
    /// A WPF `RadialGradientBrush` (relative Center / RadiusX / RadiusY).
    #[cfg(test)]
    pub fn radial(stops: Vec<GradientStop>, center: [f32; 2], radius: [f32; 2]) -> Gradient {
        Gradient { stops, angle: None, radial: Some([center[0], center[1], radius[0], radius[1]]) }
    }

    /// Stops sorted by offset (stable), non-finite offsets dropped.
    pub fn sorted_stops(&self) -> Vec<GradientStop> {
        let mut v: Vec<GradientStop> = self.stops.iter().copied().filter(|s| s.offset.is_finite()).collect();
        v.sort_by(|a, b| a.offset.partial_cmp(&b.offset).unwrap_or(std::cmp::Ordering::Equal));
        v
    }

    /// The colour at `t` (padded before the first / after the last stop), interpolated
    /// channel by channel (straight alpha, as WPF).
    #[cfg(test)]
    pub fn color_at(&self, t: f32) -> Color {
        gradient_color_at(&self.sorted_stops(), t)
    }

    /// The stops' average colour (where a single colour has to stand in).
    pub fn average(&self) -> Color {
        let s = self.sorted_stops();
        if s.is_empty() {
            return Color::TRANSPARENT;
        }
        let n = 16;
        let mut acc = [0f32; 4];
        for i in 0..=n {
            let c = gradient_color_at(&s, i as f32 / n as f32).0;
            for k in 0..4 {
                acc[k] += c[k] as f32;
            }
        }
        Color(acc.map(|v| (v / (n + 1) as f32).round().clamp(0.0, 255.0) as u8))
    }

    /// Linear direction as a unit vector (box coordinates, y down).
    pub fn direction(&self) -> [f32; 2] {
        let a = self.angle.unwrap_or(90.0).to_radians();
        [a.cos(), a.sin()]
    }

    /// The gradient parameter at `(u, v)` (fractions of the box).
    pub fn param(&self, u: f32, v: f32) -> f32 {
        if let Some([cx, cy, rx, ry]) = self.radial {
            let dx = (u - cx) / rx.abs().max(1e-4);
            let dy = (v - cy) / ry.abs().max(1e-4);
            return (dx * dx + dy * dy).sqrt();
        }
        let [dx, dy] = self.direction();
        let l = (dx.abs() + dy.abs()).max(1e-6);
        ((u - 0.5) * dx + (v - 0.5) * dy) / l + 0.5
    }

    /// Fully transparent everywhere.
    pub fn is_invisible(&self) -> bool {
        self.stops.iter().all(|s| s.color.0[3] == 0)
    }
}

/// `Gradient::color_at` over already sorted stops.
pub fn gradient_color_at(stops: &[GradientStop], t: f32) -> Color {
    let Some(first) = stops.first() else {
        return Color::TRANSPARENT;
    };
    if t <= first.offset || stops.len() == 1 {
        return first.color;
    }
    for w in stops.windows(2) {
        let (a, b) = (w[0], w[1]);
        if t <= b.offset {
            let span = b.offset - a.offset;
            let f = if span > 1e-6 { (t - a.offset) / span } else { 1.0 };
            let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * f).round().clamp(0.0, 255.0) as u8;
            let (p, q) = (a.color.0, b.color.0);
            return Color([l(p[0], q[0]), l(p[1], q[1]), l(p[2], q[2]), l(p[3], q[3])]);
        }
    }
    stops[stops.len() - 1].color
}

/// A fill: one colour (`"#rrggbb"`) or a gradient (`{ stops = [...], angle = 90 }`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Brush {
    Solid(Color),
    Gradient(Gradient),
}

impl Brush {
    /// The colour that stands in where only one can be drawn.
    pub fn average(&self) -> Color {
        match self {
            Brush::Solid(c) => *c,
            Brush::Gradient(g) => g.average(),
        }
    }

    /// The gradient, when this brush is one (with at least two stops).
    pub fn gradient(&self) -> Option<&Gradient> {
        match self {
            Brush::Gradient(g) if g.stops.len() >= 2 => Some(g),
            _ => None,
        }
    }
}

/// A soft glow around a focused item (WPF `DropShadowEffect` with no offset).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Glow {
    /// Glow colour; its alpha is the glow's strength.
    pub color: Color,
    /// How far the glow reaches past the focus border, fraction of the screen height
    /// (WPF BlurRadius px / 1080).
    #[serde(default = "default_glow_blur")]
    pub blur: f32,
}

fn default_glow_blur() -> f32 {
    0.02
}

// ---------------------------------------------------------------------------- grid tiles

/// Where a grid's selector is drawn relative to the focused item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TileLayer {
    /// Under the item's background.
    Bottom,
    /// Over the background, under the picture / name.
    Middle,
    /// Over everything.
    #[default]
    Top,
}

/// Grid tiles (`Element::Grid::tiles`): a background behind every item, a selector
/// on the focused one, and pictures smaller than the item box. Sizes are fractions of the item
/// box; colours of pictures multiply them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GridTiles {
    /// Picture behind every item, stretched over its box (theme file).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_image: Option<String>,
    /// Box behind every item; with `background_image`, the picture's tint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_color: Option<Color>,
    pub background_scale: f32,
    /// Picture on the focused item (theme file); without one, a box in `selector_color`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector_color: Option<Color>,
    pub selector_scale: f32,
    pub selector_layer: TileLayer,
    /// Corner radius of the background and selector boxes, fraction of the screen height.
    pub corner_radius: f32,
    /// Size of the item's picture in its box.
    pub image_scale: f32,
    /// Tint of the pictures, and of the focused item's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_color: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_image_color: Option<Color>,
    /// Name colour of the focused item (others use `item_text_style`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_text_color: Option<Color>,
    /// Card behind the focused item's name when it has no picture (others use the grid's
    /// `item_background`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_text_background: Option<Color>,
}

impl Default for GridTiles {
    fn default() -> Self {
        GridTiles {
            background_image: None,
            background_color: None,
            background_scale: 1.0,
            selector_image: None,
            selector_color: None,
            selector_scale: 1.0,
            selector_layer: TileLayer::Top,
            corner_radius: 0.0,
            image_scale: 1.0,
            image_color: None,
            selected_image_color: None,
            selected_text_color: None,
            selected_text_background: None,
        }
    }
}

// ---------------------------------------------------------------------------- desktop

/// Where the desktop grid draws a game's name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoverTitle {
    /// No names (a tooltip shows it).
    None,
    /// Under the cover (Lodestar's default).
    #[default]
    Under,
    /// Over the bottom of the cover, on a dark strip.
    Overlay,
    /// Over the cover only while the pointer is on it.
    Hover,
}

/// The border around the selected cover in the desktop grid.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectionBorder {
    /// None = the palette's `selection`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    /// Points (default 3).
    #[serde(default = "default_selection_width")]
    pub width: f32,
}

fn default_selection_width() -> f32 {
    3.0
}

/// A picture behind the whole desktop window (Playnite's window background image).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BackgroundPicture {
    /// Theme file (relative to the theme folder).
    pub path: String,
    #[serde(default = "default_bg_fit")]
    pub fit: Fit,
    /// How much it is darkened (0 = not at all, 1 = black).
    #[serde(default)]
    pub dim: f32,
}

fn default_bg_fit() -> Fit {
    Fit::Cover
}

/// Desktop text sizes. `normal` scales every text (1 = Lodestar's 14 pt body); the others are
/// relative to `normal`, as Playnite's FontSizeSmall … FontSizeLargest are to FontSize.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FontScale {
    pub normal: f32,
    /// Small print (default 11.5 / 14).
    pub small: f32,
    /// Slightly larger text (default 15 / 14).
    pub large: f32,
    /// Headings (default 20 / 14).
    pub larger: f32,
    /// A game's title on its page (default 26 / 14).
    pub largest: f32,
}

impl Default for FontScale {
    fn default() -> Self {
        FontScale { normal: 1.0, small: 11.5 / 14.0, large: 15.0 / 14.0, larger: 20.0 / 14.0, largest: 26.0 / 14.0 }
    }
}

impl FontScale {
    /// Points for a size `ratio` (relative to normal) at Lodestar's 14 pt base, kept readable.
    pub fn pt(&self, ratio: f32) -> f32 {
        let n = if self.normal.is_finite() { self.normal.clamp(0.6, 2.0) } else { 1.0 };
        let r = if ratio.is_finite() { ratio.clamp(0.5, 4.0) } else { 1.0 };
        14.0 * n * r
    }
}

/// How the desktop UI uses a theme beyond its palette. Every field beyond the first three is
/// optional; unset = Lodestar's own look.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopStyle {
    /// Show the selected game's background art, blurred, behind the whole window (Playnite's
    /// "background image" look). The user can force it on/off in settings.
    pub background_image: bool,
    /// How much the background is darkened (0 = not at all, 1 = black).
    pub background_dim: f32,
    /// Opacity of the panels over the background (0..1). The game grid area uses less.
    pub panel_opacity: f32,
    /// Buttons, fields, checkboxes: corner radius in points (default 5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_radius: Option<f32>,
    /// Dialogs and windows: corner radius in points (default 8; menus follow at 3/4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_radius: Option<f32>,
    /// Covers in the grid and on game pages: corner radius in points (default 6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_radius: Option<f32>,
    /// Gap between grid covers in points (default: a tenth of the cover width, 10–22).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_gap: Option<f32>,
    /// Where game names go in the grid (default under; the user's "Show game names under
    /// covers" switch still hides them).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_title: Option<CoverTitle>,
    /// Border of the selected cover (default the palette's selection colour, 3 pt).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_border: Option<SelectionBorder>,
    /// Hovered covers darken by this much (0..1) instead of lightening a little.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hover_darken: Option<f32>,
    /// Window background behind everything (default the palette's background).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_gradient: Option<Brush>,
    /// Left panel (default the palette's surface).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidebar_fill: Option<Brush>,
    /// Top bar and status bar (default a shade darker than the background).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topbar_fill: Option<Brush>,
    /// The game details panel and page (default the palette's surface / background).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details_fill: Option<Brush>,
    /// A theme picture behind the whole window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_picture: Option<BackgroundPicture>,
    /// Extra darkening of the game picture at the top of game pages (0..1; default none).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details_background_dim: Option<f32>,
    /// How much of that picture fades into the page, from the bottom (0..1; default 0.65).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details_background_fade: Option<f32>,
    /// Text sizes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_scale: Option<FontScale>,
}

impl Default for DesktopStyle {
    fn default() -> Self {
        DesktopStyle {
            background_image: false,
            background_dim: 0.55,
            panel_opacity: 0.78,
            control_radius: None,
            window_radius: None,
            cover_radius: None,
            cover_gap: None,
            cover_title: None,
            selection_border: None,
            hover_darken: None,
            window_gradient: None,
            sidebar_fill: None,
            topbar_fill: None,
            details_fill: None,
            background_picture: None,
            details_background_dim: None,
            details_background_fade: None,
            font_scale: None,
        }
    }
}

/// Points → a corner radius egui takes (0..255).
pub fn radius_u8(pt: f32) -> u8 {
    if pt.is_finite() { pt.round().clamp(0.0, 255.0) as u8 } else { 0 }
}

impl DesktopStyle {
    pub fn control_radius(&self) -> u8 {
        self.control_radius.map_or(5, radius_u8)
    }
    pub fn window_radius(&self) -> u8 {
        self.window_radius.map_or(8, radius_u8)
    }
    pub fn menu_radius(&self) -> u8 {
        self.window_radius.map_or(6, |r| radius_u8(r * 0.75))
    }
    pub fn cover_radius(&self) -> u8 {
        self.cover_radius.map_or(6, radius_u8)
    }
    pub fn cover_title(&self) -> CoverTitle {
        self.cover_title.unwrap_or_default()
    }
    /// A window background or picture is drawn behind the panels.
    pub fn has_backdrop(&self) -> bool {
        self.window_gradient.as_ref().and_then(Brush::gradient).is_some() || self.background_picture.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: &str) -> Color {
        Color::parse(s).unwrap()
    }

    #[test]
    fn stops_parse_as_colour_offset_pairs() {
        #[derive(Deserialize, Serialize)]
        struct T {
            g: Gradient,
            b: Brush,
            s: Brush,
        }
        let t: T = toml::from_str(
            r##"
s = "#102030"
[g]
stops = [["#ff0000", 0.0], ["#00ff0080", 0.4], ["#0000ff", 1.0]]
angle = 45
[b]
stops = [["#000000", 0.0], ["#ffffff", 1.0]]
radial = [0.5, 0.5, 0.6, 0.4]
"##,
        )
        .unwrap();
        assert_eq!(t.g.stops.len(), 3);
        assert_eq!(t.g.stops[1], GradientStop::new(c("#00ff0080"), 0.4));
        assert_eq!(t.g.angle, Some(45.0));
        assert_eq!(t.s, Brush::Solid(c("#102030")));
        assert_eq!(t.b.gradient().unwrap().radial, Some([0.5, 0.5, 0.6, 0.4]));
        let again: T = toml::from_str(&toml::to_string(&t).unwrap()).unwrap();
        assert_eq!(again.g, t.g);
        assert_eq!(again.b, t.b);
        assert_eq!(again.s, t.s);
    }

    #[test]
    fn colour_at_pads_and_interpolates() {
        let g = Gradient {
            stops: vec![GradientStop::new(c("#000000"), 0.2), GradientStop::new(c("#ffffff"), 0.6)],
            ..Default::default()
        };
        assert_eq!(g.color_at(0.0), c("#000000"));
        assert_eq!(g.color_at(1.0), c("#ffffff"));
        assert_eq!(g.color_at(0.3), c("#404040"));
        // Unsorted stops are sorted.
        let g2 = Gradient { stops: vec![g.stops[1], g.stops[0]], ..Default::default() };
        assert_eq!(g2.color_at(0.3), c("#404040"));
        assert_eq!(Gradient::default().color_at(0.5), Color::TRANSPARENT);
    }

    #[test]
    fn params_follow_angle_and_radial() {
        let down = Gradient { angle: None, ..Default::default() };
        assert!((down.param(0.3, 0.0) - 0.0).abs() < 1e-5);
        assert!((down.param(0.3, 1.0) - 1.0).abs() < 1e-5);
        let right = Gradient { angle: Some(0.0), ..Default::default() };
        assert!((right.param(0.25, 0.9) - 0.25).abs() < 1e-5);
        let diag = Gradient { angle: Some(45.0), ..Default::default() };
        assert!((diag.param(0.0, 0.0)).abs() < 1e-5);
        assert!((diag.param(1.0, 1.0) - 1.0).abs() < 1e-5);
        assert!((diag.param(1.0, 0.0) - 0.5).abs() < 1e-5);
        let rad = Gradient::radial(vec![], [0.5, 0.5], [0.5, 0.25]);
        assert!(rad.param(0.5, 0.5).abs() < 1e-5);
        assert!((rad.param(1.0, 0.5) - 1.0).abs() < 1e-5);
        assert!((rad.param(0.5, 0.75) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn desktop_defaults_are_todays_look() {
        let d = DesktopStyle::default();
        assert_eq!((d.control_radius(), d.window_radius(), d.menu_radius(), d.cover_radius()), (5, 8, 6, 6));
        assert_eq!(d.cover_title(), CoverTitle::Under);
        assert!(!d.has_backdrop());
        let f = FontScale::default();
        assert_eq!(f.pt(1.0), 14.0);
        assert!((f.pt(f.small) - 11.5).abs() < 1e-4 && (f.pt(f.larger) - 20.0).abs() < 1e-4);
        // A theme's desktop section parses; unset fields stay unset.
        let d: DesktopStyle = toml::from_str(
            r##"
control_radius = 2
cover_title = "overlay"
selection_border = { color = "#ff8800", width = 2 }
hover_darken = 0.3
sidebar_fill = "#101010"
topbar_fill = { stops = [["#000000", 0.0], ["#202020", 1.0]], angle = 0 }
background_picture = { path = "images/bg.png", dim = 0.4 }
details_background_dim = 0.5
details_background_fade = 0.8
font_scale = { normal = 1.1 }
"##,
        )
        .unwrap();
        assert_eq!(d.control_radius(), 2);
        assert_eq!(d.cover_title(), CoverTitle::Overlay);
        assert_eq!(d.selection_border.unwrap().width, 2.0);
        assert!(matches!(d.sidebar_fill, Some(Brush::Solid(_))));
        assert!(d.topbar_fill.as_ref().unwrap().gradient().is_some());
        assert_eq!(d.background_picture.as_ref().unwrap().fit, Fit::Cover);
        assert!(d.has_backdrop());
        assert!((d.font_scale.unwrap().pt(1.0) - 15.4).abs() < 1e-3);
        assert_eq!(d.window_radius(), 8);
    }
}
