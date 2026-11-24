//! Edit Game Details → Installation for emulated games: the console, the emulator and profile
//! this game uses (automatic or the user's pick) and its ROM files.

use super::Pal;
use super::dialogs::hint_text;
use super::emulation_ui::catalog::{self, Catalog};
use crate::app::AppState;
use crate::model::{EmuRef, Game, GameId};
use crate::ui::widgets::lazy_spinner;
use egui::{RichText, Sense, Ui, pos2, vec2};
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// The section's state inside `EditDlg.emu`. Saving (edit.rs `build_edits`) writes `emulator` /
/// `profile` to `UserEdits.emulator` / `UserEdits.emulator_profile` when `emulated`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EmuEdit {
    /// The game is an emulated game (has an `EmuRef`): the section shows and saves.
    pub emulated: bool,
    /// Emulator instance id; None (or "") = automatic.
    pub emulator: Option<String>,
    /// Profile id within `emulator`; None (or "") = the emulator's best one.
    pub profile: Option<String>,
    /// "Set up emulation…" was clicked: the Edit dialog opens Settings → Emulation.
    pub open_settings: bool,
    /// The emulator's switches for this game (absent = Default).
    pub options: BTreeMap<String, bool>,
    /// Extra arguments for this game.
    pub args: String,
    /// The game's ROM list (the scanned ROM has an empty path); the first is the default.
    pub roms: Vec<crate::model::GameRom>,
}

impl EmuEdit {
    pub(super) fn from_game(g: &Game) -> EmuEdit {
        EmuEdit {
            emulated: g.emu.is_some(),
            emulator: g.edits.emulator.clone(),
            profile: g.edits.emulator_profile.clone(),
            open_settings: false,
            options: g.edits.emulator_options.clone(),
            args: g.edits.emulator_args.clone(),
            roms: crate::emulation::roms::effective(g),
        }
    }

    /// Write the choice and the switches into edits (nothing for other games).
    pub(super) fn apply_to(&self, e: &mut crate::model::UserEdits) {
        if !self.emulated {
            return;
        }
        e.emulator = self.emulator.clone().filter(|s| !s.is_empty());
        e.emulator_profile = self.profile.clone().filter(|s| !s.is_empty());
        e.emulator_options = self.options.clone();
        e.emulator_args = self.args.trim().to_string();
        e.roms = crate::emulation::roms::to_store(&self.roms);
    }
}

/// The label column's width (the Edit form's).
const LABEL_W: f32 = 112.0;

fn field(ui: &mut Ui, label: &str, pal: &Pal, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(LABEL_W, 26.0), Sense::hover());
        ui.painter().text(
            pos2(r.left(), r.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.0),
            pal.dim,
        );
        add(ui);
    });
}

/// What is on disk for one ROM file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OnDisk {
    Bytes(u64),
    /// A folder (Wii U / PS3 games, their updates and DLC).
    Folder,
    Missing,
}

/// File sizes of a game's ROM files, read on a helper thread (kept per game in egui memory).
type Sizes = Arc<Mutex<Option<Vec<(PathBuf, OnDisk)>>>>;

fn rom_sizes(ctx: &egui::Context, id: GameId, files: Vec<PathBuf>) -> Option<Vec<(PathBuf, OnDisk)>> {
    let key = egui::Id::NULL.with(("ls_emu_rom_sizes", id, &files));
    if let Some(s) = ctx.data(|d| d.get_temp::<Sizes>(key)) {
        return s.lock().clone();
    }
    let slot: Sizes = Arc::new(Mutex::new(None));
    ctx.data_mut(|d| d.insert_temp(key, slot.clone()));
    let c = ctx.clone();
    let spawned = std::thread::Builder::new().name("rom-sizes".into()).spawn(move || {
        let v = files.into_iter().map(|f| {
            let size = match std::fs::metadata(&f) {
                Ok(m) if m.is_dir() => OnDisk::Folder,
                Ok(m) => OnDisk::Bytes(m.len()),
                Err(_) => OnDisk::Missing,
            };
            (f, size)
        });
        *slot.lock() = Some(v.collect());
        c.request_repaint();
    });
    if let Err(e) = spawned {
        log::warn!("rom sizes thread: {e}");
    }
    None
}

