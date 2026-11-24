//! Small monochrome device icons: one outline per form factor (monitor, home console, handheld,
//! dual-screen, hybrid, tablet, disc, arcade cabinet, home computer, gamepad), plus the
//! collection folders of the sidebar. Lodestar's own drawings (assets/platforms/*.svg), no
//! makers' logos. White on transparent, rendered with resvg once per size and cached as a
//! texture; painted tinted with the theme's text colour.

use crate::emulation::platforms::{self, Platform};
use crate::library::{Collection, CollectionKind};
use crate::model::Game;
use egui::{Color32, Context, Painter, Rect, TextureHandle, pos2, vec2};
use std::collections::HashMap;

/// A device outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
    /// PC (Windows, Linux, macOS, DOS).
    Monitor,
    /// Keyboard home computers (C64, Amiga, MSX, ZX Spectrum …).
    Computer,
    /// Home consoles (a box with a slot).
    Console,
    /// Upright handhelds (Game Boy).
    Handheld,
    /// Landscape handhelds (GBA, PSP, Vita, Game Gear …).
    HandheldWide,
    /// Clamshells (DS, 3DS).
    DualScreen,
    /// A tablet with controllers on its sides (Switch).
    Hybrid,
    /// A tablet controller (Wii U).
    Tablet,
    /// CD add-ons and CD consoles.
    Disc,
    /// Arcade cabinet.
    Arcade,
    /// Anything else.
    Gamepad,
    /// A manual collection (folder).
    Collection,
    /// A smart collection (folder with a flash).
    Smart,
}

impl Shape {
    #[cfg(test)]
    pub const ALL: [Shape; 13] = [
        Shape::Monitor,
        Shape::Computer,
        Shape::Console,
        Shape::Handheld,
        Shape::HandheldWide,
        Shape::DualScreen,
        Shape::Hybrid,
        Shape::Tablet,
        Shape::Disc,
        Shape::Arcade,
        Shape::Gamepad,
        Shape::Collection,
        Shape::Smart,
    ];

    fn svg(self) -> &'static [u8] {
        match self {
            Shape::Monitor => include_bytes!("../../assets/platforms/monitor.svg"),
            Shape::Computer => include_bytes!("../../assets/platforms/computer.svg"),
            Shape::Console => include_bytes!("../../assets/platforms/console.svg"),
            Shape::Handheld => include_bytes!("../../assets/platforms/handheld.svg"),
            Shape::HandheldWide => include_bytes!("../../assets/platforms/handheld_wide.svg"),
            Shape::DualScreen => include_bytes!("../../assets/platforms/dual_screen.svg"),
            Shape::Hybrid => include_bytes!("../../assets/platforms/hybrid.svg"),
            Shape::Tablet => include_bytes!("../../assets/platforms/tablet.svg"),
            Shape::Disc => include_bytes!("../../assets/platforms/disc.svg"),
            Shape::Arcade => include_bytes!("../../assets/platforms/arcade.svg"),
            Shape::Gamepad => include_bytes!("../../assets/platforms/gamepad.svg"),
            Shape::Collection => include_bytes!("../../assets/platforms/collection.svg"),
            Shape::Smart => include_bytes!("../../assets/platforms/smart.svg"),
        }
    }
}

/// The outline for a Playnite platform id.
pub fn shape_for(id: &str) -> Shape {
    match id {
        "pc_windows" | "pc_linux" | "pc_dos" | "macintosh" | "adobe_flash" | "nec_pc98" => Shape::Monitor,
        "amstrad_cpc"
        | "apple_2"
        | "atari_8bit"
        | "atari_st"
        | "atari_falcon030"
        | "commodore_64"
        | "commodore_amiga"
        | "commodore_cbm5x0"
        | "commodore_cbm2"
        | "commodore_pet"
        | "commodore_plus4"
        | "commodore_vci20"
        | "microsoft_msx"
        | "microsoft_msx2"
        | "sharp_x68000"
        | "sinclair_zxspectrum3"
        | "sinclair_zxspectrum"
        | "sinclair_zx81"
        | "thomson_mo5"
        | "thomson_to7" => Shape::Computer,
        "nintendo_gameboy" | "nintendo_gameboycolor" | "ti_83" | "watara_supervision" => Shape::Handheld,
        "nintendo_gameboyadvance"
        | "sony_psp"
        | "sony_vita"
        | "sega_gamegear"
        | "atari_lynx"
        | "snk_neogeopocket"
        | "snk_neogeopocket_color"
        | "bandai_wonderswan"
        | "bandai_wonderswan_color" => Shape::HandheldWide,
        "nintendo_ds" | "nintendo_dsi" | "nintendo_3ds" => Shape::DualScreen,
        "nintendo_switch" => Shape::Hybrid,
        "nintendo_wiiu" => Shape::Tablet,
        "sega_cd" | "nec_turbografx_cd" | "snk_neogeo_cd" | "3do" | "commodore_amiga_cd32" | "nec_pcfx" => Shape::Disc,
        "arcade" => Shape::Arcade,
        "tic_80" | "uzebox" => Shape::Gamepad,
        _ if platforms::by_id(id).is_some() => Shape::Console,
        _ => Shape::Gamepad,
    }
}

