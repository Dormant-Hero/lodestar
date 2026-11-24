//! Writing a whole theme back to `theme.toml` (the theme creator saves through this).
//!
//! The file reads like a hand-written theme: sections in a fixed order (`[theme]`, `[palette]`,
//! `[fonts]` … then the views with one `[[<view>.elements]]` block per element), each element
//! starting with `type` and `name`, values an element would get anyway left out, numbers in
//! their shortest form ("0.05", not "0.05000000074505806").
//!
//! Guarantees:
//!   * `ThemeFile::parse(&file.to_theme_toml(None)?) == file` — checked on every write; when the
//!     tidy form wouldn't parse back the same, the complete form is written instead.
//!   * Keys Lodestar doesn't know survive: inside elements through `Common::extra`; elsewhere
//!     (`[theme]`, `[palette]`, a view, an unknown section) `original` — the table the theme was
//!     read from — gives them back.
//!
//! Comments of a hand-written file are not kept.

use super::ThemeFile;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::OnceLock;
use toml::{Table, Value};

const ROOT_ORDER: [&str; 15] = [
    "theme",
    "palette",
    "fonts",
    "color_schemes",
    "collection_art",
    "sounds",
    "music",
    "desktop",
    "collections",
    "games",
    "details",
    "home",
    "variants",
    "aspects",
    "overrides",
];

const VIEWS: [&str; 4] = ["collections", "games", "details", "home"];
/// Sections whose entries are `CollectionOverride`s.
const PATCH_SETS: [&str; 3] = ["overrides", "variants", "aspects"];
/// Element keys written first, in this order (after `type` and `name`).
const ELEMENT_FIRST: [&str; 9] = ["type", "name", "visible", "visible_if", "hidden_if", "z", "pos", "size", "origin"];
/// Element keys holding nested elements (written as their own blocks).
const NESTED: [&str; 2] = ["children", "item_elements"];

impl ThemeFile {
    /// The theme as `theme.toml` text (see the module docs). `original`: the table the theme
    /// was read from, for keys Lodestar doesn't know outside elements.
    pub fn to_theme_toml(&self, original: Option<&Table>) -> anyhow::Result<String> {
        let full = match Value::try_from(self)? {
            Value::Table(t) => t,
            _ => anyhow::bail!("theme did not serialise to a table"),
        };
        let mut tidy = full.clone();
        tidy_root(&mut tidy);
        let mut tried = vec![tidy, full];
        if let Some(orig) = original {
            let known = ThemeFile::deserialize_table(orig).ok().and_then(|f| match Value::try_from(f).ok()? {
                Value::Table(t) => Some(t),
                _ => None,
            });
            if let Some(known) = known {
                for t in &mut tried {
                    carry_unknown(orig, &known, t);
                }
            }
        }
        for t in &tried {
            let text = emit_root(t);
            if ThemeFile::parse(&text).is_ok_and(|back| back == *self) {
                return Ok(text);
            }
        }
        // Last resort: the serializer's own output (parses back the same for every theme the
        // tests know).
        Ok(toml::to_string_pretty(self)?)
    }

    /// Parse an already-read table.
    pub fn deserialize_table(t: &Table) -> anyhow::Result<ThemeFile> {
        Ok(Value::Table(t.clone()).try_into()?)
    }
}

/// Numbers in their shortest form as f32 (every number of the format is an f32 or an integer),
/// recursively.
fn shorten_floats(v: &mut Value) {
    match v {
        Value::Float(f) => {
            let s = format!("{}", *f as f32);
            if let Ok(x) = s.parse::<f64>()
                && (x as f32) == (*f as f32)
            {
                *f = x;
            }
        }
        Value::Array(a) => a.iter_mut().for_each(shorten_floats),
        Value::Table(t) => t.iter_mut().map(|(_, v)| v).for_each(shorten_floats),
        _ => {}
    }
}

/// The table an element of `kind` gets when it sets nothing (its defaults), cached.
fn default_element(kind: &str) -> Option<&'static Table> {
    static CACHE: OnceLock<parking_lot::Mutex<HashMap<String, Option<&'static Table>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let mut c = cache.lock();
    if let Some(t) = c.get(kind) {
        return *t;
    }
    let mut src = Table::new();
    src.insert("type".into(), Value::String(kind.into()));
    let made = Value::Table(src).try_into::<super::Element>().ok().and_then(|e| Value::try_from(e).ok()).and_then(
        |v| match v {
            Value::Table(mut t) => {
                t.iter_mut().map(|(_, v)| v).for_each(shorten_floats);
                Some(&*Box::leak(Box::new(t)))
            }
            _ => None,
        },
    );
    c.insert(kind.to_string(), made);
    made
}

/// Leave out of an element table what equals its defaults (recursively into plain tables).
fn prune_element(t: &mut Table) {
    for k in NESTED {
        if let Some(Value::Array(a)) = t.get_mut(k) {
            for e in a.iter_mut() {
                if let Value::Table(et) = e {
                    prune_element(et);
                }
            }
        }
    }
    let Some(kind) = t.get("type").and_then(Value::as_str).map(str::to_string) else { return };
    let Some(def) = default_element(&kind) else { return };
    prune_against(t, def, true);
}

