//! The Add-ons window (app menu → Add-ons…, like Playnite's): Extensions (external add-ons by
//! kind, plus the built-in integrations), Desktop themes, Controller themes (`addons/themes.rs`),
//! Browse themes and Updates (`addons/browse.rs`). External add-ons are installed from a folder or
//! a .zip, switched on (after showing what they ask for), configured through the fields their
//! manifest declares, and removed. Installing and removing run on a helper thread.

mod browse;
pub(crate) mod themes;

use super::add::{Pick, PickKind};
use super::dialogs::{Buttons, body_height, button_row, footer, hint_text, modal};
use super::{DesktopUi, Pal};
use crate::addons::{AddonInfo, AddonKind, SettingField, SettingKind};
use crate::app::AppState;
use crate::model::Source;
use crate::ui::widgets::{Icon, icon_button, initials_tile, lazy_spinner, mix, paint_icon, toggle_switch, with_alpha};
use crossbeam_channel::{Receiver, TryRecvError};
use egui::{Align, Layout, RichText, Sense, Ui, pos2, vec2};
use std::collections::HashSet;
use std::path::PathBuf;

/// The guide for writing add-ons (in the source tree; opened when it exists).
/// The add-on guide, built in: the source tree isn't there on other machines or in the AppImage.
const GUIDE: &str = include_str!("../../../examples/addons/README.md");

/// Which page the left tree shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Section {
    /// Extensions: every installed add-on.
    All,
    Kind(AddonKind),
    BuiltIn,
    DesktopThemes,
    ControllerThemes,
    /// Community themes (Playnite's theme list).
    Browse,
    Updates,
}

impl Section {
    pub fn from_key(k: &str) -> Section {
        match k {
            "libraries" => Section::Kind(AddonKind::Library),
            "metadata" => Section::Kind(AddonKind::Metadata),
            "generic" => Section::Kind(AddonKind::Generic),
            "builtin" => Section::BuiltIn,
            "themes" | "desktop-themes" | "themes-desktop" => Section::DesktopThemes,
            "controller-themes" | "themes-controller" => Section::ControllerThemes,
            "browse" => Section::Browse,
            "updates" => Section::Updates,
            _ => Section::All,
        }
    }
}

enum PickFor {
    InstallFolder,
    InstallZip,
    /// A Folder setting: (add-on id, key).
    Field(String, String),
}

enum Job {
    Installed(PathBuf, Result<String, String>),
    Removed(String, Result<(), String>),
}

/// Where the window wants to go when it closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AddonsOutcome {
    Keep,
    Close,
    OpenSettings(super::settings::Tab),
}

pub(crate) struct AddonsDlg {
    pub section: Section,
    /// Testing hook (`LODESTAR_DESKTOP_OPEN=addons-install:<path>`): install this right away.
    pub debug_install: Option<PathBuf>,
    search: String,
    /// Expanded card (add-on id).
    pub selected: Option<String>,
    /// Waiting for the user to accept the permissions before switching it on.
    pub confirm_enable: Option<String>,
    confirm_remove: Option<String>,
    pick: Option<(PickFor, Pick)>,
    job: Option<(String, Receiver<Job>)>,
    /// Result of the last install / remove (text, is an error).
    banner: Option<(String, bool)>,
    shown_secrets: HashSet<(String, String)>,
    refreshed: bool,
    /// A library add-on was switched or its settings changed: offer a rescan.
    library_changed: bool,
    /// Theme pages, imports and the install job (`addons/themes.rs`).
    pub themes: themes::Themes,
    /// Browse themes / Updates (`addons/browse.rs`).
    browse: browse::Browse,
}

impl AddonsDlg {
    pub fn new(section: Section) -> AddonsDlg {
        AddonsDlg {
            section,
            debug_install: None,
            search: String::new(),
            selected: None,
            confirm_enable: None,
            confirm_remove: None,
            pick: None,
            job: None,
            banner: None,
            shown_secrets: HashSet::new(),
            refreshed: false,
            library_changed: false,
            themes: Default::default(),
            browse: Default::default(),
        }
    }
}

/// DB kv key remembering that the user saw (and accepted) an add-on's permissions.
fn ack_key(id: &str) -> String {
    format!("addons.ack.{id}")
}

fn perms_fingerprint(info: &AddonInfo) -> String {
    let mut p = info.manifest.permissions.clone();
    p.sort();
    p.join("\n")
}

/// The permissions were shown and accepted for this add-on (and haven't changed since).
fn acknowledged(st: &AppState, info: &AddonInfo) -> bool {
    info.manifest.permissions.is_empty()
        || st.db.kv_get(&ack_key(&info.manifest.id)).as_deref() == Some(perms_fingerprint(info).as_str())
}

fn set_enabled(st: &mut AppState, id: &str, on: bool) {
    let list = &mut st.config.addons.enabled;
    list.retain(|e| e != id);
    if on {
        list.push(id.to_string());
    }
}

fn matches_search(info: &AddonInfo, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    let m = &info.manifest;
    [&m.name, &m.author, &m.description, &m.id].iter().any(|s| s.to_lowercase().contains(q))
}

