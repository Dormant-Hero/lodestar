//! Settings → Emulation: the emulator folders, the ROM folders (with their exclusions) and
//! the BIOS folders.

use super::catalog::{self, Catalog};
use super::lists::*;
use super::*;
use crate::config::RomFolder;
use crate::emulation::platforms;
use crate::ui::widgets::icon_button;

// ------------------------------------------------------------------------- emulator folders

/// Emulators found in (the program inside the folder or one of its sub-folders).
fn emulators_in(c: &Catalog, dir: &Path) -> usize {
    c.groups
        .iter()
        .filter(|g| {
            std::iter::once(g.main)
                .chain(g.copies.iter().copied())
                .any(|i| c.emus[i].emu.program_path().is_some_and(|x| x.starts_with(dir)))
        })
        .count()
}

/// Emulator folders: portable emulators, one sub-folder each. Returns (config changed, scan
/// settings changed).
pub(super) fn emulator_folders(
    ui: &mut Ui,
    st: &mut AppState,
    p: &mut EmuPage,
    cat: Option<&Catalog>,
    pal: &Pal,
) -> (bool, bool) {
    let mut ch = false;
    group_title_at(ui, p, "folders", "EMULATOR FOLDERS", pal);
    hint_text(
        ui,
        "Folders with portable emulators, one sub-folder each (…/Emulators/DuckStation). Searched when Find emulators \
         automatically is on, like ROM folders.",
        pal,
    );
    ui.add_space(4.0);
    let home = st.paths.home.clone();
    let dirs = st.config.emulation.emulator_dirs.clone();
    let auto = st.config.emulation.auto_emulators;
    let mut remove = None;
    let mut open = None;
    card(ui, pal, |ui| {
        if dirs.is_empty() {
            empty_row(
                ui,
                "No emulator folders. Installed programs, Flatpaks, menu entries and AppImages in your home folder are \
                 found without one.",
                false,
                pal,
            );
        }
        for (i, d) in dirs.iter().enumerate() {
            if i > 0 {
                row_separator(ui, pal);
            }
            let sub = if !auto {
                ("Not searched while Find emulators automatically is off".to_string(), pal.dim)
            } else {
                match cat.map(|c| emulators_in(c, d)) {
                    None => ("Looking…".to_string(), pal.dim),
                    Some(0) => ("No emulators found in it".to_string(), tone(ChipTone::Warn, pal)),
                    Some(n) => (plural(n, "emulator", "emulators"), pal.dim),
                }
            };
            row(ui, pal, &short_path(d, &home, 70), &[sub], |ui| {
                let more = icon_button(ui, Icon::More, false, 26.0, "More");
                egui::Popup::menu(&more).show(|ui| {
                    ui.set_min_width(170.0);
                    if ui.button("Open folder").clicked() {
                        open = Some(d.clone());
                    }
                    if ui
                        .button("Remove")
                        .on_hover_text("Its emulators aren't used any more (they stay on the disk)")
                        .clicked()
                    {
                        remove = Some(i);
                    }
                });
            });
        }
        let picking = p.pick.is_some();
        let ctx = ui.ctx().clone();
        footer_button(
            ui,
            "Add folder…",
            "A folder with portable emulators (one sub-folder each), or one emulator's folder",
            pal,
            || {
                if !picking {
                    p.pick = Some((
                        PickFor::EmulatorFolder,
                        Pick::start(&ctx, PickKind::Folder, "Choose an emulator folder", None),
                    ));
                }
            },
        );
    });
    if let Some(d) = open {
        st.open_path(&d);
    }
    if let Some(i) = remove {
        let e = &mut st.config.emulation;
        let d = e.emulator_dirs.remove(i);
        // Not suggested again right away (the user just took it out).
        if !e.dismissed_emulator_dirs.contains(&d) {
            e.dismissed_emulator_dirs.push(d);
        }
        ch = true;
    }
    (ch, false)
}

// ------------------------------------------------------------------------- ROM folders

/// One ROM folder row.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct FolderRow {
    /// Config key, auto id ("auto-…") or "ignored:<n>".
    pub key: String,
    pub path: PathBuf,
    pub user: bool,
    pub enabled: bool,
}

