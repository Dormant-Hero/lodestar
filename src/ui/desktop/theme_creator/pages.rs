//! The theme-wide pages: details for sharing, colours and colour schemes, fonts, console
//! pictures, sounds and music, variants / screen shapes / console looks, the check, the whole
//! theme file; and the export window.

use super::super::Pal;
use super::files::{self, Asset};
use super::ops::Layer;
use super::validate::Level;
use super::widgets::{self, heading, note, row};
use super::{AssetFor, CreatorDlg, Done, Page, PickFor};
use crate::app::AppState;
use crate::theme::*;
use egui::{RichText, Ui, vec2};
use std::path::{Path, PathBuf};

pub fn show(ui: &mut Ui, st: &mut AppState, d: &mut CreatorDlg, pal: &Pal, page: Page) {
    match page {
        Page::Details => details(ui, st, d, pal),
        Page::Colours => colours(ui, d, pal),
        Page::Fonts => fonts(ui, d, pal),
        Page::Consoles => consoles(ui, st, d, pal),
        Page::Sounds => sounds(ui, d, pal),
        Page::Choices => choices(ui, st, d, pal),
        Page::Check => check(ui, d, pal),
        Page::File => file_page(ui, d, pal),
        Page::Layout => {}
    }
}

pub const LICENCES: [(&str, &str); 5] = [
    ("CC-BY-4.0", "Share and change it, credit me (CC BY 4.0)"),
    ("CC-BY-NC-4.0", "Share and change it, not for money, credit me (CC BY-NC 4.0)"),
    ("CC-BY-SA-4.0", "Share and change it, keep it open, credit me (CC BY-SA 4.0)"),
    ("MIT", "MIT"),
    ("All rights reserved", "All rights reserved"),
];

pub const TAGS: [&str; 14] = [
    "dark",
    "light",
    "colourful",
    "minimal",
    "carousel",
    "grid",
    "wheel",
    "list",
    "rows",
    "big-screen",
    "handheld",
    "retro",
    "videos",
    "per-console",
];

/// A catalogue id from author and name ("Marko", "Night Shelf" → "marko-night-shelf").
pub fn suggest_id(author: &str, name: &str) -> String {
    let s: String = format!("{author} {name}")
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-")
}

pub fn is_semver(v: &str) -> bool {
    let p: Vec<&str> = v.trim().split('.').collect();
    p.len() == 3 && p.iter().all(|x| !x.is_empty() && x.chars().all(|c| c.is_ascii_digit()))
}

/// What a catalogue needs that the theme lacks (plain words; empty: ready to share).
pub fn sharing_gaps(f: &ThemeFile) -> Vec<String> {
    let m = &f.theme;
    let mut v = vec![];
    if m.author.trim().is_empty() {
        v.push("Who made it (Author)".to_string());
    }
    let id_ok = !m.id.is_empty() && m.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !id_ok {
        v.push("An id of lower-case letters, digits and dashes".into());
    }
    if !is_semver(&m.version) {
        v.push("A version like 1.0.0".into());
    }
    if m.license.trim().is_empty() {
        v.push("A licence (how others may use it)".into());
    }
    if m.summary.trim().is_empty() {
        v.push("A one-line summary".into());
    }
    if m.previews.is_empty() {
        v.push("At least one preview picture (“Snapshot” on the Layout page)".into());
    }
    v
}