/// The outline for a platform as the explorer spells it ("Windows", "Sony PlayStation 2").
pub fn shape_for_label(label: &str) -> Shape {
    let l = label.trim();
    if l.eq_ignore_ascii_case("macos") || l.eq_ignore_ascii_case("pc") {
        return Shape::Monitor;
    }
    platforms::by_name(l).map_or(Shape::Gamepad, |p| shape_for(p.id))
}

/// The console a game is for: an emulated game's platform, else the platform set on the game
/// when it names a known one ("windows" → PC (Windows), "Sony PlayStation 2").
pub fn platform_of(g: &Game) -> Option<&'static Platform> {
    g.emu
        .as_ref()
        .and_then(|e| platforms::by_id(&e.platform))
        .or_else(|| g.platform.as_deref().and_then(platforms::by_name))
}

/// The outline for a game: its console's, else a gamepad (PC and store games).
pub fn game_shape(g: &Game) -> Shape {
    platform_of(g).map_or(Shape::Gamepad, |p| shape_for(p.id))
}

/// Is this one of the PC platforms (not a console)?
pub fn is_pc(id: &str) -> bool {
    matches!(id, "pc_windows" | "pc_linux" | "pc_dos" | "macintosh")
}

/// Is this explorer platform label a PC one ("Windows", "Linux", "macOS", "DOS")?
pub fn is_pc_label(label: &str) -> bool {
    let l = label.trim();
    l.eq_ignore_ascii_case("macos")
        || l.eq_ignore_ascii_case("pc")
        || l.eq_ignore_ascii_case("browser")
        || platforms::by_name(l).is_some_and(|p| is_pc(p.id))
}

/// Short name for the explorer ("Sony PlayStation 2" → "PS2", "Windows" → "Windows"); the
/// label itself when it isn't a known platform.
pub fn short_label(label: &str) -> &str {
    let l = label.trim();
    if l.eq_ignore_ascii_case("pc") {
        return l;
    }
    platforms::by_name(l).map_or(l, |p| p.short)
}

/// Makers a console name can leave out ("Sony PlayStation 2" → "PlayStation 2"): cards and
/// captions are narrow.
const CONSOLE_MAKERS: &[&str] = &[
    "Sony",
    "Nintendo",
    "Microsoft",
    "Sega",
    "NEC",
    "SNK",
    "Bandai",
    "Atari",
    "Commodore",
    "Mattel",
    "Coleco",
    "Magnavox",
    "GCE",
    "Sharp",
    "Sinclair",
    "Amstrad",
    "Thomson",
    "Watara",
];

/// A console's name without the maker when what is left still names it ("GameCube", "Wii U",
/// "PlayStation 2"; but "Nintendo 64", "Nintendo DS", "Atari 2600", "Sega CD").
pub fn console_name(id: &str) -> String {
    if id == "nintendo_nes" {
        return "NES".into();
    }
    let full = crate::library::platform_label(id);
    CONSOLE_MAKERS
        .iter()
        .find_map(|m| full.strip_prefix(m)?.strip_prefix(' '))
        .filter(|rest| rest.chars().count() >= 3 && rest.starts_with(|c: char| c.is_alphabetic()))
        .map_or_else(|| full.clone(), str::to_string)
}

/// Textures by (shape, pixel size).
#[derive(Clone, Default)]
struct Cache(HashMap<(Shape, u32), TextureHandle>);

fn cache_id() -> egui::Id {
    egui::Id::unique("ls_platform_icons")
}

/// Render `shape` as premultiplied white-on-transparent RGBA, `px` × `px`.
fn rasterize(shape: Shape, px: u32) -> Option<egui::ColorImage> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_data(shape.svg(), &usvg::Options::default()).ok()?;
    let s = tree.size();
    let scale = px as f32 / s.width().max(s.height());
    let mut pm = tiny_skia::Pixmap::new(px, px)?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pm.as_mut());
    Some(egui::ColorImage::from_rgba_premultiplied([px as usize, px as usize], pm.data()))
}

/// The texture for `shape` at `px` pixels (rounded up to a multiple of 4 to keep few sizes).
pub fn texture(ctx: &Context, shape: Shape, px: u32) -> Option<TextureHandle> {
    let px = px.clamp(8, 512).div_ceil(4) * 4;
    let key = (shape, px);
    // Look up in place (`get_temp` would clone the whole map, and every texture handle clone
    // takes the texture manager's lock).
    if let Some(t) = ctx.data_mut(|d| d.get_temp_mut_or_default::<Cache>(cache_id()).0.get(&key).cloned()) {
        return Some(t);
    }
    let img = rasterize(shape, px)?;
    let tex = ctx.load_texture(format!("ls_platform_{shape:?}_{px}"), img, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Cache>(cache_id()).0.insert(key, tex.clone()));
    Some(tex)
}