/// The folders to list: the user's, then the ones found automatically (unless switched off),
/// then auto ones the user switched off.
fn folder_rows(cfg: &crate::config::EmulationConfig, facts: &LibFacts) -> Vec<FolderRow> {
    folder_rows_with(cfg, facts, &crate::emulation::registry::noted())
}

/// [`folder_rows`] with the folders the scanner remembered (`noted`: id → path; listed even
/// when they have no games in the library yet).
pub(super) fn folder_rows_with(
    cfg: &crate::config::EmulationConfig,
    facts: &LibFacts,
    noted: &[(String, PathBuf)],
) -> Vec<FolderRow> {
    let mut v: Vec<FolderRow> = cfg
        .rom_folders
        .iter()
        .map(|(k, rf)| FolderRow { key: k.clone(), path: rf.path.clone(), user: true, enabled: rf.enabled })
        .collect();
    if cfg.auto_folders {
        let remembered = noted.iter().filter(|(id, _)| id.starts_with("auto-"));
        for (id, path) in facts.auto_roots.iter().chain(remembered.map(|(k, p)| (k, p))) {
            if cfg.ignored_folders.contains(path) || v.iter().any(|r| r.path == *path || r.key == *id) {
                continue;
            }
            v.push(FolderRow { key: id.clone(), path: path.clone(), user: false, enabled: true });
        }
    }
    for (i, path) in cfg.ignored_folders.iter().enumerate() {
        // Ignored because the user took it over (Change console / Exclusions / Import): their
        // row is the one; a second switched-off row would scan it twice when switched on.
        if cfg.rom_folders.values().any(|rf| rf.path == *path) {
            continue;
        }
        v.push(FolderRow { key: format!("ignored:{i}"), path: path.clone(), user: false, enabled: false });
    }
    v
}

/// "PlayStation 2", "Sub-folders are consoles: PS1, PS2, …".
pub(super) fn folder_console_text(platform: &str, found: Option<&std::collections::BTreeSet<String>>) -> String {
    if !platform.is_empty() {
        return catalog::console_name(platform);
    }
    let found: Vec<String> =
        found.map(|s| s.iter().filter(|x| !x.is_empty()).map(|x| catalog::short_name(x)).collect()).unwrap_or_default();
    match found.len() {
        0 => "Consoles from the folder names".to_string(),
        1 => found[0].clone(),
        _ => format!("Sub-folders are consoles: {}", catalog::join_limited(&found, 4)),
    }
}

/// Returns (config changed, scan settings changed).
pub(super) fn rom_folders(ui: &mut Ui, st: &mut AppState, p: &mut EmuPage, pal: &Pal) -> (bool, bool) {
    let mut scan = false;
    group_title(ui, "ROM FOLDERS", pal);
    hint_text(
        ui,
        "Where your games are. Folders named after consoles (ps2, gc, 3ds …) are recognised by themselves.",
        pal,
    );
    ui.add_space(4.0);
    let rows = folder_rows(&st.config.emulation, &p.facts);
    let home = st.paths.home.clone();
    card(ui, pal, |ui| {
        if rows.is_empty() {
            empty_row(
                ui,
                if st.config.emulation.auto_folders {
                    "No ROM folders found yet. Lodestar looks in ~/ROMs, ~/Emulation/roms, ~/RetroPie/roms and on \
                     connected drives; add any other folder here."
                } else {
                    "No ROM folders yet. Add the folders that hold your games."
                },
                false,
                pal,
            );
        }
        for (i, r) in rows.iter().enumerate() {
            if i > 0 {
                row_separator(ui, pal);
            }
            scan |= folder_row(ui, st, p, r, &home, pal);
        }
        if let Some(mut f) = p.add_folder.take() {
            row_separator(ui, pal);
            let mut keep = true;
            indented(ui, |ui| {
                ui.add_space(8.0);
                ui.label(RichText::new("Add a ROM folder").size(13.5).strong().color(pal.text));
                ui.label(RichText::new(f.path.display().to_string()).monospace().size(12.5).color(pal.text));
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Console").size(13.0).color(pal.dim));
                    console_combo(ui, "ls_emu_add_folder_pf", &mut f.platform, "Detect from folder names", 260.0);
                });
                hint_text(ui, "Detect: the folder's own name, else each sub-folder's name (ps1, ps2, gc …).", pal);
                ui.horizontal(|ui| {
                    if primary_button(ui, "Add folder", pal, true).clicked() {
                        add_folder(&mut st.config.emulation, &f.path, &f.platform);
                        scan = true;
                        keep = false;
                    }
                    if plain_button(ui, "Cancel").clicked() {
                        keep = false;
                    }
                });
            });
            if keep {
                p.add_folder = Some(f);
            }
        } else {
            let picking = p.pick.is_some();
            let ctx = ui.ctx().clone();
            footer_button(ui, "Add folder…", "A folder with ROMs, or with one sub-folder per console", pal, || {
                if !picking {
                    p.pick =
                        Some((PickFor::RomFolder, Pick::start(&ctx, PickKind::Folder, "Choose a ROM folder", None)));
                }
            });
        }
    });
    (scan, scan)
}