fn details(ui: &mut Ui, st: &mut AppState, d: &mut CreatorDlg, pal: &Pal) {
    let Some(doc) = d.doc.as_mut() else { return };
    let mut m = doc.file.theme.clone();
    heading(ui, "About the theme", pal);
    row(ui, "Name", pal, |ui| widgets::line(ui, &mut m.name, "", 320.0));
    row(ui, "Author", pal, |ui| widgets::line(ui, &mut m.author, "your name", 320.0));
    row(ui, "Summary", pal, |ui| {
        widgets::line(ui, &mut m.summary, "one line for lists, like “Big covers on a dark wall”", 420.0)
    });
    ui.label(RichText::new("Description").size(12.5).color(pal.dim));
    ui.add(egui::TextEdit::multiline(&mut m.description).desired_rows(3).desired_width(560.0));
    row(ui, "Version", pal, |ui| {
        widgets::line(ui, &mut m.version, "1.0.0", 90.0);
        if !is_semver(&m.version) && ui.small_button("Use 1.0.0").clicked() {
            m.version = "1.0.0".into();
        }
        if is_semver(&m.version) && ui.small_button("Next version").on_hover_text("1.2.3 → 1.2.4").clicked() {
            let mut p: Vec<u64> = m.version.split('.').filter_map(|x| x.parse().ok()).collect();
            if let Some(last) = p.last_mut() {
                *last += 1;
            }
            m.version = p.iter().map(u64::to_string).collect::<Vec<_>>().join(".");
        }
    });
    heading(ui, "Sharing", pal);
    note(ui, "Optional — what a theme catalogue shows. It goes into the .zip with the theme.", pal);
    row(ui, "Id", pal, |ui| {
        widgets::line(ui, &mut m.id, "author-theme-name", 240.0);
        if ui.small_button("Suggest").clicked() {
            m.id = suggest_id(&m.author, &m.name);
        }
    });
    row(ui, "Licence", pal, |ui| {
        let cur = LICENCES
            .iter()
            .find(|(k, _)| *k == m.license)
            .map(|(_, l)| l.to_string())
            .unwrap_or_else(|| if m.license.is_empty() { "Not chosen".into() } else { m.license.clone() });
        egui::ComboBox::from_id_salt("ls_creator_licence").selected_text(cur).width(330.0).show_ui(ui, |ui| {
            for (k, l) in LICENCES {
                ui.selectable_value(&mut m.license, k.to_string(), l);
            }
        });
    });
    row(ui, "Tags", pal, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(560.0);
            for t in TAGS {
                let mut on = m.tags.iter().any(|x| x == t);
                if ui.toggle_value(&mut on, t).changed() {
                    if on {
                        m.tags.push(t.into());
                    } else {
                        m.tags.retain(|x| x != t);
                    }
                }
            }
        });
    });
    row(ui, "Needs Lodestar", pal, |ui| {
        widgets::line(ui, &mut m.needs, env!("CARGO_PKG_VERSION"), 90.0);
        note(
            ui,
            &format!(
                "or newer · saved with {}",
                if m.made_with.is_empty() { env!("CARGO_PKG_VERSION") } else { &m.made_with }
            ),
            pal,
        );
    });
    heading(ui, "Preview pictures", pal);
    note(ui, "Up to four. Take them with “Snapshot” above the preview on the Layout page (each screen once).", pal);
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
        for (i, p) in m.previews.iter().enumerate() {
            ui.vertical(|ui| {
                let path = doc.dir.join(p);
                let (r, _) = ui.allocate_exact_size(vec2(200.0, 113.0), egui::Sense::hover());
                ui.painter().rect_filled(r, 4.0, pal.surface_alt);
                if let Some(tex) = st.images.get(&path, crate::images::ImgSize::Thumb) {
                    let s = tex.size_vec2();
                    let k = (r.width() / s.x).min(r.height() / s.y);
                    let ir = egui::Rect::from_center_size(r.center(), s * k);
                    ui.painter().image(
                        tex.id(),
                        ir,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new(p).size(11.0).color(pal.dim));
                    if ui.small_button("✕").clicked() {
                        remove = Some(i);
                    }
                });
            });
        }
    });
    if let Some(i) = remove {
        m.previews.remove(i);
    }
    let gaps = {
        let mut f = doc.file.clone();
        f.theme = m.clone();
        sharing_gaps(&f)
    };
    if !gaps.is_empty() {
        note(ui, &format!("For a theme catalogue it still needs: {}.", gaps.join("; ")), pal);
    }
    heading(ui, "Controller mode", pal);
    row(ui, "Made for", pal, |ui| {
        let mut shape = if (m.aspect - 1.6).abs() < 0.01 {
            "16:10"
        } else if (m.aspect - 4.0 / 3.0).abs() < 0.01 {
            "4:3"
        } else if (m.aspect - 21.0 / 9.0).abs() < 0.02 {
            "21:9"
        } else {
            "16:9"
        };
        let before = shape;
        egui::ComboBox::from_id_salt("ls_creator_aspect").selected_text(shape).show_ui(ui, |ui| {
            for s in ["16:9", "16:10", "4:3", "21:9"] {
                ui.selectable_value(&mut shape, s, s);
            }
        });
        if shape != before {
            m.aspect = aspect_ratio(shape).unwrap_or(16.0 / 9.0);
        }
        note(ui, "the screen shape the layout is drawn for", pal);
    });
    if m != doc.file.theme && widgets::user_acted(ui.ctx()) {
        let tag = ui.ctx().memory(|mm| mm.focused()).map(|i| format!("{i:?}")).unwrap_or_default();
        doc.change_merged(&format!("details{tag}"), "Change the theme's details", |f| f.theme = m);
    }
}

