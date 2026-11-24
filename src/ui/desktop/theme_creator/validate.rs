//! Checking a theme, in plain words: what would stop it working, what looks like a mistake,
//! and what is only worth knowing.

use super::bindings;
use super::files::theme_file_exists;
use super::ops::ElPath;
use crate::theme::*;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Controller mode can't use the theme as it is.
    Problem,
    /// Probably not what was meant.
    Warning,
    Note,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub level: Level,
    pub text: String,
    /// The element it is about (click to select it).
    pub at: Option<(ViewKind, ElPath)>,
}

pub const VIEW_NAMES: [(ViewKind, &str); 4] = [
    (ViewKind::Home, "Start page"),
    (ViewKind::Collections, "Collections"),
    (ViewKind::Games, "Games"),
    (ViewKind::Details, "Game page"),
];

pub fn view_name(k: ViewKind) -> &'static str {
    VIEW_NAMES.iter().find(|(v, _)| *v == k).map(|(_, n)| *n).unwrap_or("view")
}

fn label(e: &Element) -> String {
    let n = e.common().name.trim();
    if n.is_empty() { format!("a {}", super::ops::kind_word(e).to_lowercase()) } else { format!("“{n}”") }
}

/// Every text of an element that may hold bindings, with what it is.
fn texts(e: &Element) -> Vec<&str> {
    let c = e.common();
    let mut v: Vec<&str> = vec![];
    v.extend(c.visible_if.as_deref());
    v.extend(c.hidden_if.as_deref());
    v.extend(c.item_subtext.as_deref());
    v.extend(c.item_caption.as_deref());
    match e {
        Element::Image { path, fallbacks, fallback_text, .. } => {
            v.push(path);
            v.extend(fallbacks.iter().map(String::as_str));
            v.extend(fallback_text.as_deref());
        }
        Element::Text { text, .. } => v.push(text),
        Element::Carousel { item_image, item_fallbacks, item_text, .. }
        | Element::Grid { item_image, item_fallbacks, item_text, .. } => {
            v.push(item_image);
            v.extend(item_fallbacks.iter().map(String::as_str));
            v.push(item_text);
        }
        Element::Rows { item_image, item_fallbacks, item_text, title, .. } => {
            v.push(item_image);
            v.extend(item_fallbacks.iter().map(String::as_str));
            v.push(item_text);
            v.push(title);
        }
        Element::TextList { item_text, .. } => v.push(item_text),
        Element::Rating { value, .. } | Element::DateTime { value, .. } => v.push(value),
        Element::Rect { fill: Some(f), .. } => v.push(f),
        _ => {}
    }
    v
}

/// Theme files an element uses (pictures, videos, fonts).
fn files(e: &Element) -> Vec<&str> {
    let mut v: Vec<&str> = vec![];
    match e {
        Element::Image { path, fallbacks, video, .. } => {
            // Only the first picture must exist (the others are fallbacks); a fixed path is
            // checked.
            if !path.contains('{') {
                v.push(path);
            }
            let _ = fallbacks;
            v.extend(video.as_deref().filter(|p| !p.contains('{')));
        }
        Element::TextList { selector_image: Some(p), .. } => v.push(p),
        Element::Rating { filled_path, unfilled_path, .. } => {
            v.extend(filled_path.as_deref());
            v.extend(unfilled_path.as_deref());
        }
        _ => {}
    }
    v
}

fn text_style(e: &Element) -> Option<&TextStyle> {
    match e {
        Element::Text { style, .. }
        | Element::TextList { style, .. }
        | Element::DateTime { style, .. }
        | Element::Rail { style, .. }
        | Element::HelpBar { style, .. } => Some(style),
        Element::Carousel { item_text_style, .. } | Element::Grid { item_text_style, .. } => Some(item_text_style),
        _ => None,
    }
}

