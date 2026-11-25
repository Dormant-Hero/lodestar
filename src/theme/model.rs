//! Lodestar theme format (`theme.toml`). This is the contract between the theme renderer,
//! the built-in themes and the Playnite theme importer.
//!
//! Layout model: a theme is a set of views, each a flat list of positioned elements whose
//! sizes are fractions of the screen, so one theme fits any resolution.
//!   * Views: `collections` (pick a collection), `games` (the game list / grid of one
//!     collection), `details` (one game, optional; the controller UI falls back to a built-in
//!     layout drawn with the palette when absent) and the optional `home` page (below).
//!   * Each view is a list of elements drawn in `z` order (ties: file order).
//!   * `pos`, `size` are fractions of the screen (0..1). `origin` is the anchor inside the
//!     element (0,0 = top-left, 0.5,0.5 = centre). A size component of 0 means "auto"
//!     (keep aspect ratio from the other component / natural text size).
//!   * `font_size` is a fraction of the screen height.
//!
//! Strings may contain bindings in braces, resolved at render time:
//!   game.name game.description game.developer game.publisher game.genre game.year
//!   game.playtime game.lastplayed game.playcount game.source game.rating (0..1)
//!   game.favorite ("true"/"") game.installed
//!   art.cover art.hero art.logo art.icon art.banner   (absolute paths; "" if missing)
//!   collection.name collection.key collection.count
//!   theme.dir (absolute theme directory)
//! Added bindings (all optional to use):
//!   game.playtimelabel ("12h 30m played" / "")  game.lastplayedlabel ("Last played today" / "")
//!   game.favoritelabel ("★ Favourite" / "")  game.played ("true" when ever played)
//!   game.status game.platform game.tags game.size game.added game.id game.letter game.sourcekey
//!   collection.countlabel ("12 games")  collection.cover / .hero / .logo / .banner / .icon
//!     (art of a representative, recently played game of the collection)
//!   list.index (1-based selection) list.count
//!   game.controller ("full" / "partial" / "none" / "": unknown)  game.controllerlabel
//!     ("Full controller support" …)  game.controllershort ("Full support" / "Partial support" /
//!     "No support")  game.deck ("verified" / "playable" / "unsupported" / "")
//!     game.decklabel ("Deck Verified" …)  game.deckshort ("Verified" …)
//!   game.completion ("Beaten" / "": not set)  game.completionkey ("not_played" "plan_to_play"
//!     "playing" "on_hold" "beaten" "completed" "abandoned" / "")  game.score (user score
//!     "0".."100" / ""); game.rating falls back to score/100 when no star rating is set.
//!   In `chips` text, a part whose template uses a game.controller* binding gets a drawn gamepad
//!   icon and one using game.deck* the Steam Deck rating mark.
//!   game.genrelist (up to three genres separated by "|": one pill each in `chips` text)
//!   game.achievements ("12 / 40" / "": none or not known yet)  game.achievementsunlocked ("12")
//!     game.achievementstotal ("40")  game.achievementspercent (0..1, for bars)
//!     game.achievementslabel ("12 of 40 achievements" / ""); Steam and RetroAchievements
//!     progress, filled in once it has been looked up (Settings → Achievements)
//!   game.criticscore game.communityscore ("0".."100" / "")  game.scorelabel ("Critics 85",
//!     else "Players 78", else "Your score 90" / "")  list.countlabel ("283 games")
//!   section.name (the navigation rail's open section, "Platforms" …)
//!   system.battery ("82%" / "" without a battery)  system.charging ("true" / "")
//!   item.focused ("true" on the focused item; item templates, `Common::item_elements`)
//! A `rect` with `fill = "{game.achievementspercent}"` is a progress bar (`color` over that
//! share, `track` behind the rest); a stack child with `shrink = true` gives way to the
//! children after it (a title next to a badge).
//! `visible_if` / `hidden_if` (every element, also inside stacks and item templates) are
//! conditions: a binding is true when it resolves to something non-empty; `!{x}` negates;
//! `{a} == b`, `{a} != b` (numbers or text ignoring case, quotes optional; `== false` /
//! `== true` compare truth values), `{a} > 3`, `<`,
//! `>=`, `<=` (numbers); `&&` / `||` combine (`&&` first, no brackets).
//! Containers and motion: element `stack` (children flow down or across with a gap; hidden or
//! empty children take no space; see `Element::Stack`), `Common::item_elements` (item
//! templates over every item of a grid / carousel / rows), `Common::transitions` (focus /
//! unfocus of list items, appear / disappear of any element; model_motion.rs).
//! Optional `home` view (Playnite Fullscreen-style start page) with the `rows` element (cover
//! rows, one per collection) and the `rail` element (navigation rail: Home, All games,
//! Platforms, Sources, Collections, Filters), usable in every view.
//! Text resolution of game.lastplayed / game.added is human ("3 days ago"); DateTime elements
//! get unix seconds.
//! Optional element extras live in `Common` (item_fit, radius, item_fill, item_subtext,
//! item_caption, blur, mask, padding, chips, fade, gradient_horizontal, controller_badge,
//! favorite_badge, placeholder, gradient_stops / gradient_angle / gradient_radial,
//! selected_border_width / selected_radius / selected_glow, uninstalled_scrim,
//! caption_background) and `TextStyle::shadow`. Gradients, brushes and the desktop style
//! (`DesktopStyle`) are in model_look.rs.
//! Lodestar's own console artwork (theme/console_art.rs: original drawings for every console,
//! store and built-in collection; none for the user's own collections and tags):
//!   collection.art.card (600x800 portrait card with the name)  collection.art.hero (16:9
//!   background, no text)  collection.art.mark (the name as a white wordmark)
//!   collection.art.device (the console alone, transparent)  — "" when there is none.
//!   game.consoleart.card / .hero / .mark / .device: the same for the selected game's console
//!   (its store's for PC games without a platform).
//!   A theme replaces any of them per collection with `[collection_art.<key>]` (see
//!   `CollectionArt`); `{collection.cover}` and friends stay the collection's game artwork.
//! An image may list `fallbacks`; the first existing/non-empty candidate wins.
//! Relative paths are relative to the theme directory.
//!
//! Choices for the player (all optional):
//!   * `[variants.<name>]`: alternative layouts ("Grid", "Wheel" …), patches of the views'
//!     elements by name (same shape as `overrides`); Settings → Visuals → Theme variant. A
//!     variant can't add elements: the base view holds them hidden (`visible = false`) and the
//!     variant shows them.
//!   * `[color_schemes.<name>]`: palette changes; element colours equal to a palette colour
//!     follow it (model/schemes.rs). Settings → Visuals → Theme colours.
//!   * `[aspects."16:10"]` …: layouts for other screen shapes, chosen automatically.
//!   * `[overrides.<collection key>]`: per-console / per-collection looks.
//!
//! Keys an element doesn't know are kept (`Common::extra`) and written back; `write.rs` writes
//! a whole theme (`ThemeFile::to_theme_toml`, the theme creator's save) so that it parses back
//! to the same `ThemeFile`.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;