impl DesktopUi {
    pub(super) fn addons_dialog(&mut self, ctx: &egui::Context, st: &mut AppState, d: &mut AddonsDlg) -> AddonsOutcome {
        let pal = self.pal;
        if !d.refreshed {
            // A handful of tiny files: fine on the UI thread (once, when the window opens).
            d.refreshed = true;
            st.refresh_addons();
        }
        if let Some(p) = d.debug_install.take() {
            start_install(st, d, p);
        }
        poll_job(st, d);
        d.themes.poll(st);
        d.browse.poll(&d.themes);
        let mut config_changed = poll_pick(st, d);
        let mut out = AddonsOutcome::Keep;
        let fam = self.fonts.bold.clone();
        let in_main = (ctx.content_rect().height() - 220.0).clamp(300.0, 560.0);
        let (_, close) = modal(ctx, "ls_addons", &pal, 940.0, |ui| {
            ui.horizontal(|ui| {
                paint_icon(
                    ui.painter(),
                    egui::Rect::from_min_size(ui.cursor().min + vec2(0.0, 3.0), vec2(22.0, 22.0)),
                    Icon::Puzzle,
                    pal.text,
                );
                ui.add_space(30.0);
                ui.label(RichText::new("Add-ons").family(fam.clone()).size(20.0).strong().color(pal.text));
            });
            let intro = match d.section {
                Section::DesktopThemes => {
                    "Themes for the desktop window: colours and fonts. Use one here or in Settings → Appearance."
                }
                Section::ControllerThemes => {
                    "Themes for controller mode: the full-screen layout, colours, fonts and pictures. \
                     Playnite fullscreen themes are converted when you import them."
                }
                Section::Browse => "Install themes made by the Playnite community.",
                Section::Updates => "Newer versions of the themes you installed from the theme list.",
                _ => {
                    "Extensions add game libraries, metadata sources and more. Each runs as its own \
                     small program, so a broken one can't slow down or crash Lodestar."
                }
            };
            hint_text(ui, intro, &pal);
            ui.add_space(12.0);
            let max_h = body_height(ui, in_main);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(vec2(190.0, max_h), Layout::top_down_justified(Align::Min), |ui| {
                    ui.set_width(190.0);
                    ui.set_min_height(max_h);
                    // Scrolls in a short window.
                    egui::ScrollArea::vertical().id_salt("ls_addons_tree").max_height(max_h).show(ui, |ui| {
                        tree(ui, st, d, &pal);
                    });
                });
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(max_h);
                    match d.section {
                        Section::Browse => browse::browse_page(ui, st, &mut d.browse, &mut d.themes, &pal, max_h),
                        Section::Updates => browse::updates_page(ui, st, &mut d.browse, &mut d.themes, &pal),
                        Section::DesktopThemes | Section::ControllerThemes => {
                            let page = if d.section == Section::DesktopThemes {
                                themes::Page::Desktop
                            } else {
                                themes::Page::Controller
                            };
                            if let Some(s) = themes::page(ui, st, &mut d.themes, page, &pal, max_h) {
                                if s == Section::Browse {
                                    d.browse.filter = if page == themes::Page::Desktop {
                                        browse::Filter::Desktop
                                    } else {
                                        browse::Filter::Controller
                                    };
                                }
                                d.section = s;
                            }
                        }
                        _ => {
                            let (ch, o) = installed_page(ui, st, d, &pal, max_h);
                            config_changed |= ch;
                            if let Some(o) = o {
                                out = o;
                            }
                        }
                    }
                });
            });
            let f = footer(ui, &pal, Buttons::primary("Done", true), |ui| {
                if d.library_changed {
                    let label = if st.scan.running { "Scanning…" } else { "Rescan now" };
                    if ui.add_enabled(!st.scan.running, egui::Button::new(label).min_size(vec2(100.0, 30.0))).clicked()
                    {
                        st.save_config();
                        st.rescan();
                        d.library_changed = false;
                    }
                    ui.label(
                        RichText::new("Library add-ons changed — rescan to update the library.")
                            .size(12.0)
                            .color(pal.dim),
                    );
                }
            });
            if f.primary {
                out = AddonsOutcome::Close;
            }
        });
        themes::report_window(ctx, st, &mut d.themes, &pal);
        if config_changed {
            self.mark_config_dirty();
        }
        let report_open = !d.themes.reports.is_empty();
        if close && out == AddonsOutcome::Keep && d.confirm_enable.is_none() && !report_open {
            out = AddonsOutcome::Close;
        } else if close {
            // Esc first cancels an open confirmation.
            d.confirm_enable = None;
            d.confirm_remove = None;
        }
        if out != AddonsOutcome::Keep && self.config_dirty.is_some() {
            self.config_dirty = None;
            st.save_config();
        }
        out
    }
}

