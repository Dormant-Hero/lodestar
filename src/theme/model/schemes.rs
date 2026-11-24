//! Colour schemes (`[color_schemes.<name>]`): a theme offers a few sets of colours ("Dark",
//! "Light", "Sunset" …) the player picks in Settings without editing files.
//!
//! ```toml
//! [color_schemes.light]
//! label = "Light"                       # shown in Settings ("" = the key)
//! palette = { dark = false, background = "#f3f4f6", text = "#1b1e24", accent = "#e0533d" }
//! swap = { "#15171c" = "#ffffff" }      # optional: more colours to replace
//! ```
//!
//! How a scheme recolours a theme (`ThemeFile::with_color_scheme`): its `palette` keys replace
//! the theme's palette, and every colour in the views (and in variants, overrides, layouts for
//! other screen shapes) that has the same red / green / blue as a palette colour that changed
//! takes the new one, keeping its own transparency. So an element drawn in the accent colour
//! (`#4c8dff` when the accent is `#4c8dff`, or `#4c8dff80` half transparent) follows the
//! scheme's accent. When two palette colours are the same, the first of `SLOT_ORDER` decides.
//! `swap` maps any other colour ("#rrggbb", transparency ignored) and wins over the palette.

use super::{Color, Palette, ThemeFile};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorScheme {
    /// Name shown to the player ("" = the key).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// Palette keys that change (`dark`, `background`, `accent` …).
    #[serde(skip_serializing_if = "toml::Table::is_empty")]
    pub palette: toml::Table,
    /// More colours to replace: "#rrggbb" → new colour.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub swap: BTreeMap<String, Color>,
}

impl ColorScheme {
    /// The name to show for scheme `key`.
    pub fn label_or<'a>(&'a self, key: &'a str) -> &'a str {
        if self.label.trim().is_empty() { key } else { self.label.trim() }
    }
}

/// Palette keys in the order that decides which one an element colour follows.
pub const SLOT_ORDER: [&str; 11] = [
    "accent",
    "selection",
    "text",
    "text_dim",
    "accent_text",
    "favorite",
    "danger",
    "background",
    "surface",
    "surface_alt",
    "border",
];

/// A palette colour by key (`SLOT_ORDER`).
pub fn palette_slot(p: &Palette, key: &str) -> Option<Color> {
    Some(match key {
        "accent" => p.accent,
        "selection" => p.selection,
        "text" => p.text,
        "text_dim" => p.text_dim,
        "accent_text" => p.accent_text,
        "favorite" => p.favorite,
        "danger" => p.danger,
        "background" => p.background,
        "surface" => p.surface,
        "surface_alt" => p.surface_alt,
        "border" => p.border,
        _ => return None,
    })
}

/// Set a palette colour by key; false for an unknown key.
pub fn set_palette_slot(p: &mut Palette, key: &str, c: Color) -> bool {
    let slot = match key {
        "accent" => &mut p.accent,
        "selection" => &mut p.selection,
        "text" => &mut p.text,
        "text_dim" => &mut p.text_dim,
        "accent_text" => &mut p.accent_text,
        "favorite" => &mut p.favorite,
        "danger" => &mut p.danger,
        "background" => &mut p.background,
        "surface" => &mut p.surface,
        "surface_alt" => &mut p.surface_alt,
        "border" => &mut p.border,
        _ => return false,
    };
    *slot = c;
    true
}

fn rgb(c: Color) -> [u8; 3] {
    [c.0[0], c.0[1], c.0[2]]
}

/// Replace colours (by RGB, keeping each one's alpha) everywhere in `v`: every string that is
/// a `#` hex colour.
pub fn recolor_value(v: &mut toml::Value, map: &HashMap<[u8; 3], [u8; 3]>) {
    match v {
        toml::Value::String(s) => {
            if !s.starts_with('#') || !(s.len() == 7 || s.len() == 9) {
                return;
            }
            if let Some(c) = Color::parse(s)
                && let Some(n) = map.get(&rgb(c))
            {
                *s = Color([n[0], n[1], n[2], c.0[3]]).to_hex();
            }
        }
        toml::Value::Array(a) => a.iter_mut().for_each(|x| recolor_value(x, map)),
        toml::Value::Table(t) => t.iter_mut().map(|(_, v)| v).for_each(|x| recolor_value(x, map)),
        _ => {}
    }
}

