//! Add game → Emulated games…: pick a ROM folder, preview the games the emulation scanner would
//! find (console and emulator per row, ticks), Import saves it as a ROM folder (unticked games
//! left out) and rescans. The preview runs on a helper thread.

use super::add::{Pick, PickKind, short_path};
use super::dialogs::{Buttons, body_height, footer, hint_text, modal, title};
use super::emulation_ui::catalog;
use super::{Action, Pal};
use crate::app::AppState;
use crate::config::RomFolder;
use crate::model::{EmuRef, ScannedGame};
use crate::ui::widgets::{lazy_spinner, mix, with_alpha};
use crossbeam_channel::Receiver;
use egui::{FontFamily, RichText, Sense, Ui, vec2};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One game of the preview.
pub(crate) struct Row {
    pub game: ScannedGame,
    pub checked: bool,
    /// Already in the library (found through another ROM folder).
    pub in_library: bool,
    /// In the library through a ROM folder at this very path: it stays whatever its tick says
    /// (never written as an exclusion, or Import would drop it from the library).
    pub same_root: bool,
}

type Preview = Result<Vec<ScannedGame>, String>;

/// The dialog's state (`Dialogs.roms`).
pub(crate) struct RomImportDlg {
    /// The folder being looked at (None: not picked yet).
    pub folder: Option<PathBuf>,
    /// Console for the whole folder ("" = detect from folder names).
    platform: String,
    pick: Option<Pick>,
    rx: Option<(Receiver<Preview>, (PathBuf, String))>,
    /// What the preview showed it for (folder, console).
    shown_for: Option<(PathBuf, String)>,
    rows: Vec<Row>,
    error: Option<String>,
}

impl RomImportDlg {
    /// Open the dialog; `folder` = look at that folder right away (no file dialog; also the
    /// `roms-import:<folder>` screenshot target). Without one the folder picker opens.
    pub(super) fn open(st: &mut AppState, folder: Option<PathBuf>) -> RomImportDlg {
        let platform = folder
            .as_ref()
            .and_then(|f| st.config.emulation.rom_folders.values().find(|rf| rf.path == *f))
            .map(|rf| rf.platform.clone())
            .unwrap_or_default();
        let pick = folder.is_none().then(|| Pick::start(st.ctx(), PickKind::Folder, "Choose a folder with ROMs", None));
        RomImportDlg { folder, platform, pick, rx: None, shown_for: None, rows: vec![], error: None }
    }

    fn checked(&self) -> usize {
        self.rows.iter().filter(|r| r.checked).count()
    }

    /// Start the preview when the folder or the console changed.
    fn refresh(&mut self, st: &AppState) {
        let Some(folder) = self.folder.clone() else { return };
        let key = (folder.clone(), self.platform.clone());
        if self.shown_for.as_ref() == Some(&key) || self.rx.as_ref().is_some_and(|(_, k)| *k == key) {
            return;
        }
        let mut ctx = crate::scan::ScanContext::from_config(&st.config);
        ctx.home = st.paths.home.clone();
        let rf = RomFolder { path: folder, platform: self.platform.clone(), ..Default::default() };
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ectx = st.ctx().clone();
        let spawned = std::thread::Builder::new().name("rom-preview".into()).spawn(move || {
            let r = crate::scan::emulation::preview_folder(&ctx, &rf).map_err(|e| format!("{e:#}"));
            let _ = tx.send(r);
            ectx.request_repaint();
        });
        match spawned {
            Ok(_) => self.rx = Some((rx, key)),
            Err(e) => self.error = Some(format!("Couldn't look at the folder: {e}")),
        }
    }