fn poll_job(st: &mut AppState, d: &mut AddonsDlg) {
    let Some((_, rx)) = &d.job else { return };
    let job = match rx.try_recv() {
        Ok(j) => j,
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => {
            d.job = None;
            d.banner = Some(("The add-on job stopped unexpectedly.".into(), true));
            return;
        }
    };
    d.job = None;
    st.refresh_addons();
    match job {
        Job::Installed(src, Ok(id)) => {
            let name = st.addons.get(&id).map(|a| a.manifest.name.clone()).unwrap_or_else(|| id.clone());
            let on = crate::addons::is_enabled(&id, &st.config.addons);
            d.banner = Some((
                if on { format!("Updated {name}.") } else { format!("Installed {name}. Switch it on to use it.") },
                false,
            ));
            log::info!("add-on {id} installed from {}", src.display());
            d.selected = Some(id);
            d.section = Section::All;
            if on {
                d.library_changed = true;
            }
        }
        Job::Installed(src, Err(e)) => {
            let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            d.banner = Some((format!("Couldn't install {name}: {e}"), true));
        }
        Job::Removed(id, Ok(())) => {
            let was_on = crate::addons::is_enabled(&id, &st.config.addons);
            set_enabled(st, &id, false);
            if was_on {
                d.library_changed = true;
                st.save_config();
            }
            d.banner = Some(("Removed. Its settings are kept in case you reinstall it.".into(), false));
            if d.selected.as_deref() == Some(id.as_str()) {
                d.selected = None;
            }
        }
        Job::Removed(_, Err(e)) => d.banner = Some((format!("Couldn't remove it: {e}"), true)),
    }
}

/// A picked folder / .zip / setting folder arrived. Returns true when the config changed.
fn poll_pick(st: &mut AppState, d: &mut AddonsDlg) -> bool {
    let Some((_, pick)) = &d.pick else {
        return false;
    };
    let Some(paths) = pick.poll() else {
        return false;
    };
    let Some((what, _)) = d.pick.take() else {
        return false;
    };
    let Some(path) = paths.into_iter().next() else {
        return false; // cancelled
    };
    match what {
        PickFor::InstallFolder | PickFor::InstallZip => {
            start_install(st, d, path);
            false
        }
        PickFor::Field(id, key) => {
            st.config.addons.settings.entry(id.clone()).or_default().insert(key, path.to_string_lossy().into_owned());
            mark_library_change(st, d, &id);
            true
        }
    }
}

fn mark_library_change(st: &AppState, d: &mut AddonsDlg, id: &str) {
    let lib = st.addons.get(id).is_some_and(|a| a.manifest.kind == AddonKind::Library);
    if lib && crate::addons::is_enabled(id, &st.config.addons) {
        d.library_changed = true;
    }
}

/// Validate and copy on a helper thread (a .zip is unpacked; never run).
fn start_install(st: &AppState, d: &mut AddonsDlg, src: PathBuf) {
    if d.job.is_some() {
        return;
    }
    let mut mgr = st.addons.clone();
    let (tx, rx) = crossbeam_channel::bounded(1);
    let ctx = st.ctx().clone();
    let label =
        format!("Installing {}…", src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let spawned = std::thread::Builder::new().name("addon-install".into()).spawn(move || {
        let r = mgr.install_from(&src).map_err(|e| format!("{e:#}"));
        let _ = tx.send(Job::Installed(src, r));
        ctx.request_repaint();
    });
    match spawned {
        Ok(_) => {
            d.banner = None;
            d.job = Some((label, rx));
        }
        Err(e) => d.banner = Some((format!("Couldn't start installing: {e}"), true)),
    }
}

fn start_remove(st: &AppState, d: &mut AddonsDlg, id: String) {
    if d.job.is_some() {
        return;
    }
    let mut mgr = st.addons.clone();
    let (tx, rx) = crossbeam_channel::bounded(1);
    let ctx = st.ctx().clone();
    let spawned = std::thread::Builder::new().name("addon-remove".into()).spawn(move || {
        let r = mgr.remove(&id).map_err(|e| format!("{e:#}"));
        let _ = tx.send(Job::Removed(id, r));
        ctx.request_repaint();
    });
    match spawned {
        Ok(_) => {
            d.banner = None;
            d.job = Some(("Removing…".into(), rx));
        }
        Err(e) => d.banner = Some((format!("Couldn't start removing: {e}"), true)),
    }
}

// ------------------------------------------------------------------------------- tree

fn tree(ui: &mut Ui, st: &AppState, d: &mut AddonsDlg, pal: &Pal) {
    ui.spacing_mut().item_spacing.y = 2.0;
    let list = st.addons.list();
    let n_kind = |k: AddonKind| list.iter().filter(|a| a.manifest.kind == k).count();
    let n_builtin = super::libraries::builtin_libraries().count() + BUILTIN_METADATA.len();
    let n_themes = d.themes.list.as_ref().map(|l| l.len());
    let n_ctl = d.themes.list.as_ref().map(|l| l.iter().filter(|t| t.controller).count());
    let n_up = d.browse.updates.as_ref().map(|u| u.len()).filter(|n| *n > 0);
    let mut item = |ui: &mut Ui, s: Section, label: &str, count: Option<usize>, indent: bool| {
        let sel = d.section == s;
        let h = 30.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
        let p = ui.painter();
        if sel {
            p.rect_filled(rect, 6.0, pal.sel_bg);
        } else if resp.hovered() {
            p.rect_filled(rect, 6.0, pal.hover);
        }
        let x = rect.left() + if indent { 26.0 } else { 10.0 };
        let color = if sel { pal.text } else { mix(pal.text, pal.dim, 0.2) };
        let font = egui::FontId::proportional(if indent { 13.5 } else { 14.0 });
        let g = p.layout_no_wrap(label.to_string(), font, color);
        p.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, color);
        if let Some(n) = count {
            let cg = p.layout_no_wrap(n.to_string(), egui::FontId::proportional(12.0), pal.dim);
            p.galley(pos2(rect.right() - 10.0 - cg.size().x, rect.center().y - cg.size().y / 2.0), cg, pal.dim);
        }
        if resp.clicked() {
            d.section = s;
            d.confirm_enable = None;
            d.confirm_remove = None;
        }
    };
    item(ui, Section::All, "Extensions", Some(list.len()), false);
    item(ui, Section::Kind(AddonKind::Library), "Libraries", Some(n_kind(AddonKind::Library)), true);
    item(ui, Section::Kind(AddonKind::Metadata), "Metadata sources", Some(n_kind(AddonKind::Metadata)), true);
    item(ui, Section::Kind(AddonKind::Generic), "Generic", Some(n_kind(AddonKind::Generic)), true);
    item(ui, Section::BuiltIn, "Built-in", Some(n_builtin), true);
    ui.add_space(8.0);
    item(ui, Section::DesktopThemes, "Desktop themes", n_themes, false);
    item(ui, Section::ControllerThemes, "Controller themes", n_ctl, false);
    ui.add_space(8.0);
    item(ui, Section::Browse, "Browse themes", None, false);
    item(ui, Section::Updates, "Updates", n_up, false);
}

