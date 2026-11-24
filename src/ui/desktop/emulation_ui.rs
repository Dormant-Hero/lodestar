//! Settings → Emulation: the global switches; consoles that have games but no emulator ("Needs an
//! emulator": Get it on Flathub / Locate…); consoles several emulators can run ("Consoles with a
//! choice": one picker each); one list of emulators (each with the consoles it runs, their games
//! and BIOS, <Emulator> settings…, Start options, ⋯; copies of the same emulator under one row); the
//! emulator folders, the ROM folders and the BIOS folders. Everything is found by itself; each
//! row has the manual way next to it (Locate…, Choose a Flatpak…, Add folder…, Choose BIOS files…).
//!
//! Detection, BIOS checks and file pickers run on helper threads (`emulation_ui/catalog.rs`);
//! drawing only reads what they handed back.

use super::Pal;
use super::add::{Pick, PickKind, short_path};
use super::dialogs::{hint_text, modal, plain_button, primary_button};
use super::settings::SettingsDlg;
use crate::app::AppState;
use crate::emulation::bios::{BiosFile, BiosState, PackageKind};
use crate::emulation::detect::Emulator;
use crate::ui::widgets::{Icon, lazy_spinner, luminance, mix, paint_icon, toggle_switch, with_alpha};
use crossbeam_channel::Receiver;
use egui::{Color32, RichText, Sense, Ui, pos2, vec2};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) mod addrom;
mod bios_ui;
pub(crate) mod catalog;
pub(crate) mod choose;
mod demo;
mod emulator_add;
mod emulator_panels;
mod folders;
mod lists;
pub(crate) mod switches;
#[cfg(test)]
mod tests;

use catalog::{BiosJob, Catalog, ChipTone, LibFacts, Picked};
pub(super) use emulator_add::console_combo;
pub(super) use lists::allow_to_run;

/// What a file / folder picker is for.
#[derive(Clone, Debug, PartialEq)]
enum PickFor {
    /// The program for a console's suggested emulator (none was found).
    Locate(String),
    /// A new program for an emulator.
    ChangeProgram(String),
    /// The Add emulator form's program.
    AddEmulator,
    /// Add folder… (ROM folder).
    RomFolder,
    /// Add folder… (emulator folder).
    EmulatorFolder,
    /// Choose BIOS files… for a console.
    BiosFiles(String),
    /// Add BIOS folder…
    BiosFolder,
    /// A profile's own program (profile key "<emulator>/<profile>").
    ProfileProgram(String),
}

/// A Flatpak picked from the installed list goes to …
#[derive(Clone, Debug, PartialEq)]
enum FlatpakFor {
    Locate(String),
    ChangeProgram(String),
    AddEmulator,
}

/// The "Choose a Flatpak" list (installed apps, read on a helper thread).
#[derive(Default)]
struct FlatpakList {
    apps: Option<Vec<(String, String)>>,
    rx: Option<Receiver<Vec<(String, String)>>>,
}

/// What a BIOS panel button does once confirmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Job {
    /// Copy BIOS files / keys into the emulator's folder.
    Copy,
    /// Copy Switch firmware into the emulator's NAND.
    Firmware,
    /// Run RPCS3's own firmware installer.
    Ps3Firmware,
    /// "Set up everything" (the console's whole plan).
    Setup,
}

/// "Copy to <emulator>" / "Install firmware into <emulator>" waiting for a yes.
#[derive(Clone, Debug, PartialEq)]
struct Confirm {
    platform: String,
    emulator: String,
    job: Job,
}

/// What an emulator row shows when expanded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EmuPanel {
    Profiles,
    Args,
    /// Its switches and extra arguments for every game.
    Options,
}

/// Add emulator… form.
#[derive(Clone, Debug, Default, PartialEq)]
struct AddEmulator {
    /// Built-in definition id ("" = custom).
    follows: String,
    program: Option<Picked>,
    name: String,
    /// The user typed the name (else it follows the program / "Based on").
    name_typed: bool,
    args: String,
    /// Custom emulators: the consoles it runs.
    platforms: Vec<String>,
}

/// Add folder… after the folder was picked.
#[derive(Clone, Debug, Default, PartialEq)]
struct AddFolder {
    path: PathBuf,
    /// "" = detect from the folder names.
    platform: String,
}