/// Which colours a palette change moves: old RGB → new RGB, for every slot that changed.
pub fn palette_moves(old: &Palette, new: &Palette) -> HashMap<[u8; 3], [u8; 3]> {
    let mut map = HashMap::new();
    for k in SLOT_ORDER {
        let (Some(a), Some(b)) = (palette_slot(old, k), palette_slot(new, k)) else { continue };
        if rgb(a) != rgb(b) {
            map.entry(rgb(a)).or_insert(rgb(b));
        }
    }
    map
}

impl ThemeFile {
    /// The theme with colour scheme `key` applied (unchanged when it has no such scheme). The
    /// schemes themselves are kept, so another one can be applied to the original later.
    pub fn with_color_scheme(&self, key: &str) -> ThemeFile {
        let Some(scheme) = self.color_schemes.get(key) else {
            return self.clone();
        };
        self.recolored(&scheme.palette, &scheme.swap).unwrap_or_else(|e| {
            log::warn!("colour scheme {key:?} of theme {:?} not applied: {e:#}", self.theme.name);
            self.clone()
        })
    }

    /// `palette` keys laid over the palette, and the colours that followed them (plus `swap`)
    /// changed through the views.
    pub fn recolored(&self, palette: &toml::Table, swap: &BTreeMap<String, Color>) -> anyhow::Result<ThemeFile> {
        let toml::Value::Table(mut pal) = toml::Value::try_from(&self.palette)? else {
            anyhow::bail!("palette did not serialise to a table");
        };
        for (k, v) in palette {
            pal.insert(k.clone(), v.clone());
        }
        let new_pal: Palette = toml::Value::Table(pal).try_into()?;
        let mut map = palette_moves(&self.palette, &new_pal);
        for (from, to) in swap {
            if let Some(f) = Color::parse(from) {
                map.insert(rgb(f), rgb(*to));
            }
        }
        let mut out = if map.is_empty() {
            self.clone()
        } else {
            let mut whole = toml::Value::try_from(self)?;
            if let toml::Value::Table(t) = &mut whole {
                for (k, v) in t.iter_mut() {
                    if !matches!(k.as_str(), "theme" | "palette" | "fonts" | "color_schemes" | "sounds" | "music") {
                        recolor_value(v, &map);
                    }
                }
            }
            whole.try_into()?
        };
        out.palette = new_pal;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r##"
[theme]
name = "S"
[palette]
accent = "#4c8dff"
text = "#ffffff"
accent_text = "#ffffff"
[games]
[[games.elements]]
type = "rect"
name = "bar"
color = "#4c8dff80"
border_color = "#123456"
[[games.elements]]
type = "text"
name = "title"
text = "#4c8dff"
style = { color = "#ffffff" }
[overrides.steam.games.bar]
color = "#4c8dff"
[color_schemes.warm]
label = "Warm"
palette = { accent = "#ff8a3d", text = "#fff4e0", dark = false }
swap = { "#123456" = "#654321" }
"##;

    #[test]
    fn a_colour_scheme_moves_palette_colours_and_keeps_transparency() {
        let t = ThemeFile::parse(SRC).unwrap();
        let w = t.with_color_scheme("warm");
        assert_eq!(w.palette.accent, Color::parse("#ff8a3d").unwrap());
        assert!(!w.palette.dark);
        // accent_text was white like text, but it didn't change: white elements follow text
        // (the first slot that moved).
        assert_eq!(w.palette.accent_text, Color::WHITE);
        let crate::theme::Element::Rect { color, border_color, .. } = &w.games.elements[0] else { panic!() };
        assert_eq!(color.to_hex(), "#ff8a3d80");
        assert_eq!(border_color.unwrap().to_hex(), "#654321");
        let crate::theme::Element::Text { text, style, .. } = &w.games.elements[1] else { panic!() };
        // Text that only looks like a colour is a colour string too: replaced (documented).
        assert_eq!(text, "#ff8a3d");
        assert_eq!(style.color.to_hex(), "#fff4e0");
        assert_eq!(w.overrides["steam"].games["bar"]["color"].as_str(), Some("#ff8a3d"));
        // Unknown scheme: unchanged.
        assert_eq!(t.with_color_scheme("nope"), t);
        assert_eq!(w.color_schemes["warm"].label_or("warm"), "Warm");
    }
}