    fn poll(&mut self, st: &AppState) {
        if let Some(p) = &self.pick
            && let Some(v) = p.poll()
        {
            self.pick = None;
            if let Some(f) = v.into_iter().next() {
                if self.folder.as_ref() != Some(&f) {
                    self.platform = st
                        .config
                        .emulation
                        .rom_folders
                        .values()
                        .find(|rf| rf.path == f)
                        .map(|rf| rf.platform.clone())
                        .unwrap_or_default();
                }
                self.folder = Some(f);
            }
        }
        if let Some((rx, key)) = &self.rx
            && let Ok(r) = rx.try_recv()
        {
            let key = key.clone();
            self.rx = None;
            match r {
                Ok(games) => {
                    let exclude: Vec<String> = st
                        .config
                        .emulation
                        .rom_folders
                        .values()
                        .find(|rf| rf.path == key.0)
                        .map(|rf| rf.exclude.clone())
                        .unwrap_or_default();
                    self.rows = rows_for(games, &st.library.games, &key.0, &exclude);
                    self.error = None;
                }
                Err(e) => {
                    self.rows.clear();
                    self.error = Some(e);
                }
            }
            self.shown_for = Some(key);
        }
        self.refresh(st);
    }

    /// Testing hook (`roms-import-demo:<folder>`): the library's games from this folder as the
    /// preview, as if they were new.
    pub(super) fn demo_from_library(&mut self, st: &AppState) {
        let Some(folder) = self.folder.clone() else { return };
        let games: Vec<ScannedGame> = st
            .library
            .games
            .iter()
            .filter(|g| g.emu.as_ref().is_some_and(|e| e.rom.starts_with(&folder)))
            .map(|g| {
                let mut s = ScannedGame::new(g.source, g.source_id.clone(), g.name.clone());
                s.emu = g.emu.clone();
                s
            })
            .collect();
        self.rows = rows_for(games, &[], &folder, &[]);
        self.shown_for = Some((folder, self.platform.clone()));
        self.rx = None;
    }

    fn looking(&self) -> bool {
        self.rx.is_some()
    }
}

/// Preview rows, by console then name: games already in the library and games the folder
/// already leaves out start unticked.
pub(crate) fn rows_for(
    games: Vec<ScannedGame>,
    library: &[crate::model::Game],
    root: &Path,
    exclude: &[String],
) -> Vec<Row> {
    // ROM → the folder its library game was found in.
    let known: HashMap<&Path, Option<PathBuf>> = library
        .iter()
        .filter_map(|g| g.emu.as_ref().map(|e| (e.rom.as_path(), catalog::root_of(&g.source_id, &e.rom))))
        .collect();
    let mut rows: Vec<Row> = games
        .into_iter()
        .map(|g| {
            let lib = g.emu.as_ref().and_then(|e| known.get(e.rom.as_path()));
            let in_library = lib.is_some();
            let same_root = lib.is_some_and(|r| r.as_deref() == Some(root));
            let left_out = g.emu.as_ref().is_some_and(|e| {
                catalog::exclusions_for(root, [e]).iter().any(|x| crate::scan::emulation::walk::excluded_by(exclude, x))
            });
            Row { checked: !in_library && !left_out, in_library, same_root, game: g }
        })
        .collect();
    rows.sort_by_cached_key(|r| (console_of(&r.game).to_lowercase(), r.game.name.to_lowercase()));
    rows
}

fn console_of(g: &ScannedGame) -> String {
    g.emu.as_ref().map(|e| catalog::console_name(&e.platform)).unwrap_or_default()
}

/// What Import saves: the exclusions for unticked games (paths relative to the folder), except
/// games already in the library through this same folder (they stay).
pub(crate) fn import_exclusions(rows: &[Row], root: &Path) -> Vec<String> {
    let unticked: Vec<&EmuRef> =
        rows.iter().filter(|r| !r.checked && !r.same_root).filter_map(|r| r.game.emu.as_ref()).collect();
    catalog::exclusions_for(root, unticked)
}