type Identified = Vec<(PathBuf, &'static BiosFile)>;
/// Files copied, or why not.
type Copied = Result<usize, String>;

/// The page's own state, kept in `SettingsDlg.emu` while Settings is open.
#[derive(Default)]
pub(crate) struct EmuPage {
    facts: LibFacts,
    bios: BiosJob,
    /// Screenshot demo: show this instead of the published registry.
    demo: Option<Arc<Catalog>>,
    /// The console whose BIOS panel is open.
    bios_open: Option<String>,
    /// The console whose BIOS details (every file, single actions) are shown.
    bios_details: Option<String>,
    /// Emulator rows whose other copies are shown (the row's emulator id).
    copies_open: std::collections::BTreeSet<String>,
    /// Screenshot hook: scroll to this section ("emulators", "folders") once.
    scroll_to: Option<String>,
    pick: Option<(PickFor, Pick)>,
    flatpaks: FlatpakList,
    choose_flatpak: Option<(FlatpakFor, String)>,
    /// Per console: the last BIOS message (text, is an error).
    bios_msg: BTreeMap<String, (String, bool)>,
    /// Choose BIOS files… being identified: (console, result, files picked).
    identify: Option<(String, Receiver<Identified>, usize)>,
    /// Copy to <emulator> / Install firmware running: (console, emulator, result, what).
    install: Option<(String, String, Receiver<Copied>, Job)>,
    confirm: Option<Confirm>,
    /// Check the BIOS again on the next frame (files were copied).
    recheck: bool,
    emu_panel: Option<(String, EmuPanel)>,
    /// The user profile being edited ("<emulator>/<profile>").
    profile_edit: Option<String>,
    args_text: Option<(String, String)>,
    add_emu: Option<AddEmulator>,
    add_folder: Option<AddFolder>,
    /// The ROM folder whose exclusions are shown ("<key>" or the auto id).
    exclusions_open: Option<String>,
    new_exclusion: String,
    /// "Scan now" was clicked (Settings saves and rescans).
    recheck_scan: bool,
    /// The console whose options (switches, extra arguments) are open.
    console_opts: Option<String>,
    /// Screenshot hook: open "More options" in the options shown.
    more_open: bool,
}

impl EmuPage {
    /// Testing hook (`settings-emulation-<what>`): `demo` (made-up emulators and BIOS states),
    /// `add` (Add emulator… open), `bios:<platform>` (that console's BIOS panel), `details:<platform>`
    /// (with its details), `setup:<platform>` (its "Set up everything" confirmation), `folder`
    /// (Add folder… with ~/ROMs picked), `profile:<emulator>/<profile>` (editing that profile),
    /// `copies:<emulator>` (that row's other copies shown), `at:choice` / `at:emulators` / `at:copies` / `at:folders`
    /// (scrolled there); several joined with `+` ("demo+bios:sony_playstation2").
    pub(crate) fn debug(what: &str) -> EmuPage {
        let mut p = EmuPage::default();
        for w in what.split('+') {
            if w == "demo" {
                let (cat, bios) = demo::demo();
                p.demo = Some(Arc::new(cat));
                p.bios = BiosJob::fixed(bios);
            } else if w == "add" {
                p.add_emu = Some(AddEmulator::default());
            } else if let Some(pf) = w.strip_prefix("bios:") {
                p.bios_open = Some(pf.to_string());
            } else if let Some(pf) = w.strip_prefix("details:") {
                p.bios_open = Some(pf.to_string());
                p.bios_details = Some(pf.to_string());
            } else if let Some(at) = w.strip_prefix("at:") {
                p.scroll_to = Some(at.to_string());
            } else if let Some(id) = w.strip_prefix("copies:") {
                p.copies_open.insert(id.to_string());
            } else if let Some(pf) = w.strip_prefix("setup:") {
                p.bios_open = Some(pf.to_string());
                p.confirm = Some(Confirm { platform: pf.to_string(), emulator: String::new(), job: Job::Setup });
            } else if let Some(id) = w.strip_prefix("options:") {
                // An emulator's options, with "More options" open.
                p.emu_panel = Some((id.to_string(), EmuPanel::Options));
                p.more_open = true;
            } else if let Some(key) = w.strip_prefix("profile:") {
                // A profile of yours being edited ("mgba/test"), its More options open.
                let emu = key.split('/').next().unwrap_or_default().to_string();
                p.emu_panel = Some((emu, EmuPanel::Profiles));
                p.profile_edit = Some(key.to_string());
                p.more_open = true;
            } else if let Some(pf) = w.strip_prefix("console:") {
                p.console_opts = Some(pf.to_string());
                p.more_open = true;
            } else if w == "folder" {
                p.add_folder = Some(AddFolder {
                    path: dirs::home_dir().unwrap_or_default().join("ROMs"),
                    platform: String::new(),
                });
            }
        }
        p
    }
}

/// Draw the Emulation page. Returns true when the config changed (Settings then saves it, which
/// also re-detects emulators in the background).
pub(super) fn emulation(ui: &mut Ui, st: &mut AppState, d: &mut SettingsDlg, pal: &Pal) -> bool {
    // Room for the scroll bar on the right (it floats over the content), like Libraries.
    let (ch, scan) = egui::Frame::new()
        .inner_margin(egui::Margin { left: 0, right: 14, top: 0, bottom: 0 })
        .show(ui, |ui| page(ui, st, &mut d.emu, pal))
        .inner;
    if scan {
        d.scan_changed = true;
    }
    if d.emu.take_scan_now() {
        st.save_config();
        st.rescan();
        d.scan_changed = false;
    }
    ch || scan
}

impl EmuPage {
    fn take_scan_now(&mut self) -> bool {
        std::mem::take(&mut self.recheck_scan)
    }
}

/// Returns (config changed, scan settings changed).
fn page(ui: &mut Ui, st: &mut AppState, p: &mut EmuPage, pal: &Pal) -> (bool, bool) {
    let ctx = ui.ctx().clone();
    if p.facts.generation != st.library.generation || p.facts.generation == 0 {
        p.facts = LibFacts::compute(&st.library.games, st.library.generation);
    }
    let cat = p.demo.clone().or_else(|| catalog::catalog(&ctx));
    let mut ch = false;
    let mut scan = false;
    poll(st, p, &cat, &mut ch, &mut scan);

    let lists = ConsoleLists::new(&p.facts, cat.as_deref());
    if let Some(c) = &cat
        && !p.bios.is_fixed()
    {
        let force = std::mem::take(&mut p.recheck);
        p.bios.update(&ctx, &c.reg, &lists.with_games, force);
    }

    header(ui, st, p, cat.as_deref(), pal);
    ui.add_space(8.0);
    let (c, s) = switches(ui, st, pal);
    ch |= c;
    scan |= s;

    // ---- consoles with games but no emulator
    if !lists.needs.is_empty() {
        group_title(ui, "NEEDS AN EMULATOR", pal);
        hint_text(ui, "These consoles have games, but no emulator was found for them.", pal);
        ui.add_space(4.0);
        card(ui, pal, |ui| {
            for (i, pf) in lists.needs.iter().enumerate() {
                if i > 0 {
                    row_separator(ui, pal);
                }
                ch |= needs_row(ui, st, p, cat.as_deref(), pf, pal);
            }
        });
    }
    // ---- consoles several emulators can run
    if let Some(c) = cat.as_deref().filter(|_| !lists.choice.is_empty()) {
        group_title_at(ui, p, "choice", "CONSOLES WITH A CHOICE", pal);
        hint_text(ui, "Several of your emulators run these. A game can still have its own (Edit → Installation).", pal);
        ui.add_space(4.0);
        card(ui, pal, |ui| {
            for (i, pf) in lists.choice.iter().enumerate() {
                if i > 0 {
                    row_separator(ui, pal);
                }
                ch |= choice_row(ui, st, p, c, pf, pal);
            }
        });
    }

    // ---- emulators, emulator folders, ROM folders, BIOS folders
    let (c, s) = lists::emulators(ui, st, p, cat.as_deref(), pal);
    ch |= c;
    scan |= s;
    let (c, s) = folders::emulator_folders(ui, st, p, cat.as_deref(), pal);
    ch |= c;
    scan |= s;
    let (c, s) = folders::rom_folders(ui, st, p, pal);
    ch |= c;
    scan |= s;
    ch |= folders::bios_folders(ui, st, p, pal);
    ui.add_space(8.0);

    if let Some((target, mut filter)) = p.choose_flatpak.take() {
        match flatpak_chooser(ui.ctx(), st, p, &mut filter, pal) {
            Chooser::Keep => p.choose_flatpak = Some((target, filter)),
            Chooser::Cancel => {}
            Chooser::Picked(id, name) => {
                ch |= apply_program(st, p, cat.as_deref(), &target_of(&target), Picked::Flatpak { id, name });
            }
        }
    }
    (ch, scan)
}

fn target_of(f: &FlatpakFor) -> PickFor {
    match f {
        FlatpakFor::Locate(p) => PickFor::Locate(p.clone()),
        FlatpakFor::ChangeProgram(e) => PickFor::ChangeProgram(e.clone()),
        FlatpakFor::AddEmulator => PickFor::AddEmulator,
    }
}

/// The consoles the page talks about: those with games (by name), the ones of them no emulator
/// runs, and the ones with a real choice (two or more emulators / profiles).
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct ConsoleLists {
    pub with_games: Vec<String>,
    pub needs: Vec<String>,
    pub choice: Vec<String>,
}

impl ConsoleLists {
    pub fn new(facts: &LibFacts, cat: Option<&Catalog>) -> ConsoleLists {
        let mut with_games: Vec<String> = facts.per_platform.keys().filter(|k| !k.is_empty()).cloned().collect();
        with_games.sort_by_cached_key(|id| catalog::console_name(id).to_lowercase());
        let Some(c) = cat else { return ConsoleLists { with_games, ..Default::default() } };
        let needs = with_games.iter().filter(|pf| c.compatible(pf).is_empty()).cloned().collect();
        let choice = with_games.iter().filter(|pf| c.choices(pf).len() >= 2).cloned().collect();
        ConsoleLists { with_games, needs, choice }
    }
}

/// Deliver finished pickers and background jobs.
fn poll(st: &mut AppState, p: &mut EmuPage, cat: &Option<Arc<Catalog>>, ch: &mut bool, scan: &mut bool) {
    if let Some((what, pick)) = &p.pick
        && let Some(paths) = pick.poll()
    {
        let what = what.clone();
        p.pick = None;
        if !paths.is_empty() {
            match what {
                PickFor::RomFolder => {
                    p.add_folder = Some(AddFolder { path: paths[0].clone(), platform: String::new() });
                }
                PickFor::EmulatorFolder => {
                    let e = &mut st.config.emulation;
                    if !e.emulator_dirs.contains(&paths[0]) {
                        e.emulator_dirs.push(paths[0].clone());
                        e.dismissed_emulator_dirs.retain(|d| *d != paths[0]);
                        *ch = true;
                        *scan = true;
                    }
                }
                PickFor::BiosFolder => {
                    let e = &mut st.config.emulation;
                    if !e.bios_dirs.contains(&paths[0]) {
                        e.bios_dirs.push(paths[0].clone());
                        e.ignored_bios_dirs.retain(|d| *d != paths[0]);
                        *ch = true;
                    }
                }
                PickFor::BiosFiles(pf) => start_identify(st, p, pf, paths),
                PickFor::ProfileProgram(key) => {
                    if let Some(pe) = st.config.emulation.profiles.get_mut(&key) {
                        pe.program = paths[0].clone();
                        *ch = true;
                    }
                }
                t => {
                    *ch |= apply_program(st, p, cat.as_deref(), &t, Picked::File(paths[0].clone()));
                }
            }
        }
    }
    if let Some((pf, rx, n)) = &p.identify
        && let Ok(found) = rx.try_recv()
    {
        let (pf, n) = (pf.clone(), *n);
        p.identify = None;
        let (msg, err, dirs) = identify_message(&pf, &found, n);
        for dir in dirs {
            if !st.config.emulation.bios_dirs.contains(&dir) {
                st.config.emulation.bios_dirs.push(dir);
                *ch = true;
            }
        }
        p.bios_msg.insert(pf, (msg, err));
        p.recheck = true;
    }
    if let Some((pf, emu, rx, job)) = &p.install
        && let Ok(r) = rx.try_recv()
    {
        let name = cat.as_ref().map_or_else(|| emu.clone(), |c| c.name_by_id(emu));
        let msg = match (r, job) {
            (Ok(n), Job::Firmware) => (format!("Installed the firmware into {name} ({n} files)."), false),
            (Ok(0), Job::Setup) => ("Nothing was missing.".to_string(), false),
            (Ok(n), Job::Setup) => (format!("Done: {n} file{} put in place.", if n == 1 { "" } else { "s" }), false),
            (Err(e), Job::Setup) => (format!("Couldn't finish setting up: {e}"), true),
            (Err(e), Job::Firmware) => (format!("Couldn't install the firmware into {name}: {e}"), true),
            (Ok(0), _) => (format!("Nothing to copy to {name}."), false),
            (Ok(n), _) => (format!("Copied {n} file{} to {name}.", if n == 1 { "" } else { "s" }), false),
            (Err(e), _) => (format!("Couldn't copy to {name}: {e}"), true),
        };
        p.bios_msg.insert(pf.clone(), msg);
        p.install = None;
        p.recheck = true;
    }
    let _ = scan;
}

/// What "Choose BIOS files…" found, in plain words; plus the folders to remember.
fn identify_message(
    platform: &str,
    found: &[(PathBuf, &'static BiosFile)],
    picked: usize,
) -> (String, bool, Vec<PathBuf>) {
    if found.is_empty() {
        let what = if picked == 1 { "This file isn't" } else { "None of these files is" };
        return (format!("{what} a BIOS Lodestar knows (checked by content, not by name)."), true, vec![]);
    }
    let mut names: Vec<String> = found
        .iter()
        .map(|(path, b)| {
            let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            let console = catalog::short_name(b.platform);
            if file == b.file || b.file.is_empty() {
                format!("{file} ({console})")
            } else {
                format!("{file} = {} ({console})", b.file)
            }
        })
        .collect();
    names.dedup();
    let other = found.iter().filter(|(_, b)| b.platform != platform).count();
    let mut msg = format!("Recognised: {}.", catalog::join_limited(&names, 4));
    if found.len() < picked {
        msg.push_str(&format!(" {} not recognised.", picked - found.len()));
    }
    if other > 0 && other == found.len() {
        msg.push_str(&format!(" None of them is for {}.", catalog::console_name(platform)));
    }
    let mut dirs: Vec<PathBuf> = vec![];
    for (path, _) in found {
        // A file inside a zip: remember the zip's folder.
        let real = crate::emulation::zipread::real_path(path);
        if let Some(d) = real.parent().map(Path::to_path_buf)
            && !dirs.contains(&d)
        {
            dirs.push(d);
        }
    }
    (msg, false, dirs)
}

fn start_identify(st: &AppState, p: &mut EmuPage, platform: String, paths: Vec<PathBuf>) {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let ctx = st.ctx().clone();
    let n = paths.len();
    let spawned = std::thread::Builder::new().name("bios-identify".into()).spawn(move || {
        let _ = tx.send(crate::emulation::bios::identify(&paths));
        ctx.request_repaint();
    });
    if spawned.is_ok() {
        p.identify = Some((platform, rx, n));
    }
}

/// A program was picked (file or Flatpak) for `target`. Returns true when the config changed.
fn apply_program(st: &mut AppState, p: &mut EmuPage, cat: Option<&Catalog>, target: &PickFor, picked: Picked) -> bool {
    let cfg = &mut st.config.emulation;
    let found: Vec<String> = cat.map(|c| c.emus.iter().map(|e| e.emu.id.clone()).collect()).unwrap_or_default();
    match target {
        PickFor::Locate(platform) => {
            // The program is a built-in emulator that runs this console (Ryujinx for Switch):
            // added as that one, with its own name.
            let runs = |d: &&crate::emulation::defs::EmulatorDef| {
                d.profiles.iter().any(|pr| pr.platforms.contains(&platform.as_str()))
            };
            if let Some(def) = picked.identify().filter(runs) {
                let key = catalog::new_emulator_key(def.id, cfg, &found);
                let mut x = crate::config::EmulatorEntry {
                    name: def.name.to_string(),
                    follows: def.id.to_string(),
                    ..Default::default()
                };
                catalog::set_program(&mut x, &catalog::picked_kind(&picked));
                cfg.emulators.insert(key, x);
                return true;
            }
            // Something Lodestar doesn't know: the Add emulator form, so its name and what it
            // is based on are the user's to say (never another emulator's name).
            let suggested = catalog::suggested_def(platform);
            p.add_emu = Some(AddEmulator {
                follows: suggested.map(|d| d.id.to_string()).unwrap_or_default(),
                name: picked.name_guess(),
                program: Some(picked),
                platforms: if suggested.is_none() { vec![platform.clone()] } else { vec![] },
                args: "{rom}".into(),
                ..Default::default()
            });
            false
        }
        PickFor::ChangeProgram(id) => {
            let Some(e) = cat.and_then(|c| c.emu(id)) else { return false };
            let mut x = catalog::entry_for(&e.emu, cfg);
            catalog::set_program(&mut x, &catalog::picked_kind(&picked));
            cfg.emulators.insert(id.clone(), x);
            true
        }
        PickFor::AddEmulator => {
            if let Some(f) = &mut p.add_emu {
                // A custom one whose program Lodestar knows: based on that emulator.
                if f.follows.is_empty()
                    && f.platforms.is_empty()
                    && let Some(d) = picked.identify()
                {
                    f.follows = d.id.to_string();
                }
                if !f.name_typed || f.name.trim().is_empty() {
                    f.name = picked.name_guess();
                    f.name_typed = false;
                }
                f.program = Some(picked);
            }
            false
        }
        _ => false,
    }
}

// ------------------------------------------------------------------------- top

fn header(ui: &mut Ui, st: &mut AppState, p: &mut EmuPage, cat: Option<&Catalog>, pal: &Pal) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new("Emulation").size(15.0).strong().color(pal.text));
            let games: usize = p.facts.per_platform.values().sum();
            let line = match cat {
                _ if st.scan.running => "Scanning your ROM folders…".to_string(),
                None => "Looking for emulators…".to_string(),
                Some(c) => format!(
                    "{} · {} · {}",
                    plural(c.groups.len(), "emulator", "emulators"),
                    plural(p.facts.per_platform.len(), "console", "consoles"),
                    plural(games, "game", "games"),
                ),
            };
            ui.label(RichText::new(line).size(12.0).color(pal.dim));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let label = if st.scan.running { "Scanning…" } else { "Scan now" };
            if ui
                .add_enabled(!st.scan.running, egui::Button::new(label).min_size(vec2(104.0, 30.0)))
                .on_hover_text("Look for emulators, ROM folders and games again")
                .clicked()
            {
                st.refresh_emulation(true);
                p.recheck = true;
                p.recheck_scan = true;
            }
            if st.scan.running {
                lazy_spinner(ui, 16.0, pal.dim);
            }
        });
    });
    // The last scan's problem with ROM folders (a drive that isn't connected …).
    if let Some(e) = st.scan.last.as_ref().and_then(|l| {
        l.per_source.iter().find(|(s, _, _)| *s == crate::model::Source::Emulator).and_then(|(_, _, e)| e.clone())
    }) {
        ui.horizontal(|ui| {
            paint_icon(
                ui.painter(),
                egui::Rect::from_min_size(ui.cursor().min + vec2(0.0, 1.0), vec2(14.0, 14.0)),
                Icon::Warning,
                pal.danger,
            );
            ui.add_space(20.0);
            ui.label(RichText::new(e).size(12.0).color(pal.danger));
        });
    }
}