/// Save a new ROM folder (an existing entry for the same path is switched on instead).
pub(super) fn add_folder(cfg: &mut crate::config::EmulationConfig, path: &Path, platform: &str) -> String {
    cfg.ignored_folders.retain(|x| x != path);
    if let Some((k, rf)) = cfg.rom_folders.iter_mut().find(|(_, rf)| rf.path == path) {
        rf.enabled = true;
        rf.platform = platform.to_string();
        return k.clone();
    }
    let key = catalog::next_folder_key(&cfg.rom_folders);
    cfg.rom_folders.insert(
        key.clone(),
        RomFolder { path: path.to_path_buf(), platform: platform.to_string(), ..Default::default() },
    );
    key
}

/// Turn a folder found automatically into one of the user's (to give it a console or
/// exclusions); the automatic one is ignored from then on. Returns its key.
pub(super) fn adopt_folder(cfg: &mut crate::config::EmulationConfig, path: &Path) -> String {
    let key = add_folder(cfg, path, "");
    if !cfg.ignored_folders.iter().any(|x| x == path) {
        cfg.ignored_folders.push(path.to_path_buf());
    }
    key
}

fn folder_row(ui: &mut Ui, st: &mut AppState, p: &mut EmuPage, r: &FolderRow, home: &Path, pal: &Pal) -> bool {
    let mut ch = false;
    let cfg = &st.config.emulation;
    let platform = cfg.rom_folders.get(&r.key).map(|rf| rf.platform.clone()).unwrap_or_default();
    let n = p.facts.per_folder.get(&r.key).copied().unwrap_or(0);
    let mut sub = vec![(folder_console_text(&platform, p.facts.folder_platforms.get(&r.key)), pal.dim)];
    if r.enabled || r.user {
        sub.push((format!(" · {}", plural(n, "game", "games")), pal.dim));
    }
    if !r.user {
        sub.push((
            if r.enabled { " · found automatically".into() } else { " · found automatically, not scanned".into() },
            pal.dim,
        ));
    } else if !r.enabled {
        sub.push((" · off".into(), pal.dim));
    }
    let missing_here = r.user && r.enabled && !p.facts.per_folder.contains_key(&r.key);
    let _ = missing_here;
    let mut set_platform: Option<String> = None;
    let mut action: Option<&'static str> = None;
    let mut on = r.enabled;
    let mut toggled = false;
    row(ui, pal, &short_path(&r.path, home, 70), &sub, |ui| {
        toggled = toggle_switch(ui, &mut on, 20.0, pal.accent)
            .on_hover_text(if r.enabled {
                "Scanned. Click to stop scanning it."
            } else {
                "Not scanned. Click to scan it."
            })
            .changed();
        if r.enabled || r.user {
            let more = icon_button(ui, Icon::More, false, 26.0, "More");
            egui::Popup::menu(&more).show(|ui| {
                ui.set_min_width(200.0);
                ui.menu_button("Change console", |ui| {
                    egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                        if ui.selectable_label(platform.is_empty(), "Detect from folder names").clicked() {
                            set_platform = Some(String::new());
                        }
                        let mut all: Vec<&platforms::Platform> = platforms::all().iter().collect();
                        all.sort_by_key(|p| p.name.to_lowercase());
                        for pf in all {
                            if ui.selectable_label(platform == pf.id, pf.name).clicked() {
                                set_platform = Some(pf.id.to_string());
                            }
                        }
                    });
                });
                if ui.button("Exclusions…").on_hover_text("Files or folders in it to leave out").clicked() {
                    action = Some("exclusions");
                }
                if ui.button("Open folder").clicked() {
                    action = Some("open");
                }
                ui.separator();
                if r.user {
                    if ui.button("Remove").on_hover_text("Its games leave the library at the next scan").clicked() {
                        action = Some("remove");
                    }
                } else if ui.button("Don't scan this folder").clicked() {
                    action = Some("ignore");
                }
            });
        }
    });
    let cfg = &mut st.config.emulation;
    if toggled {
        match (r.user, on) {
            (true, _) => {
                if let Some(rf) = cfg.rom_folders.get_mut(&r.key) {
                    rf.enabled = on;
                }
            }
            (false, false) => {
                if !cfg.ignored_folders.contains(&r.path) {
                    cfg.ignored_folders.push(r.path.clone());
                }
            }
            (false, true) => cfg.ignored_folders.retain(|x| *x != r.path),
        }
        ch = true;
    }
    if let Some(pf) = set_platform {
        let key = if r.user { r.key.clone() } else { adopt_folder(cfg, &r.path) };
        if let Some(rf) = cfg.rom_folders.get_mut(&key) {
            rf.platform = pf;
        }
        ch = true;
    }
    match action {
        Some("open") => st.open_path(&r.path),
        Some("exclusions") => {
            let key = if r.user {
                r.key.clone()
            } else {
                ch = true;
                adopt_folder(cfg, &r.path)
            };
            p.exclusions_open = if p.exclusions_open.as_deref() == Some(&key) { None } else { Some(key) };
        }
        Some("remove") => {
            cfg.rom_folders.remove(&r.key);
            ch = true;
        }
        Some("ignore") => {
            if !cfg.ignored_folders.contains(&r.path) {
                cfg.ignored_folders.push(r.path.clone());
            }
            ch = true;
        }
        _ => {}
    }
    if r.user && p.exclusions_open.as_deref() == Some(&r.key) {
        indented(ui, |ui| ch |= exclusions(ui, st, p, &r.key, pal));
    }
    ch
}

