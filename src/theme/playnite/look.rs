//! `PlayniteLook`: everything the resolver (`tree.rs` + `extract.rs`) reads out of a Playnite
//! theme laid over Playnite's own default theme, as plain typed values. The importer (`mod.rs`)
//! turns it into a Lodestar theme; nothing here knows Lodestar's theme model.
//!
//! Units: Playnite Fullscreen views are designed at 1920×1080, so every length in the
//! fullscreen parts is in those design pixels. Desktop lengths are WPF device-independent
//! pixels (≈ egui points). `None` / empty means "the theme (and Playnite's default) has
//! nothing usable here": keep Lodestar's own look for it.

use super::Mode;
use crate::theme::Color;
use serde::Serialize;

/// A WPF brush with everything a renderer needs to draw it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum LookBrush {
    Solid {
        color: Color,
    },
    /// `LinearGradientBrush`: stops sorted by offset (the brush's Opacity already folded into
    /// the alpha), `start` / `end` as WPF writes them (fractions of the box; may lie outside
    /// it, e.g. `-0.5,0.5`). Use `angle()` and `box_stops()` for a box-filling gradient.
    Linear {
        stops: Vec<(Color, f32)>,
        start: [f32; 2],
        end: [f32; 2],
        /// `MappingMode="Absolute"`: start / end are pixels, not fractions (rare).
        absolute: bool,
    },
    /// `RadialGradientBrush`: `center` / `radius` as fractions of the box, `origin` = the
    /// GradientOrigin (focus point).
    Radial {
        stops: Vec<(Color, f32)>,
        center: [f32; 2],
        origin: [f32; 2],
        radius: [f32; 2],
    },
    /// `ImageBrush` / `BitmapImage`: `file` = the theme file it names (resolved against the
    /// theme's file list; None: not in the theme, e.g. a game binding or a missing file),
    /// `source` = the raw XAML value.
    Image {
        file: Option<String>,
        source: String,
        stretch: Stretch,
        opacity: f32,
    },
}

impl LookBrush {
    pub fn solid(color: Color) -> LookBrush {
        LookBrush::Solid { color }
    }

    pub fn stops(&self) -> &[(Color, f32)] {
        match self {
            LookBrush::Linear { stops, .. } | LookBrush::Radial { stops, .. } => stops,
            _ => &[],
        }
    }

    /// The brush drawn at an element Opacity (`o` multiplies every alpha).
    pub fn with_opacity(self, o: f32) -> LookBrush {
        let o = o.clamp(0.0, 1.0);
        let fade = |c: Color| Color([c.0[0], c.0[1], c.0[2], (c.0[3] as f32 * o).round() as u8]);
        match self {
            LookBrush::Solid { color } => LookBrush::Solid { color: fade(color) },
            LookBrush::Linear { stops, start, end, absolute } => LookBrush::Linear {
                stops: stops.into_iter().map(|(c, p)| (fade(c), p)).collect(),
                start,
                end,
                absolute,
            },
            LookBrush::Radial { stops, center, origin, radius } => LookBrush::Radial {
                stops: stops.into_iter().map(|(c, p)| (fade(c), p)).collect(),
                center,
                origin,
                radius,
            },
            LookBrush::Image { file, source, stretch, opacity } => {
                LookBrush::Image { file, source, stretch, opacity: opacity * o }
            }
        }
    }

    pub fn is_gradient(&self) -> bool {
        matches!(self, LookBrush::Linear { .. } | LookBrush::Radial { .. })
    }

    /// One colour standing for the brush: what the eye sees on average over the (unit) box it
    /// paints. A gradient is sampled over the box with its geometry, so a brush whose last
    /// stop lies far outside the box (WPF lets offsets and points run past it) or only reaches
    /// one corner doesn't tint the whole colour. Absolute-mapped gradients: over their stops.
    pub fn average(&self) -> Option<Color> {
        match self {
            LookBrush::Solid { color } => Some(*color),
            LookBrush::Linear { stops, absolute: true, .. } => super::xaml::gradient_average(stops),
            LookBrush::Linear { stops, start, end, .. } => {
                let (dx, dy) = (end[0] - start[0], end[1] - start[1]);
                let len2 = dx * dx + dy * dy;
                if len2 < 1e-9 {
                    return super::xaml::gradient_average(stops);
                }
                area_average(stops, |x, y| ((x - start[0]) * dx + (y - start[1]) * dy) / len2)
            }
            LookBrush::Radial { stops, center, radius, .. } => {
                let (rx, ry) = (radius[0].abs().max(1e-4), radius[1].abs().max(1e-4));
                area_average(stops, |x, y| {
                    let (u, v) = ((x - center[0]) / rx, (y - center[1]) / ry);
                    (u * u + v * v).sqrt()
                })
            }
            LookBrush::Image { .. } => None,
        }
    }