/// The three global switches. Returns (config changed, scan settings changed).
fn switches(ui: &mut Ui, st: &mut AppState, pal: &Pal) -> (bool, bool) {
    let mut ch = false;
    let mut scan = false;
    let e = &mut st.config.emulation;
    card(ui, pal, |ui| {
        ch |= switch_row(
            ui,
            "Start games fullscreen",
            "Play goes straight into the game. Play ▾ → Play in a window for the other way.",
            &mut e.fullscreen,
            pal,
        );
        row_separator(ui, pal);
        let a = switch_row(
            ui,
            "Find emulators automatically",
            "Programs, Flatpaks, AppImages in your home folder and your emulator folders",
            &mut e.auto_emulators,
            pal,
        );
        row_separator(ui, pal);
        let b = switch_row(
            ui,
            "Find ROM folders automatically",
            "~/ROMs, ~/Emulation/roms, ~/RetroPie/roms, connected drives, the emulators' own game folders",
            &mut e.auto_folders,
            pal,
        );
        scan |= a || b;
    });
    (ch, scan)
}

fn switch_row(ui: &mut Ui, title: &str, sub: &str, on: &mut bool, pal: &Pal) -> bool {
    let mut changed = false;
    row(ui, pal, title, &[(sub.to_string(), pal.dim)], |ui| {
        changed = toggle_switch(ui, on, 20.0, pal.accent).changed();
    });
    changed
}