/// The ROM first, then its other files (once each).
pub(crate) fn rom_files(e: &EmuRef) -> Vec<PathBuf> {
    let mut v = vec![e.rom.clone()];
    for f in &e.files {
        if !v.contains(f) {
            v.push(f.clone());
        }
    }
    v
}

/// Draw the section (only called for emulated games). `id` is the game being edited.
pub(super) fn section(ui: &mut Ui, st: &mut AppState, e: &mut EmuEdit, id: Option<GameId>, pal: &Pal) {
    let Some(g) = id.and_then(|id| st.library.get(id)).cloned() else { return };
    let Some(emu) = g.emu.clone() else { return };
    ui.label(RichText::new("Emulation").size(14.5).strong().color(pal.text));
    ui.add_space(2.0);
    field(ui, "Console", pal, |ui| {
        ui.label(RichText::new(catalog::console_name(&emu.platform)).size(13.0).color(pal.text));
    });
    let cat = catalog::catalog(ui.ctx());
    field(ui, "Emulator", pal, |ui| match &cat {
        Some(c) => emulator_pickers(ui, c, &g, &emu, e, pal),
        None => {
            lazy_spinner(ui, 14.0, pal.dim);
            ui.label(RichText::new("Looking for emulators…").size(12.5).color(pal.dim));
        }
    });
    // The game's own files can't be started (a Wii U folder Cemu can't open): say why.
    let unplayable = crate::emulation::registry::unplayable(&g);
    if let Some(why) = &unplayable {
        field(ui, "", pal, |ui| {
            ui.add(egui::Label::new(RichText::new(why).size(12.0).color(pal.danger)).wrap());
        });
    }
    // The emulator picked (here or for the console) isn't allowed to run yet: say so, offer it.
    let mut pending = g.clone();
    pending.edits.emulator = e.emulator.clone();
    if unplayable.is_none()
        && let Some(b) = crate::emulation::registry::blocked_choice_now(&pending)
    {
        let warn = super::emulation_ui::tone(catalog::ChipTone::Warn, pal);
        field(ui, "", pal, |ui| {
            ui.label(RichText::new(format!("{} isn't allowed to run yet", b.label_in_text())).size(12.0).color(warn));
            if ui
                .button("Allow it to run")
                .on_hover_text(
                    "This AppImage isn't marked as a program yet. Lodestar marks it (like chmod u+x) so it can start.",
                )
                .clicked()
            {
                super::emulation_ui::allow_to_run(st, &b);
            }
        });
    }
    if let Some(c) = &cat
        && c.compatible(&emu.platform).is_empty()
    {
        if unplayable.is_none() {
            let why = crate::emulation::registry::missing_reason(&g)
                .unwrap_or_else(|| "No emulator for this console yet.".into());
            field(ui, "", pal, |ui| hint_text(ui, &why, pal));
        }
        field(ui, "", pal, |ui| {
            if ui.button("Set up emulation…").on_hover_text("Settings → Emulation: find or add an emulator").clicked()
            {
                e.open_settings = true;
            }
        });
    } else {
        field(ui, "", pal, |ui| hint_text(ui, "Automatic follows Settings → Emulation (the console's emulator).", pal));
    }
    if unplayable.is_none() {
        game_switches(ui, st, e, &g, &emu, pal);
    }
    ui.add_space(6.0);
    rom_list(ui, st, e, &g, &emu, pal);
}

/// The file picker of "Add ROM…" (kept in egui memory per game while it is open).
type RomPick = Arc<Mutex<Option<super::add::Pick>>>;

fn rom_pick_id(id: GameId) -> egui::Id {
    egui::Id::NULL.with(("ls_emu_add_rom", id))
}