/// Paint `shape` into the square centred in `rect`, tinted `color`.
pub fn paint(p: &Painter, rect: Rect, shape: Shape, color: Color32) {
    let side = rect.width().min(rect.height());
    if side < 1.0 || color.a() == 0 {
        return;
    }
    let sq = Rect::from_center_size(rect.center(), vec2(side, side));
    let px = (side * p.ctx().pixels_per_point()).ceil() as u32;
    if let Some(tex) = texture(p.ctx(), shape, px) {
        p.image(tex.id(), sq, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), color);
    }
}

/// The outline for a controller-mode collection card, if it has one (consoles and the user's
/// collections).
pub fn collection_shape(c: &Collection) -> Option<Shape> {
    match &c.kind {
        CollectionKind::Platform(id) => Some(shape_for(id)),
        CollectionKind::User(u) if u.is_smart() => Some(Shape::Smart),
        CollectionKind::User(_) => Some(Shape::Collection),
        _ => None,
    }
}

/// Controller themes: a collection card without artwork (the theme has no console art for it)
/// gets its outline above the label. Returns the rect to centre the label in (moved down when
/// an icon was drawn).
pub fn theme_card_icon(p: &Painter, rect: Rect, c: Option<&Collection>, color: Color32, angle: f32) -> Rect {
    let Some(shape) = c.and_then(collection_shape) else {
        return rect;
    };
    if angle != 0.0 || rect.height() < 24.0 {
        return rect;
    }
    let side = (rect.width().min(rect.height()) * 0.36).min(rect.height() * 0.42);
    let c = pos2(rect.center().x, rect.top() + rect.height() * 0.38);
    paint(p, Rect::from_center_size(c, vec2(side, side)), shape, color);
    rect.translate(vec2(0.0, rect.height() * 0.2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_renders() {
        for s in Shape::ALL {
            let img = rasterize(s, 32).unwrap_or_else(|| panic!("{s:?} does not parse"));
            assert_eq!(img.size, [32, 32]);
            let lit = img.pixels.iter().filter(|p| p.a() > 0).count();
            assert!(lit > 40, "{s:?}: only {lit} pixels drawn");
            // White, so tinting gives the theme's colour.
            assert!(img.pixels.iter().all(|p| p.r() == p.a() && p.g() == p.a() && p.b() == p.a()));
        }
    }

    #[test]
    fn shapes_and_short_names() {
        assert_eq!(shape_for("sony_playstation2"), Shape::Console);
        assert_eq!(shape_for("nintendo_switch"), Shape::Hybrid);
        assert_eq!(shape_for("nintendo_3ds"), Shape::DualScreen);
        assert_eq!(shape_for("nintendo_gameboy"), Shape::Handheld);
        assert_eq!(shape_for("sony_psp"), Shape::HandheldWide);
        assert_eq!(shape_for("nintendo_wiiu"), Shape::Tablet);
        assert_eq!(shape_for("arcade"), Shape::Arcade);
        assert_eq!(shape_for("commodore_64"), Shape::Computer);
        assert_eq!(shape_for("nope"), Shape::Gamepad);
        assert_eq!(shape_for_label("Windows"), Shape::Monitor);
        assert_eq!(shape_for_label("Linux"), Shape::Monitor);
        assert_eq!(shape_for_label("macOS"), Shape::Monitor);
        assert_eq!(shape_for_label("Sony PlayStation 2"), Shape::Console);
        assert_eq!(short_label("Sony PlayStation 2"), "PS2");
        assert_eq!(short_label("Sony PlayStation"), "PS1");
        assert_eq!(short_label("Nintendo Wii U"), "Wii U");
        assert_eq!(short_label("Windows"), "Windows");
        assert_eq!(short_label("Linux"), "Linux");
        assert_eq!(short_label("macOS"), "macOS");
        assert!(is_pc_label("Windows") && is_pc_label("Linux") && is_pc_label("macOS"));
        assert!(!is_pc_label("Sony PlayStation 2"));
        // Every platform has an outline other than the fallback, except the fantasy consoles.
        for p in platforms::all() {
            if !matches!(p.id, "tic_80" | "uzebox") {
                assert_ne!(shape_for(p.id), Shape::Gamepad, "{}", p.id);
            }
        }
    }

    #[test]
    fn console_names_drop_the_maker_when_it_still_reads() {
        assert_eq!(console_name("sony_playstation2"), "PlayStation 2");
        assert_eq!(console_name("nintendo_gamecube"), "GameCube");
        assert_eq!(console_name("nintendo_64"), "Nintendo 64");
        assert_eq!(console_name("nintendo_ds"), "Nintendo DS");
        assert_eq!(console_name("sega_cd"), "Sega CD");
        assert_eq!(console_name("nintendo_nes"), "NES");
        assert_eq!(console_name("atari_2600"), "Atari 2600");
    }

    #[test]
    fn platform_of_games() {
        let mut g = Game::blank("x");
        assert!(platform_of(&g).is_none());
        g.platform = Some("windows".into());
        assert_eq!(platform_of(&g).map(|p| p.id), Some("pc_windows"));
        g.emu = Some(crate::model::EmuRef { platform: "nintendo_switch".into(), ..Default::default() });
        assert_eq!(platform_of(&g).map(|p| p.id), Some("nintendo_switch"));
    }
}