// ------------------------------------------------------------------------- consoles

/// A console with games and no emulator: Get <emulator> on Flathub, Locate… (a program or a
/// Flatpak); its BIOS panel opens below when asked for.
fn needs_row(ui: &mut Ui, st: &mut AppState, p: &mut EmuPage, cat: Option<&Catalog>, pf: &str, pal: &Pal) -> bool {
    let mut ch = false;
    let n = p.facts.per_platform.get(pf).copied().unwrap_or(0);
    let sub =
        vec![(plural(n, "game", "games"), pal.dim), (" · No emulator found".to_string(), tone(ChipTone::Warn, pal))];
    let mut pick_locate = false;
    let mut flatpak_locate = false;
    let short = crate::emulation::platforms::by_id(pf)
        .map_or_else(|| catalog::console_name(pf), |p| crate::emulation::console_label(p).to_string());
    row_short(ui, pal, &catalog::console_name(pf), &short, &sub, |ui| {
        ui.menu_button("Locate…", |ui| {
            if ui.button("A program or AppImage…").clicked() {
                pick_locate = true;
                ui.close();
            }
            if ui.button("An installed Flatpak…").clicked() {
                flatpak_locate = true;
                ui.close();
            }
        })
        .response
        .on_hover_text("Lodestar didn't find an emulator for it: show it where yours is");
        if let Some(def) = catalog::suggested_def(pf)
            && let Some(id) = def.flathub
            && ui.button(format!("Get {} on Flathub", def.name)).on_hover_text(catalog::flathub_url(id)).clicked()
        {
            st.open_url(&catalog::flathub_url(id));
        }
    });
    if pick_locate {
        p.pick = Some((
            PickFor::Locate(pf.to_string()),
            Pick::start(ui.ctx(), PickKind::File, "Choose the emulator program or AppImage", None),
        ));
    }
    if flatpak_locate {
        open_flatpak_chooser(st, p, FlatpakFor::Locate(pf.to_string()));
    }
    // Its BIOS (found already, or still missing) while the emulator isn't there yet.
    let chip = p.bios.data.as_ref().and_then(|d| d.by_platform.get(pf)).and_then(|i| catalog::bios_chip_for(&i.status));
    if let Some((text, t)) = chip {
        let open = p.bios_open.as_deref() == Some(pf);
        let mut toggle = false;
        egui::Frame::new().inner_margin(egui::Margin { left: 14, right: 14, top: 0, bottom: 8 }).show(ui, |ui| {
            toggle = chip_button(ui, &text, t, open, pal).clicked();
        });
        if toggle {
            p.bios_open = if open { None } else { Some(pf.to_string()) };
        }
    }
    if p.bios_open.as_deref() == Some(pf)
        && let Some(c) = cat
    {
        indented(ui, |ui| ch |= bios_ui::bios_panel(ui, st, p, c, pf, pal));
    }
    ch
}

/// A console several emulators (or profiles) can run: which one its games start with.
fn choice_row(ui: &mut Ui, st: &mut AppState, p: &mut EmuPage, c: &Catalog, pf: &str, pal: &Pal) -> bool {
    let mut ch = false;
    let n = p.facts.per_platform.get(pf).copied().unwrap_or(0);
    let narrow = ui.available_width() < NARROW_PAGE;
    let short = crate::emulation::platforms::by_id(pf)
        .map_or_else(|| catalog::console_name(pf), |p| crate::emulation::console_label(p).to_string());
    row_short(ui, pal, &catalog::console_name(pf), &short, &[(plural(n, "game", "games"), pal.dim)], |ui| {
        ch |= choice_combo(ui, st, c, pf, narrow);
    });
    ch
}