fn colours(ui: &mut Ui, d: &mut CreatorDlg, pal: &Pal) {
    let Some(doc) = d.doc.as_mut() else { return };
    heading(ui, "Palette", pal);
    note(
        ui,
        "The theme's colours. Controller-mode menus and the desktop use them too. Elements drawn in a palette colour follow it when it changes.",
        pal,
    );
    let follow_id = egui::Id::unique("ls_creator_follow");
    let mut follow: bool = ui.data(|m| m.get_temp(follow_id)).unwrap_or(true);
    if ui.checkbox(&mut follow, "Elements in a palette colour change with it").changed() {
        ui.data_mut(|m| m.insert_temp(follow_id, follow));
    }
    let p = doc.file.palette.clone();
    let mut np = p.clone();
    ui.checkbox(&mut np.dark, "Dark theme");
    egui::Grid::new("ls_creator_palette").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
        for k in SLOT_ORDER {
            ui.label(RichText::new(widgets::slot_label(k)).size(12.5).color(pal.dim));
            if let Some(mut c) = palette_slot(&np, k) {
                let mut rgba = c.0;
                ui.horizontal(|ui| {
                    if ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed() {
                        c = Color(rgba);
                    }
                    let mut hex = c.to_hex();
                    if ui
                        .add(egui::TextEdit::singleline(&mut hex).desired_width(80.0).font(egui::TextStyle::Monospace))
                        .changed()
                        && let Some(n) = Color::parse(&hex)
                    {
                        c = n;
                    }
                });
                set_palette_slot(&mut np, k, c);
            }
            ui.end_row();
        }
    });
    if np != p && widgets::user_acted(ui.ctx()) {
        let tag = ui
            .ctx()
            .dragged_id()
            .or_else(|| ui.ctx().memory(|m| m.focused()))
            .map(|i| format!("{i:?}"))
            .unwrap_or_default();
        doc.change_merged(&format!("pal{tag}"), "Change the palette", |f| {
            if follow {
                let mut patch = toml::Table::new();
                if let Ok(toml::Value::Table(t)) = toml::Value::try_from(&np) {
                    patch = t;
                }
                if let Ok(n) = f.recolored(&patch, &Default::default()) {
                    let schemes = std::mem::take(&mut f.color_schemes);
                    *f = n;
                    f.color_schemes = schemes;
                    return;
                }
            }
            f.palette = np;
        });
    }
    heading(ui, "Colour schemes", pal);
    note(
        ui,
        "Other sets of colours players can pick in Settings → Visuals → Theme colours, like a light version or another accent.",
        pal,
    );
    let keys: Vec<String> = doc.file.color_schemes.keys().cloned().collect();
    let mut del = None;
    for k in &keys {
        let s = doc.file.color_schemes[k].clone();
        let mut ns = s.clone();
        egui::Frame::new().fill(pal.surface_alt).corner_radius(6).inner_margin(8).show(ui, |ui| {
            ui.set_width(ui.available_width().min(640.0));
            ui.horizontal(|ui| {
                ui.label(RichText::new(s.label_or(k)).strong().color(pal.text));
                widgets::line(ui, &mut ns.label, "name shown to players", 180.0);
                let previewing = d.pv.scheme.as_deref() == Some(k.as_str());
                if ui.selectable_label(previewing, "Preview").clicked() {
                    d.pv.scheme = if previewing { None } else { Some(k.clone()) };
                }
                if crate::ui::widgets::icon_button(ui, crate::ui::widgets::Icon::Trash, false, 22.0, "Remove").clicked()
                {
                    del = Some(k.clone());
                }
            });
            egui::Grid::new(("ls_creator_scheme", k)).num_columns(3).show(ui, |ui| {
                let mut dark = ns.palette.get("dark").and_then(|v| v.as_bool());
                ui.label("Dark");
                let mut on = dark.is_some();
                if ui.checkbox(&mut on, "changes").changed() {
                    dark = on.then_some(!p.dark);
                }
                if let Some(v) = &mut dark {
                    ui.checkbox(v, "dark");
                    ns.palette.insert("dark".into(), toml::Value::Boolean(*v));
                } else {
                    ui.label("");
                    ns.palette.remove("dark");
                }
                ui.end_row();
                for slot in SLOT_ORDER {
                    ui.label(widgets::slot_label(slot));
                    let cur = ns.palette.get(slot).and_then(|v| v.as_str()).and_then(Color::parse);
                    let mut on = cur.is_some();
                    if ui.checkbox(&mut on, "changes").changed() {
                        if on {
                            let c = palette_slot(&p, slot).unwrap_or(Color::WHITE);
                            ns.palette.insert(slot.into(), toml::Value::String(c.to_hex()));
                        } else {
                            ns.palette.remove(slot);
                        }
                    }
                    if let Some(mut c) = ns.palette.get(slot).and_then(|v| v.as_str()).and_then(Color::parse) {
                        let mut rgba = c.0;
                        if ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed() {
                            c = Color(rgba);
                            ns.palette.insert(slot.into(), toml::Value::String(c.to_hex()));
                        }
                    } else {
                        ui.label("");
                    }
                    ui.end_row();
                }
            });
        });
        if ns != s && widgets::user_acted(ui.ctx()) {
            let tag = ui
                .ctx()
                .dragged_id()
                .or_else(|| ui.ctx().memory(|m| m.focused()))
                .map(|i| format!("{i:?}"))
                .unwrap_or_default();
            doc.change_merged(&format!("scheme{k}{tag}"), "Change a colour scheme", |f| {
                f.color_schemes.insert(k.clone(), ns);
            });
        }
    }
    if let Some(k) = del {
        doc.change("Delete a colour scheme", |f| {
            f.color_schemes.remove(&k);
        });
        if d.pv.scheme.as_deref() == Some(k.as_str()) {
            d.pv.scheme = None;
        }
    }
    ui.horizontal(|ui| {
        if ui.button("＋ Light version").clicked() {
            let k = free_key_in(&doc.file.color_schemes, "light");
            doc.change("Add a colour scheme", |f| {
                f.color_schemes.insert(k.clone(), light_scheme(&f.palette));
            });
        }
        if ui.button("＋ Another accent").clicked() {
            let k = free_key_in(&doc.file.color_schemes, "accent");
            doc.change("Add a colour scheme", |f| {
                let mut s = ColorScheme { label: "Warm accent".into(), ..Default::default() };
                for (slot, c) in [("accent", "#ff8a3d"), ("selection", "#ffb070")] {
                    s.palette.insert(slot.into(), toml::Value::String(c.into()));
                }
                f.color_schemes.insert(k.clone(), s);
            });
        }
    });
}

fn free_key_in<V>(m: &std::collections::BTreeMap<String, V>, base: &str) -> String {
    if !m.contains_key(base) {
        return base.into();
    }
    (2..).map(|n| format!("{base}{n}")).find(|k| !m.contains_key(k)).expect("a free key")
}

/// A light scheme from a dark palette (and the other way round): a good start to adjust.
pub fn light_scheme(p: &Palette) -> ColorScheme {
    let mut s = ColorScheme { label: if p.dark { "Light" } else { "Dark" }.into(), ..Default::default() };
    let set: &[(&str, &str)] = if p.dark {
        &[
            ("background", "#eef0f4"),
            ("surface", "#ffffff"),
            ("surface_alt", "#e2e6ee"),
            ("text", "#1b1e24"),
            ("text_dim", "#5d6675"),
            ("border", "#cfd5df"),
        ]
    } else {
        &[
            ("background", "#15181e"),
            ("surface", "#1d2128"),
            ("surface_alt", "#272c35"),
            ("text", "#e8ebf0"),
            ("text_dim", "#9aa3b2"),
            ("border", "#323844"),
        ]
    };
    s.palette.insert("dark".into(), toml::Value::Boolean(!p.dark));
    for (k, v) in set {
        s.palette.insert((*k).into(), toml::Value::String((*v).into()));
    }
    s
}

/// `[fonts]`: set a name to a font reference (None: remove an extra name / clear a main one).
pub fn set_font(f: &mut Fonts, name: &str, v: Option<String>) {
    match name {
        "regular" => f.regular = v,
        "bold" => f.bold = v,
        "title" => f.title = v,
        _ => match v {
            Some(v) => {
                f.extra.insert(name.to_string(), v);
            }
            None => {
                f.extra.remove(name);
            }
        },
    }
}

const SYSTEM_FONTS: [(&str, &str); 9] = [
    ("system:sans", "Sans"),
    ("system:sans-light", "Sans light"),
    ("system:sans-medium", "Sans medium"),
    ("system:sans-semibold", "Sans semibold"),
    ("system:sans-bold", "Sans bold"),
    ("system:display", "Display (titles)"),
    ("system:condensed", "Condensed"),
    ("system:serif", "Serif"),
    ("system:mono", "Monospace"),
];

fn font_value_label(v: &str) -> String {
    if let Some((_, l)) = SYSTEM_FONTS.iter().find(|(k, _)| *k == v) {
        return format!("{l} (from this computer)");
    }
    if let Some(b) = v.strip_prefix("builtin:") {
        return format!("{b} (comes with Lodestar)");
    }
    v.to_string()
}

