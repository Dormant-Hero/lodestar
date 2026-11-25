//! Settings → Emulation → Add emulator…: the form, and adding the emulator to the config.

use super::catalog::{self, Catalog};
use super::lists::*;
use super::*;
use crate::config::{EmulatorEntry, ProfileEntry};
use crate::emulation::{defs, platforms};

/// Tick consoles from the full list. True when changed.
pub(crate) fn consoles_picker(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    list: &mut Vec<String>,
    w: f32,
) -> bool {
    let mut ch = false;
    let names: Vec<String> = list.iter().map(|x| catalog::short_name(x)).collect();
    let text = if names.is_empty() { "Choose…".to_string() } else { catalog::join_limited(&names, 4) };
    egui::ComboBox::from_id_salt(id)
        .width(w)
        .selected_text(text)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                let mut all: Vec<&platforms::Platform> = platforms::all().iter().collect();
                all.sort_by_key(|p| p.name.to_lowercase());
                for pf in all {
                    let mut on = list.iter().any(|x| x == pf.id);
                    if ui.checkbox(&mut on, pf.name).changed() {
                        list.retain(|x| x != pf.id);
                        if on {
                            list.push(pf.id.to_string());
                        }
                        ch = true;
                    }
                }
            });
        });
    ch
}

/// One console from the full list, or "" (`none` names that choice).
pub(crate) fn console_combo(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: &mut String,
    none: &str,
    w: f32,
) -> bool {
    let mut ch = false;
    let text = if value.is_empty() { none.to_string() } else { catalog::console_name(value) };
    egui::ComboBox::from_id_salt(id).width(w).selected_text(text).show_ui(ui, |ui| {
        egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
            if ui.selectable_label(value.is_empty(), none).clicked() && !value.is_empty() {
                value.clear();
                ch = true;
            }
            let mut all: Vec<&platforms::Platform> = platforms::all().iter().collect();
            all.sort_by_key(|p| p.name.to_lowercase());
            for pf in all {
                if ui.selectable_label(value == pf.id, pf.name).clicked() && value != pf.id {
                    *value = pf.id.to_string();
                    ch = true;
                }
            }
        });
    });
    ch
}