// ------------------------------------------------------------------------------- pages

fn guide_link(ui: &mut Ui, st: &mut AppState, pal: &Pal) {
    let r = ui
        .add(egui::Button::new(RichText::new("How to write an add-on").color(pal.selection)).frame(false))
        .on_hover_text("Opens the add-on guide (a text file)");
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if r.clicked() {
        // A copy in the cache folder, opened with the user's viewer for Markdown / text.
        let path = st.paths.cache_dir.join("addon-guide.md");
        let fresh = std::fs::read_to_string(&path).is_ok_and(|t| t == GUIDE);
        let written =
            fresh || std::fs::create_dir_all(&st.paths.cache_dir).and_then(|_| std::fs::write(&path, GUIDE)).is_ok();
        if written {
            st.open_path(&path);
        } else {
            st.notify_error(format!("Couldn't write {}", path.display()));
        }
    }
}

/// Built-in metadata sources: (name, what it does, settings tab).
const BUILTIN_METADATA: [(&str, &str); 3] = [
    ("Steam store", "Covers, descriptions and details of Steam games; no account needed"),
    ("SteamGridDB", "Artwork for every game, with your free API key"),
    ("ProtonDB", "How well Windows games run on Linux; no account needed"),
];

/// Returns (config changed, outcome).
fn installed_page(
    ui: &mut Ui,
    st: &mut AppState,
    d: &mut AddonsDlg,
    pal: &Pal,
    max_h: f32,
) -> (bool, Option<AddonsOutcome>) {
    let mut ch = false;
    let mut out = None;
    // ---- toolbar
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut d.search).hint_text("Search add-ons").desired_width(220.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icon_button(ui, Icon::Folder, false, 28.0, "Open the add-ons folder").clicked() {
                let dir = st.addons.addons_dir().to_path_buf();
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    st.notify_error(format!("Couldn't create {}: {e}", dir.display()));
                } else {
                    st.open_path(&dir);
                }
            }
            let busy = d.job.is_some() || d.pick.is_some();
            if ui
                .add_enabled(!busy, egui::Button::new("Install from .zip…"))
                .on_hover_text("A .zip with an addon.toml inside (it is checked, never run)")
                .clicked()
            {
                d.pick = Some((
                    PickFor::InstallZip,
                    Pick::start(ui.ctx(), PickKind::Zip, "Install add-on from a .zip", None),
                ));
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Install from folder…"))
                .on_hover_text("A folder with an addon.toml (copied; it is checked, never run)")
                .clicked()
            {
                d.pick = Some((
                    PickFor::InstallFolder,
                    Pick::start(ui.ctx(), PickKind::Folder, "Install add-on from a folder", None),
                ));
            }
        });
    });
    ui.add_space(8.0);
    if let Some((label, _)) = &d.job {
        ui.horizontal(|ui| {
            lazy_spinner(ui, 16.0, pal.dim);
            ui.label(RichText::new(label).color(pal.dim));
        });
        ui.add_space(6.0);
    }
    if let Some((text, err)) = d.banner.clone() {
        let tint = if err { pal.danger } else { pal.accent };
        egui::Frame::new()
            .fill(with_alpha(tint, 26))
            .stroke(egui::Stroke::new(1.0, with_alpha(tint, 70)))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(10, 7))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let icon = if err { Icon::Warning } else { Icon::CheckCircle };
                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    paint_icon(ui.painter(), r, icon, if err { pal.danger } else { pal.text });
                    ui.add(egui::Label::new(RichText::new(text).size(13.0).color(pal.text)).wrap());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icon_button(ui, Icon::Close, false, 20.0, "Dismiss").clicked() {
                            d.banner = None;
                        }
                    });
                });
            });
        ui.add_space(8.0);
    }

    let q = d.search.trim().to_lowercase();
    let infos: Vec<AddonInfo> = st
        .addons
        .list()
        .iter()
        .filter(|a| match d.section {
            Section::Kind(k) => a.manifest.kind == k,
            Section::BuiltIn => false,
            _ => true,
        })
        .filter(|a| matches_search(a, &q))
        .cloned()
        .collect();
    let show_builtin_libs = matches!(d.section, Section::All | Section::BuiltIn | Section::Kind(AddonKind::Library));
    let show_builtin_meta = matches!(d.section, Section::All | Section::BuiltIn | Section::Kind(AddonKind::Metadata));
    let scan_addon_error = st
        .scan
        .last
        .as_ref()
        .and_then(|l| l.per_source.iter().find(|(s, _, _)| *s == Source::Addon).and_then(|(_, _, e)| e.clone()));

    egui::ScrollArea::vertical().id_salt("ls_addons_scroll").max_height(max_h - 50.0).auto_shrink([false, false]).show(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            if d.section != Section::BuiltIn {
                if infos.is_empty() {
                    let any = !st.addons.list().is_empty();
                    empty_card(ui, st, pal, any, &q);
                }
                for info in &infos {
                    let r = addon_card(ui, st, d, info, pal, scan_addon_error.as_deref());
                    ch |= r;
                }
            }
            if show_builtin_libs || show_builtin_meta {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("BUILT IN").size(11.0).strong().color(pal.dim).extra_letter_spacing(0.8));
                    ui.label(RichText::new("Part of Lodestar; nothing to install.").size(12.0).color(pal.dim));
                });
                if show_builtin_libs && let Some(o) = builtin_card(ui, st, pal, &q, true) {
                    out = Some(o);
                }
                if show_builtin_meta && let Some(o) = builtin_card(ui, st, pal, &q, false) {
                    out = Some(o);
                }
            }
        },
    );
    (ch, out)
}