/// The game's ROMs: one (the scanned file / disc set) shows as before; with several, a name,
/// the path and Move up / down / Remove each (the first is the default). "Add ROM…" picks a
/// file, starting in the game's folder.
fn rom_list(ui: &mut Ui, st: &mut AppState, e: &mut EmuEdit, g: &Game, emu: &EmuRef, pal: &Pal) {
    use crate::emulation::roms;
    use crate::ui::widgets::{Icon, icon_button};
    let install = g.install_dir.clone();
    // A file the picker returned.
    let pid = rom_pick_id(g.id);
    if let Some(slot) = ui.data(|d| d.get_temp::<RomPick>(pid)) {
        let res = slot.lock().as_ref().and_then(|p| p.poll());
        if let Some(paths) = res {
            ui.data_mut(|d| d.remove::<RomPick>(pid));
            for p in paths {
                let stored = roms::stored(&p, install.as_deref());
                let exists = e.roms.iter().any(|r| {
                    let rp = if r.path.trim().is_empty() {
                        emu.rom.clone()
                    } else {
                        roms::expand(&r.path, install.as_deref())
                    };
                    rp == p
                });
                if exists {
                    st.notify("That file is already one of this game's ROMs");
                } else {
                    e.roms.push(crate::model::GameRom { name: roms::file_label(&p), path: stored });
                }
            }
        }
    }
    let picking = ui.data(|d| d.get_temp::<RomPick>(pid)).is_some();
    if e.roms.is_empty() {
        e.roms.push(crate::model::GameRom::default());
    }
    let path_of = |r: &crate::model::GameRom| -> PathBuf {
        if r.path.trim().is_empty() { emu.rom.clone() } else { roms::expand(&r.path, install.as_deref()) }
    };
    // Sizes of every file shown (the scanned ROM's bins / discs too).
    let mut all: Vec<PathBuf> = vec![];
    for r in &e.roms {
        if r.path.trim().is_empty() {
            all.extend(rom_files(emu));
        } else {
            all.push(path_of(r));
        }
    }
    let sizes = rom_sizes(ui.ctx(), g.id, all);
    let size_of = |p: &PathBuf| sizes.as_ref().and_then(|v| v.iter().find(|(q, _)| q == p)).map(|(_, s)| *s);
    let home = st.paths.home.clone();
    let several = e.roms.len() > 1;
    let n = e.roms.len();
    let mut op: Option<(usize, i32)> = None;
    for (i, rom) in e.roms.iter_mut().enumerate() {
        let r = rom.clone();
        let scanned = r.path.trim().is_empty();
        let files = if scanned { rom_files(emu) } else { vec![path_of(&r)] };
        let label = match (several, i, files.len()) {
            (true, 0, _) => "ROMs",
            (true, _, _) => "",
            (false, _, 1) => "ROM file",
            (false, _, _) => "ROM files",
        };
        if several {
            field(ui, label, pal, |ui| {
                let hint = if scanned { g.display_name().to_string() } else { roms::file_label(&files[0]) };
                ui.add(egui::TextEdit::singleline(&mut rom.name).hint_text(hint).desired_width(220.0));
                if i == 0 {
                    ui.label(RichText::new("Default").size(11.5).color(pal.accent))
                        .on_hover_text("Play starts this one unless you choose another (Play ▾)");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if icon_button(ui, Icon::Trash, false, 22.0, "Remove from this game (the file stays)").clicked() {
                        op = Some((i, 0));
                    }
                    ui.add_enabled_ui(i + 1 < n, |ui| {
                        if icon_button(ui, Icon::ArrowDown, false, 22.0, "Move down").clicked() {
                            op = Some((i, 1));
                        }
                    });
                    ui.add_enabled_ui(i > 0, |ui| {
                        if icon_button(ui, Icon::ArrowUp, false, 22.0, "Move up (the first is the default)").clicked() {
                            op = Some((i, -1));
                        }
                    });
                });
            });
        }
        for (j, f) in files.iter().enumerate() {
            let lab = if several || j > 0 { "" } else { label };
            field(ui, lab, pal, |ui| {
                let size = size_of(f);
                let name = super::add::short_path(f, &home, 60);
                let missing = size == Some(OnDisk::Missing);
                let color = if missing {
                    pal.danger
                } else if several {
                    pal.dim
                } else {
                    pal.text
                };
                let shown = if several { format!("   {name}") } else { name };
                ui.add(egui::Label::new(RichText::new(shown).monospace().size(12.5).color(color)).truncate())
                    .on_hover_text(f.display().to_string());
                match size {
                    Some(OnDisk::Bytes(b)) => {
                        ui.label(RichText::new(catalog::size_text(b)).size(12.0).color(pal.dim));
                    }
                    Some(OnDisk::Missing) => {
                        ui.label(RichText::new("not found").size(12.0).color(pal.danger));
                    }
                    _ => {}
                }
            });
        }
    }
    match op {
        Some((i, 0)) => {
            e.roms.remove(i);
        }
        Some((i, -1)) => e.roms.swap(i, i - 1),
        Some((i, _)) => e.roms.swap(i, i + 1),
        None => {}
    }
    field(ui, "", pal, |ui| {
        let start = install.clone().filter(|d| d.is_absolute()).or_else(|| emu.rom.parent().map(PathBuf::from));
        if ui
            .add_enabled(!picking, egui::Button::new("Add ROM…"))
            .on_hover_text(
                "Another file of this game (a second version, an extra mode): Play then asks which one to start",
            )
            .clicked()
        {
            let pick = super::add::Pick::start(ui.ctx(), super::add::PickKind::Files, "Add a ROM to this game", start);
            ui.data_mut(|d| d.insert_temp::<RomPick>(pid, Arc::new(Mutex::new(Some(pick)))));
        }
        let dir = if emu.rom.extension().is_none() && g.install_dir.as_ref() == Some(&emu.rom) {
            emu.rom.clone()
        } else {
            emu.rom.parent().map(PathBuf::from).unwrap_or_else(|| emu.rom.clone())
        };
        if ui.button("Open ROM folder").clicked() {
            st.open_path(&dir);
        }
    });
    if several {
        field(ui, "", pal, |ui| {
            hint_text(
                ui,
                "Play asks which ROM to start (or set it per action in Actions). Files in the game's folder are kept as {InstallDir}/….",
                pal,
            );
        });
    }
}