/// The folder's exclusions after Import: its existing patterns that leave out no ticked game
/// (the user's own `*(Beta)*` survives), then the unticked games no kept pattern covers.
pub(crate) fn merged_exclusions(rows: &[Row], root: &Path, existing: &[String]) -> Vec<String> {
    let ticked = catalog::exclusions_for(root, rows.iter().filter(|r| r.checked).filter_map(|r| r.game.emu.as_ref()));
    let mut out: Vec<String> = existing
        .iter()
        .filter(|p| !ticked.iter().any(|t| crate::scan::emulation::walk::excluded_by(std::slice::from_ref(*p), t)))
        .cloned()
        .collect();
    for x in import_exclusions(rows, root) {
        if !crate::scan::emulation::walk::excluded_by(&out, &x) {
            out.push(x);
        }
    }
    out
}

/// Draw the dialog when it is open (`*dlg` is Some); set `*dlg = None` to close it. A returned
/// action is applied after drawing (e.g. `Action::Settings(Tab::Emulation)`, `Action::Select`).
pub(super) fn show(
    ctx: &egui::Context,
    st: &mut AppState,
    dlg: &mut Option<RomImportDlg>,
    pal: &Pal,
    fam: &FontFamily,
) -> Option<Action> {
    let d = dlg.as_mut()?;
    d.poll(st);
    let mut close = false;
    let mut out = None;
    let screen = ctx.content_rect();
    let width = (screen.width() - 64.0).clamp(600.0, 860.0);
    let in_main = (screen.height() - 330.0).clamp(160.0, 460.0);
    let home = st.paths.home.clone();
    let cat = catalog::catalog(ctx);
    let (_, x) = modal(ctx, "ls_roms_import", pal, width, |ui| {
        title(ui, "Emulated games", Some("Choose a folder with ROMs, then tick the games to add."), pal, fam);
        // ---- folder + console
        ui.horizontal(|ui| {
            ui.label(RichText::new("Folder").size(13.0).color(pal.dim));
            let text = match &d.folder {
                Some(f) => RichText::new(short_path(f, &home, 60)).monospace().size(12.5).color(pal.text),
                None => RichText::new("None chosen yet").size(13.0).color(pal.dim),
            };
            ui.add(egui::Label::new(text).truncate());
            if ui
                .add_enabled(
                    d.pick.is_none(),
                    egui::Button::new(if d.folder.is_some() { "Change…" } else { "Choose…" }),
                )
                .clicked()
            {
                d.pick = Some(Pick::start(ui.ctx(), PickKind::Folder, "Choose a folder with ROMs", d.folder.clone()));
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Console").size(13.0).color(pal.dim));
            super::emulation_ui::console_combo(
                ui,
                "ls_roms_console",
                &mut d.platform,
                "Detect from folder names",
                260.0,
            );
            hint_text(ui, "Detect: the folder's name, else each sub-folder's (ps1, ps2, gc …).", pal);
        });
        ui.add_space(4.0);
        // ---- status
        ui.horizontal(|ui| {
            ui.set_min_height(22.0);
            if d.looking() {
                lazy_spinner(ui, 14.0, pal.accent);
                ui.label(RichText::new("Looking at the folder…").size(12.5).color(pal.dim));
            } else if let Some(e) = &d.error {
                ui.add(egui::Label::new(RichText::new(e).size(12.5).color(pal.danger)).truncate());
            } else if d.shown_for.is_some() && !d.rows.is_empty() {
                let n = d.rows.len();
                let in_lib = d.rows.iter().filter(|r| r.in_library).count();
                let mut t = super::emulation_ui::plural(n, "game found", "games found");
                if in_lib > 0 {
                    t.push_str(&format!(", {in_lib} already in your library"));
                }
                ui.label(RichText::new(t).size(12.5).color(pal.dim));
            }
        });
        let table_h = body_height(ui, in_main);
        rom_table(ui, d, cat.as_deref(), pal, &home, table_h);
        // ---- footer: the list tools on the left
        let n = d.checked();
        // Nothing found: the folder can still be added (a scan with the right console, or
        // emulators set up later, finds its games).
        let empty = d.rows.is_empty() && d.shown_for.is_some();
        let label = match n {
            _ if empty => "Add folder".to_string(),
            0 => "Import".to_string(),
            1 => "Import 1 game".to_string(),
            n => format!("Import {n} games"),
        };
        let ready = (n > 0 || empty) && d.folder.is_some() && !d.looking();
        let mut buttons = Buttons::primary(&label, ready).cancel("Cancel").disabled_hint("Tick the games to add");
        if empty {
            buttons = buttons.hint("Save it as a ROM folder; its games show after a scan");
        }
        let f = footer(ui, pal, buttons, |ui| {
            let any = !d.rows.is_empty();
            if ui.add_enabled(any, egui::Button::new("Select all").min_size(vec2(0.0, 30.0))).clicked() {
                d.rows.iter_mut().for_each(|r| r.checked = true);
            }
            if ui.add_enabled(any, egui::Button::new("Select none").min_size(vec2(0.0, 30.0))).clicked() {
                d.rows.iter_mut().for_each(|r| r.checked = false);
            }
            if ui
                .add(egui::Button::new(RichText::new("Set up emulation…").color(pal.accent)).frame(false))
                .on_hover_text("Settings → Emulation: emulators, BIOS, ROM folders")
                .clicked()
            {
                out = Some(Action::Settings(super::settings::Tab::Emulation));
                close = true;
            }
        });
        if f.primary
            && let Some(folder) = d.folder.clone()
        {
            let cfg = &mut st.config.emulation;
            let existing = cfg.rom_folders.values().find(|rf| rf.path == folder).map(|rf| rf.exclude.clone());
            let exclude = merged_exclusions(&d.rows, &folder, &existing.unwrap_or_default());
            // A folder that was found automatically becomes the user's: scanned once, under its
            // new id (its games move over, playtime and all).
            let auto = catalog::LibFacts::compute(&st.library.games, 0).auto_roots.values().any(|p| *p == folder);
            let cfg = &mut st.config.emulation;
            super::emulation_ui::save_rom_folder(cfg, &folder, &d.platform, exclude);
            if auto && !cfg.ignored_folders.contains(&folder) {
                cfg.ignored_folders.push(folder.clone());
            }
            st.save_config();
            st.rescan();
            let what = if empty { "the folder".to_string() } else { label.trim_start_matches("Import ").to_string() };
            st.notify(format!("Adding {what} from {}", short_path(&folder, &home, 40)));
            close = true;
        }
        if f.cancel {
            close = true;
        }
    });
    // The backdrop / Esc closes, except while the folder picker is up.
    if close || (x && d.pick.is_none()) {
        *dlg = None;
    }
    out
}

const ROW_H: f32 = 28.0;

/// The checklist: a header row per console (with its own tick), then its games.
fn rom_table(ui: &mut Ui, d: &mut RomImportDlg, cat: Option<&catalog::Catalog>, pal: &Pal, home: &Path, height: f32) {
    let w = ui.available_width();
    let chk_w = 28.0;
    let name_w = (w * 0.34).clamp(170.0, 320.0);
    let emu_w = (w * 0.2).clamp(110.0, 180.0);
    let file_w = (w - chk_w - name_w - emu_w - 4.0 * 8.0).max(80.0);
    egui::Frame::new()
        .fill(mix(pal.bg, pal.surface, 0.35))
        .stroke(egui::Stroke::new(1.0, with_alpha(pal.border, 150)))
        .corner_radius(6)
        .show(ui, |ui| {
            ui.set_width(w);
            if d.rows.is_empty() {
                let (r, _) = ui.allocate_exact_size(vec2(w, height), Sense::hover());
                let text = if d.folder.is_none() {
                    "Choose a folder to see its games."
                } else if d.looking() || d.shown_for.is_none() {
                    ""
                } else if d.platform.is_empty() {
                    "No games found here. If the folder holds games of one console, choose it above."
                } else {
                    "No games of this console found here. Try another console, or Detect from folder names."
                };
                ui.painter().text(
                    r.center(),
                    egui::Align2::CENTER_CENTER,
                    text,
                    egui::FontId::proportional(13.5),
                    pal.dim,
                );
                return;
            }
            // Console groups (rows are sorted by console).
            let mut groups: Vec<(String, std::ops::Range<usize>)> = vec![];
            for (i, r) in d.rows.iter().enumerate() {
                let c = console_of(&r.game);
                match groups.last_mut() {
                    Some((g, range)) if *g == c => range.end = i + 1,
                    _ => groups.push((c, i..i + 1)),
                }
            }
            egui::ScrollArea::vertical()
                .id_salt("ls_roms_rows")
                .max_height(height)
                .min_scrolled_height(height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (console, range) in groups {
                        // group header
                        let (rect, _) = ui.allocate_exact_size(vec2(w, ROW_H + 2.0), Sense::hover());
                        ui.painter().rect_filled(rect, 0.0, with_alpha(pal.surface_alt, 110));
                        let mut child = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(rect.shrink2(vec2(6.0, 0.0)))
                                .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        child.spacing_mut().item_spacing.x = 8.0;
                        let all = d.rows[range.clone()].iter().all(|r| r.checked);
                        let mut a = all;
                        if child.add_sized([chk_w - 6.0, 20.0], egui::Checkbox::without_text(&mut a)).changed() {
                            d.rows[range.clone()].iter_mut().for_each(|r| r.checked = a);
                        }
                        let name = if console.is_empty() { "Unknown console".to_string() } else { console };
                        child.label(RichText::new(name).size(13.0).strong().color(pal.text));
                        child.label(RichText::new(format!("{}", range.len())).size(12.0).color(pal.dim));
                        for i in range {
                            let row = &mut d.rows[i];
                            let (rect, _) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::hover());
                            let mut child = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(rect.shrink2(vec2(6.0, 0.0)))
                                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                            );
                            let ui = &mut child;
                            ui.spacing_mut().item_spacing.x = 8.0;
                            ui.add_sized([chk_w - 6.0, 20.0], egui::Checkbox::without_text(&mut row.checked));
                            let col = if row.in_library { pal.dim } else { pal.text };
                            cell(ui, RichText::new(&row.game.name).size(13.0).color(col), name_w);
                            let rom = row.game.emu.as_ref().map(|e| e.rom.clone()).unwrap_or_default();
                            let shown = d
                                .folder
                                .as_ref()
                                .and_then(|f| rom.strip_prefix(f).ok())
                                .map_or_else(|| short_path(&rom, home, 60), |r| r.display().to_string());
                            cell(ui, RichText::new(shown).monospace().size(11.5).color(pal.dim), file_w)
                                .on_hover_text(rom.display().to_string());
                            let (text, color) = if row.in_library {
                                ("In library".to_string(), pal.dim)
                            } else {
                                emulator_text(row.game.emu.as_ref(), cat, pal)
                            };
                            cell(ui, RichText::new(text).size(12.0).color(color), emu_w);
                        }
                    }
                });
        });
}

fn cell(ui: &mut Ui, text: RichText, w: f32) -> egui::Response {
    ui.allocate_ui_with_layout(vec2(w, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_width(w);
        ui.add(egui::Label::new(text).truncate())
    })
    .inner
}

/// The emulator a game would start with, or "No emulator" (amber).
fn emulator_text(e: Option<&EmuRef>, cat: Option<&catalog::Catalog>, pal: &Pal) -> (String, egui::Color32) {
    let warn = super::emulation_ui::tone(catalog::ChipTone::Warn, pal);
    let Some(e) = e else { return ("No emulator".into(), warn) };
    if !e.emulator.is_empty() {
        let name = cat.map_or_else(|| e.emulator.clone(), |c| c.name_by_id(&e.emulator));
        return (name, pal.dim);
    }
    match cat.and_then(|c| c.auto.get(&e.platform).and_then(|(id, _)| c.emu(id)).map(|x| c.name_of(&x.emu))) {
        Some(n) => (n, pal.dim),
        None => ("No emulator".into(), warn),
    }
}