fn card_frame(pal: &Pal, selected: bool) -> egui::Frame {
    egui::Frame::new()
        .fill(if selected { mix(pal.surface, pal.surface_alt, 0.9) } else { mix(pal.surface, pal.surface_alt, 0.55) })
        .stroke(egui::Stroke::new(
            1.0,
            if selected { with_alpha(pal.selection, 120) } else { with_alpha(pal.border, 160) },
        ))
        .corner_radius(8)
        .inner_margin(egui::Margin::same(12))
}

fn empty_card(ui: &mut Ui, st: &mut AppState, pal: &Pal, any: bool, q: &str) {
    card_frame(pal, false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
            ui.painter().rect_filled(r, 10.0, with_alpha(pal.text, 12));
            paint_icon(ui.painter(), r.shrink(12.0), Icon::Puzzle, pal.dim);
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                let (title, body) = if !q.is_empty() {
                    ("No add-ons match", "Try another word.".to_string())
                } else if any {
                    ("None of this kind", "Installed add-ons of other kinds are under Installed.".to_string())
                } else {
                    (
                        "No add-ons installed",
                        "Install one from a folder or a .zip (buttons above). Lodestar checks \
                         it and copies it into its add-ons folder; nothing runs until you \
                         switch it on."
                            .to_string(),
                    )
                };
                ui.label(RichText::new(title).size(14.5).strong().color(pal.text));
                ui.label(RichText::new(body).size(12.5).color(pal.dim));
                if !any && q.is_empty() {
                    guide_link(ui, st, pal);
                }
            });
        });
    });
}

/// One add-on card as of this frame.
struct Card<'a> {
    info: &'a AddonInfo,
    id: String,
    /// Expanded (its details shown).
    selected: bool,
    enabled: bool,
    usable: bool,
    /// The newest problem: broken manifest, else the last call's error, else the scan's.
    problem: Option<String>,
}

/// One external add-on. Returns true when the config changed.
fn addon_card(
    ui: &mut Ui,
    st: &mut AppState,
    d: &mut AddonsDlg,
    info: &AddonInfo,
    pal: &Pal,
    scan_error: Option<&str>,
) -> bool {
    let id = info.manifest.id.clone();
    let enabled = crate::addons::is_enabled(&id, &st.config.addons);
    let last_error = st.addons.last_error(&id).or_else(|| info.last_error.clone());
    let problem: Option<String> = info
        .error
        .clone()
        .or(last_error)
        .or_else(|| scan_error.filter(|e| enabled && e.contains(&id)).map(str::to_string));
    let c = Card {
        info,
        selected: d.selected.as_deref() == Some(id.as_str()),
        enabled,
        usable: info.is_usable(),
        problem,
        id,
    };
    let mut ch = false;
    card_frame(pal, c.selected).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ch |= card_header(ui, st, d, &c, pal);
        if d.confirm_enable.as_deref() == Some(c.id.as_str()) {
            ch |= enable_prompt(ui, st, d, &c, pal);
        }
        if c.selected {
            ch |= card_details(ui, st, d, &c, pal);
        }
    });
    ch
}

