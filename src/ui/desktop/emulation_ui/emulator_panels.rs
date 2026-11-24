//! Settings → Emulation, an emulator row's panels: Start options, arguments, profiles.

use super::catalog::{self, CatEmu};
use super::emulator_add::consoles_picker;
use super::lists::*;
use super::*;
use crate::config::ProfileEntry;

/// An emulator's options: its key switches (On / Off for every game); under "More options" the
/// rest of them, extra arguments for every game, its command-line page and its profiles.
pub(super) fn options_panel(ui: &mut Ui, st: &mut AppState, p: &mut EmuPage, e: &CatEmu, pal: &Pal) -> bool {
    use super::switches;
    let mut ch = false;
    let Some(prof) = switches::profile_for(&e.profiles, None) else {
        hint_text(ui, "It has no profiles yet.", pal);
        return false;
    };
    let sws = switches::switches_of(&e.emu, prof, &st.config.emulation);
    let mut set = st.config.emulation.emulator_settings.get(&e.emu.id).cloned().unwrap_or_default();
    let before = set.clone();
    ui.add_space(4.0);
    if sws.is_empty() {
        hint_text(
            ui,
            &format!("Lodestar knows no switches for {}; extra arguments are under More options.", e.emu.name),
            pal,
        );
    }
    switches::toggle_rows(ui, &sws, &mut set.options, &|s| s.key, pal);
    let more_id = egui::Id::NULL.with(("ls_emu_options_more", &e.emu.id));
    let debug_open = std::mem::take(&mut p.more_open);
    if debug_open {
        switches::set_more_open(ui.ctx(), more_id, true);
    }
    if switches::more_options(ui, more_id, pal) {
        switches::toggle_rows(ui, &sws, &mut set.options, &|s| !s.key, pal);
        ui.label(RichText::new("Extra arguments for every game").size(13.0).color(pal.text));
        ch |= switches::buffered_args(ui, ("emu", e.emu.id.as_str()), &mut set.extra_args, "extra arguments", pal);
        switches::docs_link(ui, st, &e.emu, pal);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .button("Profiles…")
                .on_hover_text("How it starts each console's games (built in, or your own)")
                .clicked()
            {
                p.emu_panel = Some((e.emu.id.clone(), EmuPanel::Profiles));
            }
            if ui.button("Custom arguments…").on_hover_text("Replace the profiles' arguments for every game").clicked()
            {
                p.emu_panel = Some((e.emu.id.clone(), EmuPanel::Args));
                p.args_text = Some((e.emu.id.clone(), catalog::entry_for(&e.emu, &st.config.emulation).args));
            }
        });
        if debug_open {
            ui.scroll_to_cursor(Some(egui::Align::Center));
        }
    }
    ui.add_space(4.0);
    if set != before {
        if set.is_empty() {
            st.config.emulation.emulator_settings.remove(&e.emu.id);
        } else {
            st.config.emulation.emulator_settings.insert(e.emu.id.clone(), set);
        }
        ch = true;
    }
    ch
}

pub(super) fn args_panel(ui: &mut Ui, st: &mut AppState, p: &mut EmuPage, e: &CatEmu, pal: &Pal) -> bool {
    let mut ch = false;
    let default = e.profiles.first().map(|p| p.args.clone()).unwrap_or_default();
    let text = match &mut p.args_text {
        Some((id, t)) if *id == e.emu.id => t,
        _ => {
            p.args_text = Some((e.emu.id.clone(), catalog::entry_for(&e.emu, &st.config.emulation).args));
            &mut p.args_text.as_mut().expect("just set").1
        }
    };
    ui.label(RichText::new("Arguments for every game").size(13.0).strong().color(pal.text));
    hint_text(ui, "Empty: each profile's own arguments. Anything here replaces them.", pal);
    ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(if default.is_empty() { "{rom}".to_string() } else { default })
            .font(egui::TextStyle::Monospace)
            .desired_width(f32::INFINITY),
    );
    hint_text(ui, ARGS_HELP, pal);
    let mut save = None;
    ui.horizontal(|ui| {
        if primary_button(ui, "Save", pal, true).clicked() {
            save = Some(text.trim().to_string());
        }
        if plain_button(ui, "Use the profile's").clicked() {
            save = Some(String::new());
        }
        if plain_button(ui, "Close").clicked() {
            p.emu_panel = None;
        }
    });
    if let Some(args) = save {
        let cfg = &mut st.config.emulation;
        let mut x = catalog::entry_for(&e.emu, cfg);
        if x.args != args || !cfg.emulators.contains_key(&e.emu.id) {
            x.args = args;
            cfg.emulators.insert(e.emu.id.clone(), x);
            ch = true;
        }
        p.emu_panel = None;
    }
    ch
}