    /// Fully transparent (a "nothing" brush such as `Transparent`).
    pub fn is_clear(&self) -> bool {
        match self {
            LookBrush::Solid { color } => color.0[3] == 0,
            LookBrush::Linear { stops, .. } | LookBrush::Radial { stops, .. } => stops.iter().all(|(c, _)| c.0[3] == 0),
            LookBrush::Image { .. } => false,
        }
    }

    /// Direction of a linear gradient in degrees, screen coordinates, clockwise:
    /// 0 = left → right, 90 = top → bottom, 180 = right → left, 270 = bottom → top.
    /// (Measured on a square box; WPF's relative points stretch with the box.)
    pub fn angle(&self) -> Option<f32> {
        let LookBrush::Linear { start, end, .. } = self else {
            return None;
        };
        let (dx, dy) = (end[0] - start[0], end[1] - start[1]);
        if dx.abs() < 1e-6 && dy.abs() < 1e-6 {
            return Some(90.0);
        }
        Some(dy.atan2(dx).to_degrees().rem_euclid(360.0))
    }

    /// The stops of a linear gradient re-spread so offset 0..1 runs exactly across the (unit)
    /// box along `angle()`: 0 at the box corner the gradient starts from, 1 at the opposite one
    /// (CSS `linear-gradient(angle)` semantics). WPF lets start / end lie inside or outside the
    /// box and pads beyond the last stops; that's folded in, with colours interpolated at 0 / 1.
    /// Radial gradients: their stops unchanged. Solid / image: empty.
    pub fn box_stops(&self) -> Vec<(Color, f32)> {
        let LookBrush::Linear { stops, start, end, .. } = self else {
            return self.stops().to_vec();
        };
        if stops.is_empty() {
            return vec![];
        }
        let (dx, dy) = (end[0] - start[0], end[1] - start[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-6 {
            return stops.clone();
        }
        let (ux, uy) = (dx / len, dy / len);
        // Projection of the box onto the direction: centre ± half extent.
        let half = (ux.abs() + uy.abs()) / 2.0;
        let c = 0.5 * ux + 0.5 * uy;
        let s0 = start[0] * ux + start[1] * uy;
        // WPF offset o sits at s0 + o·len along the axis; the box spans [c-half, c+half].
        let to_box = |o: f32| ((s0 + o * len) - (c - half)) / (2.0 * half).max(1e-6);
        let from_box = |t: f32| ((c - half) + t * 2.0 * half - s0) / len;
        let mut out: Vec<(Color, f32)> = vec![];
        out.push((color_at(stops, from_box(0.0)), 0.0));
        for (col, o) in stops {
            let t = to_box(*o);
            if t > 0.0 && t < 1.0 {
                out.push((*col, t));
            }
        }
        out.push((color_at(stops, from_box(1.0)), 1.0));
        out
    }

    /// A radial gradient as `[cx, cy, rx, ry]` (fractions of the box).
    pub fn radial(&self) -> Option<[f32; 4]> {
        match self {
            LookBrush::Radial { center, radius, .. } => Some([center[0], center[1], radius[0], radius[1]]),
            _ => None,
        }
    }

    /// Alpha (0–1) of the brush at a point of the unit box (for opacity masks).
    pub fn alpha_at(&self, x: f32, y: f32) -> f32 {
        let a = |c: Color| c.0[3] as f32 / 255.0;
        match self {
            LookBrush::Solid { color } => a(*color),
            LookBrush::Image { opacity, .. } => *opacity,
            LookBrush::Linear { stops, start, end, .. } => {
                let (dx, dy) = (end[0] - start[0], end[1] - start[1]);
                let l2 = (dx * dx + dy * dy).max(1e-9);
                let o = ((x - start[0]) * dx + (y - start[1]) * dy) / l2;
                a(color_at(stops, o))
            }
            LookBrush::Radial { stops, center, radius, .. } => {
                let nx = (x - center[0]) / radius[0].max(1e-6);
                let ny = (y - center[1]) / radius[1].max(1e-6);
                a(color_at(stops, (nx * nx + ny * ny).sqrt()))
            }
        }
    }
}

/// The colour of a stop list at offset `o` (WPF pads beyond the first / last stop).
/// The average of `color_at(stops, offset(x, y))` over the unit box (16 × 16 samples).
fn area_average(stops: &[(Color, f32)], offset: impl Fn(f32, f32) -> f32) -> Option<Color> {
    if stops.is_empty() {
        return None;
    }
    const N: usize = 16;
    let mut sum = [0f32; 4];
    for j in 0..N {
        for i in 0..N {
            let (x, y) = ((i as f32 + 0.5) / N as f32, (j as f32 + 0.5) / N as f32);
            let c = color_at(stops, offset(x, y));
            for (k, v) in sum.iter_mut().enumerate() {
                *v += c.0[k] as f32;
            }
        }
    }
    let n = (N * N) as f32;
    Some(Color(sum.map(|v| (v / n).round().clamp(0.0, 255.0) as u8)))
}

pub fn color_at(stops: &[(Color, f32)], o: f32) -> Color {
    let Some(first) = stops.first() else {
        return Color::TRANSPARENT;
    };
    if o <= first.1 {
        return first.0;
    }
    for w in stops.windows(2) {
        let (a, b) = (w[0], w[1]);
        if o <= b.1 {
            let k = ((o - a.1) / (b.1 - a.1).max(1e-6)).clamp(0.0, 1.0);
            let ch = |i: usize| (a.0.0[i] as f32 + (b.0.0[i] as f32 - a.0.0[i] as f32) * k).round() as u8;
            return Color([ch(0), ch(1), ch(2), ch(3)]);
        }
    }
    stops[stops.len() - 1].0
}

/// WPF `Stretch`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum Stretch {
    None,
    Fill,
    #[default]
    Uniform,
    UniformToFill,
}

impl Stretch {
    pub fn parse(s: &str) -> Option<Stretch> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "none" => Stretch::None,
            "fill" => Stretch::Fill,
            "uniform" => Stretch::Uniform,
            "uniformtofill" => Stretch::UniformToFill,
            _ => return None,
        })
    }
}