/// Tile, name, version, kind, author, description, problem and the on / off switch; a click on
/// it expands the card. True when the config changed.
fn card_header(ui: &mut Ui, st: &mut AppState, d: &mut AddonsDlg, c: &Card, pal: &Pal) -> bool {
    let m = &c.info.manifest;
    let id = &c.id;
    let mut ch = false;
    let top = ui.horizontal_top(|ui| {
        let (tr, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
        initials_tile(ui.painter(), tr, &m.name, pal.surface_alt, pal.text);
        ui.add_space(6.0);
        let text_w = ui.available_width() - 70.0;
        let text = ui.allocate_ui_with_layout(vec2(text_w, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new(&m.name).size(14.5).strong().color(pal.text));
                if !m.version.is_empty() {
                    ui.label(RichText::new(&m.version).size(12.0).color(pal.dim));
                }
                kind_chip(ui, m.kind.label(), pal);
            });
            let by = if m.author.is_empty() { m.id.clone() } else { format!("by {}", m.author) };
            ui.label(RichText::new(by).size(12.0).color(pal.dim));
            if !m.description.is_empty() {
                ui.add(
                    egui::Label::new(RichText::new(&m.description).size(12.5).color(mix(pal.text, pal.dim, 0.3)))
                        .wrap(),
                );
            }
            if let Some(e) = &c.problem {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                    paint_icon(ui.painter(), r, Icon::Warning, pal.danger);
                    let short: String = e.lines().next().unwrap_or("").chars().take(160).collect();
                    ui.add(egui::Label::new(RichText::new(short).size(12.0).color(pal.danger)).wrap())
                        .on_hover_text(e.as_str());
                });
            }
        });
        let header_rect = tr.union(text.response.rect);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            let mut on = c.enabled;
            let r = ui.add_enabled_ui(c.usable, |ui| toggle_switch(ui, &mut on, 20.0, pal.accent)).inner;
            let r = if c.usable {
                r.on_hover_text(if c.enabled { "On. Click to switch it off." } else { "Off. Click to switch it on." })
            } else {
                r.on_disabled_hover_text("This add-on has a problem (see below the name).")
            };
            if r.changed() {
                if on && !acknowledged(st, c.info) {
                    d.confirm_enable = Some(id.clone());
                    d.selected = Some(id.clone());
                } else {
                    set_enabled(st, id, on);
                    ch = true;
                    mark_library_change_kind(d, m.kind);
                }
            }
        });
        header_rect
    });
    let header = ui.interact(top.inner, ui.scope_id().with(("ls_addon_hdr", id)), Sense::click());
    let header = match &c.problem {
        Some(e) => header.on_hover_text(e.as_str()),
        None => header,
    };
    if header.clicked() {
        d.selected = if c.selected { None } else { Some(id.clone()) };
        d.confirm_remove = None;
    }
    if header.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ch
}

/// "Switch on …?" with what the add-on says it needs. True when it was switched on.
fn enable_prompt(ui: &mut Ui, st: &mut AppState, d: &mut AddonsDlg, c: &Card, pal: &Pal) -> bool {
    let m = &c.info.manifest;
    let mut ch = false;
    ui.add_space(10.0);
    egui::Frame::new()
        .fill(with_alpha(pal.accent, 22))
        .stroke(egui::Stroke::new(1.0, with_alpha(pal.accent, 80)))
        .corner_radius(6)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(format!("Switch on {}?", m.name)).size(14.0).strong().color(pal.text));
            let needs = if m.permissions.is_empty() { "It asks for nothing special." } else { "It says it needs:" };
            ui.label(RichText::new(needs).size(12.5).color(pal.dim));
            for p in &m.permissions {
                ui.label(RichText::new(format!("•  {p}")).size(13.0).color(pal.text));
            }
            ui.label(
                RichText::new("It runs as you, like any program you start. Only switch on add-ons you trust.")
                    .size(12.0)
                    .color(pal.dim),
            );
            ui.add_space(6.0);
            let f = button_row(ui, pal, Buttons::primary("Switch on", true).cancel("Cancel"), |_| {});
            if f.primary {
                if let Err(e) = st.db.kv_set(&ack_key(&c.id), &perms_fingerprint(c.info)) {
                    log::warn!("remembering that add-on {} was allowed: {e:#}", c.id);
                }
                set_enabled(st, &c.id, true);
                ch = true;
                d.confirm_enable = None;
                mark_library_change_kind(d, m.kind);
            }
            if f.cancel {
                d.confirm_enable = None;
            }
        });
    ch
}