/// The console's emulator: "Eden (automatic)" (what Lodestar picks by itself) or one of the
/// others. Picking the automatic one = automatic (the console follows the best one found).
fn choice_combo(ui: &mut Ui, st: &mut AppState, c: &Catalog, pf: &str, narrow: bool) -> bool {
    let label = |(e, pr): &(String, String)| c.choice_label(e, pr, pf).unwrap_or_else(|| format!("{e} (not found)"));
    let auto = c.auto.get(pf).cloned();
    let cur = st.config.emulation.platform_emulator.get(pf).map(|v| {
        let (e, pr) = catalog::split_choice(v);
        (e.to_string(), pr.to_string())
    });
    // A saved choice that is the automatic one counts as automatic.
    let cur =
        cur.filter(|x| Some(x) != auto.as_ref() && !(x.1.is_empty() && auto.as_ref().is_some_and(|a| a.0 == x.0)));
    let selected = match (&cur, &auto) {
        (Some(x), _) => label(x),
        (None, Some(a)) => Catalog::automatic_label(&label(a)),
        (None, None) => "Automatic".to_string(),
    };
    let mut pick: Option<Option<String>> = None;
    let (shown, width) = if narrow { (Catalog::short_label(&selected), 190.0) } else { (selected.clone(), 240.0) };
    let full = (shown != selected).then_some(selected);
    let r = egui::ComboBox::from_id_salt(("ls_emu_console", pf))
        .width(width)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            if let Some(a) = &auto
                && ui
                    .selectable_label(cur.is_none(), Catalog::automatic_label(&label(a)))
                    .on_hover_text("The best one found; it follows what you install")
                    .clicked()
            {
                pick = Some(None);
            }
            for x in c.choices(pf).into_iter().filter(|x| Some(x) != auto.as_ref()) {
                if ui.selectable_label(cur.as_ref() == Some(&x), label(&x)).clicked() {
                    pick = Some(Some(catalog::choice_value(&x.0, &x.1)));
                }
            }
        })
        .response;
    let tip = "Games of this console start with this emulator, unless a game has its own (Edit… → Installation)";
    match full {
        Some(f) => r.on_hover_text(format!("{f}\n\n{tip}")),
        None => r.on_hover_text(tip),
    };
    let saved = st.config.emulation.platform_emulator.get(pf).cloned();
    match pick {
        Some(None) if saved.is_some() => {
            st.config.emulation.platform_emulator.remove(pf);
            true
        }
        Some(Some(v)) if saved.as_deref() != Some(v.as_str()) => {
            st.config.emulation.platform_emulator.insert(pf.to_string(), v);
            true
        }
        _ => false,
    }
}

/// A console's options: the switches of the emulator its games start with (Default = the
/// emulator's own setting) and, under "More options", the rest of them and extra arguments for
/// every game of the console.
pub(super) fn console_options(
    ui: &mut Ui,
    st: &mut AppState,
    p: &mut EmuPage,
    c: &Catalog,
    pf: &str,
    pal: &Pal,
) -> bool {
    let mut ch = false;
    let pick = st.config.emulation.platform_emulator.get(pf).map(|v| {
        let (e, pr) = catalog::split_choice(v);
        (e.to_string(), pr.to_string())
    });
    let (eid, pid) = match pick.or_else(|| c.auto.get(pf).cloned()) {
        Some(x) => x,
        None => return false,
    };
    let Some(ce) = c.emu(&eid) else { return false };
    let Some(prof) = ce.profiles.iter().find(|x| x.id == pid).or_else(|| switches::profile_for(&ce.profiles, Some(pf)))
    else {
        return false;
    };
    let cfg = &st.config.emulation;
    let sws = switches::switches_of(&ce.emu, prof, cfg);
    let below = switches::levels_below(cfg, None, &ce.emu.id);
    let inherit = |s: &crate::emulation::options::Switch| switches::inherited_from(s, &below);
    ui.add_space(4.0);
    ui.label(
        RichText::new(format!("{} games with {}", catalog::console_name(pf), c.name_of(&ce.emu)))
            .size(13.0)
            .strong()
            .color(pal.text),
    );
    hint_text(ui, &format!("Default follows {}'s start options (above).", c.name_of(&ce.emu)), pal);
    let mut set = st.config.emulation.console_settings.get(pf).cloned().unwrap_or_default();
    let before = set.clone();
    switches::tri_rows(ui, ("console", pf), &sws, &inherit, &mut set.options, &|s| s.key, pal);
    let more_id = egui::Id::NULL.with(("ls_emu_console_more", pf));
    let debug_open = std::mem::take(&mut p.more_open);
    if debug_open {
        switches::set_more_open(ui.ctx(), more_id, true);
    }
    if switches::more_options(ui, more_id, pal) {
        switches::tri_rows(ui, ("console", pf), &sws, &inherit, &mut set.options, &|s| !s.key, pal);
        ui.label(RichText::new("Extra arguments for every game").size(13.0).color(pal.text));
        ch |= switches::buffered_args(ui, ("console", pf), &mut set.extra_args, "extra arguments", pal);
        switches::docs_link(ui, st, &ce.emu, pal);
        if debug_open {
            ui.scroll_to_cursor(Some(egui::Align::Center));
        }
    }
    ui.add_space(4.0);
    if set != before {
        if set.is_empty() {
            st.config.emulation.console_settings.remove(pf);
        } else {
            st.config.emulation.console_settings.insert(pf.to_string(), set);
        }
        ch = true;
    }
    ch
}

// ------------------------------------------------------------------------- Flatpak chooser

fn open_flatpak_chooser(st: &AppState, p: &mut EmuPage, target: FlatpakFor) {
    if p.flatpaks.apps.is_none() && p.flatpaks.rx.is_none() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let mut sctx = crate::scan::ScanContext::from_config(&st.config);
        sctx.home = st.paths.home.clone();
        let ctx = st.ctx().clone();
        let spawned = std::thread::Builder::new().name("flatpak-list".into()).spawn(move || {
            let apps = crate::addgame::flatpak_apps(&sctx).into_iter().map(|a| (a.id, a.info.name)).collect();
            let _ = tx.send(apps);
            ctx.request_repaint();
        });
        if spawned.is_ok() {
            p.flatpaks.rx = Some(rx);
        }
    }
    p.choose_flatpak = Some((target, String::new()));
}

enum Chooser {
    Keep,
    Cancel,
    Picked(String, String),
}