/// WPF Horizontal/VerticalAlignment (Left/Top = Start, Right/Bottom = End).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum Align {
    Start,
    Center,
    End,
    #[default]
    Stretch,
}

impl Align {
    pub fn parse(s: &str) -> Option<Align> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "left" | "top" => Align::Start,
            "center" => Align::Center,
            "right" | "bottom" => Align::End,
            "stretch" => Align::Stretch,
            _ => return None,
        })
    }
}

/// A border drawn around an item (focus / selection frame).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Frame {
    /// Thickness (the largest side), px.
    pub width: f32,
    pub brush: Option<LookBrush>,
    /// Corner radius (the largest corner), px.
    pub radius: f32,
    /// How far the frame sits outside the item (a negative Margin of -5 → 5), px.
    pub outset: f32,
}

/// A `DropShadowEffect` (glow when depth is 0).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Glow {
    pub color: Color,
    pub blur: f32,
    pub opacity: f32,
    /// ShadowDepth (0 = a glow all around).
    pub depth: f32,
}

/// An opacity mask: the brush and how opaque it is at the middle of each edge
/// (`[top, right, bottom, left]`, 0 = faded out), so "fades at the bottom" is
/// `edge_alpha[2] < 0.5`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Mask {
    pub brush: LookBrush,
    pub edge_alpha: [f32; 4],
}

/// When a cover caption shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Shown {
    /// Always.
    #[default]
    Always,
    /// When the user's "show game titles" setting is on (Playnite's `{Settings …}`).
    Setting,
    /// Only on the focused / selected item.
    Focus,
    /// Only while the mouse is over it (desktop).
    Hover,
}

/// The title strip of a cover.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Caption {
    pub shown: Shown,
    /// Drawn over the cover (true) or under it (false).
    pub overlay: bool,
    pub background: Option<LookBrush>,
    pub foreground: Option<LookBrush>,
    /// TextAlignment / HorizontalAlignment of the text.
    pub align: Option<Align>,
    /// VerticalAlignment inside the cover (End = bottom strip).
    pub valign: Option<Align>,
    pub font_size: Option<f32>,
    /// The resource key the size came from (FontSizeSmall …), if any.
    pub font_size_key: Option<String>,
}

