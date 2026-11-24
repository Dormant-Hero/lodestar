//! Built-in themes, embedded in the binary (`assets/themes/*.toml`).
//!
//! CONTRACT: `pub fn all() -> Vec<(&'static str, ThemeFile)>`; the first entry is the default
//! ("Lodestar"). Must never panic (parse failures of embedded TOML are caught by a unit test).
//!
//! Built-in themes have no directory, so they reference no image files; they use artwork
//! bindings only (game art, Lodestar's console art) and `system:` / `builtin:` fonts.

use super::model::*;
use std::sync::OnceLock;

/// (key, embedded TOML). The first one is the default.
pub const SOURCES: [(&str, &str); 10] = [
    ("Lodestar", include_str!("../../assets/themes/lodestar.toml")),
    ("Shelf", include_str!("../../assets/themes/shelf.toml")),
    ("Classic", include_str!("../../assets/themes/classic.toml")),
    ("Lodestar Ambient", include_str!("../../assets/themes/lodestar-ambient.toml")),
    // Playnite Fullscreen-style: navigation rail, home page with cover rows.
    ("Fullscreen", include_str!("../../assets/themes/fullscreen.toml")),
    // Playnite Fullscreen's own arrangement (top bar, cover grid, bottom bar); also the base
    // of imported Playnite fullscreen themes with a cover grid.
    ("Top Bar", include_str!("../../assets/themes/top-bar.toml")),
    // Console style: big artwork above one row of covers (PS5 / Xbox-style imports).
    ("Single Row", include_str!("../../assets/themes/single-row.toml")),
    // Lodestar's console artwork ({collection.art.*}): a console carousel over each console's
    // backdrop, a wall of console cards, and a quiet light list.
    ("Meridian", include_str!("../../assets/themes/meridian.toml")),
    ("Atlas", include_str!("../../assets/themes/atlas.toml")),
    ("Northline", include_str!("../../assets/themes/northline.toml")),
];

fn parsed() -> &'static Vec<(&'static str, ThemeFile)> {
    static CACHE: OnceLock<Vec<(&'static str, ThemeFile)>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut v = Vec::new();
        for (key, src) in SOURCES {
            match ThemeFile::parse(src) {
                Ok(f) => v.push((key, f)),
                Err(e) => log::error!("built-in theme {key} is invalid: {e:#}"),
            }
        }
        if v.is_empty() || v[0].0 != "Lodestar" {
            // Never leave the app without a default.
            let mut t = ThemeFile::default();
            t.theme.name = "Lodestar".into();
            v.insert(0, ("Lodestar", t));
        }
        v
    })
}

/// Built-in themes that were renamed: (old key, new key), so a saved choice keeps working.
pub const RENAMED: &[(&str, &str)] = &[("Playnite Classic", "Top Bar")];

/// The current key of a built-in theme saved under an older name (else `key` itself).
pub fn current_key(key: &str) -> &str {
    RENAMED.iter().find(|(old, _)| old.eq_ignore_ascii_case(key)).map_or(key, |(_, new)| new)
}