fn fonts(ui: &mut Ui, d: &mut CreatorDlg, pal: &Pal) {
    let ctx = ui.ctx().clone();
    let Some(doc) = d.doc.as_mut() else { return };
    heading(ui, "Fonts", pal);
    note(ui, "Elements use these by name. A font file is copied into the theme, so it travels with it.", pal);
    let mut names: Vec<(String, Option<String>)> = vec![
        ("regular".into(), doc.file.fonts.regular.clone()),
        ("bold".into(), doc.file.fonts.bold.clone()),
        ("title".into(), doc.file.fonts.title.clone()),
    ];
    names.extend(doc.file.fonts.extra.iter().map(|(k, v)| (k.clone(), Some(v.clone()))));
    let theme_files = files::list_assets(&doc.dir, Asset::Font);
    let mut set: Option<(String, Option<String>)> = None;
    let mut pick: Option<String> = None;
    for (name, v) in &names {
        row(ui, name, pal, |ui| {
            let cur = v.as_deref().map(font_value_label).unwrap_or_else(|| "Lodestar's font".into());
            egui::ComboBox::from_id_salt(("ls_creator_font", name)).selected_text(cur).width(300.0).show_ui(ui, |ui| {
                if ui.selectable_label(v.is_none(), "Lodestar's font").clicked() {
                    set = Some((name.clone(), None));
                }
                ui.separator();
                for (k, _) in crate::theme::fonts::bundled::NAMES {
                    let val = format!("builtin:{k}");
                    if ui.selectable_label(v.as_deref() == Some(val.as_str()), font_value_label(&val)).clicked() {
                        set = Some((name.clone(), Some(val)));
                    }
                }
                ui.separator();
                for (k, _) in SYSTEM_FONTS {
                    if ui.selectable_label(v.as_deref() == Some(k), font_value_label(k)).clicked() {
                        set = Some((name.clone(), Some(k.to_string())));
                    }
                }
                if !theme_files.is_empty() {
                    ui.separator();
                    for f in &theme_files {
                        if ui.selectable_label(v.as_deref() == Some(f.as_str()), f).clicked() {
                            set = Some((name.clone(), Some(f.clone())));
                        }
                    }
                }
            });
            if ui.button("Font file…").on_hover_text("A .ttf / .otf file (copied into the theme)").clicked() {
                pick = Some(name.clone());
            }
            if !["regular", "bold", "title"].contains(&name.as_str())
                && crate::ui::widgets::icon_button(ui, crate::ui::widgets::Icon::Trash, false, 22.0, "Remove").clicked()
            {
                set = Some((name.clone(), None));
            }
        });
    }
    ui.horizontal(|ui| {
        let id = egui::Id::unique("ls_creator_newfont");
        let mut n: String = ui.data(|m| m.get_temp(id)).unwrap_or_default();
        if widgets::line(ui, &mut n, "another name, like “medium”", 200.0).changed() {
            ui.data_mut(|m| m.insert_temp(id, n.clone()));
        }
        let clean = n.trim().to_lowercase().replace(' ', "_");
        if ui.add_enabled(!clean.is_empty(), egui::Button::new("＋ Add a font name")).clicked() {
            set = Some((clean, Some("system:sans-medium".into())));
            ui.data_mut(|m| m.remove::<String>(id));
        }
    });
    if let Some((n, v)) = set {
        doc.change("Change a font", |f| set_font(&mut f.fonts, &n, v));
    }
    if let Some(n) = pick {
        d.pick(&ctx, PickFor::Asset(Asset::Font, AssetFor::Font(n)), "Choose a font file");
    }
}

/// Console pictures page state.
#[derive(Default)]
pub struct ConsolesPage {
    /// The picture being set: "card", "hero", "mark", "device".
    pub slot: Option<String>,
    pub all: bool,
    /// Files a folder import couldn't match (assign them by hand).
    pub unmatched: Vec<PathBuf>,
    pub last: Option<String>,
}

impl ConsolesPage {
    pub fn after_import(&mut self, n: usize, rest: Vec<PathBuf>) {
        self.last = Some(if rest.is_empty() {
            format!("Added {n} pictures.")
        } else {
            format!("Added {n} pictures; {} couldn't be matched to a console — pick one for each below.", rest.len())
        });
        self.unmatched.extend(rest);
    }
}

/// The console or collection a picture file is named for ("ps2.png", "snes.jpg",
/// "megadrive.png", "Nintendo 64.png", "sony_playstation2.png", "steam.png"), as its
/// `[collection_art]` key. The short system names other frontends' themes use, Lodestar's
/// platform ids and console names all work.
pub fn match_console(stem: &str, colls: &[crate::library::Collection]) -> Option<String> {
    let s = stem.trim().to_lowercase();
    if s.is_empty() {
        return None;
    }
    for c in colls {
        let art = crate::theme::console_art::key_for(c);
        if c.key().eq_ignore_ascii_case(&s) || art.is_some_and(|a| a == s) || c.name.eq_ignore_ascii_case(&s) {
            return Some(files::art_key(c));
        }
    }
    if let Some((_, k)) = COLLECTION_NAMES.iter().find(|(a, _)| *a == s) {
        return Some((*k).to_string());
    }
    let alias = ALIASES.iter().find(|(a, _)| *a == s).map(|(_, id)| *id);
    let p = alias
        .and_then(crate::emulation::platforms::by_id)
        .or_else(|| crate::emulation::platforms::all().iter().find(|p| p.es.contains(&s.as_str()) || p.id == s))
        .or_else(|| crate::emulation::platforms::by_folder_name(&s))
        .or_else(|| crate::emulation::platforms::by_name(&s))?;
    Some(p.id.to_string())
}

/// Names other frontends give their automatic lists, and stores.
const COLLECTION_NAMES: [(&str, &str); 10] = [
    ("all", "all"),
    ("allgames", "all"),
    ("auto-allgames", "all"),
    ("favorites", "favorites"),
    ("favourites", "favorites"),
    ("auto-favorites", "favorites"),
    ("recent", "recent"),
    ("lastplayed", "recent"),
    ("auto-lastplayed", "recent"),
    ("steam", "steam"),
];