mod look;
pub use look::*;
mod motion;
pub use motion::*;
mod schemes;
pub use schemes::*;
mod write;

/// RGBA colour. Serialised as "#RRGGBB" or "#RRGGBBAA" (the same hex forms without
/// '#' are accepted when parsing).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color(pub [u8; 4]);

impl Color {
    pub const WHITE: Color = Color([255, 255, 255, 255]);
    pub const BLACK: Color = Color([0, 0, 0, 255]);
    pub const TRANSPARENT: Color = Color([0, 0, 0, 0]);

    pub fn parse(s: &str) -> Option<Color> {
        let h = s.trim().trim_start_matches('#').trim_start_matches("0x");
        let v = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
        match h.len() {
            6 => Some(Color([v(0)?, v(2)?, v(4)?, 255])),
            8 => Some(Color([v(0)?, v(2)?, v(4)?, v(6)?])),
            3 => {
                let d = |i: usize| u8::from_str_radix(h.get(i..i + 1)?, 16).ok().map(|x| x * 17);
                Some(Color([d(0)?, d(1)?, d(2)?, 255]))
            }
            _ => None,
        }
    }

    pub fn to_hex(self) -> String {
        let [r, g, b, a] = self.0;
        if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
    }

    pub fn to_egui(self) -> egui::Color32 {
        let [r, g, b, a] = self.0;
        egui::Color32::from_rgba_unmultiplied(r, g, b, a)
    }

    pub fn with_alpha(self, a: u8) -> Color {
        let [r, g, b, _] = self.0;
        Color([r, g, b, a])
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Color::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("bad colour {s:?}")))
    }
}

/// Colours used by the desktop UI and by controller-mode fallbacks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Palette {
    pub dark: bool,
    pub background: Color,
    /// Panels, sidebar.
    pub surface: Color,
    /// Cards, inputs, hovered rows.
    pub surface_alt: Color,
    pub text: Color,
    pub text_dim: Color,
    pub accent: Color,
    /// Text drawn on top of `accent`.
    pub accent_text: Color,
    /// Selection highlight (focused tile border, selected row).
    pub selection: Color,
    pub border: Color,
    pub favorite: Color,
    pub danger: Color,
}

impl Default for Palette {
    fn default() -> Self {
        Palette {
            dark: true,
            background: Color::parse("#0f1218").unwrap(),
            surface: Color::parse("#161b24").unwrap(),
            surface_alt: Color::parse("#1f2632").unwrap(),
            text: Color::parse("#e8ecf2").unwrap(),
            text_dim: Color::parse("#8a94a6").unwrap(),
            accent: Color::parse("#4f8cff").unwrap(),
            accent_text: Color::WHITE,
            selection: Color::parse("#7fb0ff").unwrap(),
            border: Color::parse("#2a3240").unwrap(),
            favorite: Color::parse("#ffc857").unwrap(),
            danger: Color::parse("#ff5d5d").unwrap(),
        }
    }
}

/// Font files (relative to the theme dir) registered under these names. Elements refer to a
/// font by name (`font = "title"`) or by relative path (`font = "fonts/Foo.ttf"`).
/// Missing/invalid fonts fall back to the built-in font.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fonts {
    pub regular: Option<String>,
    pub bold: Option<String>,
    pub title: Option<String>,
    /// Additional named fonts.
    #[serde(flatten)]
    pub extra: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeMeta {
    pub name: String,
    pub author: String,
    pub version: String,
    pub description: String,
    /// Where the theme comes from: "lodestar" for native themes, "playnite" for imported ones.
    pub origin: String,
    /// Aspect ratio the layout was designed for (width / height). Informational.
    pub aspect: f32,
    /// Whether the theme defines controller-mode views (desktop-only themes just set a palette).
    pub controller: bool,
    /// Where controller mode opens when the theme has no `[home]` view: "" = the collections,
    /// "games" = straight into the All games list (Playnite Fullscreen's main screen; B still
    /// goes to the collections).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub start: String,

    // ---- Sharing (all optional; the theme creator fills them in, a theme catalogue reads
    // them). See docs/theme-creator.md "Sharing a theme". ----
    /// Stable id for catalogues: "author-name" in lower case ("marko-night-shelf").
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// One line for lists ("Big covers on a dark wall"); `description` is the long text.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub summary: String,
    /// "CC-BY-4.0", "All rights reserved" … (SPDX ids where there is one).
    #[serde(skip_serializing_if = "String::is_empty", alias = "licence")]
    pub license: String,
    /// Words for finding it: "dark", "light", "carousel", "grid", "wheel", "minimal" …
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Up to four pictures of the theme (theme files, "previews/games.png").
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub previews: Vec<String>,
    /// The Lodestar version it was last saved with ("0.17.0").
    #[serde(skip_serializing_if = "String::is_empty")]
    pub made_with: String,
    /// The oldest Lodestar version it works with ("0.17").
    #[serde(skip_serializing_if = "String::is_empty")]
    pub needs: String,
}