pub(super) fn add_form(ui: &mut Ui, st: &mut AppState, p: &mut EmuPage, cat: Option<&Catalog>, pal: &Pal) -> bool {
    let Some(f) = p.add_emu.as_mut() else { return false };
    let mut ch = false;
    let mut pick_file = false;
    let mut pick_flatpak = false;
    let mut close = false;
    ui.label(RichText::new("Add an emulator").size(13.5).strong().color(pal.text));
    ui.add_space(4.0);
    egui::Grid::new("ls_emu_add").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
        ui.label(RichText::new("Based on").size(13.0).color(pal.dim));
        let label = defs::by_id(&f.follows).map_or("Custom (your own arguments)".to_string(), |d| d.name.to_string());
        egui::ComboBox::from_id_salt("ls_emu_add_def").width(300.0).selected_text(label).show_ui(ui, |ui| {
            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                if ui.selectable_label(f.follows.is_empty(), "Custom (your own arguments)").clicked() {
                    f.follows.clear();
                }
                let mut all: Vec<&defs::EmulatorDef> = defs::all().iter().collect();
                all.sort_by_key(|d| d.name.to_lowercase());
                for d in all {
                    if ui.selectable_label(f.follows == d.id, d.name).clicked() {
                        // The name follows the program (Ryujinx stays Ryujinx when based on
                        // Eden); "Based on" names it only while no program is chosen.
                        if !f.name_typed || f.name.trim().is_empty() {
                            f.name = f.program.as_ref().map_or_else(|| d.name.to_string(), |x| x.name_guess());
                            f.name_typed = false;
                        }
                        f.follows = d.id.to_string();
                    }
                }
            });
        });
        ui.end_row();
        ui.label(RichText::new("Program").size(13.0).color(pal.dim));
        ui.horizontal(|ui| {
            let busy = p.pick.is_some();
            if ui.add_enabled(!busy, egui::Button::new("Choose a Flatpak…")).clicked() {
                pick_flatpak = true;
            }
            if ui.add_enabled(!busy, egui::Button::new("Locate…")).on_hover_text("A program or an AppImage").clicked()
            {
                pick_file = true;
            }
        });
        ui.end_row();
        ui.label("");
        let (t, c) = match &f.program {
            Some(x) => (x.label(), pal.text),
            None => ("Not chosen yet".to_string(), pal.dim),
        };
        ui.add(egui::Label::new(RichText::new(t).size(12.0).color(c)).truncate());
        ui.end_row();
        ui.label(RichText::new("Name").size(13.0).color(pal.dim));
        if ui
            .add(egui::TextEdit::singleline(&mut f.name).hint_text("Its name in Lodestar").desired_width(300.0))
            .changed()
        {
            f.name_typed = true;
        }
        ui.end_row();
        if f.follows.is_empty() {
            ui.label(RichText::new("Consoles").size(13.0).color(pal.dim));
            consoles_picker(ui, "ls_emu_add_pf", &mut f.platforms, 300.0);
            ui.end_row();
            ui.label(RichText::new("Arguments").size(13.0).color(pal.dim));
            ui.add(
                egui::TextEdit::singleline(&mut f.args)
                    .hint_text("{rom}")
                    .font(egui::TextStyle::Monospace)
                    .desired_width(300.0),
            );
            ui.end_row();
        }
    });
    if f.follows.is_empty() {
        hint_text(ui, ARGS_HELP, pal);
    }
    let ready = f.program.is_some() && !f.name.trim().is_empty() && (!f.follows.is_empty() || !f.platforms.is_empty());
    ui.add_space(4.0);
    let why = if f.program.is_none() {
        "Choose its program first"
    } else if f.name.trim().is_empty() {
        "Give it a name"
    } else {
        "Tick the consoles it runs"
    };
    let clicked =
        button_row(ui, pal, Buttons::primary("Add emulator", ready).disabled_hint(why).cancel("Cancel"), |_| {});
    if clicked.primary {
        let found: Vec<String> = cat.map(|c| c.emus.iter().map(|e| e.emu.id.clone()).collect()).unwrap_or_default();
        add_emulator(&mut st.config.emulation, f, &found);
        ch = true;
    }
    if clicked.primary || clicked.cancel {
        close = true;
    }
    if close {
        p.add_emu = None;
    }
    if pick_file {
        p.pick = Some((
            PickFor::AddEmulator,
            Pick::start(ui.ctx(), PickKind::File, "Choose the emulator program or AppImage", None),
        ));
    }
    if pick_flatpak {
        open_flatpak_chooser(st, p, FlatpakFor::AddEmulator);
    }
    ch
}

/// Save the Add emulator form: the entry, and for a custom one its profile.
pub(super) fn add_emulator(cfg: &mut crate::config::EmulationConfig, f: &AddEmulator, found: &[String]) -> String {
    let base = if f.follows.is_empty() { slug(&f.name) } else { f.follows.clone() };
    let key = catalog::new_emulator_key(&base, cfg, found);
    let mut x = EmulatorEntry { name: f.name.trim().to_string(), follows: f.follows.clone(), ..Default::default() };
    if let Some(pk) = &f.program {
        catalog::set_program(&mut x, &catalog::picked_kind(pk));
    }
    if f.follows.is_empty() {
        let mut exts: Vec<String> = vec![];
        for pf in &f.platforms {
            for e in platforms::by_id(pf).map(|p| p.extensions).unwrap_or_default() {
                if !exts.iter().any(|x| x == e) {
                    exts.push(e.to_string());
                }
            }
        }
        let args = if f.args.trim().is_empty() { "{rom}".to_string() } else { f.args.trim().to_string() };
        cfg.profiles.insert(
            format!("{key}/default"),
            ProfileEntry {
                name: "Default".into(),
                emulator: key.clone(),
                platforms: f.platforms.clone(),
                extensions: exts,
                args,
                core: String::new(),
                ..Default::default()
            },
        );
    }
    cfg.emulators.insert(key.clone(), x);
    key
}

/// "My Emu 2!" → "my-emu-2".
pub(super) fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() { "custom".into() } else { s }
}