/// Names other frontends' themes use that Lodestar's tables don't already know.
const ALIASES: [(&str, &str); 5] = [
    ("genesis", "sega_genesis"),
    ("famicom", "nintendo_nes"),
    ("sfc", "nintendo_snes"),
    ("pce", "nec_turbografx_16"),
    ("tg16", "nec_turbografx_16"),
];

/// Copy every picture of `folder` named after a console into the theme for `slot`. Returns the
/// matched (art key, theme path) and the files that matched nothing.
pub fn import_console_folder(
    dir: &Path,
    slot: &str,
    folder: &Path,
    colls: &[crate::library::Collection],
) -> (Vec<(String, String)>, Vec<PathBuf>) {
    let mut ok: Vec<(String, String)> = vec![];
    let mut rest = vec![];
    let mut entries: Vec<PathBuf> =
        std::fs::read_dir(folder).into_iter().flatten().flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries.into_iter().take(2000) {
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        if !p.is_file() || !files::CONSOLE_EXTS.contains(&ext.as_str()) {
            continue;
        }
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        match match_console(stem, colls) {
            Some(key) if !ok.iter().any(|(k, _)| *k == key) => match files::copy_console_picture(dir, slot, &key, &p) {
                Ok(rel) => ok.push((key, rel)),
                Err(_) => rest.push(p),
            },
            _ => rest.push(p),
        }
    }
    (ok, rest)
}

/// Set (or with None remove) a console's picture for a slot.
pub fn set_art(f: &mut ThemeFile, key: &str, slot: &str, rel: Option<String>) {
    let a = f.collection_art.entry(key.to_string()).or_default();
    let field = match slot {
        "card" => &mut a.card,
        "hero" => &mut a.hero,
        "mark" => &mut a.mark,
        _ => &mut a.device,
    };
    *field = rel;
    if *a == CollectionArt::default() {
        f.collection_art.remove(key);
    }
}

fn consoles(ui: &mut Ui, st: &mut AppState, d: &mut CreatorDlg, pal: &Pal) {
    let ctx = ui.ctx().clone();
    let Some(doc) = d.doc.as_ref() else { return };
    let dir = doc.dir.clone();
    heading(ui, "Console pictures", pal);
    note(
        ui,
        "Every console and collection has Lodestar's own pictures: a card, a background, a name badge and the device. Put your own in their place here — the theme's elements show them through values like {collection.art.card}.",
        pal,
    );
    let slot = d.consoles.slot.clone().unwrap_or_else(|| "card".into());
    ui.horizontal(|ui| {
        for (k, l) in files::ART_SLOTS {
            if ui.selectable_label(slot == k, l).clicked() {
                d.consoles.slot = Some(k.into());
            }
        }
    });
    ui.horizontal(|ui| {
        if ui
            .button("Add pictures from a folder…")
            .on_hover_text(
                "Files named after consoles (ps2.png, snes.jpg, megadrive.png, Nintendo 64.png …) are matched for you",
            )
            .clicked()
        {
            d.pick(&ctx, PickFor::ConsoleFolder(slot.clone()), "Choose a folder of console pictures");
        }
        ui.checkbox(&mut d.consoles.all, "Every console Lodestar knows");
    });
    if let Some(m) = &d.consoles.last {
        note(ui, m, pal);
    }
    use_slot_row(ui, d, &slot, pal);
    // The consoles: the library's collections first, then every platform.
    let mut items: Vec<(String, String, Option<String>)> = vec![];
    for c in st.library.collections() {
        let label = match &c.kind {
            crate::library::CollectionKind::Platform(id) => crate::ui::platform_icons::console_name(id),
            _ => c.name.clone(),
        };
        let builtin = crate::theme::console_art::key_for(&c).map(str::to_string);
        items.push((label, files::art_key(&c), builtin));
    }
    if d.consoles.all {
        for p in crate::emulation::platforms::all() {
            if !items.iter().any(|(_, k, _)| k == p.id) {
                items.push((crate::ui::platform_icons::console_name(p.id), p.id.to_string(), Some(p.id.to_string())));
            }
        }
    }
    let colls_keys: Vec<(String, String)> = items.iter().map(|(l, k, _)| (l.clone(), k.clone())).collect();
    if !d.consoles.unmatched.is_empty() {
        heading(ui, "Not matched", pal);
        let mut assign: Option<(usize, String)> = None;
        for (i, f) in d.consoles.unmatched.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                egui::ComboBox::from_id_salt(("ls_creator_unmatched", i))
                    .selected_text("Which console?")
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        for (label, key) in &colls_keys {
                            if ui.selectable_label(false, label).clicked() {
                                assign = Some((i, key.clone()));
                            }
                        }
                    });
            });
        }
        if ui.small_button("Skip these").clicked() {
            d.consoles.unmatched.clear();
        }
        if let Some((i, key)) = assign {
            let f = d.consoles.unmatched.remove(i);
            d.pick_done_console(&ctx, slot.clone(), key, f);
        }
    }
    let Some(doc) = d.doc.as_ref() else { return };
    let kind = crate::theme::console_art::ArtKind::parse(&slot).unwrap_or(crate::theme::console_art::ArtKind::Card);
    let mut pick: Option<String> = None;
    let mut remove: Option<String> = None;
    let cell = vec2(150.0, if slot == "hero" { 84.0 } else { 100.0 });
    let cols = ((ui.available_width() / (cell.x + 8.0)).floor() as usize).max(1);
    egui::Grid::new("ls_creator_consoles").spacing([8.0, 8.0]).show(ui, |ui| {
        for (i, (label, key, builtin)) in items.iter().enumerate() {
            let own = doc.file.collection_art.get(key).and_then(|a| a.get(&slot)).map(|p| dir.join(p));
            let shown =
                own.clone().or_else(|| builtin.as_deref().and_then(|b| crate::theme::console_art::path(b, kind)));
            egui::Frame::new().fill(pal.surface_alt).corner_radius(6).inner_margin(6).show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(cell.x);
                    let (r, resp) = ui.allocate_exact_size(cell, egui::Sense::click());
                    ui.painter().rect_filled(r, 4.0, pal.surface);
                    match shown.as_ref().and_then(|p| st.images.get(p, crate::images::ImgSize::Thumb)) {
                        Some(tex) => {
                            let s = tex.size_vec2();
                            let k = (r.width() / s.x).min(r.height() / s.y);
                            let ir = egui::Rect::from_center_size(r.center(), s * k);
                            let tint =
                                if own.is_some() { egui::Color32::WHITE } else { egui::Color32::from_white_alpha(110) };
                            ui.painter().image(
                                tex.id(),
                                ir,
                                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                tint,
                            );
                        }
                        None => {
                            ui.painter().text(
                                r.center(),
                                egui::Align2::CENTER_CENTER,
                                "＋",
                                egui::FontId::proportional(20.0),
                                pal.dim,
                            );
                        }
                    }
                    if resp.on_hover_text("Pick a picture").clicked() {
                        pick = Some(key.clone());
                    }
                    ui.add(egui::Label::new(RichText::new(label).size(12.0).color(pal.text)).truncate());
                    ui.horizontal(|ui| {
                        let what = if own.is_some() {
                            "the theme's"
                        } else if shown.is_some() {
                            "Lodestar's"
                        } else {
                            "none"
                        };
                        ui.label(RichText::new(what).size(10.5).color(pal.dim));
                        if own.is_some() && ui.small_button("✕").on_hover_text("Back to Lodestar's picture").clicked()
                        {
                            remove = Some(key.clone());
                        }
                    });
                });
            });
            if (i + 1) % cols == 0 {
                ui.end_row();
            }
        }
    });
    if let Some(k) = pick {
        d.pick(&ctx, PickFor::ConsoleOne(slot.clone(), k), "Choose a picture for this console");
    }
    if let Some(k) = remove
        && let Some(doc) = d.doc.as_mut()
    {
        doc.change("Remove a console picture", |f| set_art(f, &k, &slot, None));
    }
}