/// Check the whole theme. `dir`: its folder (None: files aren't checked).
pub fn check(file: &ThemeFile, dir: Option<&Path>) -> Vec<Finding> {
    let mut out = vec![];
    let mut add = |level, text: String, at: Option<(ViewKind, ElPath)>| out.push(Finding { level, text, at });
    if file.theme.name.trim().is_empty() {
        add(Level::Problem, "The theme needs a name (Theme → Name).".into(), None);
    }
    let font_names: Vec<String> =
        ["regular", "bold", "title"].iter().map(|s| s.to_string()).chain(file.fonts.extra.keys().cloned()).collect();
    if let Some(d) = dir {
        for (k, v) in [("regular", &file.fonts.regular), ("bold", &file.fonts.bold), ("title", &file.fonts.title)] {
            if let Some(p) = v
                && !theme_file_exists(d, p)
            {
                add(Level::Warning, format!("The {k} font file “{p}” isn't in the theme folder."), None);
            }
        }
        for (k, p) in &file.fonts.extra {
            if !theme_file_exists(d, p) {
                add(Level::Warning, format!("The font “{k}” points to “{p}”, which isn't in the theme folder."), None);
            }
        }
        for (k, p) in &file.sounds {
            if !theme_file_exists(d, p) {
                add(Level::Warning, format!("The sound for “{k}” (“{p}”) isn't in the theme folder."), None);
            }
        }
        for (k, a) in &file.collection_art {
            for (slot, label) in super::files::ART_SLOTS {
                if let Some(p) = a.get(slot)
                    && !theme_file_exists(d, p)
                {
                    add(
                        Level::Warning,
                        format!("The {} picture for “{k}” (“{p}”) isn't in the theme folder.", label.to_lowercase()),
                        None,
                    );
                }
            }
        }
        for p in &file.music {
            if !theme_file_exists(d, p) {
                add(Level::Warning, format!("The music “{p}” isn't in the theme folder."), None);
            }
        }
    }
    for (kind, vname) in VIEW_NAMES {
        let Some(view) = file.view(kind) else { continue };
        let needs_list = matches!(kind, ViewKind::Collections | ViewKind::Games);
        if needs_list && !view.elements.iter().any(|e| e.is_focusable() && e.common().visible) {
            let what = if kind == ViewKind::Games { "games" } else { "collections" };
            add(
                Level::Problem,
                format!(
                    "The {vname} screen has no list to pick {what} from. Add a carousel, wheel, grid or list (Add → Lists)."
                ),
                None,
            );
        }
        if view.elements.iter().filter(|e| e.is_focusable() && e.common().visible).count() > 1 {
            add(
                Level::Note,
                format!(
                    "The {vname} screen has more than one list; the controller moves through the first one in the elements list."
                ),
                None,
            );
        }
        let mut names = std::collections::HashMap::<String, usize>::new();
        for (i, top) in view.elements.iter().enumerate() {
            let mut stack: Vec<(ElPath, &Element)> = vec![(ElPath::top(i), top)];
            while let Some((p, e)) = stack.pop() {
                let c = e.common();
                let at = Some((kind, p.clone()));
                if !c.name.trim().is_empty() && p.sub.is_empty() {
                    *names.entry(c.name.trim().to_string()).or_default() += 1;
                }
                for t in texts(e) {
                    for k in bindings::unknown_in(t) {
                        add(
                            Level::Warning,
                            format!("{} ({vname}) uses {{{k}}}, which Lodestar doesn't know — a typo?", label(e)),
                            at.clone(),
                        );
                    }
                }
                if let Some(d) = dir {
                    for f in files(e) {
                        if !theme_file_exists(d, f) {
                            add(
                                Level::Warning,
                                format!("{} ({vname}) shows “{f}”, which isn't in the theme folder.", label(e)),
                                at.clone(),
                            );
                        }
                    }
                }
                if let Some(s) = text_style(e)
                    && let Some(f) = s.font.as_deref()
                    && !f.contains('/')
                    && !f.contains('.')
                    && !f.contains(':')
                    && !font_names.iter().any(|n| n == f)
                {
                    add(
                        Level::Warning,
                        format!("{} ({vname}) uses the font “{f}”, which the theme doesn't have.", label(e)),
                        at.clone(),
                    );
                }
                if p.sub.is_empty() && c.visible && c.size[0] > 0.0 && c.size[1] > 0.0 {
                    let (x0, y0) = (c.pos[0] - c.origin[0] * c.size[0], c.pos[1] - c.origin[1] * c.size[1]);
                    if x0 >= 1.0 || y0 >= 1.0 || x0 + c.size[0] <= 0.0 || y0 + c.size[1] <= 0.0 {
                        add(Level::Warning, format!("{} ({vname}) is outside the screen.", label(e)), at.clone());
                    }
                }
                if c.visible && c.opacity <= 0.0 {
                    add(Level::Note, format!("{} ({vname}) is fully transparent.", label(e)), at.clone());
                }
                if !c.extra.is_empty() {
                    let keys: Vec<&str> = c.extra.keys().map(String::as_str).collect();
                    add(
                        Level::Note,
                        format!(
                            "{} ({vname}) has settings Lodestar doesn't use: {} (kept in the file).",
                            label(e),
                            keys.join(", ")
                        ),
                        at.clone(),
                    );
                }
                if let Element::Stack { children, .. } = e {
                    for (j, ch) in children.iter().enumerate() {
                        stack.push((p.child(super::ops::Nest::Children, j), ch));
                    }
                }
                for (j, ch) in c.item_elements.iter().enumerate() {
                    stack.push((p.child(super::ops::Nest::Items, j), ch));
                }
            }
        }
        let mut dups: Vec<&String> = names.iter().filter(|(_, n)| **n > 1).map(|(k, _)| k).collect();
        dups.sort();
        for n in dups {
            add(
                Level::Warning,
                format!(
                    "Two elements on the {vname} screen are called “{n}”: changes for one variant, screen shape or console reach both."
                ),
                None,
            );
        }
    }
    for (set_name, sets) in
        [("variant", &file.variants), ("screen shape", &file.aspects), ("console look", &file.overrides)]
    {
        for (k, o) in sets {
            for (kind, _) in VIEW_NAMES {
                let tables = super::ops::tables(o, kind);
                let Some(view) = file.view(kind) else { continue };
                for name in tables.keys() {
                    if !view.elements.iter().any(|e| e.common().name == *name) {
                        add(
                            Level::Warning,
                            format!(
                                "The {set_name} “{k}” changes “{name}” on the {} screen, but there is no element with that name.",
                                view_name(kind)
                            ),
                            None,
                        );
                    }
                }
            }
        }
    }
    for k in file.aspects.keys() {
        if aspect_ratio(k).is_none() {
            add(Level::Warning, format!("The screen shape “{k}” isn't a shape like 16:10 or 4:3."), None);
        }
    }
    for (k, s) in &file.color_schemes {
        for slot in s.palette.keys() {
            if slot != "dark" && palette_slot(&file.palette, slot).is_none() {
                add(
                    Level::Warning,
                    format!("The colour scheme “{k}” sets “{slot}”, which isn't a palette colour."),
                    None,
                );
            }
        }
    }
    out.sort_by_key(|f| f.level);
    out
}