/// The look of one game cover in a list (Fullscreen: ListGameItemStyle +
/// ListGameItemTemplate; desktop: GridViewItemStyle + GridViewItemTemplate).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ItemLook {
    /// The frame shown on the focused (fullscreen) / selected (desktop) item.
    pub focus_frame: Option<Frame>,
    /// A glow around the focused item (DropShadowEffect on the focus frame).
    pub focus_glow: Option<Glow>,
    /// Scale of the focused item (FocusedOn storyboard `To`, relative to the resting scale).
    pub focus_scale: Option<f32>,
    /// Opacity of items that aren't focused (1 / None = fully visible).
    pub unfocused_opacity: Option<f32>,
    /// A frame every item has, focused or not.
    pub frame: Option<Frame>,
    pub caption: Option<Caption>,
    /// The scrim over games that aren't installed ("BorderDarken").
    pub uninstalled_scrim: Option<Color>,
    /// A dark layer while the mouse is over the item (desktop "BorderMouseOver").
    pub hover_scrim: Option<Color>,
    /// Rounded cover corners (CornerRadius around / masking PART_ImageCover), px.
    pub cover_radius: Option<f32>,
    /// The cover has an OpacityMask (other than plain rounding).
    pub cover_mask: Option<Mask>,
    /// The item's fixed size, px (None: sized by Playnite's columns / rows setting).
    pub size: Option<[f32; 2]>,
    /// Space around each item (Margin of the item root: left, top, right, bottom), px.
    pub spacing: Option<[f32; 4]>,
    /// The fill behind each item (desktop GridItemBackgroundBrush).
    pub background: Option<LookBrush>,
}

/// A bar across the screen (Fullscreen top / bottom bar).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Bar {
    /// Height in design px (1080 high screen).
    pub height: f32,
    pub background: Option<LookBrush>,
}

/// Where and how the selected game's background picture (PART_ImageBackground) is drawn.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct BackgroundPicture {
    /// Spans the whole view (all rows / columns) rather than one area.
    pub full: bool,
    pub halign: Option<Align>,
    pub valign: Option<Align>,
    pub stretch: Option<Stretch>,
    pub opacity: Option<f32>,
    pub mask: Option<Mask>,
    /// BlurEffect radius, px.
    pub blur: Option<f32>,
    /// A layer drawn over the picture to darken it (colour incl. alpha).
    pub darken: Option<Color>,
    /// The picture's area as fractions of the view `[x, y, w, h]` when it isn't `full`.
    pub area: Option<[f32; 4]>,
}

/// The game art the desktop Library view draws behind everything (PART_ImageBackground).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct WindowArt {
    /// Its placement, opacity, blur and darkening.
    pub picture: BackgroundPicture,
    /// Gradient opacity masks on it and on the containers around it (they multiply).
    pub masks: Vec<LookBrush>,
    /// Translucent layers the theme draws right over it (any brush, element Opacity folded).
    pub layers: Vec<LookBrush>,
    /// It sits in a row below the top panel instead of behind it.
    pub below_top_bar: bool,
    /// The theme's window picture is drawn after it, over it.
    pub picture_over_art: bool,
}

/// A box drawn around the facts of a game page.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FactsBox {
    pub fill: LookBrush,
    /// Its border brush and thickness (the thickest side), px.
    pub border: Option<(LookBrush, f32)>,
    pub radius: Option<f32>,
}

/// How the details view's game list draws its rows (DetailsViewItemStyle).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ListRows {
    pub selected_fill: Option<LookBrush>,
    /// A bar along the left edge of the selected row: its brush and width, px.
    pub selected_bar: Option<(LookBrush, f32)>,
    pub selected_text: Option<LookBrush>,
    pub radius: Option<f32>,
    pub text: Option<LookBrush>,
    pub text_uninstalled: Option<LookBrush>,
}

/// The shape of the main game list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ListShape {
    /// Playnite's own: a cover grid of the user's rows / columns.
    #[default]
    Grid,
    /// One horizontal row of covers (PS5 / Xbox style).
    Row,
    /// A carousel driven by the selected index (a big centred item).
    Carousel,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ListLook {
    pub shape: ListShape,
    /// Why that shape was picked (for the report / coverage).
    pub evidence: Vec<String>,
    /// PART_ListGameItems Margin (left, top, right, bottom), design px.
    pub margin: Option<[f32; 4]>,
    /// Its fixed size, design px.
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub valign: Option<Align>,
    /// The list's area as fractions of the screen `[x, y, w, h]` (from the row grid).
    pub area: Option<[f32; 4]>,
}