/// The installed Flatpaks, with a filter; click one to use it.
fn flatpak_chooser(ctx: &egui::Context, st: &AppState, p: &mut EmuPage, filter: &mut String, pal: &Pal) -> Chooser {
    if let Some(rx) = &p.flatpaks.rx
        && let Ok(v) = rx.try_recv()
    {
        p.flatpaks.apps = Some(v);
        p.flatpaks.rx = None;
    }
    let _ = st;
    let mut out = Chooser::Keep;
    let (_, close) = modal(ctx, "ls_emu_flatpaks", pal, 460.0, |ui| {
        ui.label(RichText::new("Choose a Flatpak").size(17.0).strong().color(pal.text));
        hint_text(ui, "Installed Flatpak apps. Pick the emulator.", pal);
        ui.add_space(6.0);
        ui.add(egui::TextEdit::singleline(filter).hint_text("Filter").desired_width(f32::INFINITY));
        ui.add_space(4.0);
        egui::Frame::new()
            .fill(mix(pal.bg, pal.surface, 0.35))
            .stroke(egui::Stroke::new(1.0, with_alpha(pal.border, 150)))
            .corner_radius(6)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .min_scrolled_height(300.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| match &p.flatpaks.apps {
                        None => {
                            ui.horizontal(|ui| {
                                lazy_spinner(ui, 14.0, pal.dim);
                                ui.label(RichText::new("Reading the installed apps…").size(12.5).color(pal.dim));
                            });
                        }
                        Some(apps) => {
                            let f = filter.trim().to_lowercase();
                            let mut any = false;
                            for (id, name) in apps.iter().filter(|(id, name)| {
                                f.is_empty() || id.to_lowercase().contains(&f) || name.to_lowercase().contains(&f)
                            }) {
                                any = true;
                                let text = RichText::new(format!("{name}   ")).size(13.5).color(pal.text);
                                let r = ui.add(
                                    egui::Button::selectable(false, text)
                                        .right_text(RichText::new(id).size(11.5).color(pal.dim))
                                        .min_size(vec2(ui.available_width(), 28.0)),
                                );
                                if r.clicked() {
                                    out = Chooser::Picked(id.clone(), name.clone());
                                }
                            }
                            if !any {
                                ui.label(RichText::new("No installed Flatpak matches.").size(12.5).color(pal.dim));
                            }
                        }
                    });
            });
        ui.add_space(8.0);
        if plain_button(ui, "Cancel").clicked() {
            out = Chooser::Cancel;
        }
    });
    if close && matches!(out, Chooser::Keep) {
        return Chooser::Cancel;
    }
    out
}

// ------------------------------------------------------------------------- running emulators

/// Start an emulator on its own (its settings, controllers, firmware): a plain detached program,
/// no playtime, nothing tracked.
pub(crate) fn open_emulator(st: &mut AppState, emu: &Emulator) {
    let spec = crate::emulation::registry::open_spec(emu);
    run_detached(st, spec, &emu.name, &format!("Opening {}", emu.name));
}

/// Start a program on its own (an emulator's window, RPCS3's firmware installer): detached, no
/// playtime, nothing tracked. Only on the user's click; screenshot runs skip it.
pub(crate) fn run_detached(st: &mut AppState, spec: crate::model::LaunchSpec, name: &str, done: &str) {
    if st.no_launch {
        log::warn!("screenshot run: not starting {name}");
        st.notify(format!("Skipped (screenshot run): {name}"));
        return;
    }
    let crate::model::LaunchSpec::Command { program, args, cwd, env } = spec else {
        st.notify_error(format!("Couldn't open {name}"));
        return;
    };
    let name = name.to_string();
    let done = done.to_string();
    let spawned = std::thread::Builder::new().name("emulator-open".into()).spawn(move || {
        use std::os::unix::process::CommandExt as _;
        let mut cmd = std::process::Command::new(&program);
        // Like a game launch: none of Lodestar's own variables, none of its descriptors (the
        // controllers' evdev handles would stay open for as long as the emulator runs).
        for (k, _) in std::env::vars_os() {
            if k.to_str().is_some_and(crate::launch::strip_env_var) {
                cmd.env_remove(&k);
            }
        }
        cmd.args(&args)
            .envs(env)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0);
        // SAFETY: only async-signal-safe syscalls between fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                crate::launch::mark_inherited_fds_cloexec();
                Ok(())
            });
        }
        if let Some(d) = cwd.filter(|d| d.is_dir()) {
            cmd.current_dir(d);
        }
        match cmd.spawn() {
            Ok(mut child) => {
                let _ = child.wait();
            }
            Err(e) => log::warn!("open emulator {program}: {e}"),
        }
    });
    match spawned {
        Ok(_) => st.notify(done),
        Err(e) => st.notify_error(format!("Couldn't open {name}: {e}")),
    }
}

/// Play ▾ (and the right-click menu) for an emulated game: "Play in a window", "Play with ▸"
/// (another emulator / profile of its console, once or from now on) and "<Emulator> settings…"
/// (the emulator itself, no game). Draws nothing for other games. True when anything was drawn.
pub(super) fn play_menu_entries(
    ui: &mut Ui,
    g: &crate::model::Game,
    enabled: bool,
    acts: &mut Vec<super::Action>,
) -> bool {
    let Some(e) = &g.emu else { return false };
    let cat = catalog::catalog(ui.ctx());
    let emu = cat
        .as_ref()
        .and_then(|c| crate::emu_app::play_emulator(&c.reg, g).or_else(|| c.game_emulator(g).map(|(e, _)| e)));
    let windowed = emu.is_some() && crate::emulation::registry::spec_windowed(g, e).is_some();
    let reason =
        || crate::emulation::registry::missing_reason(g).unwrap_or_else(|| "No emulator found for it yet.".into());
    let r = ui.add_enabled(enabled && windowed, egui::Button::new("Play in a window"));
    let r = if windowed {
        r.on_hover_text("Start it in a window instead of fullscreen")
    } else {
        r.on_disabled_hover_text(reason())
    };
    if r.clicked() {
        acts.push(super::Action::PlayWindowed(g.id));
    }
    let with = cat.as_ref().map(|c| crate::emu_app::choices(&c.reg, g)).unwrap_or_default();
    if !with.is_empty() {
        let always_id = egui::Id::NULL.with(("ls_play_with_always", g.id));
        ui.add_enabled_ui(enabled, |ui| {
            ui.menu_button("Play with", |ui| {
                ui.set_min_width(260.0);
                let mut always = ui.data(|d| d.get_temp::<bool>(always_id)).unwrap_or(false);
                for c in &with {
                    let text = if c.current { format!("✓  {}", c.label) } else { format!("     {}", c.label) };
                    let tip = if c.current { "What Play uses now" } else { "Play with this one" };
                    if ui.button(text).on_hover_text(tip).clicked() {
                        acts.push(super::Action::PlayWith(g.id, c.emulator.clone(), c.profile.clone(), always));
                        ui.data_mut(|d| d.remove::<bool>(always_id));
                        ui.close();
                    }
                }
                ui.separator();
                if ui
                    .checkbox(&mut always, "Always use this for this game")
                    .on_hover_text("The one you pick becomes this game's emulator (Edit → Installation)")
                    .changed()
                {
                    ui.data_mut(|d| d.insert_temp(always_id, always));
                }
            });
        });
    }
    let label = match &emu {
        Some(x) => settings_label(&cat.as_ref().map_or_else(|| x.name.clone(), |c| c.name_of(x))),
        None => "Emulator settings…".to_string(),
    };
    let r = ui.add_enabled(emu.is_some(), egui::Button::new(label));
    let r = match &emu {
        Some(x) => r.on_hover_text(settings_tip(&x.name)),
        None => r.on_disabled_hover_text(reason()),
    };
    if r.clicked() {
        acts.push(super::Action::OpenEmulator(g.id));
    }
    true
}

/// "PCSX2 settings…": opens the emulator itself, without a game.
pub(crate) fn settings_label(name: &str) -> String {
    format!("{name} settings…")
}

/// The tooltip of every "<Emulator> settings…".
pub(crate) fn settings_tip(name: &str) -> String {
    format!(
        "Opens {name} itself, without a game, to change its controls, graphics and other settings. To play, use Play."
    )
}

/// Play for an emulated game whose chosen emulator isn't allowed to run yet (a downloaded
/// AppImage without the executable bit): ask instead of starting. True when it asked (the
/// caller doesn't launch).
pub(super) fn play_needs_allow(ctx: &egui::Context, st: &AppState, id: crate::model::GameId) -> bool {
    let Some(b) = st.library.get(id).and_then(crate::emulation::registry::blocked_choice_now) else { return false };
    ctx.data_mut(|d| d.insert_temp(allow_prompt_id(), Some((id, b))));
    true
}