pub(super) fn profiles_panel(ui: &mut Ui, st: &mut AppState, p: &mut EmuPage, e: &CatEmu, pal: &Pal) -> bool {
    let mut ch = false;
    let has_cores = !e.emu.cores.is_empty();
    if e.profiles.is_empty() {
        hint_text(ui, "No profiles yet.", pal);
    }
    for pr in &e.profiles {
        let key = format!("{}/{}", e.emu.id, pr.id);
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(&pr.name).size(13.0).strong().color(pal.text));
            ui.label(RichText::new(if pr.custom { "yours" } else { "built in" }).size(11.5).color(pal.dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if pr.custom {
                    if ui.small_button("Delete").clicked() {
                        st.config.emulation.profiles.remove(&key);
                        ch = true;
                    }
                    if ui.small_button("Edit").clicked() {
                        p.profile_edit = if p.profile_edit.as_deref() == Some(&key) { None } else { Some(key.clone()) };
                    }
                } else if ui
                    .small_button("Copy and edit")
                    .on_hover_text("Your own profile based on this one (the built-in one stays as it is)")
                    .clicked()
                {
                    let cfg = &mut st.config.emulation;
                    let k = catalog::new_profile_key(&e.emu.id, cfg);
                    cfg.profiles.insert(
                        k.clone(),
                        ProfileEntry {
                            name: format!("{} (mine)", pr.name),
                            emulator: e.emu.id.clone(),
                            platforms: pr.platforms.clone(),
                            extensions: pr.extensions.clone(),
                            args: pr.args.clone(),
                            core: pr.core.clone().unwrap_or_default(),
                            program: pr.program.clone().unwrap_or_default(),
                            cwd: pr.cwd.clone(),
                            tracking: pr.tracking,
                        },
                    );
                    p.profile_edit = Some(k);
                    ch = true;
                }
            });
        });
        let consoles: Vec<String> = pr.platforms.iter().map(|x| catalog::short_name(x)).collect();
        let mut facts = vec![catalog::join_limited(&consoles, 8)];
        if !pr.extensions.is_empty() {
            facts.push(pr.extensions.iter().map(|x| format!(".{x}")).collect::<Vec<_>>().join(" "));
        }
        if let Some(core) = &pr.core {
            facts.push(format!("core {core}"));
        }
        if let Some(prog) = &pr.program {
            facts.push(format!("runs {}", prog.display()));
        }
        hint_text(ui, &facts.join(" · "), pal);
        ui.label(
            RichText::new(if pr.args.is_empty() { "{rom}" } else { &pr.args }).monospace().size(12.0).color(pal.dim),
        );
    }
    // A profile being edited that the registry doesn't show yet (just copied).
    if let Some(k) = p.profile_edit.clone()
        && k.starts_with(&format!("{}/", e.emu.id))
        && st.config.emulation.profiles.contains_key(&k)
    {
        ui.add_space(6.0);
        ch |= profile_editor(ui, st, p, &k, has_cores, pal);
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button("New profile").on_hover_text("Your own arguments for some consoles").clicked() {
            let cfg = &mut st.config.emulation;
            let k = catalog::new_profile_key(&e.emu.id, cfg);
            cfg.profiles.insert(
                k.clone(),
                ProfileEntry {
                    name: "My profile".into(),
                    emulator: e.emu.id.clone(),
                    args: "{rom}".into(),
                    ..Default::default()
                },
            );
            p.profile_edit = Some(k);
            ch = true;
        }
        if plain_button(ui, "Close").clicked() {
            p.emu_panel = None;
            p.profile_edit = None;
        }
    });
    ch
}

/// How Play follows games of a profile, in words.
fn profile_tracking_label(t: crate::model::TrackingMode) -> &'static str {
    use crate::model::TrackingMode as T;
    match t {
        T::Default => "Default (the emulator and what it starts)",
        T::Process => "Only the program itself",
        T::Folder => "Any process in the program's folder",
        T::None => "Don't track (no play time)",
    }
}