fn prune_against(t: &mut Table, def: &Table, top: bool) {
    let keys: Vec<String> = t.keys().cloned().collect();
    for k in keys {
        if top && (k == "type" || NESTED.contains(&k.as_str())) {
            continue;
        }
        let Some(d) = def.get(&k) else { continue };
        let same = match (t.get_mut(&k), d) {
            (Some(Value::Table(sub)), Value::Table(dsub)) => {
                prune_against(sub, dsub, false);
                sub.is_empty()
            }
            (Some(v), d) => v == d,
            (None, _) => false,
        };
        if same {
            t.remove(&k);
        }
    }
}

fn tidy_root(t: &mut Table) {
    t.iter_mut().map(|(_, v)| v).for_each(shorten_floats);
    for view in VIEWS {
        if let Some(Value::Table(vt)) = t.get_mut(view)
            && let Some(Value::Array(els)) = vt.get_mut("elements")
        {
            for e in els.iter_mut() {
                if let Value::Table(et) = e {
                    prune_element(et);
                }
            }
        }
    }
    // Empty per-view tables of overrides / variants / layouts, and empty sets.
    for set in PATCH_SETS {
        if let Some(Value::Table(s)) = t.get_mut(set) {
            for o in s.iter_mut().map(|(_, v)| v) {
                if let Value::Table(ot) = o {
                    ot.retain(|_, v| !matches!(v, Value::Table(x) if x.is_empty()));
                }
            }
        }
        if matches!(t.get(set), Some(Value::Table(s)) if s.is_empty()) {
            t.remove(set);
        }
    }
}

/// Copy keys of `raw` that the model doesn't know (absent from `known`, the model's reading of
/// `raw` written out again) into `out`, through tables only (arrays are the model's own).
fn carry_unknown(raw: &Table, known: &Table, out: &mut Table) {
    for (k, rv) in raw {
        match known.get(k) {
            None => {
                if !out.contains_key(k) {
                    out.insert(k.clone(), rv.clone());
                }
            }
            Some(Value::Table(kt)) => {
                if let (Value::Table(rt), Some(Value::Table(ot))) = (rv, out.get_mut(k)) {
                    carry_unknown(rt, kt, ot);
                }
            }
            Some(_) => {}
        }
    }
}

/// A key as TOML writes it: bare when it can be.
fn toml_key(k: &str) -> String {
    if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        k.to_string()
    } else {
        Value::String(k.to_string()).to_string()
    }
}

fn path_of(parts: &[&str]) -> String {
    parts.iter().map(|p| toml_key(p)).collect::<Vec<_>>().join(".")
}

fn inline(out: &mut String, k: &str, v: &Value) {
    let _ = writeln!(out, "{} = {}", toml_key(k), v);
}

/// What goes into its own `[section]` (true) instead of a `key = value` line.
#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    View,
    Element,
    PatchSet,
    Patch,
    /// Everything inline.
    Flat,
}

fn emit_root(t: &Table) -> String {
    let mut out = String::new();
    // Plain values first: they belong to the root only before any [section].
    let mut keys: Vec<&String> = t.keys().collect();
    keys.sort_by_key(|k| (ROOT_ORDER.iter().position(|r| r == k).unwrap_or(ROOT_ORDER.len()), (*k).clone()));
    for k in &keys {
        if !matches!(t[k.as_str()], Value::Table(_)) {
            inline(&mut out, k, &t[k.as_str()]);
        }
    }
    for k in &keys {
        if let Value::Table(sub) = &t[k.as_str()] {
            let ctx = if VIEWS.contains(&k.as_str()) {
                Ctx::View
            } else if PATCH_SETS.contains(&k.as_str()) || matches!(k.as_str(), "color_schemes" | "collection_art") {
                Ctx::PatchSet
            } else {
                Ctx::Flat
            };
            emit_section(&mut out, &[k.as_str()], sub, ctx, false);
        }
    }
    out
}

fn emit_section(out: &mut String, path: &[&str], t: &Table, ctx: Ctx, array: bool) {
    if !out.is_empty() {
        out.push('\n');
    }
    if array {
        let _ = writeln!(out, "[[{}]]", path_of(path));
    } else {
        let _ = writeln!(out, "[{}]", path_of(path));
    }
    let mut keys: Vec<&String> = t.keys().collect();
    if ctx == Ctx::Element {
        keys.sort_by_key(|k| (ELEMENT_FIRST.iter().position(|r| r == k).unwrap_or(ELEMENT_FIRST.len()), (*k).clone()));
    }
    let own_section = |k: &str, v: &Value| -> bool {
        match ctx {
            Ctx::View => k == "elements" && is_table_array(v),
            Ctx::Element => NESTED.contains(&k) && is_table_array(v),
            Ctx::PatchSet => matches!(v, Value::Table(_)),
            Ctx::Patch => VIEWS.contains(&k) && matches!(v, Value::Table(_)),
            Ctx::Flat => false,
        }
    };
    for k in &keys {
        let v = &t[k.as_str()];
        if !own_section(k, v) {
            inline(out, k, v);
        }
    }
    for k in &keys {
        let v = &t[k.as_str()];
        if !own_section(k, v) {
            continue;
        }
        let mut p: Vec<&str> = path.to_vec();
        p.push(k.as_str());
        match (ctx, v) {
            (Ctx::View | Ctx::Element, Value::Array(a)) => {
                for e in a {
                    if let Value::Table(et) = e {
                        emit_section(out, &p, et, Ctx::Element, true);
                    }
                }
            }
            (Ctx::PatchSet, Value::Table(sub)) => {
                let next = if matches!(path.first(), Some(&"color_schemes" | &"collection_art")) {
                    Ctx::Flat
                } else {
                    Ctx::Patch
                };
                emit_section(out, &p, sub, next, false);
            }
            (Ctx::Patch, Value::Table(sub)) => emit_section(out, &p, sub, Ctx::Flat, false),
            _ => {}
        }
    }
}