fn allow_prompt_id() -> egui::Id {
    egui::Id::unique("ls_emu_allow_prompt")
}

/// The "allow it to run?" question [`play_needs_allow`] asked: Allow it and play / Cancel.
/// Drawn every frame by the desktop (nothing while no question is open).
pub(super) fn allow_prompt(ctx: &egui::Context, st: &mut AppState, pal: &Pal) {
    let Some((id, emu)) =
        ctx.data(|d| d.get_temp::<Option<(crate::model::GameId, Emulator)>>(allow_prompt_id())).flatten()
    else {
        return;
    };
    let game = crate::ui::game_name(st, id);
    let path = emu.program_path().map(|p| short_path(p, &st.paths.home, 60)).unwrap_or_default();
    let mut answer: Option<bool> = None;
    let (_, close) = super::dialogs::modal(ctx, "ls_emu_allow_prompt", pal, 420.0, |ui| {
        let what = if matches!(emu.kind, crate::emulation::detect::InstallKind::AppImage { .. }) {
            "AppImage"
        } else {
            "program"
        };
        ui.label(RichText::new(format!("Allow {} to run?", emu.name)).size(17.0).strong().color(pal.text));
        ui.add_space(6.0);
        for line in [
            format!("{game} starts with {}, and this {what} isn't marked as a program yet.", emu.label_in_text()),
            "Lodestar can mark it (like chmod u+x) and start the game.".to_string(),
        ] {
            ui.add(egui::Label::new(RichText::new(line).size(13.0).color(pal.text)).wrap());
        }
        if !path.is_empty() {
            hint_text(ui, &path, pal);
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if primary_button(ui, "Allow and play", pal, true).clicked() {
                answer = Some(true);
            }
            if plain_button(ui, "Cancel").clicked() {
                answer = Some(false);
            }
        });
    });
    if close && answer.is_none() {
        answer = Some(false);
    }
    let Some(go) = answer else { return };
    ctx.data_mut(|d| d.remove_temp::<Option<(crate::model::GameId, Emulator)>>(allow_prompt_id()));
    if !go {
        return;
    }
    if st.no_launch {
        st.notify(format!("Skipped (screenshot run): allow {} to run", emu.name));
        return;
    }
    let Some(path) = emu.program_path().map(Path::to_path_buf) else { return };
    match crate::emulation::detect::allow_to_run(&path) {
        Ok(()) => {
            crate::emulation::registry::mark_allowed(&emu.id);
            st.refresh_emulation(true);
            // Several ways to play: "Choose what to play" opens, as for a plain Play.
            choose::play(ctx, st, id);
        }
        Err(err) => st.notify_error(format!("Couldn't allow {} to run: {err}", emu.name)),
    }
}

/// Action::OpenEmulator: the emulator this game would start with.
pub(super) fn open_game_emulator(st: &mut AppState, id: crate::model::GameId) {
    let Some(g) = st.library.get(id).cloned() else { return };
    let emu = catalog::catalog(st.ctx())
        .and_then(|c| crate::emu_app::play_emulator(&c.reg, &g).or_else(|| c.game_emulator(&g).map(|(e, _)| e)));
    match emu {
        Some(e) => open_emulator(st, &e),
        None => {
            let why = crate::emulation::registry::missing_reason(&g).unwrap_or_else(|| "No emulator found".into());
            st.notify_error(why);
        }
    }
}

// ------------------------------------------------------------------------- widgets

pub(super) fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// Green / red / amber / dim, toned for a dark or light background (like the Deck ratings).
pub(crate) fn tone(t: ChipTone, pal: &Pal) -> Color32 {
    let light = luminance(pal.bg) > 0.5;
    match t {
        ChipTone::Good if light => Color32::from_rgb(46, 128, 52),
        ChipTone::Good => mix(Color32::from_rgb(110, 205, 95), pal.text, 0.2),
        ChipTone::Warn if light => Color32::from_rgb(160, 108, 0),
        ChipTone::Warn => mix(Color32::from_rgb(235, 190, 70), pal.text, 0.2),
        ChipTone::Bad => pal.danger,
        ChipTone::Dim => pal.dim,
    }
}

/// A small rounded status chip (icon + text) that can be clicked.
fn chip_button(ui: &mut Ui, text: &str, t: ChipTone, open: bool, pal: &Pal) -> egui::Response {
    let c = tone(t, pal);
    let font = egui::FontId::proportional(12.0);
    let gal = ui.painter().layout_no_wrap(text.to_string(), font, c);
    let icon = match t {
        ChipTone::Good => Some(Icon::Check),
        ChipTone::Bad => Some(Icon::Close),
        ChipTone::Warn => Some(Icon::Warning),
        ChipTone::Dim => None,
    };
    let w = gal.size().x + 20.0 + if icon.is_some() { 16.0 } else { 0.0 } + 12.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
    let p = ui.painter();
    let fill = with_alpha(c, if resp.hovered() || open { 46 } else { 26 });
    p.rect_filled(rect, 12.0, fill);
    let mut x = rect.left() + 10.0;
    if let Some(i) = icon {
        paint_icon(p, egui::Rect::from_center_size(pos2(x + 6.0, rect.center().y), vec2(12.0, 12.0)), i, c);
        x += 16.0;
    }
    p.galley(pos2(x, rect.center().y - gal.size().y / 2.0), gal, c);
    paint_icon(
        p,
        egui::Rect::from_center_size(pos2(rect.right() - 12.0, rect.center().y), vec2(10.0, 10.0)),
        if open { Icon::ChevronDown } else { Icon::ChevronRight },
        c,
    );
    resp.on_hover_text("BIOS details")
}

pub(super) fn group_title(ui: &mut Ui, text: &str, pal: &Pal) {
    ui.add_space(14.0);
    ui.label(RichText::new(text).size(11.0).strong().color(pal.dim).extra_letter_spacing(0.8));
}

/// [`group_title`] that the page scrolls to once when the screenshot hook asks (`at:<what>`).
pub(super) fn group_title_at(ui: &mut Ui, p: &mut EmuPage, what: &str, text: &str, pal: &Pal) {
    let here = p.scroll_to.as_deref() == Some(what);
    ui.add_space(14.0);
    let r = ui.label(RichText::new(text).size(11.0).strong().color(pal.dim).extra_letter_spacing(0.8));
    if here {
        // Every frame while the hook is on: the sections above fill in as results arrive.
        r.scroll_to_me(Some(egui::Align::TOP));
    }
}

/// The rounded card the rows sit in (Libraries' look).
pub(super) fn card(ui: &mut Ui, pal: &Pal, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(mix(pal.surface, pal.surface_alt, 0.55))
        .stroke(egui::Stroke::new(1.0, with_alpha(pal.border, 160)))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(0, 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui);
        });
}

pub(super) fn row_separator(ui: &mut Ui, pal: &Pal) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        (r.left() + 14.0)..=(r.right() - 14.0),
        r.center().y,
        egui::Stroke::new(1.0, with_alpha(pal.border, 200)),
    );
}