/// "Show these in: <element>" — points an element's picture at `{collection.art.<slot>}` (what
/// it showed before stays as the fallback for collections without one).
fn use_slot_row(ui: &mut Ui, d: &mut CreatorDlg, slot: &str, pal: &Pal) {
    let Some(doc) = d.doc.as_mut() else { return };
    let binding = format!("{{collection.art.{slot}}}");
    let mut targets: Vec<(ViewKind, usize, String)> = vec![];
    for (k, vname) in super::validate::VIEW_NAMES {
        if let Some(v) = doc.file.view(k) {
            for (i, e) in v.elements.iter().enumerate() {
                let uses = match e {
                    Element::Image { path, .. } => Some(path == &binding),
                    Element::Carousel { item_image, .. } | Element::Grid { item_image, .. } => {
                        Some(item_image == &binding)
                    }
                    _ => None,
                };
                if let Some(uses) = uses {
                    let n = if e.common().name.is_empty() {
                        super::ops::kind_word(e).to_string()
                    } else {
                        e.common().name.clone()
                    };
                    targets.push((k, i, format!("{vname}: {n}{}", if uses { " ✓" } else { "" })));
                }
            }
        }
    }
    let mut chosen = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Show these in").size(12.5).color(pal.dim));
        egui::ComboBox::from_id_salt("ls_creator_use_slot").selected_text("an element…").width(260.0).show_ui(
            ui,
            |ui| {
                for (k, i, l) in &targets {
                    if ui.selectable_label(false, l).clicked() {
                        chosen = Some((*k, *i));
                    }
                }
            },
        );
    });
    if let Some((k, i)) = chosen {
        doc.change("Show console pictures", |f| {
            if let Some(v) = super::panel::view_mut(f, k)
                && let Some(e) = v.elements.get_mut(i)
            {
                let chain = |main: &mut String, fallbacks: &mut Vec<String>| {
                    if *main == binding {
                        return;
                    }
                    let old = std::mem::replace(main, binding.clone());
                    if !old.is_empty() {
                        fallbacks.insert(0, old);
                    }
                };
                match e {
                    Element::Image { path, fallbacks, .. } => chain(path, fallbacks),
                    Element::Carousel { item_image, item_fallbacks, .. }
                    | Element::Grid { item_image, item_fallbacks, .. } => chain(item_image, item_fallbacks),
                    _ => {}
                }
            }
        });
    }
}

/// Friendly names of the navigation sounds.
const SOUNDS: [(&str, &str); 7] = [
    ("scroll", "Moving through a list"),
    ("systembrowse", "Moving between collections"),
    ("quicksysselect", "Jumping (letters, pages)"),
    ("select", "Choosing"),
    ("back", "Going back"),
    ("favorite", "Marking a favourite"),
    ("launch", "Starting a game"),
];

fn sounds(ui: &mut Ui, d: &mut CreatorDlg, pal: &Pal) {
    let ctx = ui.ctx().clone();
    let Some(doc) = d.doc.as_mut() else { return };
    heading(ui, "Navigation sounds", pal);
    note(ui, "Played when players turn sounds on (controller Settings → Sound). Files are copied into the theme.", pal);
    let mut pick: Option<AssetFor> = None;
    let mut clear: Option<String> = None;
    for (k, label) in SOUNDS {
        row(ui, label, pal, |ui| {
            let cur = doc.file.sounds.get(k).cloned();
            ui.label(RichText::new(cur.clone().unwrap_or_else(|| "none".into())).size(12.0).color(if cur.is_some() {
                pal.text
            } else {
                pal.dim
            }));
            if ui.small_button("Sound file…").clicked() {
                pick = Some(AssetFor::Sound(k.into()));
            }
            if cur.is_some() && ui.small_button("✕").clicked() {
                clear = Some(k.into());
            }
        });
    }
    heading(ui, "Background music", pal);
    note(ui, "Plays when players turn music on.", pal);
    let mut remove = None;
    for (i, m) in doc.file.music.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(m);
            if ui.small_button("✕").clicked() {
                remove = Some(i);
            }
        });
    }
    if ui.button("＋ Music file…").clicked() {
        pick = Some(AssetFor::Music);
    }
    heading(ui, "Videos", pal);
    note(
        ui,
        "A video plays inside a picture element: select one on the Layout page and pick a video, or ＋ Add → This screen's background → Video.",
        pal,
    );
    if let Some(k) = clear {
        doc.change("Remove a sound", |f| {
            f.sounds.remove(&k);
        });
    }
    if let Some(i) = remove {
        doc.change("Remove music", |f| {
            f.music.remove(i);
        });
    }
    if let Some(t) = pick {
        let kind = if matches!(t, AssetFor::Music | AssetFor::Sound(_)) { Asset::Sound } else { Asset::Video };
        d.pick(&ctx, PickFor::Asset(kind, t), "Choose a sound file");
    }
}