/// Fullscreen Main view.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct MainLook {
    /// The whole view's background (PART_ViewHost Background, MainBackgourndBrush).
    pub background: Option<LookBrush>,
    pub top_bar: Option<Bar>,
    pub bottom_bar: Option<Bar>,
    pub picture: Option<BackgroundPicture>,
    pub list: ListLook,
    pub clock: bool,
    pub battery: bool,
    pub battery_percent: bool,
    /// The filter preset tabs (PART_FilterPresetSelector).
    pub filter_presets: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
}

/// Fullscreen GameDetails view.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DetailsLook {
    pub background: Option<LookBrush>,
    /// The panel / gradient over the background picture (GameDetailsBackgroundBrush).
    pub overlay: Option<LookBrush>,
    pub picture: Option<BackgroundPicture>,
    /// Which side the cover is on, relative to the title / text.
    pub cover_side: Option<Side>,
    /// The details view shows a logo (ExtraMetadataLoader hook or a logo binding).
    pub logo: bool,
    pub title_font: Option<FontRole>,
    pub title_size: Option<f32>,
    pub title_shadow: Option<Glow>,
    pub description_size: Option<f32>,
}

/// How the desktop grid shows a game's name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CoverTitle {
    #[default]
    None,
    /// Under the cover (Playnite's "show names under covers").
    Under,
    /// On the cover.
    Overlay,
    /// On the cover while hovered.
    Hover,
}

/// Desktop mode.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DesktopLook {
    /// ControlCornerRadius (buttons, boxes), px.
    pub control_radius: Option<f32>,
    /// The main window's corner radius (WindowBase / MainWindowStyle border), px.
    pub window_radius: Option<f32>,
    /// The grid view item: cover radius, selection frame, hover scrim, item fill, spacing.
    pub grid: ItemLook,
    pub cover_title: CoverTitle,
    pub window_background: Option<LookBrush>,
    pub top_panel: Option<LookBrush>,
    pub sidebar: Option<LookBrush>,
    /// The details panel's fill (DetailsViewGameOverview / GridViewGameOverview root).
    pub details_background: Option<LookBrush>,
    /// A picture the theme puts behind the whole window (theme file).
    pub background_picture: Option<LookBrush>,
    /// The selected game's background picture in the details view (opacity, mask, darken).
    pub details_picture: Option<BackgroundPicture>,
    /// The selected game's art behind the whole window (the Library view's
    /// PART_ImageBackground), with its darkening, masks and the layers over it.
    pub window_art: Option<WindowArt>,
    /// The details view's fill (LibraryDetailsView's root: under the list and the game).
    pub game_list: Option<LookBrush>,
    /// The game list column's own fill (the panel around PART_ListGames, or the list's).
    pub list_fill: Option<LookBrush>,
    /// A strip the grid item paints behind the game's name only.
    pub caption_fill: Option<LookBrush>,
    /// The box around the facts on the game page (DetailsViewGameOverview).
    pub facts_box: Option<FactsBox>,
    /// The game list's rows (DetailsViewItemStyle).
    pub rows: Option<ListRows>,
    pub panel_separator: Option<LookBrush>,
    /// The details view title (PART_TextDisplayName) size.
    pub title_size: Option<f32>,
    /// What the Play button (`PlayButton` style) is filled with: the theme's primary colour.
    pub play_button: Option<LookBrush>,
}

/// A font as a theme names it.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct FontRole {
    /// The FontFamily text as written ("Segoe UI", "./Fonts/#Comfortaa", "Fonts/#Lato Light").
    pub value: Option<String>,
    /// A Playnite font key it came from (FontTitilliumWebBold …), when no text was found.
    pub key: Option<String>,
    /// The family name (from the value or the key).
    pub family: Option<String>,
    /// CSS weight (FontWeight setter, or the key / face name's style).
    pub weight: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct LookFonts {
    /// Body text (TextBlockBaseStyle / BaseTextBlockStyle / FontFamily).
    pub body: Option<FontRole>,
    /// Titles (game name in details, FontTitilliumWebBold-style keys).
    pub title: Option<FontRole>,
}