/// One row: title and a sub-line (coloured parts) on the left, `right` laid out from the right
/// edge (first added = rightmost). The text is cut to the room the right part leaves.
pub(super) fn row(ui: &mut Ui, pal: &Pal, title: &str, sub: &[(String, Color32)], right: impl FnOnce(&mut Ui)) {
    row_inner(ui, pal, title, None, None, sub, right);
}

/// [`row`] whose title has a shorter form for when the full one doesn't fit ("Sony
/// PlayStation 2" → "PlayStation 2"; the full name is then the hover text).
pub(super) fn row_short(
    ui: &mut Ui,
    pal: &Pal,
    title: &str,
    short: &str,
    sub: &[(String, Color32)],
    right: impl FnOnce(&mut Ui),
) {
    row_inner(ui, pal, title, Some(short), None, sub, right);
}

/// [`row`] with a small tag after the title (an emulator's kind: "Flatpak", "AppImage").
pub(super) fn row_tagged(
    ui: &mut Ui,
    pal: &Pal,
    title: &str,
    tag: Option<&str>,
    sub: &[(String, Color32)],
    right: impl FnOnce(&mut Ui),
) {
    row_inner(ui, pal, title, None, tag, sub, right);
}

fn row_inner(
    ui: &mut Ui,
    pal: &Pal,
    title: &str,
    short: Option<&str>,
    tag: Option<&str>,
    sub: &[(String, Color32)],
    right: impl FnOnce(&mut Ui),
) {
    let w = ui.available_width();
    // Narrow (a small Settings window): when the title wouldn't fit beside the controls even in
    // its short form, the row wraps — title and sub-line on their own line, the controls below.
    // Decided from the controls' width the frame before (a changed decision redraws at once).
    let font = egui::FontId::proportional(14.0);
    let tag_w = tag.map_or(0.0, |t| {
        ui.painter().layout_no_wrap(t.to_string(), egui::FontId::proportional(11.0), pal.dim).size().x + 16.0
    });
    let title_need =
        ui.painter().layout_no_wrap(short.unwrap_or(title).to_string(), font.clone(), pal.text).size().x + tag_w;
    // A sub-line that warns ("No emulator found") is never cut either, nor an emulator's (where
    // it is: short, and what tells two rows apart).
    let warn_need = if tag.is_some() || sub.iter().any(|(_, c)| *c != pal.dim) {
        let text: String = sub.iter().map(|(t, _)| t.as_str()).collect();
        ui.painter().layout_no_wrap(text, egui::FontId::proportional(11.5), pal.dim).size().x
    } else {
        0.0
    };
    let title_need = title_need.max(warn_need);
    let wraps = |right_w: f32| row_wraps(w, right_w, title_need);
    let key = ui.scope_id().with(("ls_emu_row_right", title));
    let prev = ui.data(|d| d.get_temp::<f32>(key));
    let wrapped = prev.is_some_and(wraps);
    let (rect, _) = ui.allocate_exact_size(vec2(w, if wrapped { 46.0 + ROW_WRAP_EXTRA } else { 46.0 }), Sense::hover());
    let controls =
        if wrapped { egui::Rect::from_min_max(pos2(rect.left(), rect.bottom() - 40.0), rect.max) } else { rect };
    let text_rect =
        if wrapped { egui::Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - 34.0)) } else { rect };
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(controls.shrink2(vec2(14.0, 0.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    child.spacing_mut().item_spacing.x = 6.0;
    right(&mut child);
    let used = child.min_rect().width();
    if prev != Some(used) {
        ui.data_mut(|d| d.insert_temp(key, used));
    }
    if wrapped != wraps(used) {
        ui.ctx().request_discard("settings row wraps");
    }
    let rect = text_rect;
    let text_x = rect.left() + 14.0;
    let max_w = if wrapped { rect.width() - 28.0 } else { (rect.width() - 28.0 - used - 16.0).max(40.0) };
    let p = ui.painter();
    let tag_g = tag.map(|t| p.layout_no_wrap(t.to_string(), egui::FontId::proportional(11.0), pal.dim));
    let title_w = (max_w - tag_w).max(40.0);
    let mut ng = crate::ui::widgets::truncated(p, title, font.clone(), pal.text, title_w);
    let mut shortened = false;
    if let Some(s) = short.filter(|s| ng.elided && *s != title) {
        ng = crate::ui::widgets::truncated(p, s, font, pal.text, title_w);
        shortened = true;
    }
    let mut job = egui::text::LayoutJob::default();
    for (t, c) in sub {
        job.append(t, 0.0, egui::TextFormat::simple(egui::FontId::proportional(11.5), *c));
    }
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
    let sg = p.layout_job(job);
    let has_sub = !sub.is_empty() && sub.iter().any(|(t, _)| !t.is_empty());
    let total = ng.size().y + if has_sub { 1.0 + sg.size().y } else { 0.0 };
    let y0 = rect.center().y - total / 2.0;
    let full = title.to_string();
    let title_rect = egui::Rect::from_min_size(pos2(text_x, y0), ng.size());
    let cut = ng.elided || shortened;
    p.galley(pos2(text_x, y0), ng.clone(), pal.text);
    if let Some(g) = tag_g {
        let r = egui::Rect::from_min_size(
            pos2(text_x + ng.size().x + 8.0, y0 + (ng.size().y - g.size().y - 4.0) / 2.0),
            vec2(g.size().x + 10.0, g.size().y + 4.0),
        );
        p.rect_filled(r, 4.0, with_alpha(pal.dim, 34));
        p.galley(pos2(r.left() + 5.0, r.top() + 2.0), g, pal.dim);
    }
    if has_sub {
        p.galley(pos2(text_x, y0 + ng.size().y + 1.0), sg, pal.dim);
    }
    if cut {
        ui.interact(title_rect, ui.scope_id().with(("ls_emu_row_tip", &full)), Sense::hover()).on_hover_text(full);
    }
}

/// Narrower than this, the Emulation page shortens its emulator pickers.
const NARROW_PAGE: f32 = 560.0;

/// Height a wrapped row adds (the controls' own line).
const ROW_WRAP_EXTRA: f32 = 34.0;

/// A settings row `w` wide wraps when its title (`title_w`, the short form if it has one) can't
/// sit beside controls `right_w` wide.
pub(super) fn row_wraps(w: f32, right_w: f32, title_w: f32) -> bool {
    title_w > w - 28.0 - right_w - 16.0
}

/// A row that only says something (empty states), with an optional spinner.
pub(super) fn empty_row(ui: &mut Ui, text: &str, spinner: bool, pal: &Pal) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        if spinner {
            lazy_spinner(ui, 14.0, pal.dim);
        }
        ui.add(egui::Label::new(RichText::new(text).size(12.5).color(pal.dim)).wrap());
    });
    ui.add_space(8.0);
}

/// The expanded part under a row.
pub(super) fn indented(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new().inner_margin(egui::Margin { left: 26, right: 14, top: 0, bottom: 10 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 4.0;
        add(ui);
    });
}

/// Save a ROM folder (Add game → Emulated games… → Import): a new entry, or the existing one
/// for that path switched on, with this console and these exclusions. Returns its key.
pub(super) fn save_rom_folder(
    cfg: &mut crate::config::EmulationConfig,
    path: &Path,
    platform: &str,
    exclude: Vec<String>,
) -> String {
    let key = folders::add_folder(cfg, path, platform);
    if let Some(rf) = cfg.rom_folders.get_mut(&key) {
        rf.exclude = exclude;
    }
    key
}