/// Emulator ("X (automatic)" + the other compatible ones) and Profile ("Automatic" + its profiles
/// for this console) drop-downs.
fn emulator_pickers(ui: &mut Ui, c: &Catalog, g: &Game, emu: &EmuRef, e: &mut EmuEdit, pal: &Pal) {
    // What Automatic means for this game: everything but its own choice.
    let mut plain = g.clone();
    plain.edits.emulator = None;
    plain.edits.emulator_profile = None;
    let auto = c.game_emulator(&plain).map(|(x, _)| x);
    pickers(ui, c, &emu.platform, auto.as_ref(), &mut e.emulator, &mut e.profile, None, "ls_edit_emulator", pal);
}

/// "Choose when playing" in an emulator action's profile picker: the flag, and the profiles
/// of the emulator Automatic stands for (what it offers while no emulator is picked).
type ChooseProfile<'a> = (&'a mut bool, Vec<crate::emulation::detect::Profile>);

/// The two drop-downs over an (emulator, profile) choice; None = automatic (`auto` names it).
#[allow(clippy::too_many_arguments)]
fn pickers(
    ui: &mut Ui,
    c: &Catalog,
    pf: &str,
    auto: Option<&crate::emulation::detect::Emulator>,
    emulator: &mut Option<String>,
    profile: &mut Option<String>,
    mut choose: Option<ChooseProfile>,
    salt: impl std::hash::Hash + std::fmt::Debug + Copy,
    pal: &Pal,
) {
    // "Eden (automatic)" stands for Eden too: it isn't listed a second time.
    let auto_id = auto.map(|x| x.id.clone());
    let auto_label = auto.map_or_else(|| "Automatic".to_string(), |x| Catalog::automatic_label(&c.name_of(x)));
    let emus = c.emulators_for(pf);
    let cur = emulator.clone().filter(|s| !s.is_empty());
    let selected = match &cur {
        None => auto_label.clone(),
        Some(id) => c.emu(id).map_or_else(|| format!("{id} (not found)"), |x| c.name_of(&x.emu)),
    };
    egui::ComboBox::from_id_salt((salt, "emu")).width(220.0).selected_text(selected).show_ui(ui, |ui| {
        if ui.selectable_label(cur.is_none() || cur == auto_id, auto_label).clicked() {
            *emulator = None;
            *profile = None;
        }
        for x in emus.iter().filter(|x| Some(&x.emu.id) != auto_id.as_ref()) {
            if ui.selectable_label(cur.as_deref() == Some(x.emu.id.as_str()), c.name_of(&x.emu)).clicked()
                && cur.as_deref() != Some(x.emu.id.as_str())
            {
                *emulator = Some(x.emu.id.clone());
                *profile = None;
            }
        }
    });
    // The automatic emulator's profiles too (picking one keeps that emulator).
    let profiles: Vec<_> = cur
        .as_deref()
        .or(auto_id.as_deref())
        .and_then(|id| c.emu(id))
        .map(|x| x.profiles.iter().filter(|p| p.platforms.iter().any(|q| q == pf)).cloned().collect())
        .unwrap_or_default();
    let pcur = profile.clone().filter(|s| !s.is_empty());
    let choosing = choose.as_ref().is_some_and(|(c, _)| **c);
    // "Choose when playing" lists the profiles of the emulator Play uses (Automatic's too).
    let choosable = match &choose {
        Some((_, auto_profiles)) if cur.is_none() => {
            auto_profiles.iter().filter(|p| p.platforms.iter().any(|q| q == pf)).count() > 1
        }
        Some(_) => profiles.len() > 1,
        None => false,
    };
    let ptext = match &pcur {
        _ if choosing => "Choose when playing".to_string(),
        None => "Automatic".to_string(),
        Some(id) => profiles.iter().find(|p| p.id == *id).map_or_else(|| id.clone(), |p| p.name.clone()),
    };
    ui.label(RichText::new("Profile").size(13.0).color(pal.dim));
    let auto_pick = if cur.is_none() { auto_id.clone() } else { None };
    ui.add_enabled_ui(profiles.len() > 1 || choosable || choosing, |ui| {
        egui::ComboBox::from_id_salt((salt, "profile")).width(170.0).selected_text(ptext).show_ui(ui, |ui| {
            if ui.selectable_label(pcur.is_none() && !choosing, "Automatic").clicked() {
                *profile = None;
                if let Some((c, _)) = choose.as_mut() {
                    **c = false;
                }
            }
            for p in &profiles {
                if ui.selectable_label(pcur.as_deref() == Some(p.id.as_str()) && !choosing, &p.name).clicked() {
                    *profile = Some(p.id.clone());
                    if let Some(a) = &auto_pick {
                        *emulator = Some(a.clone());
                    }
                    if let Some((c, _)) = choose.as_mut() {
                        **c = false;
                    }
                }
            }
            if let Some((c, _)) = choose.as_mut()
                && (choosable || **c)
                && ui
                    .selectable_label(**c, "Choose when playing")
                    .on_hover_text("Play asks which profile to use: one choice per profile")
                    .clicked()
            {
                **c = true;
                *profile = None;
            }
        });
    })
    .response
    .on_disabled_hover_text(if cur.is_none() {
        "Pick an emulator first"
    } else {
        "It has one profile for this console"
    });
}