fn is_table_array(v: &Value) -> bool {
    matches!(v, Value::Array(a) if !a.is_empty() && a.iter().all(|e| matches!(e, Value::Table(_))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_theme_is_written_tidy_and_reads_back_the_same() {
        for (key, src) in crate::theme::builtin::SOURCES {
            let t = ThemeFile::parse(src).unwrap();
            let text = t.to_theme_toml(None).unwrap();
            assert_eq!(ThemeFile::parse(&text).unwrap(), t, "{key}");
            // The tidy form, not a fallback: elements start with their type, no float noise,
            // defaults left out.
            assert!(text.contains("[[games.elements]]\ntype = "), "{key}:\n{text}");
            let noisy = text
                .split(|c: char| !(c.is_ascii_digit() || c == '.'))
                .any(|w| w.split_once('.').is_some_and(|(a, b)| !a.is_empty() && b.len() > 7));
            assert!(!noisy, "{key}: float noise\n{text}");
            assert!(!text.contains("visible = true"), "{key}: defaults written\n{text}");
            // Writing again gives the very same text (stable files, quiet diffs).
            assert_eq!(ThemeFile::parse(&text).unwrap().to_theme_toml(None).unwrap(), text, "{key}");
        }
    }

    #[test]
    fn unknown_keys_survive_a_save_everywhere() {
        let src = r##"
top_level_note = "made by hand"
[theme]
name = "U"
website = "example.org"
[palette]
accent = "#ff0000"
glow = "#00ff00"
[games]
mood = "calm"
[[games.elements]]
type = "image"
name = "bg"
path = "{art.hero}"
future_effect = { strength = 0.5 }
[[games.elements]]
type = "stack"
name = "s"
[[games.elements.children]]
type = "text"
text = "{game.name}"
sparkle = true
[future_section]
a = 1
"##;
        let raw: Table = toml::from_str(src).unwrap();
        let t = ThemeFile::parse(src).unwrap();
        let text = t.to_theme_toml(Some(&raw)).unwrap();
        let back: Table = toml::from_str(&text).unwrap();
        assert_eq!(back["top_level_note"].as_str(), Some("made by hand"));
        assert_eq!(back["theme"]["website"].as_str(), Some("example.org"));
        assert_eq!(back["palette"]["glow"].as_str(), Some("#00ff00"));
        assert_eq!(back["games"]["mood"].as_str(), Some("calm"));
        assert_eq!(back["future_section"]["a"].as_integer(), Some(1));
        let els = back["games"]["elements"].as_array().unwrap();
        assert_eq!(els[0]["future_effect"]["strength"].as_float(), Some(0.5));
        assert_eq!(els[1]["children"][0]["sparkle"].as_bool(), Some(true));
        assert_eq!(ThemeFile::parse(&text).unwrap(), t);
        // The element's unknown key is in the model (the creator shows it).
        assert!(t.games.elements[0].common().extra.contains_key("future_effect"));
    }

    #[test]
    fn overrides_variants_and_schemes_are_written_as_readable_sections() {
        let src = r##"
[theme]
name = "O"
[games]
[[games.elements]]
type = "image"
name = "bg"
[overrides."platform:sony_playstation2".games.bg]
path = "consoles/ps2.jpg"
[overrides.steam]
display_name = "Steam"
[variants.Grid.games.bg]
opacity = 0.5
[aspects."4:3".games.bg]
size = [1.0, 0.8]
[color_schemes.light]
label = "Light"
palette = { dark = false }
"##;
        let t = ThemeFile::parse(src).unwrap();
        let text = t.to_theme_toml(None).unwrap();
        assert_eq!(ThemeFile::parse(&text).unwrap(), t);
        assert!(text.contains("[overrides.\"platform:sony_playstation2\".games]\nbg = "), "{text}");
        assert!(text.contains("[variants.Grid.games]"), "{text}");
        assert!(text.contains("[aspects.\"4:3\".games]"), "{text}");
        assert!(text.contains("[color_schemes.light]\nlabel = \"Light\""), "{text}");
        // Empty per-view tables aren't written.
        assert!(!text.contains("[overrides.steam.games]"), "{text}");
    }
}