pub fn all() -> Vec<(&'static str, ThemeFile)> {
    parsed().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_starter_example_theme_in_the_guide_parses_and_has_its_views() {
        let src = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/themes/starter/theme.toml"));
        let t = ThemeFile::parse(src).expect("examples/themes/starter/theme.toml parses");
        assert_eq!(t.theme.name, "Starter");
        assert!(t.collections.focus_index().is_some(), "the collections carousel can take focus");
        assert!(t.games.focus_index().is_some(), "the games grid can take focus");
    }

    #[test]
    fn a_renamed_builtin_still_loads_under_its_old_name() {
        assert_eq!(current_key("Playnite Classic"), "Top Bar");
        assert_eq!(current_key("playnite classic"), "Top Bar");
        assert_eq!(current_key("Shelf"), "Shelf");
        let m = crate::theme::ThemeManager::new(std::path::PathBuf::from("/nonexistent-themes"));
        let (t, problem) = m.load_checked("Playnite Classic");
        assert_eq!((t.key.as_str(), problem), ("Top Bar", None));
    }

    #[test]
    fn every_builtin_parses() {
        for (key, src) in SOURCES {
            let t = ThemeFile::parse(src).unwrap_or_else(|e| panic!("{key}: {e:#}"));
            assert_eq!(t.theme.name, key);
            assert!(t.collections.focus_index().is_some(), "{key}: collections view needs a focusable element");
            assert!(t.games.focus_index().is_some(), "{key}: games view needs a focusable element");
            // Round-trips through the serializer (the ES converter and overrides rely on it).
            let again = ThemeFile::parse(&t.to_toml().unwrap()).unwrap();
            assert_eq!(t, again, "{key} round-trip");
            if let Some(h) = &t.home {
                assert!(h.focus_index().is_some(), "{key}: the home view needs a focusable element");
            }
            // Every element survives the override merge path unchanged.
            let home = t.home.iter().flat_map(|h| &h.elements);
            for el in t.collections.elements.iter().chain(&t.games.elements).chain(home) {
                let merged = crate::theme::render::merge_element(el, &toml::Table::new()).unwrap();
                assert_eq!(&merged, el, "{key}: {}", el.common().name);
            }
        }
        let all = all();
        assert_eq!(all[0].0, "Lodestar");
        assert_eq!(all.len(), SOURCES.len());
    }

    #[test]
    fn every_builtin_marks_favourites_in_its_game_list() {
        for (key, t) in all() {
            let focus = t.games.focus_index().unwrap();
            let c = t.games.elements[focus].common();
            assert!(c.favorite_badge, "{key}: the game list should show the favourite badge");
            assert!(c.controller_badge, "{key}");
        }
        // Off unless a theme asks for it (converted ES themes opt in), and not written out.
        let el: Element = toml::from_str("type = \"grid\"").unwrap();
        assert!(!el.common().favorite_badge);
        let el: Element = toml::from_str("type = \"grid\"\nfavorite_badge = true").unwrap();
        assert!(el.common().favorite_badge);
        assert!(
            !toml::to_string(&Element::Rect {
                common: Common::default(),
                color: Color::WHITE,
                corner_radius: 0.0,
                border_color: None,
                border_width: 0.0,
                gradient: None,
                fill: None,
                track: None,
            })
            .unwrap()
            .contains("favorite_badge")
        );
    }

    #[test]
    fn the_console_art_themes_show_lodestars_console_art() {
        let all = all();
        for key in ["Meridian", "Atlas", "Northline"] {
            let (_, t) = all.iter().find(|(k, _)| *k == key).unwrap_or_else(|| panic!("{key} missing"));
            let text = t.to_toml().unwrap();
            assert!(text.contains("{collection.art.card}"), "{key}: the collections show the console cards");
            // Every collection reads: one without console art shows its name.
            let focus = t.collections.focus_index().unwrap();
            let el = &t.collections.elements[focus];
            let shows_name = match el {
                Element::Carousel { item_text, .. } | Element::Grid { item_text, .. } => {
                    item_text.contains("{collection.name}")
                }
                Element::TextList { item_text, .. } => item_text.contains("{collection.name}"),
                _ => false,
            };
            assert!(shows_name, "{key}");
            // Only built-in fonts: the art's typeface, the same on every machine.
            for f in [&t.fonts.regular, &t.fonts.bold, &t.fonts.title] {
                assert!(f.as_deref().is_some_and(|f| f.starts_with("builtin:titillium")), "{key}: {f:?}");
            }
        }
    }

    #[test]
    fn lodestar_palette_is_exact() {
        let (_, t) = &all()[0];
        let p = &t.palette;
        let hex = |c: Color| c.to_hex();
        assert_eq!(hex(p.background), "#1b1e24");
        assert_eq!(hex(p.surface), "#22262e");
        assert_eq!(hex(p.surface_alt), "#2b3039");
        assert_eq!(hex(p.text), "#e6e9ef");
        assert_eq!(hex(p.text_dim), "#9aa3b2");
        assert_eq!(hex(p.accent), "#4c8dff");
        assert_eq!(hex(p.accent_text), "#ffffff");
        assert_eq!(hex(p.selection), "#69a2ff");
        assert_eq!(hex(p.border), "#333945");
        assert_eq!(hex(p.favorite), "#ffc44d");
        assert_eq!(hex(p.danger), "#ef5350");
    }
}