/// Edit → Actions, an action of type Emulator: the emulator and profile (Automatic = the
/// game's), its key switches; under "More options" the rest of them, the extra arguments (or
/// the whole argument list with "Replace the profile's arguments") and the emulator's
/// command-line page. `edit` is the Installation tab's state (the game's choice and switches,
/// which Default inherits).
pub(super) fn emulator_action(
    ui: &mut Ui,
    st: &mut AppState,
    edit: &EmuEdit,
    id: Option<GameId>,
    i: usize,
    a: &mut crate::model::GameAction,
    pal: &Pal,
) {
    use super::emulation_ui::switches as sw;
    let Some(g) = id.and_then(|id| st.library.get(id)).cloned() else { return };
    let Some(emu) = g.emu.clone() else { return };
    let mut pending = g.clone();
    edit.apply_to(&mut pending.edits);
    let Some(c) = catalog::catalog(ui.ctx()) else {
        field(ui, "Emulator", pal, |ui| {
            lazy_spinner(ui, 14.0, pal.dim);
            ui.label(RichText::new("Looking for emulators…").size(12.5).color(pal.dim));
        });
        return;
    };
    let auto_emu = c.game_emulator(&pending).map(|(x, _)| x);
    let auto_profiles = auto_emu.as_ref().and_then(|x| c.emu(&x.id)).map(|x| x.profiles.clone()).unwrap_or_default();
    field(ui, "Emulator", pal, |ui| {
        let choose = Some((&mut a.choose_profile, auto_profiles));
        pickers(
            ui,
            &c,
            &emu.platform,
            auto_emu.as_ref(),
            &mut a.emulator,
            &mut a.profile,
            choose,
            ("ls_act_emu", i),
            pal,
        );
    });
    // Which of the game's ROMs it starts (only with several).
    let roms = crate::emulation::roms::list(&pending);
    if roms.len() > 1 {
        field(ui, "ROM", pal, |ui| rom_picker(ui, a, &roms, i));
    } else if a.rom.is_some() && !roms.iter().any(|r| Some(&r.key) == a.rom.as_ref()) {
        // Its ROM was removed: back to the default (there is only one).
        a.rom = None;
    }
    let chosen = crate::emulation::registry::action_choice(&c.reg, &pending, a).ok();
    let Some((x, prof)) = chosen else { return };
    let cfg = &st.config.emulation;
    let sws = sw::switches_of(&x, &prof, cfg);
    // Default = the game's (Edit → Installation), then the console's and the emulator's.
    let mut below = vec![(edit.options.clone(), crate::launch::split_command(&edit.args).unwrap_or_default())];
    below.extend(sw::levels_below(cfg, Some(&emu.platform), &x.id));
    let inherit = |s: &crate::emulation::options::Switch| sw::inherited_from(s, &below);
    if sws.iter().any(|s| s.key) {
        field(ui, "Start options", pal, |ui| {
            ui.vertical(|ui| {
                sw::tri_rows(ui, ("act", i), &sws, &inherit, &mut a.options, &|s| s.key, pal);
            });
        });
    }
    let more_id = egui::Id::NULL.with(("ls_act_emu_more", i));
    // Something set under More options: show it open.
    if a.replace_args || !a.args.trim().is_empty() {
        sw::set_more_open(ui.ctx(), more_id, ui.data(|d| d.get_temp::<bool>(more_id)).unwrap_or(true));
    }
    field(ui, "", pal, |ui| {
        sw::more_options(ui, more_id, pal);
    });
    if !ui.data(|d| d.get_temp::<bool>(more_id)).unwrap_or(false) {
        return;
    }
    if sws.iter().any(|s| !s.key) {
        field(ui, "", pal, |ui| {
            ui.vertical(|ui| {
                sw::tri_rows(ui, ("act", i), &sws, &inherit, &mut a.options, &|s| !s.key, pal);
            });
        });
    }
    let label = if a.replace_args { "Arguments" } else { "Extra arguments" };
    let template = crate::emulation::options::template(&x, &prof).to_string();
    field(ui, label, pal, |ui| {
        ui.vertical(|ui| {
            let hint = if a.replace_args { template.clone() } else { "-nofullscreen".to_string() };
            sw::extra_args_box(ui, &mut a.args, &hint, pal);
            ui.checkbox(&mut a.replace_args, "Replace the profile's arguments").on_hover_text(format!(
                "Use these arguments instead of the profile's ({template}). {{Rom}} is the game file; when they don't name it, it goes at the end."
            ));
            sw::docs_link(ui, st, &x, pal);
        });
    });
}