/// A user profile's fields (name, consoles, file types, arguments, core, its own program;
/// working folder and tracking under "More options").
fn profile_editor(ui: &mut Ui, st: &mut AppState, p: &mut EmuPage, key: &str, has_cores: bool, pal: &Pal) -> bool {
    let mut ch = false;
    let mut pick_program = false;
    let Some(pe) = st.config.emulation.profiles.get_mut(key) else { return false };
    egui::Frame::new()
        .fill(mix(pal.surface, pal.bg, 0.4))
        .stroke(egui::Stroke::new(1.0, with_alpha(pal.border, 200)))
        .corner_radius(6)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Grid::new(("ls_emu_profile", key)).num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label(RichText::new("Name").size(13.0).color(pal.dim));
                ch |= ui.add(egui::TextEdit::singleline(&mut pe.name).desired_width(300.0)).changed();
                ui.end_row();
                ui.label(RichText::new("Consoles").size(13.0).color(pal.dim));
                ch |= consoles_picker(ui, ("ls_emu_profile_pf", key), &mut pe.platforms, 300.0);
                ui.end_row();
                ui.label(RichText::new("File types").size(13.0).color(pal.dim));
                let id = egui::Id::NULL.with(("ls_emu_profile_ext", key));
                let mut buf: String = ui.data(|d| d.get_temp(id)).unwrap_or_else(|| pe.extensions.join(", "));
                if ui.add(egui::TextEdit::singleline(&mut buf).hint_text("iso, chd, cue").desired_width(300.0)).changed() {
                    pe.extensions = catalog::parse_extensions(&buf);
                    ch = true;
                }
                ui.data_mut(|d| d.insert_temp(id, buf));
                ui.end_row();
                ui.label(RichText::new("Arguments").size(13.0).color(pal.dim));
                ch |= ui
                    .add(egui::TextEdit::singleline(&mut pe.args).hint_text("{rom}").font(egui::TextStyle::Monospace).desired_width(300.0))
                    .changed();
                ui.end_row();
                if has_cores {
                    ui.label(RichText::new("Core").size(13.0).color(pal.dim));
                    ch |= ui.add(egui::TextEdit::singleline(&mut pe.core).hint_text("swanstation_libretro").desired_width(300.0)).changed();
                    ui.end_row();
                }
                // Its own program: a fork, a test build or a second install next to the usual one.
                ui.label(RichText::new("Program").size(13.0).color(pal.dim));
                ui.horizontal(|ui| {
                    let mut text = pe.program.to_string_lossy().into_owned();
                    if ui
                        .add(egui::TextEdit::singleline(&mut text).hint_text("the emulator's own").font(egui::TextStyle::Monospace).desired_width(262.0))
                        .on_hover_text("Run another program with this profile (a fork, a test build, a second install). Empty: the emulator's own.")
                        .changed()
                    {
                        pe.program = PathBuf::from(text.trim());
                        ch = true;
                    }
                    if ui.add_enabled(p.pick.is_none(), egui::Button::new("…")).on_hover_text("Choose the program or AppImage").clicked() {
                        pick_program = true;
                    }
                });
                ui.end_row();
            });
            let more_id = egui::Id::NULL.with(("ls_emu_profile_more", key));
            let has_more = !pe.cwd.trim().is_empty() || pe.tracking != crate::model::TrackingMode::Default;
            let mut more = ui.data(|d| d.get_temp::<bool>(more_id)).unwrap_or(has_more);
            let text = if more { "▾  More options" } else { "▸  More options" };
            if ui
                .add(egui::Button::new(RichText::new(text).size(12.5).color(pal.dim)).frame(false))
                .on_hover_text("Working folder and how Play follows its games")
                .clicked()
            {
                more = !more;
            }
            ui.data_mut(|d| d.insert_temp(more_id, more));
            if more {
                egui::Grid::new(("ls_emu_profile_more_grid", key)).num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                    ui.label(RichText::new("Working folder").size(13.0).color(pal.dim));
                    ch |= ui
                        .add(
                            egui::TextEdit::singleline(&mut pe.cwd)
                                .hint_text("automatic")
                                .font(egui::TextStyle::Monospace)
                                .desired_width(300.0),
                        )
                        .on_hover_text("Where it starts: {EmulatorDir} (the emulator's folder), {InstallDir} (the game's folder) or any folder. Automatic: its own program's folder.")
                        .changed();
                    ui.end_row();
                    ui.label(RichText::new("Tracking").size(13.0).color(pal.dim));
                    egui::ComboBox::from_id_salt(("ls_emu_profile_track", key))
                        .width(300.0)
                        .selected_text(profile_tracking_label(pe.tracking))
                        .show_ui(ui, |ui| {
                            for t in [crate::model::TrackingMode::Default, crate::model::TrackingMode::Process, crate::model::TrackingMode::Folder, crate::model::TrackingMode::None] {
                                ch |= ui.selectable_value(&mut pe.tracking, t, profile_tracking_label(t)).changed();
                            }
                        });
                    ui.end_row();
                });
            }
            hint_text(ui, ARGS_HELP, pal);
            if plain_button(ui, "Done").clicked() {
                p.profile_edit = None;
            }
        });
    if pick_program {
        let start = st
            .config
            .emulation
            .profiles
            .get(key)
            .and_then(|pe| pe.program.parent().map(Path::to_path_buf))
            .filter(|d| d.is_absolute());
        p.pick = Some((
            PickFor::ProfileProgram(key.to_string()),
            Pick::start(ui.ctx(), PickKind::File, "Choose the program for this profile", start),
        ));
    }
    ch
}