impl Default for ThemeMeta {
    fn default() -> Self {
        ThemeMeta {
            name: "Untitled".into(),
            author: String::new(),
            version: "1".into(),
            description: String::new(),
            origin: "lodestar".into(),
            aspect: 16.0 / 9.0,
            controller: true,
            start: String::new(),
            id: String::new(),
            summary: String::new(),
            license: String::new(),
            tags: Vec::new(),
            previews: Vec::new(),
            made_with: String::new(),
            needs: String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewKind {
    Collections,
    Games,
    Details,
    /// Optional start page (Playnite Fullscreen style: navigation rail, hero, cover rows).
    /// Themes without one start on `collections`.
    Home,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VAlign {
    #[default]
    Top,
    Center,
    Bottom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    /// Fit inside the box keeping aspect (letterbox).
    #[default]
    Contain,
    /// Fill the box keeping aspect (crop).
    Cover,
    /// Fill the box ignoring aspect.
    Stretch,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    #[default]
    Horizontal,
    Vertical,
    /// Items on an arc around the strip's centre, laid along its longer side.
    Wheel,
    /// Vertical wheel: items above and below the selection, each turned by
    /// `wheel_angle` more around `wheel_origin` (a point in item coordinates, often far to one
    /// side, so the items run along an arc).
    #[serde(rename = "vertical_wheel")]
    VerticalWheel,
    /// Horizontal wheel: the same, with the items to the left and right.
    #[serde(rename = "horizontal_wheel")]
    HorizontalWheel,
}

impl Orientation {
    /// A wheel that turns its items around a point (`VerticalWheel` / `HorizontalWheel`).
    pub fn is_arc_wheel(self) -> bool {
        matches!(self, Orientation::VerticalWheel | Orientation::HorizontalWheel)
    }
}

/// Fields every element has.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Common {
    /// Name (used by overrides and for debugging). Imported element names are kept.
    pub name: String,
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub origin: [f32; 2],
    pub z: f32,
    pub visible: bool,
    /// 0..1
    pub opacity: f32,
    /// Degrees, clockwise, around `rotation_origin` (fraction of the element box).
    pub rotation: f32,
    pub rotation_origin: [f32; 2],
    /// Only draw when this binding resolves to a non-empty string, e.g. "{art.logo}".
    pub visible_if: Option<String>,
    /// Only draw when this binding resolves to an empty string.
    pub hidden_if: Option<String>,

    // ---- Optional extras (added after v1; all default to "off"). Interpreted only by the
    // element types noted; other elements ignore them. ----
    /// Carousel / grid: how item images fill the item box (default `contain`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_fit: Option<Fit>,
    /// Corner radius (fraction of screen height) for carousel items and text backgrounds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    /// Carousel: card colour behind items / drawn when an item has no image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_fill: Option<Color>,
    /// Carousel / grid: second line on text-styled cards, e.g. "{collection.count} games".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_subtext: Option<String>,
    /// Image: blur radius (fraction of screen height). Blurred images load at thumbnail size.
    #[serde(skip_serializing_if = "is_zero")]
    pub blur: f32,
    /// Text: padding inside `background`, [x, y] as fractions of screen height.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding: Option<[f32; 2]>,
    /// Image: cross-fade seconds when the resolved path changes. Default: 0.25 s for paths
    /// with bindings, 0 for fixed paths.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fade: Option<f32>,
    /// Image / rect: `gradient` runs left → right instead of top → bottom.
    #[serde(skip_serializing_if = "is_false")]
    pub gradient_horizontal: bool,
    /// Carousel / grid: caption drawn over the bottom of items that have an image
    /// (on a dark scrim), e.g. "{collection.name}". `item_subtext` goes under it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_caption: Option<String>,
    /// Text: split the text on "|" and draw each part that resolves non-empty as a pill
    /// (using `background`, `radius`, `padding`), flowing left to right.
    #[serde(skip_serializing_if = "is_false")]
    pub chips: bool,
    /// Image: fade the picture itself to transparent towards its edges, as fractions of the
    /// element size [left, top, right, bottom] (reveals what is drawn underneath).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask: Option<[f32; 4]>,
    /// Carousel / grid / text list of games: mark games with full controller support (a small
    /// gamepad badge in the bottom-right corner of covers; a gamepad glyph at the end of list
    /// rows). Off unless a theme sets it (the built-in themes do).
    #[serde(skip_serializing_if = "is_false")]
    pub controller_badge: bool,
    /// Carousel / grid / text list of games: mark favourites (a small ★ badge in the top-right
    /// corner of covers, in the palette's accent colour on a dark disc; a ★ at the end of list
    /// rows). Off unless a theme sets it (the built-in themes do).
    #[serde(skip_serializing_if = "is_false")]
    pub favorite_badge: bool,
    /// Image bound to a game's artwork: when the game has none of the pictures asked for, draw
    /// a soft stand-in (a wash in a colour picked from its console / store, with the console's
    /// outline) instead of leaving the area empty.
    #[serde(skip_serializing_if = "is_false")]
    pub placeholder: bool,
    /// Image / rect: a gradient with any number of stops (`[[colour, offset] …]`), replacing
    /// `gradient` (see model_look.rs for the angle / radial conventions).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gradient_stops: Option<Vec<GradientStop>>,
    /// Image / rect: direction of the gradient in degrees clockwise from left → right
    /// (90 = top → bottom). Also turns the two-colour `gradient`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gradient_angle: Option<f32>,
    /// Image / rect: a radial gradient, [cx, cy, rx, ry] as fractions of the element box.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gradient_radial: Option<[f32; 4]>,
    /// Carousel / grid / rows: width of the focused item's `selected_border`, fraction of the
    /// screen height (default 0.004, at least 2 px).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_border_width: Option<f32>,
    /// Carousel / grid / rows: corner radius of the focused item's border and glow, fraction
    /// of the screen height (default the item's radius; the picture keeps its own `radius`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_radius: Option<f32>,
    /// Carousel / grid / rows: glow around the focused item (default a soft glow in the
    /// `selected_border` colour; a fully transparent colour turns it off).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_glow: Option<Glow>,
    /// Carousel / grid / rows: colour laid over games that aren't installed (when the user's
    /// "Darken not-installed games" is on) instead of the default grey dimming.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uninstalled_scrim: Option<Color>,
    /// Carousel / grid / rows: background of the `item_caption` strip (default a dark fade
    /// from the bottom).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption_background: Option<Brush>,
    /// Animations on focus / unfocus (grid, carousel and rows items) and when the element
    /// appears / disappears (any element). See model_motion.rs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transitions: Option<Box<Transitions>>,
    /// Inside a `stack`: room kept around the element, [left, top, right, bottom] as fractions
    /// of the screen height (WPF Margin / 1080). Ignored elsewhere.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin: Option<[f32; 4]>,
    /// Grid / carousel / rows: elements drawn over each item (Playnite's item template:
    /// overlays, badges, a title strip …). Their `pos` / `size` and font sizes are fractions
    /// of the item's box (it is their "screen"), their bindings resolve to that item, and
    /// `{item.focused}` is "true" on the focused one. Lists inside are not drawn.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub item_elements: Vec<Element>,
    /// Image: its picture's brightness; carousel / grid: the item pictures'
    /// (-2..2: every colour channel moves by 0.3 per unit, 0 = unchanged).
    #[serde(skip_serializing_if = "is_zero")]
    pub brightness: f32,
    /// Image / carousel / grid pictures: colour saturation (0 = grey, 1 = unchanged).
    #[serde(skip_serializing_if = "is_one")]
    pub saturation: f32,
    /// Inside a `stack`: this child gets only the room the children after it leave (a long
    /// title gives way to the badge next to it instead of pushing it off its line). Ignored
    /// elsewhere.
    #[serde(skip_serializing_if = "is_false")]
    pub shrink: bool,
    /// Keys this version of Lodestar doesn't know (a theme made for a newer version, a typo):
    /// kept as they are, so opening and saving a theme (the theme creator) never loses them.
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Common {
    /// The N-stop / angled / radial gradient this element asks for, if any. `two` is the
    /// element's two-colour `gradient`, which keeps its own (cheaper, identical) drawing
    /// unless an angle or radial is set too.
    pub fn gradient_spec(&self, two: Option<[Color; 2]>) -> Option<Gradient> {
        let stops = match (&self.gradient_stops, two) {
            (Some(s), _) if !s.is_empty() => s.clone(),
            (_, Some([a, b])) if self.gradient_angle.is_some() || self.gradient_radial.is_some() => {
                vec![GradientStop::new(a, 0.0), GradientStop::new(b, 1.0)]
            }
            _ => return None,
        };
        let angle = self.gradient_angle.or(Some(if self.gradient_horizontal { 0.0 } else { 90.0 }));
        Some(Gradient { stops, angle, radial: self.gradient_radial })
    }
}

fn is_false(v: &bool) -> bool {
    !*v
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

fn is_one(v: &f32) -> bool {
    *v == 1.0
}

impl Default for Common {
    fn default() -> Self {
        Common {
            name: String::new(),
            pos: [0.0, 0.0],
            size: [0.0, 0.0],
            origin: [0.0, 0.0],
            z: 0.0,
            visible: true,
            opacity: 1.0,
            rotation: 0.0,
            rotation_origin: [0.5, 0.5],
            visible_if: None,
            hidden_if: None,
            item_fit: None,
            radius: None,
            item_fill: None,
            item_subtext: None,
            blur: 0.0,
            padding: None,
            fade: None,
            gradient_horizontal: false,
            item_caption: None,
            chips: false,
            mask: None,
            controller_badge: false,
            favorite_badge: false,
            placeholder: false,
            gradient_stops: None,
            gradient_angle: None,
            gradient_radial: None,
            selected_border_width: None,
            selected_radius: None,
            selected_glow: None,
            uninstalled_scrim: None,
            caption_background: None,
            transitions: None,
            margin: None,
            item_elements: Vec::new(),
            brightness: 0.0,
            saturation: 1.0,
            shrink: false,
            extra: toml::Table::new(),
        }
    }
}

/// Text styling shared by text-bearing elements.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextStyle {
    /// Font name from `[fonts]` or a relative font path. None = theme regular font.
    pub font: Option<String>,
    /// Fraction of screen height.
    pub font_size: f32,
    pub color: Color,
    pub align: Align,
    pub valign: VAlign,
    pub uppercase: bool,
    /// Line height multiplier.
    pub line_spacing: f32,
    /// Letter spacing in fractions of the font size.
    pub letter_spacing: f32,
    /// Soft drop shadow behind the glyphs (legibility over artwork).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow: Option<Color>,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            font: None,
            font_size: 0.035,
            color: Color::WHITE,
            align: Align::Left,
            valign: VAlign::Top,
            uppercase: false,
            line_spacing: 1.2,
            letter_spacing: 0.0,
            shadow: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Element {
    /// A picture. `path` may be a binding (`{art.cover}`) or a theme file.
    Image {
        #[serde(flatten)]
        common: Common,
        #[serde(default)]
        path: String,
        #[serde(default)]
        fallbacks: Vec<String>,
        #[serde(default)]
        fit: Fit,
        /// Multiply colour.
        #[serde(default)]
        tint: Option<Color>,
        /// Repeat the image to fill the box at its natural size.
        #[serde(default)]
        tile: bool,
        /// Rounded corners, fraction of screen height.
        #[serde(default)]
        corner_radius: f32,
        /// Vertical gradient overlay (top colour, bottom colour) drawn over the image.
        #[serde(default)]
        gradient: Option<[Color; 2]>,
        /// With `tint`: the multiply colour at the bottom (or right, `gradient_horizontal`) end,
        /// for a tint that changes across the picture.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tint_end: Option<Color>,
        /// Drawn in the image's box when none of its pictures exist, e.g. "{collection.name}"
        /// for a per-console logo the theme has no file for ("" = nothing).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fallback_text: Option<String>,
        /// Style of `fallback_text` (its size is fitted to the box; None = the theme's text).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fallback_style: Option<TextStyle>,
        /// A theme video (theme file) played in the element's box
        /// when videos can play (ffmpeg installed, Settings → Visuals → Theme videos); the
        /// picture (`path`, `fallbacks`) shows while it starts and without ffmpeg.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        video: Option<String>,
    },
    /// Text with bindings. Long text wraps inside `size`; `scroll` auto-scrolls overflow slowly.
    Text {
        #[serde(flatten)]
        common: Common,
        #[serde(default)]
        text: String,
        #[serde(default)]
        style: TextStyle,
        #[serde(default)]
        background: Option<Color>,
        #[serde(default)]
        wrap: bool,
        #[serde(default)]
        max_lines: Option<u32>,
        #[serde(default)]
        scroll: bool,
    },
    /// Solid / bordered rectangle.
    Rect {
        #[serde(flatten)]
        common: Common,
        #[serde(default = "default_rect_color")]
        color: Color,
        #[serde(default)]
        corner_radius: f32,
        #[serde(default)]
        border_color: Option<Color>,
        /// Fraction of screen height.
        #[serde(default)]
        border_width: f32,
        #[serde(default)]
        gradient: Option<[Color; 2]>,
        /// A progress bar: a binding giving 0..1 (`"{game.achievementspercent}"`); `color` fills
        /// that share of the box from the left (nothing when it resolves to nothing).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fill: Option<String>,
        /// With `fill`: the colour of the rest of the bar (default: none).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        track: Option<Color>,
    },
    /// Scrolling strip of items (collections or games). Focusable.
    Carousel {
        #[serde(flatten)]
        common: Common,
        #[serde(default)]
        orientation: Orientation,
        /// Item box, fraction of screen.
        #[serde(default = "default_item_size")]
        item_size: [f32; 2],
        /// How many items are visible (centre one selected).
        #[serde(default = "default_visible_items")]
        visible_items: f32,
        /// What each item shows: an image path with bindings, e.g. "{art.cover}" for games or
        /// "{collection.logo}" for collections. Bindings resolve per *item*.
        #[serde(default)]
        item_image: String,
        #[serde(default)]
        item_fallbacks: Vec<String>,
        /// Text drawn when no item image resolves.
        #[serde(default = "default_item_text")]
        item_text: String,
        #[serde(default)]
        item_text_style: TextStyle,
        #[serde(default = "default_selected_scale")]
        selected_scale: f32,
        #[serde(default = "default_unselected_opacity")]
        unselected_opacity: f32,
        #[serde(default)]
        background: Option<Color>,
        /// Item alignment across the strip: 0 = start, 0.5 = centre, 1 = end.
        #[serde(default = "half")]
        item_align: f32,
        /// Where the selected item sits along the strip (0..1). 0.5 = centred.
        #[serde(default = "half")]
        selected_position: f32,
        /// Wheel only: degrees between items.
        #[serde(default = "default_wheel_angle")]
        wheel_angle: f32,
        #[serde(default)]
        selected_border: Option<Color>,
        /// Wheels: the point the items turn around, in item coordinates (default
        /// [-3, 0.5]: three item widths to the left).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wheel_origin: Option<[f32; 2]>,
        /// Wheels: where the selection sits across the strip (0 = left / top edge,
        /// 0.5 = centre, 1 = right / bottom edge).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wheel_align: Option<f32>,
        /// Wheels: the selection moved across the strip by this fraction of its size.
        #[serde(default, skip_serializing_if = "is_zero")]
        cross_offset: f32,
        /// Wheels: items drawn before and after the selection.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        items_around: Option<[u32; 2]>,
        /// Multiply colour for item pictures.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item_tint: Option<Color>,
        /// Room before and after the selected item along the strip (instead of the automatic
        /// push away from a scaled-up selection), fractions of the
        /// screen width (horizontal) or height (vertical); negative pulls the neighbour in.
        /// Not for wheels.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selected_margins: Option<[f32; 2]>,
    },
    /// 2D grid of items. Focusable.
    Grid {
        #[serde(flatten)]
        common: Common,
        /// Columns; rows follow from `item_aspect` and the box height.
        #[serde(default = "default_columns")]
        columns: u32,
        /// Item height / width.
        #[serde(default = "default_item_aspect")]
        item_aspect: f32,
        /// Gap between items, fraction of screen height.
        #[serde(default = "default_spacing")]
        spacing: f32,
        #[serde(default)]
        item_image: String,
        #[serde(default)]
        item_fallbacks: Vec<String>,
        #[serde(default = "default_item_text")]
        item_text: String,
        #[serde(default)]
        item_text_style: TextStyle,
        /// Show `item_text` under every item (otherwise only when there is no image).
        #[serde(default)]
        show_labels: bool,
        #[serde(default = "default_grid_selected_scale")]
        selected_scale: f32,
        #[serde(default)]
        selected_border: Option<Color>,
        #[serde(default)]
        item_background: Option<Color>,
        #[serde(default)]
        corner_radius: f32,
        #[serde(default = "default_unselected_opacity")]
        unselected_opacity: f32,
        /// Tiles: backgrounds behind the items, a selector, smaller pictures (see
        /// `GridTiles`). With them, `item_background` is only the card of items without a
        /// picture and `selected_border` is not drawn unless set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tiles: Option<Box<GridTiles>>,
    },
    /// Vertical list of text rows. Focusable.
    TextList {
        #[serde(flatten)]
        common: Common,
        #[serde(default = "default_item_text")]
        item_text: String,
        #[serde(default)]
        style: TextStyle,
        /// Colour of the selected row's text.
        #[serde(default)]
        selected_color: Option<Color>,
        /// Selector bar colour.
        #[serde(default)]
        selector_color: Option<Color>,
        /// Selector image (stretched over the row) instead of / on top of the colour.
        #[serde(default)]
        selector_image: Option<String>,
        /// Row height as a multiple of the font size (about 1.5 reads well).
        #[serde(default = "default_row_height")]
        row_height: f32,
        /// Horizontal padding inside rows, fraction of screen width.
        #[serde(default)]
        margin: f32,
        /// Text colour for favourites; None = style.color.
        #[serde(default)]
        favorite_color: Option<Color>,
        /// Selector corner radius, fraction of screen height.
        #[serde(default)]
        corner_radius: f32,
        /// Selector width, fraction of screen width (None = the whole row). A small value
        /// gives a thin side marker.
        #[serde(default)]
        selector_width: Option<f32>,
        /// Selector x offset from the list's left edge, fraction of screen width
        /// (`selectorHorizontalOffset`).
        #[serde(default)]
        selector_offset: f32,
        /// A box behind the selected row's text, drawn over the selector bar, with
        /// `corner_radius`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selected_background: Option<Color>,
        /// How far that box reaches past the text on the left and right, fractions of the
        /// screen width (None = the whole row).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selected_background_margins: Option<[f32; 2]>,
    },
    /// 0..5 stars from a 0..1 binding (default `{game.rating}`).
    Rating {
        #[serde(flatten)]
        common: Common,
        #[serde(default = "default_rating_value")]
        value: String,
        #[serde(default)]
        filled_path: Option<String>,
        #[serde(default)]
        unfilled_path: Option<String>,
        #[serde(default = "default_rating_color")]
        color: Color,
    },
    /// Date/time of a binding (`{game.lastplayed}` style raw unix value) or the clock when `clock`.
    DateTime {
        #[serde(flatten)]
        common: Common,
        #[serde(default)]
        value: String,
        #[serde(default)]
        style: TextStyle,
        #[serde(default)]
        clock: bool,
        /// strftime-like: %Y %m %d %H %M, or "relative" for "3 days ago".
        #[serde(default = "default_date_format")]
        format: String,
        /// Box behind the text, like `Text::background` (uses `padding` / `radius`): a
        /// clock on a pill.
        #[serde(default)]
        background: Option<Color>,
    },
    /// Navigation rail (Playnite Fullscreen's side menu): a vertical list of sections with
    /// icons and labels — Home, All games, Platforms, Sources, Collections, Filters. Moving
    /// left off the first column of the focused list (or B on the home page) puts the focus on
    /// it; sections without games are left out by the controller UI.
    Rail {
        #[serde(flatten)]
        common: Common,
        /// Sections in order. Empty = the default set (see `RailEntry::defaults`).
        #[serde(default)]
        entries: Vec<RailEntry>,
        /// Label style (`font_size` also sets the row height and icon size).
        #[serde(default)]
        style: TextStyle,
        /// Row height as a multiple of the font size.
        #[serde(default = "default_rail_row")]
        row_height: f32,
        /// Text / icon colour of the section that is open.
        #[serde(default)]
        current_color: Option<Color>,
        /// Selector colour while the rail has the focus.
        #[serde(default)]
        selector_color: Option<Color>,
        /// Panel colour behind the rail (None = transparent).
        #[serde(default)]
        background: Option<Color>,
        /// Show the labels only while the rail has the focus (icons only otherwise; the
        /// panel widens over the page then).
        #[serde(default)]
        compact: bool,
    },
    /// Rows of game covers, one row per collection (Playnite Fullscreen's home page: "Recently
    /// played", "Favourites", a console or two …). Up / Down move between rows, Left / Right
    /// within a row; the focused row scrolls to the top of the box. Focusable.
    Rows {
        #[serde(flatten)]
        common: Common,
        /// Which rows, in order: "recent", "favorites", "installed", "controller", "all",
        /// "platforms" (one row per console, most games first), "sources", "collections" (the
        /// user's own), or one collection by key ("platform:sony_playstation2", "steam",
        /// "tag:RPG" …). Empty rows are skipped, and so are categories the player hid; ones the
        /// player added (Settings → Categories) join the rows of their kind.
        #[serde(default = "default_rows")]
        rows: Vec<String>,
        /// At most this many rows.
        #[serde(default = "default_max_rows")]
        max_rows: u32,
        /// At most this many games per row.
        #[serde(default = "default_row_items")]
        row_items: u32,
        /// Item box [w, h], fractions of the screen.
        #[serde(default = "default_rows_item_size")]
        item_size: [f32; 2],
        /// Gap between items, fraction of screen width.
        #[serde(default = "default_spacing")]
        spacing: f32,
        /// Gap between rows (below a row's items), fraction of screen height.
        #[serde(default = "default_spacing")]
        row_spacing: f32,
        /// Row heading, e.g. "{collection.name}" ("" = none).
        #[serde(default = "default_row_title")]
        title: String,
        #[serde(default)]
        title_style: TextStyle,
        #[serde(default)]
        item_image: String,
        #[serde(default)]
        item_fallbacks: Vec<String>,
        /// Text drawn when no item image resolves.
        #[serde(default = "default_item_text")]
        item_text: String,
        #[serde(default)]
        item_text_style: TextStyle,
        #[serde(default = "default_grid_selected_scale")]
        selected_scale: f32,
        #[serde(default)]
        selected_border: Option<Color>,
        /// Opacity of the items of the rows that don't have the focus.
        #[serde(default = "default_unselected_opacity")]
        unselected_opacity: f32,
    },
    /// Small icons for what is true of the shown game: favourite, completed,
    /// controller support … Only the ones that apply are drawn, packed from the start.
    Badges {
        #[serde(flatten)]
        common: Common,
        /// Which badges, in order: "favorite", "completed", "controller"; other names
        /// are kept but never shown (Lodestar has no data for them).
        #[serde(default = "default_badge_slots")]
        slots: Vec<String>,
        /// Fill top to bottom, then the next column (default: left to right, then down).
        #[serde(default)]
        column: bool,
        /// Rows (or columns, with `column`).
        #[serde(default = "default_badge_lines")]
        lines: u32,
        /// Badges per row (or column).
        #[serde(default = "default_badge_per_line")]
        per_line: u32,
        /// Gap between badges [x, y], fractions of the screen.
        #[serde(default)]
        margin: [f32; 2],
        /// `Right`: rows start at the right edge.
        #[serde(default)]
        align: Align,
        /// Theme pictures per badge ("favorite" → path); badges without one are drawn.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        icons: BTreeMap<String, String>,
        /// Multiply colour for the pictures.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<Color>,
    },
    /// A container: its `children` (any elements, also stacks) flow one after the other,
    /// down (`direction = "vertical"`) or across (`"horizontal"`), `gap` apart (fraction of the
    /// screen height). Children that are hidden (`visible`, `visible_if`, `hidden_if`) or empty
    /// (text resolving to "", an image without a picture) take no space. A child's own `pos` /
    /// `origin` are ignored; its `size` is kept where set (0 = from its content: text is
    /// measured with the real font and wraps at the stack's width, images keep their aspect),
    /// `margin` adds room around it. `align` places children across the flow (start / center /
    /// end, or `stretch` = children without a size take the full width / height), `justify`
    /// along it when the stack is bigger than its content. A stack without a size is as big as
    /// its children (`origin` then anchors it: [0, 1] grows upwards from `pos`). It may draw a
    /// `background` (colour or gradient brush) with `corner_radius`, a border, and `padding`
    /// ([x, y], fractions of the screen height).
    Stack {
        #[serde(flatten)]
        common: Common,
        #[serde(default)]
        direction: Direction,
        #[serde(default)]
        gap: f32,
        #[serde(default)]
        align: StackAlign,
        #[serde(default = "stack_start")]
        justify: StackAlign,
        #[serde(default)]
        children: Vec<Element>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        background: Option<Brush>,
        #[serde(default, skip_serializing_if = "is_zero")]
        corner_radius: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        border_color: Option<Color>,
        /// Fraction of screen height.
        #[serde(default, skip_serializing_if = "is_zero")]
        border_width: f32,
        /// A horizontal stack without a width: the most it may take (fraction of the screen
        /// width; 0 = no limit). Wider children have their text drawn smaller until they fit,
        /// so a row of tabs never runs into what sits beside it.
        #[serde(default, skip_serializing_if = "is_zero")]
        max_width: f32,
    },
    /// Controller button hints for the current screen ("A Launch  B Back  X Favourite …").
    HelpBar {
        #[serde(flatten)]
        common: Common,
        #[serde(default)]
        style: TextStyle,
        #[serde(default)]
        icon_color: Option<Color>,
        /// Box behind the hints (with `radius`, and `padding` around them).
        #[serde(default)]
        background: Option<Color>,
        /// Only the hints of these buttons ("a", "b", "x", "y", "start", "select", "lb",
        /// "rb"), in the screen's order; empty = all. Several bars can share a screen's hints.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        buttons: Vec<String>,
        /// Room between hints and between a button and its label, fractions of the screen
        /// width (None = 1.5 and 0.45 times the font size).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entry_spacing: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label_spacing: Option<f32>,
        /// Button height as a multiple of the font size (None = 1.35).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        button_scale: Option<f32>,
        /// Only the glyphs of `buttons`, without labels, shown whatever the screen's hints are:
        /// a theme's own button prompt drawn next to its own text ("Ⓐ Select").
        #[serde(default, skip_serializing_if = "is_false")]
        glyphs: bool,
    },
}

fn stack_start() -> StackAlign {
    StackAlign::Start
}
fn default_badge_slots() -> Vec<String> {
    vec!["favorite".into(), "completed".into(), "controller".into()]
}
fn default_badge_lines() -> u32 {
    1
}
fn default_badge_per_line() -> u32 {
    4
}
fn default_rail_row() -> f32 {
    2.4
}
fn default_rows() -> Vec<String> {
    vec!["recent".into(), "favorites".into(), "platforms".into()]
}
fn default_max_rows() -> u32 {
    6
}
fn default_row_items() -> u32 {
    30
}
fn default_rows_item_size() -> [f32; 2] {
    [0.1, 0.24]
}
fn default_row_title() -> String {
    "{collection.name}".into()
}

/// One section of a `rail`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RailEntry {
    /// What it opens: "home", "all", "platforms", "sources", "collections", "filters",
    /// "search", "menu".
    pub key: String,
    /// Label ("" = the default label for the key).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// A drawn icon ("home", "grid", "console", "monitor", "folder", "filter", "search",
    /// "menu", "star", "clock") or an image path (theme-relative). None = the key's icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

impl RailEntry {
    pub const KEYS: [&'static str; 8] =
        ["home", "all", "platforms", "sources", "collections", "filters", "search", "menu"];

    /// The default rail: every section that browses the library.
    pub fn defaults() -> Vec<RailEntry> {
        Self::KEYS[..6].iter().map(|k| RailEntry { key: (*k).into(), label: String::new(), icon: None }).collect()
    }

    pub fn label(&self) -> &str {
        if !self.label.trim().is_empty() {
            return self.label.trim();
        }
        match self.key.as_str() {
            "home" => "Home",
            "all" => "All games",
            "platforms" => "Platforms",
            "sources" => "Sources",
            "collections" => "Collections",
            "filters" => "Filters",
            "search" => "Search",
            "menu" => "Menu",
            other => other,
        }
    }

    pub fn icon(&self) -> &str {
        if let Some(i) = self.icon.as_deref().filter(|i| !i.trim().is_empty()) {
            return i.trim();
        }
        match self.key.as_str() {
            "home" => "home",
            "all" => "grid",
            "platforms" => "console",
            "sources" => "monitor",
            "collections" => "folder",
            "filters" => "filter",
            "search" => "search",
            "menu" => "menu",
            _ => "grid",
        }
    }
}

fn default_rect_color() -> Color {
    Color([0, 0, 0, 160])
}
fn default_item_size() -> [f32; 2] {
    [0.25, 0.15]
}
fn default_visible_items() -> f32 {
    5.0
}
fn default_item_text() -> String {
    "{game.name}".into()
}
fn default_selected_scale() -> f32 {
    1.2
}
fn default_grid_selected_scale() -> f32 {
    1.06
}
fn default_unselected_opacity() -> f32 {
    0.6
}
fn half() -> f32 {
    0.5
}
fn default_wheel_angle() -> f32 {
    12.0
}
fn default_columns() -> u32 {
    6
}
fn default_item_aspect() -> f32 {
    1.5
}
fn default_spacing() -> f32 {
    0.02
}
fn default_row_height() -> f32 {
    1.5
}
fn default_rating_value() -> String {
    "{game.rating}".into()
}
fn default_rating_color() -> Color {
    Color::parse("#ffc857").unwrap()
}

impl Color {
    /// `Element::Rating::color` when the theme sets none (vector stars only: image stars are
    /// tinted by `color` only when a theme sets one).
    pub const RATING_DEFAULT: Color = Color([0xff, 0xc8, 0x57, 0xff]);
}

fn default_date_format() -> String {
    "%d %b %Y".into()
}

impl Element {
    pub fn common(&self) -> &Common {
        match self {
            Element::Image { common, .. }
            | Element::Text { common, .. }
            | Element::Rect { common, .. }
            | Element::Carousel { common, .. }
            | Element::Grid { common, .. }
            | Element::TextList { common, .. }
            | Element::Rating { common, .. }
            | Element::DateTime { common, .. }
            | Element::Rail { common, .. }
            | Element::Rows { common, .. }
            | Element::Badges { common, .. }
            | Element::Stack { common, .. }
            | Element::HelpBar { common, .. } => common,
        }
    }

    pub fn common_mut(&mut self) -> &mut Common {
        match self {
            Element::Image { common, .. }
            | Element::Text { common, .. }
            | Element::Rect { common, .. }
            | Element::Carousel { common, .. }
            | Element::Grid { common, .. }
            | Element::TextList { common, .. }
            | Element::Rating { common, .. }
            | Element::DateTime { common, .. }
            | Element::Rail { common, .. }
            | Element::Rows { common, .. }
            | Element::Badges { common, .. }
            | Element::Stack { common, .. }
            | Element::HelpBar { common, .. } => common,
        }
    }

    /// This element and every element nested in it (a stack's children, item templates),
    /// depth first.
    pub fn for_each_nested(&self, f: &mut dyn FnMut(&Element)) {
        f(self);
        if let Element::Stack { children, .. } = self {
            for c in children {
                c.for_each_nested(f);
            }
        }
        for c in &self.common().item_elements {
            c.for_each_nested(f);
        }
    }

    /// `pred` holds for this element or one nested in it (`for_each_nested`).
    pub fn any_nested(&self, pred: &dyn Fn(&Element) -> bool) -> bool {
        pred(self)
            || matches!(self, Element::Stack { children, .. } if children.iter().any(|c| c.any_nested(pred)))
            || self.common().item_elements.iter().any(|c| c.any_nested(pred))
    }

    /// A list of items (carousel, grid, text list, rows, rail).
    pub fn is_list(&self) -> bool {
        self.is_focusable() || matches!(self, Element::Rail { .. })
    }

    /// Carousel / Grid / TextList / Rows receive navigation. (A `Rail` gets the focus only
    /// when the user moves onto it, so it is not the view's focus element.)
    pub fn is_focusable(&self) -> bool {
        matches!(
            self,
            Element::Carousel { .. } | Element::Grid { .. } | Element::TextList { .. } | Element::Rows { .. }
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct View {
    /// Background colour drawn before any element.
    pub background: Option<Color>,
    pub elements: Vec<Element>,
    /// The size of the screen the view was laid out on ([1920, 1080]), when it is to be scaled
    /// as one piece the way WPF's Viewbox scales Playnite's fullscreen views: one factor for
    /// both directions, centred, so texts keep the proportions of their boxes on any screen
    /// shape. Elements that span the view (backgrounds, full-width bars) extend into the
    /// spare room at the edges. None: positions and sizes stretch with the screen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewbox: Option<[f32; 2]>,
}

impl View {
    /// The element that receives navigation: the first focusable one.
    #[cfg(test)]
    pub fn focus_index(&self) -> Option<usize> {
        self.elements.iter().position(Element::is_focusable)
    }

    /// The view's navigation rail, if it has one.
    pub fn rail_index(&self) -> Option<usize> {
        self.elements.iter().position(|e| matches!(e, Element::Rail { .. }) && e.common().visible)
    }
}

/// Per-collection tweaks. Keyed in `Theme::overrides` by a collection key ("all", "steam",
/// "platform:sony_playstation2", "tag:RPG" …); the renderer applies the one whose key matches
/// the collection's.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CollectionOverride {
    /// Element name -> replacement fields (a partial TOML table merged over the element).
    pub collections: BTreeMap<String, toml::Table>,
    pub games: BTreeMap<String, toml::Table>,
    pub details: BTreeMap<String, toml::Table>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub home: BTreeMap<String, toml::Table>,
    /// Display name for the collection.
    pub display_name: Option<String>,
}

/// A theme's own pictures for one collection, replacing Lodestar's console artwork in the
/// `{collection.art.*}` / `{game.consoleart.*}` bindings. Keyed in `ThemeFile::collection_art`
/// by the collection key ("all", "favorites", "recent", "installed", "controller", "pc", a store
/// key like "steam", "platform:sony_playstation2", "coll:<id>", "tag:RPG") or by the bare
/// platform id ("sony_playstation2"); the collection key is tried first. Paths are relative to
/// the theme folder (or absolute). A picture left out keeps Lodestar's; a file that doesn't
/// exist falls back to it too.
///
/// ```toml
/// [collection_art.sony_playstation2]
/// card = "images/ps2-card.png"
/// hero = "images/ps2-wide.jpg"
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CollectionArt {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hero: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mark: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
}

impl CollectionArt {
    /// The picture for `kind` ("card", "hero", "mark", "device"), if this theme sets one.
    pub fn get(&self, kind: &str) -> Option<&str> {
        match kind {
            "card" => self.card.as_deref(),
            "hero" => self.hero.as_deref(),
            "mark" => self.mark.as_deref(),
            "device" => self.device.as_deref(),
            _ => None,
        }
        .map(str::trim)
        .filter(|s| !s.is_empty())
    }
}

fn is_default_desktop(d: &DesktopStyle) -> bool {
    *d == DesktopStyle::default()
}

/// "16:10" → 1.6, "5:3_vertical" → 0.6 (aspect ratio names).
pub fn aspect_ratio(name: &str) -> Option<f32> {
    let core = name.split('_').next().unwrap_or(name);
    let (w, h) = core.split_once(':')?;
    let r = w.trim().parse::<f32>().ok()? / h.trim().parse::<f32>().ok()?;
    let r = if name.contains("vertical") { 1.0 / r } else { r };
    (r.is_finite() && r > 0.1 && r < 10.0).then_some(r)
}

/// A whole theme as stored in `theme.toml`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeFile {
    pub theme: ThemeMeta,
    pub palette: Palette,
    pub fonts: Fonts,
    pub collections: View,
    pub games: View,
    pub details: Option<View>,
    /// Optional start page (see `ViewKind::Home`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home: Option<View>,
    pub overrides: BTreeMap<String, CollectionOverride>,
    /// Named variants: name -> per-view element overrides like CollectionOverride.
    pub variants: BTreeMap<String, CollectionOverride>,
    /// Layouts for other screen shapes ("16:10", "4:3" …), applied after
    /// the variant when the screen is closer to that shape than to `theme.aspect`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub aspects: BTreeMap<String, CollectionOverride>,
    /// Desktop-mode extras (ambient background).
    #[serde(skip_serializing_if = "is_default_desktop")]
    pub desktop: DesktopStyle,
    /// Navigation sounds: sound name ("scroll", "systembrowse", "quicksysselect",
    /// "select", "back", "favorite", "launch") → theme file. Played only when the user turns
    /// navigation sounds on.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub sounds: BTreeMap<String, String>,
    /// Background music: theme files or folders (played only when the user turns music on).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub music: Vec<String>,
    /// Colour schemes the player can choose from (controller Settings → Visuals → Theme
    /// colours; desktop Settings → Appearance), see model/schemes.rs.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub color_schemes: BTreeMap<String, ColorScheme>,
    /// The theme's own pictures per collection (see `CollectionArt`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub collection_art: BTreeMap<String, CollectionArt>,
}

impl ThemeFile {
    pub fn view(&self, kind: ViewKind) -> Option<&View> {
        match kind {
            ViewKind::Collections => Some(&self.collections),
            ViewKind::Games => Some(&self.games),
            ViewKind::Details => self.details.as_ref(),
            ViewKind::Home => self.home.as_ref(),
        }
    }

    pub fn parse(text: &str) -> anyhow::Result<ThemeFile> {
        Ok(toml::from_str(text)?)
    }

    /// The `aspects` layout that suits a screen of `screen` (width / height) best; None when
    /// the theme's own shape does.
    pub fn aspect_key(&self, screen: f32) -> Option<&str> {
        if self.aspects.is_empty() || !(screen.is_finite() && screen > 0.0) {
            return None;
        }
        let dist = |r: f32| (r.ln() - screen.ln()).abs();
        let own = if self.theme.aspect > 0.0 { self.theme.aspect } else { 16.0 / 9.0 };
        let mut best: (f32, Option<&str>) = (dist(own), None);
        for k in self.aspects.keys() {
            if let Some(r) = aspect_ratio(k) {
                let d = dist(r);
                if d + 1e-4 < best.0 {
                    best = (d, Some(k.as_str()));
                }
            }
        }
        best.1
    }

    pub fn to_toml(&self) -> anyhow::Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours() {
        assert_eq!(Color::parse("ff0000"), Some(Color([255, 0, 0, 255])));
        assert_eq!(Color::parse("#00ff0080"), Some(Color([0, 255, 0, 128])));
        assert_eq!(Color::parse("#fff"), Some(Color::WHITE));
        assert_eq!(Color::parse("nope"), None);
    }

    #[test]
    fn toml_roundtrip() {
        let src = r##"
[theme]
name = "T"
[palette]
accent = "#ff8800"
[games]
background = "#000000"
[[games.elements]]
type = "image"
name = "bg"
path = "{art.hero}"
pos = [0.0, 0.0]
size = [1.0, 1.0]
fit = "cover"
[[games.elements]]
type = "grid"
name = "list"
pos = [0.05, 0.2]
size = [0.9, 0.7]
columns = 5
item_image = "{art.cover}"
[[games.elements]]
type = "text"
name = "title"
text = "{game.name}"
style = { font_size = 0.05, align = "center" }
[overrides.steam.games.bg]
path = "steam-bg.jpg"
"##;
        let t = ThemeFile::parse(src).unwrap();
        assert_eq!(t.games.elements.len(), 3);
        assert_eq!(t.games.focus_index(), Some(1));
        assert_eq!(t.palette.accent, Color([255, 136, 0, 255]));
        let again = ThemeFile::parse(&t.to_toml().unwrap()).unwrap();
        assert_eq!(t, again);
    }
}