/// FontSizeSmall … FontSizeLargest (desktop has all five, Fullscreen Small + normal).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct FontSizes {
    pub small: Option<f32>,
    pub normal: Option<f32>,
    pub large: Option<f32>,
    pub larger: Option<f32>,
    pub largest: Option<f32>,
}

/// Playnite add-ons the theme hooks into.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Plugins {
    /// Add-on names with how often the theme refers to them, most used first.
    pub names: Vec<(String, usize)>,
    /// The theme shows game logos (ExtraMetadataLoader logo controls / logo bindings).
    pub logos: bool,
    /// The theme shows per-source banners (ThemeExtras banners / themeExtras.yaml).
    pub banners: bool,
    /// Ratings / favourite / completion widgets from ThemeExtras.
    pub ratings: bool,
    /// Total PluginSettings / PluginStatus / PluginConverter uses.
    pub uses: usize,
}

/// One user option from `thememodifier.yaml` with its current (default) value.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ThemeOption {
    /// The Constants key it flips.
    pub key: String,
    pub label: String,
    /// The section heading it is listed under.
    pub section: String,
    pub kind: OptionKind,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum OptionKind {
    Switch { default: Option<bool> },
    Range { min: f64, max: f64, default: Option<f64> },
}

/// Which Playnite sound a theme file is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SoundRole {
    /// `audio/navigation.*`: moving the selection (Lodestar "scroll").
    Navigation,
    /// `audio/activation.*`: confirming / opening (Lodestar "select").
    Activation,
    /// Any other sound the theme plays from its own XAML (name = the file stem).
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ThemeSound {
    pub role: SoundRole,
    /// Theme file.
    pub file: String,
    pub bytes: u64,
}

/// What a theme video is used for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoRole {
    /// A looping full-view background.
    Background,
    /// A start-up / intro clip (played once).
    Intro,
    /// Anything else (a small decoration, a panel).
    Other,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ThemeVideo {
    /// Theme file.
    pub file: String,
    pub bytes: u64,
    pub role: VideoRole,
    /// The view it is in ("Main", "GameDetails" … = the XAML file's name).
    pub view: String,
    pub repeat: bool,
    pub muted: bool,
    pub stretch: Option<Stretch>,
    pub opacity: Option<f32>,
}

/// The theme's videos and sounds (Playnite Fullscreen plays `audio/background.*` as music and
/// `audio/navigation.*` / `audio/activation.*` as interface sounds by itself).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct MediaLook {
    /// MediaElement videos found in the theme's XAML, best background first.
    pub videos: Vec<ThemeVideo>,
    /// Video files in the theme that no XAML names (or only through an add-on).
    pub unused_videos: Vec<String>,
    /// Background music (theme files, non-empty).
    pub music: Vec<String>,
    pub sounds: Vec<ThemeSound>,
    /// Total size of everything above, bytes (the importer asks before copying it).
    pub bytes: u64,
}

/// What the resolver understood, for the coverage report.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Coverage {
    pub theme_files: usize,
    pub default_files: usize,
    pub nodes: usize,
    pub keyed_resources: usize,
    pub implicit_styles: usize,
    pub templates: usize,
    pub triggers: usize,
    pub data_triggers: usize,
    pub storyboards: usize,
    pub media_elements: usize,
    pub visual_brushes: usize,
    pub effects: usize,
    pub opacity_masks: usize,
    /// Files skipped (too big, over the file / node caps, not XML), with why.
    pub skipped: Vec<String>,
    /// XAML files that failed to parse.
    pub broken: Vec<String>,
    /// The theme's ThemeApiVersion (theme.yaml), if given.
    pub api_version: Option<String>,
}