pub(super) fn exclusions(ui: &mut Ui, st: &mut AppState, p: &mut EmuPage, key: &str, pal: &Pal) -> bool {
    let mut ch = false;
    let Some(rf) = st.config.emulation.rom_folders.get_mut(key) else { return false };
    ui.label(RichText::new("Left out").size(13.0).strong().color(pal.text));
    hint_text(
        ui,
        "Paths inside the folder; * matches anything (*(Demo)*, ps2/Unfinished/*). Games you untick when \
         importing land here.",
        pal,
    );
    let mut remove = None;
    if rf.exclude.is_empty() {
        hint_text(ui, "Nothing is left out.", pal);
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for (i, w) in rf.exclude.iter().enumerate() {
            if ui
                .add(egui::Button::new(RichText::new(format!("{w}  ×")).size(12.5)).corner_radius(12))
                .on_hover_text("Scan it again")
                .clicked()
            {
                remove = Some(i);
            }
        }
    });
    if let Some(i) = remove {
        rf.exclude.remove(i);
        ch = true;
    }
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut p.new_exclusion).hint_text("*(Beta)*").desired_width(240.0));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let w = p.new_exclusion.trim().to_string();
        if (ui.button("Add").clicked() || enter) && !w.is_empty() {
            if !rf.exclude.contains(&w) {
                rf.exclude.push(w);
                ch = true;
            }
            p.new_exclusion.clear();
        }
        if ui.button("Close").clicked() {
            p.exclusions_open = None;
        }
    });
    ch
}

// ------------------------------------------------------------------------- BIOS folders