/// An expanded card: permissions, the add-on's settings, its last call, Remove / Open folder.
/// True when the config changed.
fn card_details(ui: &mut Ui, st: &mut AppState, d: &mut AddonsDlg, c: &Card, pal: &Pal) -> bool {
    let m = &c.info.manifest;
    let id = &c.id;
    let mut ch = false;
    ui.add_space(10.0);
    ui.separator();
    ui.add_space(4.0);
    // (Already listed in the prompt while it is open.)
    if d.confirm_enable.as_deref() != Some(id.as_str()) {
        ui.label(RichText::new("Permissions").size(13.0).strong().color(pal.text));
        if m.permissions.is_empty() {
            hint_text(ui, "Asks for nothing special.", pal);
        } else {
            for p in &m.permissions {
                ui.label(RichText::new(format!("•  {p}")).size(12.5).color(mix(pal.text, pal.dim, 0.2)));
            }
        }
    }
    if !m.settings.is_empty() {
        ui.add_space(8.0);
        ui.label(RichText::new("Settings").size(13.0).strong().color(pal.text));
        if settings_form(ui, st, d, c.info, pal) {
            ch = true;
            mark_library_change_kind_if_on(d, m.kind, c.enabled);
        }
    }
    if let Some(s) = st.addons.status(id) {
        ui.add_space(6.0);
        let ago = s.at.elapsed().as_secs();
        let when = if ago < 60 { "just now".to_string() } else { format!("{} min ago", ago / 60) };
        let txt = match &s.error {
            None => format!("Last call: {} — ok in {} ms, {when}", s.command, s.took.as_millis()),
            Some(_) => format!("Last call: {} — failed, {when}", s.command),
        };
        hint_text(ui, &txt, pal);
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if d.confirm_remove.as_deref() == Some(id.as_str()) {
                if ui
                    .add(
                        egui::Button::new(RichText::new("Remove").color(egui::Color32::WHITE).strong())
                            .fill(pal.danger),
                    )
                    .clicked()
                {
                    d.confirm_remove = None;
                    start_remove(st, d, id.clone());
                }
                if ui.button("Cancel").clicked() {
                    d.confirm_remove = None;
                }
                ui.label(RichText::new("Delete its folder? Settings are kept.").size(12.5).color(pal.text));
            } else {
                if ui.add_enabled(d.job.is_none(), egui::Button::new("Remove…")).clicked() {
                    d.confirm_remove = Some(id.clone());
                }
                if ui.button("Open folder").on_hover_text(c.info.dir.display().to_string()).clicked() {
                    st.open_path(&c.info.dir);
                }
            }
        });
    });
    ch
}

fn mark_library_change_kind(d: &mut AddonsDlg, kind: AddonKind) {
    if kind == AddonKind::Library {
        d.library_changed = true;
    }
}

fn mark_library_change_kind_if_on(d: &mut AddonsDlg, kind: AddonKind, on: bool) {
    if on {
        mark_library_change_kind(d, kind);
    }
}

fn kind_chip(ui: &mut Ui, label: &str, pal: &Pal) {
    let tint = mix(pal.text, pal.dim, 0.35);
    let g = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(11.0), tint);
    let (r, _) = ui.allocate_exact_size(vec2(g.size().x + 14.0, 18.0), Sense::hover());
    ui.painter().rect_filled(r, 9.0, with_alpha(tint, 22));
    ui.painter().galley(r.center() - g.size() / 2.0, g, tint);
}

/// The manifest's fields. Returns true when a value changed.
fn settings_form(ui: &mut Ui, st: &mut AppState, d: &mut AddonsDlg, info: &AddonInfo, pal: &Pal) -> bool {
    let id = &info.manifest.id;
    let mut ch = false;
    let mut pick_field: Option<(String, String)> = None;
    egui::Grid::new(("ls_addon_settings", id)).num_columns(2).spacing([14.0, 8.0]).min_col_width(120.0).show(
        ui,
        |ui| {
            for f in &info.manifest.settings {
                field_row(ui, st, d, id, f, pal, &mut ch, &mut pick_field);
                ui.end_row();
            }
        },
    );
    if let Some((id, key)) = pick_field {
        let start = st.config.addons.settings.get(&id).and_then(|m| m.get(&key)).map(PathBuf::from);
        d.pick = Some((PickFor::Field(id, key), Pick::start(ui.ctx(), PickKind::Folder, "Choose a folder", start)));
    }
    ch
}

#[allow(clippy::too_many_arguments)]
fn field_row(
    ui: &mut Ui,
    st: &mut AppState,
    d: &mut AddonsDlg,
    id: &str,
    f: &SettingField,
    pal: &Pal,
    ch: &mut bool,
    pick_field: &mut Option<(String, String)>,
) {
    let label = if f.label.is_empty() { &f.key } else { &f.label };
    let lr = ui.label(RichText::new(label).size(13.0).color(pal.text));
    if !f.help.is_empty() {
        lr.on_hover_text(&f.help);
    }
    let current =
        st.config.addons.settings.get(id).and_then(|m| m.get(&f.key)).cloned().unwrap_or_else(|| f.default.clone());
    let mut value = current.clone();
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        match f.kind {
            SettingKind::Toggle => {
                let mut on = matches!(value.trim(), "true" | "1" | "yes" | "on");
                if toggle_switch(ui, &mut on, 18.0, pal.accent).changed() {
                    value = if on { "true" } else { "false" }.into();
                }
            }
            SettingKind::Text => {
                ui.add(egui::TextEdit::singleline(&mut value).desired_width(320.0));
            }
            SettingKind::Secret => {
                let k = (id.to_string(), f.key.clone());
                let shown = d.shown_secrets.contains(&k);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut value)
                            .password(!shown)
                            .desired_width(270.0)
                            .font(egui::TextStyle::Monospace),
                    );
                    if ui.small_button(if shown { "Hide" } else { "Show" }).clicked() {
                        if shown {
                            d.shown_secrets.remove(&k);
                        } else {
                            d.shown_secrets.insert(k);
                        }
                    }
                });
            }
            SettingKind::Folder => {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut value).desired_width(250.0).font(egui::TextStyle::Monospace),
                    );
                    let picking = matches!(&d.pick, Some((PickFor::Field(a, b), _)) if a == id && *b == f.key);
                    if ui.add_enabled(!picking, egui::Button::new("Browse…")).clicked() {
                        *pick_field = Some((id.to_string(), f.key.clone()));
                    }
                });
            }
        }
        if !f.help.is_empty() {
            ui.label(RichText::new(&f.help).size(11.5).color(pal.dim));
        }
    });
    if value != current {
        st.config.addons.settings.entry(id.to_string()).or_default().insert(f.key.clone(), value);
        *ch = true;
    }
}