/// An emulator action's ROM: the default (the first of the game's list), one of them, or
/// "Choose when playing" (Play asks).
fn rom_picker(ui: &mut Ui, a: &mut crate::model::GameAction, roms: &[crate::emulation::roms::Rom], i: usize) {
    let first = roms.first().map(|r| r.name.clone()).unwrap_or_default();
    let text = if a.choose_rom {
        "Choose when playing".to_string()
    } else {
        match &a.rom {
            None => format!("Default ROM ({first})"),
            Some(k) => roms
                .iter()
                .find(|r| r.key == *k)
                .map_or_else(|| "(removed — pick another)".to_string(), |r| r.name.clone()),
        }
    };
    egui::ComboBox::from_id_salt(("ls_act_rom", i)).width(300.0).selected_text(text).show_ui(ui, |ui| {
        if ui
            .selectable_label(a.rom.is_none() && !a.choose_rom, format!("Default ROM ({first})"))
            .on_hover_text("The first of the game's ROMs (Edit → Installation)")
            .clicked()
        {
            a.rom = None;
            a.choose_rom = false;
        }
        for r in roms {
            if ui
                .selectable_label(a.rom.as_deref() == Some(r.key.as_str()) && !a.choose_rom, &r.name)
                .on_hover_text(r.path.display().to_string())
                .clicked()
            {
                a.rom = Some(r.key.clone());
                a.choose_rom = false;
            }
        }
        if ui
            .selectable_label(a.choose_rom, "Choose when playing")
            .on_hover_text("Play asks which ROM to start")
            .clicked()
        {
            a.choose_rom = true;
            a.rom = None;
        }
    });
}