pub(super) fn bios_folders(ui: &mut Ui, st: &mut AppState, p: &mut EmuPage, pal: &Pal) -> bool {
    let mut ch = false;
    group_title(ui, "BIOS FOLDERS", pal);
    hint_text(
        ui,
        "Folders with BIOS files. An emulator's own folder comes first: what is there is what it uses. Also found by \
         themselves: folders and .zip files named like “bios” in your home folder, Documents, Downloads, ROM and \
         emulator folders, and Emulation/bios on your drives — never backups or folders named “old” or “unused”. \
         Remove one and Lodestar never looks there again.",
        pal,
    );
    ui.add_space(4.0);
    let home = st.paths.home.clone();
    let mine = st.config.emulation.bios_dirs.clone();
    let found: Vec<PathBuf> = p
        .bios
        .data
        .as_ref()
        .map(|d| d.search_dirs.iter().filter(|x| !mine.contains(x)).cloned().collect())
        .unwrap_or_default();
    let own = p.bios.data.as_ref().map(|d| d.own.clone()).unwrap_or_default();
    let mut remove_mine = None;
    let mut remove_found = None;
    card(ui, pal, |ui| {
        if mine.is_empty() && found.is_empty() {
            empty_row(ui, "No BIOS folders yet.", false, pal);
        }
        let mut first = true;
        for (i, d) in mine.iter().enumerate() {
            if !first {
                row_separator(ui, pal);
            }
            first = false;
            row(ui, pal, &short_path(d, &home, 70), &[("Chosen by you".into(), pal.dim)], |ui| {
                if ui.small_button("Remove").on_hover_text("Lodestar stops looking in this folder").clicked() {
                    remove_mine = Some(i);
                }
            });
        }
        for d in &found {
            if !first {
                row_separator(ui, pal);
            }
            first = false;
            // An emulator's own folder is where it reads its BIOS: nothing to remove.
            if let Some(name) = own.get(d) {
                row(ui, pal, &short_path(d, &home, 70), &[(format!("{name}'s own folder"), pal.dim)], |_| {});
                continue;
            }
            row(ui, pal, &short_path(d, &home, 70), &[("Found automatically".into(), pal.dim)], |ui| {
                if ui.small_button("Remove").on_hover_text("Lodestar stops looking in this folder").clicked() {
                    remove_found = Some(d.clone());
                }
            });
        }
        ch |= removed_bios_line(ui, st, &home, pal);
        let picking = p.pick.is_some();
        let ctx = ui.ctx().clone();
        footer_button(ui, "Add BIOS folder…", "A folder with the BIOS files you dumped", pal, || {
            if !picking {
                p.pick = Some((PickFor::BiosFolder, Pick::start(&ctx, PickKind::Folder, "Choose a BIOS folder", None)));
            }
        });
    });
    let e = &mut st.config.emulation;
    // A removed folder is never searched again, also when it would be found by itself.
    let mut ignore = |d: PathBuf| {
        if !e.ignored_bios_dirs.contains(&d) {
            e.ignored_bios_dirs.push(d);
        }
    };
    if let Some(i) = remove_mine {
        ignore(e.bios_dirs.remove(i));
        ch = true;
    }
    if let Some(d) = remove_found {
        ignore(d);
        ch = true;
    }
    ch
}

/// BIOS folders the user removed: one quiet, folded line ("2 removed by you"), each with "Use
/// again".
fn removed_bios_line(ui: &mut Ui, st: &mut AppState, home: &std::path::Path, pal: &Pal) -> bool {
    let removed = st.config.emulation.ignored_bios_dirs.clone();
    if removed.is_empty() {
        return false;
    }
    let mut restore = None;
    row_separator(ui, pal);
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        egui::CollapsingHeader::new(
            RichText::new(format!("{} removed by you", removed.len())).size(12.0).color(pal.dim),
        )
        .id_salt("bios_dirs_removed")
        .default_open(false)
        .show(ui, |ui| {
            for d in &removed {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(short_path(d, home, 70)).size(12.5).color(pal.text))
                        .on_hover_text(d.display().to_string());
                    if ui.small_button("Use again").on_hover_text("Lodestar looks in this folder again").clicked() {
                        restore = Some(d.clone());
                    }
                });
            }
        });
    });
    ui.add_space(2.0);
    if let Some(d) = restore {
        st.config.emulation.ignored_bios_dirs.retain(|x| *x != d);
        return true;
    }
    false
}