/// The built-in libraries (or metadata sources) as one card of compact rows.
fn builtin_card(ui: &mut Ui, st: &AppState, pal: &Pal, q: &str, libraries: bool) -> Option<AddonsOutcome> {
    let mut out = None;
    let last = st.scan.last.as_ref().map(|l| l.per_source.as_slice());
    let rows: Vec<(String, String, String)> = if libraries {
        super::libraries::builtin_libraries()
            .filter(|(_, name, via)| q.is_empty() || name.to_lowercase().contains(q) || via.to_lowercase().contains(q))
            .map(|(s, name, via)| {
                let on = st.config.source_enabled(s);
                let status = match (on, last.and_then(|l| l.iter().find(|x| x.0 == s))) {
                    (false, _) => "Off".to_string(),
                    (true, Some((_, _, Some(_)))) => "Error".to_string(),
                    (true, Some((_, 0, None))) => "Not found".to_string(),
                    (true, Some((_, n, None))) => {
                        if *n == 1 {
                            "1 game".into()
                        } else {
                            format!("{n} games")
                        }
                    }
                    (true, None) => "On".to_string(),
                };
                (name.to_string(), via.to_string(), status)
            })
            .collect()
    } else {
        BUILTIN_METADATA
            .iter()
            .filter(|(name, what)| q.is_empty() || name.to_lowercase().contains(q) || what.to_lowercase().contains(q))
            .map(|(name, what)| {
                let status = match *name {
                    "Steam store" => {
                        if st.config.metadata.steam_store {
                            "On"
                        } else {
                            "Off"
                        }
                    }
                    "SteamGridDB" => {
                        if st.config.metadata.steamgriddb_key.as_deref().is_some_and(|k| !k.trim().is_empty()) {
                            "On"
                        } else {
                            "Needs a key"
                        }
                    }
                    _ => "On",
                };
                (name.to_string(), what.to_string(), status.to_string())
            })
            .collect()
    };
    if rows.is_empty() {
        return None;
    }
    card_frame(pal, false).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(if libraries { "Libraries" } else { "Metadata sources" })
                    .size(14.0)
                    .strong()
                    .color(pal.text),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (label, tab) = if libraries {
                    ("Settings → Libraries", super::settings::Tab::Library)
                } else {
                    ("Settings → Metadata", super::settings::Tab::Metadata)
                };
                if ui.button(label).on_hover_text("Switch them on or off, add paths, keys").clicked() {
                    out = Some(AddonsOutcome::OpenSettings(tab));
                }
            });
        });
        ui.add_space(2.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        for (name, via, status) in rows {
            let h = 34.0;
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
            let p = ui.painter();
            let tile = egui::Rect::from_min_size(pos2(r.left(), r.center().y - 12.0), vec2(24.0, 24.0));
            initials_tile(p, tile, &name, pal.surface_alt, pal.text);
            let off = status == "Off";
            let sc = match status.as_str() {
                "Error" => pal.danger,
                "Off" | "Not found" | "Needs a key" => pal.dim,
                _ => mix(pal.text, pal.dim, 0.2),
            };
            let sg = p.layout_no_wrap(status, egui::FontId::proportional(12.0), sc);
            let sx = r.right() - sg.size().x;
            p.galley(pos2(sx, r.center().y - sg.size().y / 2.0), sg, sc);
            let x = tile.right() + 10.0;
            let avail = (sx - 16.0 - x).max(40.0);
            let nc = if off { pal.dim } else { pal.text };
            let ng = crate::ui::widgets::truncated(p, &name, egui::FontId::proportional(13.5), nc, avail * 0.45);
            let nw = ng.size().x;
            p.galley(pos2(x, r.center().y - ng.size().y / 2.0), ng, nc);
            let vg =
                crate::ui::widgets::truncated(p, &via, egui::FontId::proportional(12.0), pal.dim, avail - nw - 12.0);
            p.galley(pos2(x + nw + 12.0, r.center().y - vg.size().y / 2.0), vg, pal.dim);
        }
    });
    out
}