/// The game's emulator switches (the key ones; the rest, extra arguments, the emulator's
/// command-line page and "Will run" under "More options").
fn game_switches(ui: &mut Ui, st: &mut AppState, e: &mut EmuEdit, g: &Game, emu: &EmuRef, pal: &Pal) {
    use super::emulation_ui::switches as sw;
    let Some(reg) = crate::emulation::registry::current() else { return };
    let mut pending = g.clone();
    e.apply_to(&mut pending.edits);
    let Some((x, prof)) = crate::emulation::registry::choose_for(&reg, Some(&pending), emu) else { return };
    let cfg = &st.config.emulation;
    let sws = sw::switches_of(&x, &prof, cfg);
    let below = sw::levels_below(cfg, Some(&emu.platform), &x.id);
    let inherit = |s: &crate::emulation::options::Switch| sw::inherited_from(s, &below);
    if sws.iter().any(|s| s.key) {
        field(ui, "Start options", pal, |ui| {
            ui.vertical(|ui| {
                sw::tri_rows(ui, ("game", g.id), &sws, &inherit, &mut e.options, &|s| s.key, pal);
            });
        });
    }
    let more_id = egui::Id::NULL.with(("ls_edit_emu_more", g.id));
    field(ui, "", pal, |ui| {
        sw::more_options(ui, more_id, pal);
    });
    if !ui.data(|d| d.get_temp::<bool>(more_id)).unwrap_or(false) {
        return;
    }
    if sws.iter().any(|s| !s.key) {
        field(ui, "", pal, |ui| {
            ui.vertical(|ui| {
                sw::tri_rows(ui, ("game", g.id), &sws, &inherit, &mut e.options, &|s| !s.key, pal);
            });
        });
    }
    field(ui, "Extra arguments", pal, |ui| {
        ui.vertical(|ui| {
            sw::extra_args_box(ui, &mut e.args, "for this game only", pal);
            sw::docs_link(ui, st, &x, pal);
        });
    });
    let mut now = g.clone();
    e.apply_to(&mut now.edits);
    let (text, col) = match crate::emulation::registry::spec_with(&reg, &now, emu, None) {
        Some(spec) => (spec_text(&spec), pal.dim),
        None => ("Can't build the command — check the extra arguments.".to_string(), pal.danger),
    };
    field(ui, "Will run", pal, |ui| {
        ui.add(egui::Label::new(RichText::new(&text).size(12.0).monospace().color(col)).truncate())
            .on_hover_text(&text);
    });
}

/// A command in words, quoted like a shell would need it.
pub(super) fn spec_text(spec: &crate::model::LaunchSpec) -> String {
    match spec {
        crate::model::LaunchSpec::Command { program, args, .. } => std::iter::once(program)
            .chain(args.iter())
            .map(|w| crate::launch::shell_quote(w))
            .collect::<Vec<_>>()
            .join(" "),
        crate::model::LaunchSpec::Url(u) => u.clone(),
        crate::model::LaunchSpec::Steam { appid } => format!("steam://rungameid/{appid}"),
    }
}