fn choices(ui: &mut Ui, st: &AppState, d: &mut CreatorDlg, pal: &Pal) {
    let Some(doc) = d.doc.as_mut() else { return };
    heading(ui, "Variants", pal);
    note(
        ui,
        "Other layouts of the same theme that players pick in Settings → Visuals → Theme variant (“Grid”, “Wheel”, “Compact” …). A variant changes elements of the theme; an element only one variant uses is in the theme hidden, and that variant shows it.",
        pal,
    );
    let mut go: Option<Layer> = None;
    let mut del: Option<(u8, String)> = None;
    let mut rename: Option<(String, String)> = None;
    for k in doc.file.variants.keys() {
        ui.horizontal(|ui| {
            let id = egui::Id::unique(("ls_creator_rename", k));
            let mut n: String = ui.data(|m| m.get_temp(id)).unwrap_or_else(|| k.clone());
            if widgets::line(ui, &mut n, "", 160.0).changed() {
                ui.data_mut(|m| m.insert_temp(id, n.clone()));
            }
            if n.trim() != k && !n.trim().is_empty() && ui.small_button("Rename").clicked() {
                rename = Some((k.clone(), n.trim().to_string()));
                ui.data_mut(|m| m.remove::<String>(id));
            }
            let changes: usize = super::validate::VIEW_NAMES
                .iter()
                .map(|(v, _)| super::ops::tables(&doc.file.variants[k], *v).len())
                .sum();
            widgets::note(ui, &format!("changes {changes} element(s)"), pal);
            if ui.button("Edit on the Layout page").clicked() {
                go = Some(Layer::Variant(k.clone()));
            }
            if crate::ui::widgets::icon_button(ui, crate::ui::widgets::Icon::Trash, false, 22.0, "Remove").clicked() {
                del = Some((0, k.clone()));
            }
        });
    }
    if ui.button("＋ New variant").clicked() {
        let k = free_key_in(&doc.file.variants, "Variant");
        doc.change("Add a variant", |f| {
            f.variants.insert(k.clone(), CollectionOverride::default());
        });
        go = Some(Layer::Variant(k));
    }
    heading(ui, "Screen shapes", pal);
    note(
        ui,
        "A layout for screens of another shape (a Steam Deck is 16:10, old TVs 4:3), used on its own on such screens. Without one, the theme stretches to fit — or turn on “keep proportions” for a screen with nothing selected.",
        pal,
    );
    for k in doc.file.aspects.keys() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{k} screens")).color(pal.text));
            if ui.button("Edit on the Layout page").clicked() {
                go = Some(Layer::Aspect(k.clone()));
            }
            if crate::ui::widgets::icon_button(ui, crate::ui::widgets::Icon::Trash, false, 22.0, "Remove").clicked() {
                del = Some((1, k.clone()));
            }
        });
    }
    ui.horizontal(|ui| {
        for s in ["16:10", "4:3", "21:9", "16:9", "3:2"] {
            if !doc.file.aspects.contains_key(s) && ui.button(format!("＋ {s}")).clicked() {
                doc.change("Add a screen shape", |f| {
                    f.aspects.insert(s.to_string(), CollectionOverride::default());
                });
                go = Some(Layer::Aspect(s.into()));
            }
        }
    });
    heading(ui, "Console looks", pal);
    note(
        ui,
        "Changes for one console or collection: another background for PlayStation 2, another colour for Favourites. On the Layout page, pick “<console> only” under “Changes go to”.",
        pal,
    );
    for k in doc.file.overrides.keys() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(super::preview::console_label(k, st)).color(pal.text));
            let o = &doc.file.overrides[k];
            if let Some(n) = &o.display_name {
                widgets::note(ui, &format!("shown as “{n}”"), pal);
            }
            if ui.button("Edit on the Layout page").clicked() {
                go = Some(Layer::Console(k.clone()));
            }
            if crate::ui::widgets::icon_button(ui, crate::ui::widgets::Icon::Trash, false, 22.0, "Remove").clicked() {
                del = Some((2, k.clone()));
            }
        });
    }
    if let Some((which, k)) = del {
        doc.change("Remove", |f| {
            match which {
                0 => f.variants.remove(&k),
                1 => f.aspects.remove(&k),
                _ => f.overrides.remove(&k),
            };
        });
        if d.layer != Layer::Base {
            d.layer = Layer::Base;
        }
    }
    if let Some((old, new)) = rename {
        doc.change("Rename a variant", |f| {
            if let Some(v) = f.variants.remove(&old) {
                f.variants.insert(new.clone(), v);
            }
        });
    }
    if let Some(l) = go {
        if let Layer::Aspect(a) = &l {
            d.pv.shape = super::preview::Shape::from_key(a);
        }
        if let Layer::Console(k) = &l {
            d.pv.show_console(k, d.view);
        }
        d.layer = l;
        d.page = Page::Layout;
    }
}