/// Everything read from one theme.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PlayniteLook {
    pub mode: Mode,
    /// Fullscreen: the cover in the game list.
    pub item: ItemLook,
    pub main: MainLook,
    pub details: DetailsLook,
    /// Desktop mode (filled for desktop themes; fullscreen themes leave it default).
    pub desktop: DesktopLook,
    pub fonts: LookFonts,
    pub font_sizes: FontSizes,
    pub plugins: Plugins,
    pub options: Vec<ThemeOption>,
    pub media: MediaLook,
    pub coverage: Coverage,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(r: u8, a: u8) -> Color {
        Color([r, 0, 0, a])
    }

    #[test]
    fn linear_angles_are_clockwise_from_the_right() {
        let lin = |s: [f32; 2], e: [f32; 2]| LookBrush::Linear {
            stops: vec![(c(0, 255), 0.0), (c(255, 255), 1.0)],
            start: s,
            end: e,
            absolute: false,
        };
        assert_eq!(lin([0.0, 0.5], [1.0, 0.5]).angle(), Some(0.0));
        assert_eq!(lin([0.5, 0.0], [0.5, 1.0]).angle(), Some(90.0));
        assert_eq!(lin([1.0, 0.5], [0.0, 0.5]).angle(), Some(180.0));
        assert_eq!(lin([0.5, 1.0], [0.5, 0.0]).angle(), Some(270.0));
        assert!((lin([0.0, 0.0], [1.0, 1.0]).angle().unwrap() - 45.0).abs() < 1e-4);
    }

    #[test]
    fn averages_follow_the_box_the_brush_paints() {
        // Grey over most of the box, red only past its bottom-right corner (offset 1.5).
        let grey = Color([0x30, 0x30, 0x30, 255]);
        let red = Color([0x80, 0, 0, 255]);
        let b = LookBrush::Linear {
            stops: vec![(grey, 0.0), (red, 1.5)],
            start: [0.6, 0.8],
            end: [1.0, 1.0],
            absolute: false,
        };
        let a = b.average().unwrap();
        assert!(a.0[0] < 0x3c && a.0[1] >= 0x2a, "{a:?}");
        // A plain top-to-bottom gradient: the middle colour.
        let v = LookBrush::Linear {
            stops: vec![(grey, 0.0), (red, 1.0)],
            start: [0.5, 0.0],
            end: [0.5, 1.0],
            absolute: false,
        };
        let m = v.average().unwrap();
        assert!((m.0[0] as i32 - 0x58).abs() <= 1 && (m.0[1] as i32 - 0x18).abs() <= 1, "{m:?}");
        // Radial: the centre colour dominates a big ellipse.
        let r = LookBrush::Radial {
            stops: vec![(grey, 0.0), (red, 1.0)],
            center: [0.5, 0.5],
            origin: [0.5, 0.5],
            radius: [4.0, 4.0],
        };
        assert!(r.average().unwrap().0[0] < 0x40);
    }

    #[test]
    fn box_stops_fold_in_points_outside_the_box() {
        // Playnite's TopBarBackgroundBrush: StartPoint -0.5,0.5 → EndPoint 1,0.5. At the box's
        // left edge (x = 0) a third of the way along: colour 1/3 of the way.
        let b = LookBrush::Linear {
            stops: vec![(c(0, 255), 0.0), (c(255, 255), 1.0)],
            start: [-0.5, 0.5],
            end: [1.0, 0.5],
            absolute: false,
        };
        let s = b.box_stops();
        assert_eq!(s.first().unwrap().1, 0.0);
        assert_eq!(s.last().unwrap().1, 1.0);
        assert_eq!(s.first().unwrap().0.0[0], 85);
        assert_eq!(s.last().unwrap().0.0[0], 255);
        // A gradient that ends inside the box pads: offset 1 at x = 0.5.
        let b = LookBrush::Linear {
            stops: vec![(c(0, 255), 0.0), (c(255, 255), 1.0)],
            start: [0.5, 0.0],
            end: [0.5, 0.5],
            absolute: false,
        };
        let s = b.box_stops();
        assert_eq!(s, vec![(c(0, 255), 0.0), (c(255, 255), 0.5), (c(255, 255), 1.0)]);
    }

    #[test]
    fn mask_alpha_follows_the_brush() {
        let b = LookBrush::Linear {
            stops: vec![(c(0, 255), 0.0), (c(0, 0), 1.0)],
            start: [0.5, 0.0],
            end: [0.5, 1.0],
            absolute: false,
        };
        assert!(b.alpha_at(0.5, 0.0) > 0.99);
        assert!(b.alpha_at(0.5, 1.0) < 0.01);
        let r = LookBrush::Radial {
            stops: vec![(c(0, 255), 0.3), (c(0, 0), 1.0)],
            center: [0.5, 0.0],
            origin: [0.5, 0.0],
            radius: [2.0, 0.9],
        };
        assert!(r.alpha_at(0.5, 0.0) > 0.99);
        assert!(r.alpha_at(0.5, 1.0) < 0.01);
    }
}