fn check(ui: &mut Ui, d: &mut CreatorDlg, pal: &Pal) {
    let Some(doc) = d.doc.as_ref() else { return };
    let found = d.pv.findings(doc).to_vec();
    heading(ui, "Check", pal);
    if found.is_empty() {
        ui.label(RichText::new("✓ Everything looks right.").color(pal.text));
    }
    let mut go = None;
    for f in &found {
        let (mark, col) = match f.level {
            Level::Problem => ("●", pal.danger),
            Level::Warning => ("▲", pal.fav),
            Level::Note => ("·", pal.dim),
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(mark).color(col));
            ui.add(egui::Label::new(RichText::new(&f.text).size(12.5).color(pal.text)).wrap());
            if f.at.is_some() && ui.small_button("Show").clicked() {
                go = f.at.clone();
            }
        });
    }
    let gaps = sharing_gaps(&doc.file);
    if !gaps.is_empty() {
        heading(ui, "Before sharing it in a catalogue", pal);
        for g in gaps {
            ui.label(RichText::new(format!("· {g}")).size(12.5).color(pal.dim));
        }
    }
    if let Some((k, p)) = go {
        d.view = k;
        d.sel = Some(p);
        d.page = Page::Layout;
    }
}

fn file_page(ui: &mut Ui, d: &mut CreatorDlg, pal: &Pal) {
    let Some(doc) = d.doc.as_mut() else { return };
    heading(ui, "Theme file", pal);
    note(ui, "The whole theme.toml as it is saved — for anything the pages don't cover. “Apply” checks it first.", pal);
    if d.file_text.as_ref().is_none_or(|(rev, _, err)| *rev != doc.rev && err.is_none()) {
        d.file_text = Some((doc.rev, doc.text().unwrap_or_default(), None));
    }
    let Some((_, text, err)) = d.file_text.as_mut() else { return };
    ui.add(
        egui::TextEdit::multiline(text)
            .font(egui::TextStyle::Monospace)
            .desired_rows(30)
            .desired_width(f32::INFINITY)
            .code_editor(),
    );
    if let Some(e) = err {
        ui.label(RichText::new(e.as_str()).size(12.0).color(pal.danger));
    }
    let mut apply = false;
    let mut revert = false;
    ui.horizontal(|ui| {
        apply = super::super::dialogs::primary_button(ui, "Apply", pal, true).clicked();
        revert = ui.button("Revert").clicked();
    });
    if revert {
        d.file_text = None;
    } else if apply {
        let t = text.clone();
        match toml::from_str::<toml::Table>(&t)
            .map_err(|e| e.to_string())
            .and_then(|raw| ThemeFile::deserialize_table(&raw).map(|f| (raw, f)).map_err(|e| e.to_string()))
        {
            Ok((raw, f)) => {
                doc.raw = raw;
                doc.replace("Edit the theme file", f);
                d.file_text = None;
            }
            Err(e) => *err = Some(format!("Not applied: {e}")),
        }
    }
}

/// The export window's state.
#[derive(Default)]
pub struct ExportDlg {
    /// Asked to save (the path dialog runs).
    pub request: Option<()>,
    pub done: Option<PathBuf>,
    started: bool,
}

/// The export window: what goes in, what a catalogue would still need, "Save as .zip…".
/// False when closed.
pub fn export_window(
    ctx: &egui::Context,
    st: &AppState,
    doc: Option<&super::doc::Doc>,
    e: &mut ExportDlg,
    pal: &Pal,
) -> bool {
    let Some(doc) = doc else { return false };
    let mut keep = true;
    let (n, bytes) = super::zip::export_size(&doc.dir);
    let gaps = sharing_gaps(&doc.file);
    let _ = st;
    let (_, close) = super::super::dialogs::modal(ctx, "ls_creator_export", pal, 460.0, |ui| {
        ui.label(RichText::new("Export the theme").size(17.0).strong().color(pal.text));
        ui.add_space(6.0);
        ui.set_width(460.0);
        ui.label(RichText::new(&doc.file.theme.name).size(16.0).strong().color(pal.text));
        let v = if doc.file.theme.version.is_empty() {
            String::new()
        } else {
            format!("version {} · ", doc.file.theme.version)
        };
        let size = if bytes >= 1 << 20 {
            format!("{:.1} MB", bytes as f64 / 1_048_576.0)
        } else {
            format!("{} KB", bytes.div_ceil(1024))
        };
        note(ui, &format!("{v}{n} file{} · {size} before packing", if n == 1 { "" } else { "s" }), pal);
        ui.add_space(6.0);
        ui.label(RichText::new("Everything in the theme's folder goes in: theme.toml, pictures, fonts, sounds and videos. Someone with Lodestar adds it with Theme creator → Add a theme .zip.").size(12.5).color(pal.text));
        ui.add_space(6.0);
        if gaps.is_empty() {
            ui.label(RichText::new("✓ It has everything a theme catalogue needs.").size(12.5).color(pal.accent));
        } else {
            ui.label(RichText::new("For a theme catalogue it still needs (Details page):").size(12.5).color(pal.dim));
            for g in &gaps {
                ui.label(RichText::new(format!("· {g}")).size(12.5).color(pal.dim));
            }
        }
        if let Some(p) = &e.done {
            ui.add_space(6.0);
            ui.label(RichText::new(format!("Saved as {}", p.display())).size(12.5).color(pal.accent));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if super::super::dialogs::primary_button(ui, "Save as .zip…", pal, !e.started || e.done.is_some()).clicked()
            {
                e.request = Some(());
                e.started = true;
                e.done = None;
            }
            if ui.add(egui::Button::new("Close").min_size(vec2(80.0, 30.0))).clicked() {
                keep = false;
            }
        });
    });
    keep && !close
}

/// Ask where to save (file dialog on a helper thread), then export there.
pub fn start_export(d: &mut CreatorDlg, ctx: &egui::Context, _req: ()) {
    let Some(doc) = &d.doc else { return };
    let name = format!("{}.zip", doc.key);
    let start = dirs::download_dir().or_else(dirs::home_dir);
    d.spawn(ctx, "Choosing where to save…", move || {
        let mut dlg = rfd::FileDialog::new()
            .set_title("Export the theme")
            .set_file_name(&name)
            .add_filter("Zip archives", &["zip"]);
        if let Some(s) = start {
            dlg = dlg.set_directory(s);
        }
        let p = dlg.save_file().map(|p| if p.extension().is_none() { p.with_extension("zip") } else { p });
        Done::SavePath(p)
    });
}
