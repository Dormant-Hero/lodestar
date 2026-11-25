//! Add-ons → Desktop themes / Controller themes: the installed themes (built-in marked, the
//! others with where they came from) with Use / Open folder / Remove, Playnite theme imports
//! (themes found on this machine get an Import button; file / folder pickers as the manual way), and the install / import job shared with Browse and Updates,
//! ending in a report window (what came along, what couldn't, "Use this theme").

use super::super::Pal;
use super::super::add::{Pick, PickKind};
use super::super::dialogs::{Buttons, button_row, footer, hint_text, modal};
use super::{Section, card_frame, kind_chip};
use crate::app::AppState;
use crate::theme::catalog::install::{self, Progress, Report};
use crate::theme::catalog::installed::{self, From, InstalledTheme};
use crate::theme::catalog::{self, Entry, Index};
use crate::theme::playnite::{self, Found, Mode};
use crate::ui::widgets::{Icon, icon_button, lazy_spinner, mix, paint_icon, with_alpha};
use crossbeam_channel::{Receiver, TryRecvError};
use egui::{Align, Layout, RichText, Sense, Ui, vec2};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// Which theme page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    Desktop,
    Controller,
}

enum Done {
    Reports(Vec<Result<Report, String>>),
    /// (key, name, result)
    Removed(String, String, Result<(), String>),
}

pub(crate) struct Job {
    pub label: String,
    pub progress: Arc<Progress>,
    rx: Receiver<Done>,
    /// The list entries being installed (Browse / Updates show progress on them).
    pub entries: Vec<(Index, String)>,
}

#[derive(Default)]
pub(crate) struct Themes {
    /// Installed themes with origins (listed on a helper thread).
    pub list: Option<Vec<InstalledTheme>>,
    list_rx: Option<Receiver<Vec<InstalledTheme>>>,
    pn_found: Option<Vec<Found>>,
    pn_rx: Option<Receiver<Vec<Found>>>,
    /// The Playnite theme picker while it is open.
    pick: Option<Pick>,
    pub job: Option<Job>,
    /// The finished job's results, shown in the report window until closed.
    pub reports: Vec<Result<Report, String>>,
    /// The reports come from installing list themes (Browse / Updates), not a hand import.
    reports_from_list: bool,
    /// A removal / job error shown above the list (Browse shows it too).
    pub(super) banner: Option<(String, bool)>,
    confirm_remove: Option<String>,
    /// The remove confirmation was just opened: scroll it into view once.
    confirm_scroll: bool,
    search: String,
    /// Import… row open.
    pub(crate) show_import: bool,
    /// Bumped whenever the installed list changes (Updates re-checks).
    pub generation: u64,
    /// Open the theme creator (on a user theme's key, or its start).
    pub open_creator: Option<Option<String>>,
}

/// A job asked for while another one runs (one theme job at a time).
pub(super) const BUSY: &str = "Another theme job is still running — try again in a moment.";

fn work_dir(st: &AppState) -> PathBuf {
    catalog::cache_dir(&st.paths.cache_dir).join("downloads")
}

impl Themes {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    pub fn installing(&self, index: Index, id: &str) -> bool {
        self.job.as_ref().is_some_and(|j| j.entries.iter().any(|(i, e)| *i == index && e == id))
    }

    /// (Re)list the installed themes on a helper thread.
    pub fn refresh_list(&mut self, st: &AppState) {
        let infos = st.themes.list().to_vec();
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = st.ctx().clone();
        let spawned = std::thread::Builder::new().name("theme-list".into()).spawn(move || {
            let _ = tx.send(installed::list(&infos));
            ctx.request_repaint();
        });
        if spawned.is_ok() {
            self.list_rx = Some(rx);
        }
    }

    fn discover(&mut self, st: &AppState) {
        if self.pn_found.is_none() && self.pn_rx.is_none() {
            let (tx, rx) = crossbeam_channel::bounded(1);
            let home = st.paths.home.clone();
            let ctx = st.ctx().clone();
            if std::thread::Builder::new()
                .name("playnite-themes".into())
                .spawn(move || {
                    let _ = tx.send(playnite::discover(&home));
                    ctx.request_repaint();
                })
                .is_ok()
            {
                self.pn_rx = Some(rx);
            } else {
                self.pn_found = Some(vec![]);
            }
        }
    }

    /// Start a job on the theme-job thread. False (with a message above the list) when it
    /// couldn't start, e.g. while another job still runs: the caller then changes nothing.
    fn spawn(
        &mut self,
        st: &AppState,
        label: String,
        entries: Vec<(Index, String)>,
        work: impl FnOnce(&Progress) -> Done + Send + 'static,
    ) -> bool {
        if self.job.is_some() {
            self.banner = Some((BUSY.into(), true));
            return false;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        let progress = Arc::new(Progress::default());
        let p = progress.clone();
        let ctx = st.ctx().clone();
        // Repaint now and then while it runs (download progress).
        let ticker = ctx.clone();
        let spawned = std::thread::Builder::new().name("theme-job".into()).spawn(move || {
            let done = work(&p);
            let _ = tx.send(done);
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => {
                ticker.request_repaint();
                self.banner = None;
                self.job = Some(Job { label, progress, rx, entries });
                true
            }
            Err(e) => {
                self.banner = Some((format!("Couldn't start: {e}"), true));
                false
            }
        }
    }

    /// Install list entries one after another (Browse → Install, Updates → Update / Update all).
    pub fn start_install(&mut self, st: &AppState, entries: Vec<Entry>) {
        if entries.is_empty() {
            return;
        }
        let label = if entries.len() == 1 {
            format!("Installing {}…", entries[0].name)
        } else {
            format!("Updating {} themes…", entries.len())
        };
        let ids = entries.iter().map(|e| (e.index, e.id.clone())).collect();
        let themes_dir = st.themes.themes_dir().to_path_buf();
        let work = work_dir(st);
        self.spawn(st, label, ids, move |p| {
            let mut out = vec![];
            for e in &entries {
                if p.cancel.load(Ordering::Relaxed) {
                    break;
                }
                p.bytes.store(0, Ordering::Relaxed);
                out.push(install::install(e, &themes_dir, &work, p).map_err(|err| format!("{}: {err:#}", e.name)));
            }
            Done::Reports(out)
        });
    }

    /// Import a Playnite theme package / folder (also the testing hook `playnite-import:<path>`).
    pub fn start_import_playnite(&mut self, st: &AppState, src: PathBuf) {
        let themes_dir = st.themes.themes_dir().to_path_buf();
        let label = format!("Importing {}…", file_label(&src));
        self.spawn(st, label, vec![], move |p| {
            Done::Reports(vec![install::import_playnite(&src, &themes_dir, p).map_err(|e| format!("{e:#}"))])
        });
    }

    /// Import the report's Playnite theme again with other choices (layout, videos and sounds).
    fn start_reimport(&mut self, st: &AppState, prev: Report, opts: playnite::ImportOptions) {
        let themes_dir = st.themes.themes_dir().to_path_buf();
        let label = match opts.family {
            Some(f) if Some(f) != prev.playnite.as_ref().and_then(|r| r.family) => {
                format!("Trying {}…", f.label().to_lowercase())
            }
            _ if opts.media == Some(true) => "Adding its videos and sounds…".to_string(),
            _ if opts.media == Some(false) => "Leaving out its videos and sounds…".to_string(),
            _ => "Importing again…".to_string(),
        };
        let from_list = self.reports_from_list;
        self.spawn(st, label, vec![], move |p| {
            Done::Reports(vec![install::reimport_playnite(&prev, &opts, &themes_dir, p).map_err(|e| format!("{e:#}"))])
        });
        self.reports_from_list = from_list;
    }

    pub(super) fn start_remove(&mut self, st: &mut AppState, key: String, name: String) {
        let themes_dir = st.themes.themes_dir().to_path_buf();
        let k = key.clone();
        let started = self.spawn(st, "Removing…".into(), vec![], move |_| {
            let r = installed::remove(&themes_dir, &k).map_err(|e| format!("{e:#}"));
            Done::Removed(k, name, r)
        });
        // Never leave a removed theme in use, but switch only once the removal really runs:
        // switching while another job blocked it left the theme installed and no longer in use.
        if started {
            if st.config.desktop_theme == key {
                st.set_desktop_theme("Lodestar");
            }
            if st.config.controller.theme == key {
                st.set_controller_theme("Lodestar");
            }
        }
    }

    /// Every frame the Add-ons window is open (whatever page is shown).
    pub fn poll(&mut self, st: &mut AppState) {
        if st.no_launch {
            // Screenshot runs: Install / Update clicks never reach the internet.
            catalog::net::set_local_only();
        }
        if self.list.is_none() && self.list_rx.is_none() {
            self.refresh_list(st);
        }
        if let Some(rx) = &self.list_rx
            && let Ok(l) = rx.try_recv()
        {
            self.list = Some(l);
            self.list_rx = None;
            self.generation += 1;
        }
        if let Some(rx) = &self.pn_rx
            && let Ok(f) = rx.try_recv()
        {
            self.pn_found = Some(f);
            self.pn_rx = None;
        }
        if let Some(p) = &self.pick
            && let Some(paths) = p.poll()
        {
            self.pick = None;
            if let Some(path) = paths.into_iter().next() {
                self.start_import_playnite(st, path);
            }
        }
        if let Some(job) = &self.job {
            let done = match job.rx.try_recv() {
                Ok(d) => Some(d),
                Err(TryRecvError::Empty) => {
                    // Download progress: repaint a few times a second.
                    st.ctx().request_repaint_after(std::time::Duration::from_millis(250));
                    None
                }
                Err(TryRecvError::Disconnected) => {
                    Some(Done::Reports(vec![Err("the job stopped unexpectedly".into())]))
                }
            };
            if let Some(done) = done {
                let rerun = self.job.as_ref().is_some_and(|j| j.entries.is_empty()) && !self.reports.is_empty();
                let from_list =
                    self.job.take().is_some_and(|j| !j.entries.is_empty()) || (rerun && self.reports_from_list);
                st.themes.refresh();
                match done {
                    Done::Reports(reports) => {
                        for rep in reports.iter().flatten() {
                            log::info!("theme {} ({}) installed as {}", rep.name, rep.what, rep.key);
                            // A re-install of a theme in use shows right away.
                            if st.config.desktop_theme == rep.key {
                                st.set_desktop_theme(&rep.key);
                            }
                            if st.config.controller.theme == rep.key {
                                st.set_controller_theme(&rep.key);
                            }
                        }
                        let cancelled = reports.iter().any(|r| r.as_ref().is_err_and(|e| e.ends_with("cancelled")));
                        // A failed re-import replaces its report: the download kept for it goes.
                        if rerun && !reports.iter().any(|r| r.is_ok()) {
                            for r in self.reports.drain(..).flatten() {
                                install::discard(&r);
                            }
                        }
                        if !cancelled || reports.iter().any(|r| r.is_ok()) {
                            self.reports = reports;
                            self.reports_from_list = from_list;
                        }
                    }
                    Done::Removed(key, name, Ok(())) => {
                        log::info!("theme {key} removed");
                        self.banner = Some((format!("Removed “{name}”."), false));
                    }
                    Done::Removed(_, name, Err(e)) => {
                        self.banner = Some((format!("Couldn't remove “{name}”: {e}"), true));
                    }
                }
                self.refresh_list(st);
            }
        }
    }
}

/// A job left running when the Add-ons window closed: polled every frame; when it ends the
/// result becomes a notice. Returns false once it's done.
pub(crate) fn poll_background(st: &mut AppState, t: &mut Themes) -> bool {
    t.poll(st);
    if t.job.is_some() {
        return true;
    }
    for r in t.reports.drain(..) {
        if let Ok(rep) = &r {
            install::discard(rep);
        }
        match r {
            Ok(rep) => {
                let page = if rep.controller { "Controller themes" } else { "Desktop themes" };
                st.notify_long(format!("Theme “{}” is ready — use it in Add-ons → {page}", rep.name));
            }
            Err(e) => st.notify_error(format!("Theme install failed: {e}")),
        }
    }
    false
}

fn file_label(p: &std::path::Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The running job: spinner, what it does, MB downloaded, Cancel.
pub(super) fn job_line(ui: &mut Ui, t: &Themes, pal: &Pal) {
    let Some(job) = &t.job else { return };
    ui.horizontal(|ui| {
        lazy_spinner(ui, 16.0, pal.dim);
        let stage = job.progress.stage.lock().clone();
        let mb = job.progress.bytes.load(Ordering::Relaxed) as f64 / 1_048_576.0;
        let mut text = job.label.clone();
        if !stage.is_empty() {
            text.push_str(&format!("  {stage}"));
        }
        if mb >= 0.1 && stage.starts_with("Downloading") {
            text.push_str(&format!(" {mb:.1} MB"));
        }
        ui.label(RichText::new(text).color(pal.dim));
        if !job.entries.is_empty() && ui.small_button("Cancel").clicked() {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
    });
    ui.add_space(6.0);
}

pub(super) fn banner(ui: &mut Ui, text: &str, err: bool, pal: &Pal) -> bool {
    let tint = if err { pal.danger } else { pal.accent };
    let mut dismiss = false;
    egui::Frame::new()
        .fill(with_alpha(tint, 26))
        .stroke(egui::Stroke::new(1.0, with_alpha(tint, 70)))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let icon = if err { Icon::Warning } else { Icon::InfoCircle };
                let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                paint_icon(ui.painter(), r, icon, if err { pal.danger } else { pal.text });
                ui.add(egui::Label::new(RichText::new(text).size(12.5).color(pal.text)).wrap());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if icon_button(ui, Icon::Close, false, 20.0, "Dismiss").clicked() {
                        dismiss = true;
                    }
                });
            });
        });
    ui.add_space(8.0);
    dismiss
}

/// A theme's colours as a small tile: background, a card, the accent and a text bar.
fn swatch(ui: &mut Ui, t: &InstalledTheme) {
    let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
    let p = ui.painter();
    let [bg, card, accent, text] = t.swatch.map(|c| c.to_egui());
    p.rect_filled(r, 9.0, bg);
    p.rect_stroke(r, 9.0, egui::Stroke::new(1.0, with_alpha(text, 40)), egui::StrokeKind::Inside);
    let inner = egui::Rect::from_min_size(r.min + vec2(7.0, 8.0), vec2(30.0, 20.0));
    p.rect_filled(inner, 4.0, card);
    p.rect_filled(egui::Rect::from_min_size(inner.min + vec2(4.0, 5.0), vec2(16.0, 3.0)), 1.5, text);
    p.rect_filled(egui::Rect::from_min_size(inner.min + vec2(4.0, 11.0), vec2(10.0, 3.0)), 1.5, with_alpha(text, 120));
    p.rect_filled(egui::Rect::from_min_size(r.min + vec2(7.0, 32.0), vec2(30.0, 5.0)), 2.5, accent);
}

/// Which themes a page lists: (shown first, "also usable here").
fn split_for(page: Page, list: &[InstalledTheme]) -> (Vec<&InstalledTheme>, Vec<&InstalledTheme>) {
    match page {
        // Every theme colours the desktop; themes made for controller mode come second.
        Page::Desktop => list.iter().partition(|t| t.builtin() || !t.controller),
        Page::Controller => (list.iter().filter(|t| t.controller).collect(), vec![]),
    }
}

/// Desktop themes / Controller themes. Returns a section to switch to (Browse).
pub(super) fn page(
    ui: &mut Ui,
    st: &mut AppState,
    t: &mut Themes,
    page: Page,
    pal: &Pal,
    max_h: f32,
) -> Option<Section> {
    t.discover(st);
    let mut go = None;
    // ---- toolbar
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut t.search).hint_text("Search themes").desired_width(220.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icon_button(ui, Icon::Folder, false, 28.0, "Open the themes folder").clicked() {
                let dir = st.themes.themes_dir().to_path_buf();
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    st.notify_error(format!("Couldn't create {}: {e}", dir.display()));
                } else {
                    st.open_path(&dir);
                }
            }
            if page == Page::Controller
                && ui
                    .button("Create theme…")
                    .on_hover_text("Make your own controller theme in the theme creator")
                    .clicked()
            {
                t.open_creator = Some(None);
            }
            let r = ui.selectable_label(t.show_import, "Import…");
            if r.on_hover_text("Import a Playnite theme from a folder or file").clicked() {
                t.show_import = !t.show_import;
            }
            if ui
                .button("Get more themes")
                .on_hover_text("Browse community themes from Playnite's theme list")
                .clicked()
            {
                go = Some(Section::Browse);
            }
        });
    });
    if t.show_import {
        import_row(ui, t, page, pal);
    }
    ui.add_space(8.0);
    job_line(ui, t, pal);
    if let Some((text, err)) = t.banner.clone()
        && banner(ui, &text, err, pal)
    {
        t.banner = None;
    }

    let q = t.search.trim().to_lowercase();
    let list: Vec<InstalledTheme> = t.list.clone().unwrap_or_default();
    let shown: Vec<InstalledTheme> = list
        .into_iter()
        .filter(|x| q.is_empty() || x.name.to_lowercase().contains(&q) || x.author.to_lowercase().contains(&q))
        .collect();
    let (main, more) = split_for(page, &shown);
    egui::ScrollArea::vertical()
        .id_salt(("ls_addon_themes", page == Page::Desktop))
        .max_height(max_h - 50.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            if t.list.is_none() {
                ui.horizontal(|ui| {
                    lazy_spinner(ui, 16.0, pal.dim);
                    ui.label(RichText::new("Looking at the installed themes…").color(pal.dim));
                });
            } else if main.is_empty() && more.is_empty() {
                hint_text(ui, "No theme matches.", pal);
            }
            for th in &main {
                theme_card(ui, st, t, th, page, pal);
            }
            if !more.is_empty() {
                egui::CollapsingHeader::new(
                    RichText::new(format!("Controller themes can colour the desktop too ({})", more.len()))
                        .size(13.0)
                        .color(pal.dim),
                )
                .id_salt("ls_addon_themes_more")
                .show(ui, |ui| {
                    for th in &more {
                        theme_card(ui, st, t, th, page, pal);
                    }
                });
            }
            ui.add_space(8.0);
            found_section(ui, st, t, page, pal, &mut go);
        });
    go
}

/// Import… opened: the file / folder pickers (themes found on this machine are listed below).
fn import_row(ui: &mut Ui, t: &mut Themes, page: Page, pal: &Pal) {
    ui.add_space(6.0);
    egui::Frame::new()
        .fill(mix(pal.surface, pal.surface_alt, 0.55))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let busy = t.busy() || t.pick.is_some();
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let what =
                        if page == Page::Controller { "Playnite fullscreen theme…" } else { "Playnite theme…" };
                    if ui
                        .button(what)
                        .on_hover_text("A .pthm theme package, or the theme.yaml inside a theme folder")
                        .clicked()
                    {
                        t.pick = Some(Pick::start(
                            ui.ctx(),
                            PickKind::PlayniteTheme,
                            "Choose a Playnite theme (.pthm package or theme.yaml)",
                            None,
                        ));
                    }
                    if ui.button("Playnite theme folder…").clicked() {
                        t.pick = Some(Pick::start(ui.ctx(), PickKind::Folder, "Choose a Playnite theme folder", None));
                    }
                });
            });
            hint_text(ui, "Lodestar converts the theme into its own format; the original isn't changed.", pal);
        });
}

fn theme_card(ui: &mut Ui, st: &mut AppState, t: &mut Themes, th: &InstalledTheme, page: Page, pal: &Pal) {
    let in_use = match page {
        Page::Desktop => st.config.desktop_theme == th.key,
        Page::Controller => st.config.controller.theme == th.key,
    };
    let also = match page {
        Page::Desktop => th.controller && st.config.controller.theme == th.key,
        Page::Controller => st.config.desktop_theme == th.key,
    };
    let busy = t.busy();
    card_frame(pal, in_use).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            swatch(ui, th);
            ui.add_space(6.0);
            let text_w = (ui.available_width() - 190.0).max(120.0);
            ui.allocate_ui_with_layout(vec2(text_w, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(RichText::new(&th.name).size(14.5).strong().color(pal.text));
                    if !th.version.is_empty() && th.version != "1" && !th.builtin() {
                        let v = match &th.from {
                            From::Catalog(r) => r.version.clone(),
                            _ => th.version.clone(),
                        };
                        ui.label(RichText::new(v).size(12.0).color(pal.dim));
                    }
                    if th.builtin() {
                        kind_chip(ui, "Built in", pal);
                    }
                    if in_use {
                        kind_chip(ui, "In use", pal);
                    } else if also {
                        let other =
                            if page == Page::Desktop { "In use in controller mode" } else { "In use on the desktop" };
                        kind_chip(ui, other, pal);
                    }
                });
                let mut sub = th.origin_text();
                if th.builtin() {
                    sub = builtin_blurb(&th.name, th.description.trim(), page);
                } else if !th.author.is_empty() {
                    sub = format!("by {} · {sub}", th.author);
                }
                // Up to two lines: a one-line cut hid most of the built-in themes' descriptions.
                let mut job = egui::text::LayoutJob::simple(sub, egui::FontId::proportional(12.0), pal.dim, text_w);
                job.wrap.max_rows = 2;
                job.wrap.overflow_character = Some('…');
                ui.add(egui::Label::new(job)).on_hover_text(
                    th.dir.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| "Part of Lodestar".into()),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                ui.add_space(0.0);
                if let Some(dir) = &th.dir {
                    if icon_button(ui, Icon::Trash, false, 28.0, "Remove this theme").clicked() && !busy {
                        t.confirm_remove = Some(th.key.clone());
                        t.confirm_scroll = true;
                    }
                    if icon_button(ui, Icon::Folder, false, 28.0, "Open its folder").clicked() {
                        let d = dir.clone();
                        st.open_path(&d);
                    }
                    if th.controller
                        && icon_button(ui, Icon::Pencil, false, 28.0, "Edit it in the theme creator").clicked()
                    {
                        t.open_creator = Some(Some(th.key.clone()));
                    }
                }
                let label = if in_use { "In use" } else { "Use" };
                let r = ui.add_enabled(!in_use, egui::Button::new(label).min_size(vec2(64.0, 28.0)));
                let r = if page == Page::Controller {
                    r.on_hover_text("Use it in controller mode")
                } else {
                    r.on_hover_text("Use it for the desktop window")
                };
                if r.clicked() {
                    match page {
                        Page::Desktop => st.set_desktop_theme(&th.key),
                        Page::Controller => st.set_controller_theme(&th.key),
                    }
                    st.notify("Theme changed");
                }
            });
        });
        if t.confirm_remove.as_deref() == Some(th.key.as_str()) {
            ui.add_space(8.0);
            egui::Frame::new()
                .fill(with_alpha(pal.danger, 18))
                .stroke(egui::Stroke::new(1.0, with_alpha(pal.danger, 70)))
                .corner_radius(6)
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(format!("Remove “{}”?", th.name)).strong().color(pal.text));
                    let mut body = "Its folder is deleted from Lodestar's themes folder.".to_string();
                    if in_use || also {
                        body.push_str(" It's in use, so Lodestar switches back to its own theme.");
                    }
                    if !matches!(th.from, From::Own) {
                        body.push_str(" You can install or import it again any time.");
                    }
                    hint_text(ui, &body, pal);
                    let buttons = Buttons::primary("Remove", !busy).danger().disabled_hint(BUSY).cancel("Cancel");
                    let f = button_row(ui, pal, buttons, |_| {});
                    if f.primary {
                        t.start_remove(st, th.key.clone(), th.name.clone());
                    }
                    if f.primary || f.cancel {
                        t.confirm_remove = None;
                    }
                });
            if std::mem::take(&mut t.confirm_scroll) {
                ui.scroll_to_cursor(Some(Align::Max));
            }
        }
    });
}

/// A built-in theme's line on a page. The built-ins are controller layouts; on the desktop only
/// their colours and fonts apply, so the Desktop page says that instead of describing a layout
/// the desktop never shows (and Lodestar Ambient's difference is a desktop one).
fn builtin_blurb(name: &str, description: &str, page: Page) -> String {
    match (page, name) {
        (Page::Desktop, "Lodestar Ambient") => {
            "Lodestar's colours, with the selected game's art blended behind the window.".into()
        }
        (Page::Desktop, "Lodestar") => "Lodestar's own colours and fonts.".into(),
        (Page::Desktop, n) => format!("The colours and fonts of the {n} controller theme."),
        (Page::Controller, "Lodestar Ambient") => {
            "The Lodestar layout here; it differs from Lodestar on the desktop only.".into()
        }
        _ if description.is_empty() => "Part of Lodestar".into(),
        _ => description.to_string(),
    }
}

/// "Found on this machine": Playnite themes with an Import button each.
fn found_section(ui: &mut Ui, st: &mut AppState, t: &mut Themes, page: Page, pal: &Pal, go: &mut Option<Section>) {
    ui.label(RichText::new("FOUND ON THIS MACHINE").size(11.0).strong().color(pal.dim).extra_letter_spacing(0.8));
    let busy = t.busy() || t.pick.is_some();
    let looking = t.pn_found.is_none();
    let want = if page == Page::Controller { Mode::Fullscreen } else { Mode::Desktop };
    let pn: Vec<Found> = t.pn_found.clone().unwrap_or_default().into_iter().filter(|f| f.mode == want).collect();
    let mut import_pn = None;
    for f in &pn {
        let what = if f.mode == Mode::Fullscreen { "Playnite fullscreen theme" } else { "Playnite desktop theme" };
        row(ui, pal, &f.name, what, &f.path, busy, || import_pn = Some(f.path.clone()));
    }
    if let Some(p) = import_pn {
        t.start_import_playnite(st, p);
    }
    if looking {
        ui.horizontal(|ui| {
            lazy_spinner(ui, 14.0, pal.dim);
            ui.label(RichText::new("Looking for themes…").size(12.5).color(pal.dim));
        });
    } else if pn.is_empty() {
        let what = if page == Page::Controller {
            "No Playnite fullscreen themes found (Lodestar looks in ~/Downloads and Playnite inside Wine / Proton / Bottles)."
        } else {
            "No Playnite desktop themes found (Lodestar looks in ~/Downloads and Playnite inside Wine / Proton / Bottles)."
        };
        hint_text(ui, what, pal);
    }
    ui.horizontal(|ui| {
        hint_text(ui, "Or use Import… above for a theme somewhere else, or", pal);
        if ui
            .add(
                egui::Button::new(RichText::new("browse community themes").size(12.0).color(pal.selection))
                    .frame(false),
            )
            .clicked()
        {
            *go = Some(Section::Browse);
        }
    });
}

fn row(ui: &mut Ui, pal: &Pal, name: &str, what: &str, path: &std::path::Path, busy: bool, on_import: impl FnOnce()) {
    ui.horizontal(|ui| {
        if ui.add_enabled(!busy, egui::Button::new("Import")).clicked() {
            on_import();
        }
        ui.label(RichText::new(name).strong().color(pal.text));
        ui.label(RichText::new(what).size(11.5).color(pal.dim));
        ui.add(
            egui::Label::new(RichText::new(path.display().to_string()).size(11.5).color(mix(
                pal.dim,
                pal.surface,
                0.2,
            )))
            .truncate(),
        )
        .on_hover_text(path.display().to_string());
    });
}

/// The report after an install / import (drawn over the Add-ons window).
pub(super) fn report_window(ctx: &egui::Context, st: &mut AppState, t: &mut Themes, pal: &Pal) {
    if t.reports.is_empty() {
        return;
    }
    let mut close = false;
    let mut use_it: Option<(String, bool)> = None;
    let mut rerun: Option<playnite::ImportOptions> = None;
    let reports = t.reports.clone();
    let from_list = t.reports_from_list;
    let (_, dismissed) = modal(ctx, "ls_theme_report", pal, 580.0, |ui| {
        if reports.len() > 1 {
            let ok = reports.iter().filter(|r| r.is_ok()).count();
            ui.label(
                RichText::new(format!("Updated {ok} of {} themes", reports.len())).size(19.0).strong().color(pal.text),
            );
            ui.add_space(8.0);
            for r in &reports {
                match r {
                    Ok(rep) => line(ui, "✓", pal.accent, &format!("{} — {}", rep.name, version_text(rep)), pal.text),
                    Err(e) => line(ui, "!", pal.danger, e, pal.danger),
                }
            }
            close = footer(ui, pal, Buttons::primary("Close", true), |_| {}).primary;
            return;
        }
        match &reports[0] {
            Ok(rep) => {
                let verb = if rep.community { "Installed" } else { "Imported" };
                ui.label(RichText::new(format!("{verb} “{}”", rep.name)).size(19.0).strong().color(pal.text));
                let mut sub = rep.what.clone();
                let v = version_text(rep);
                if !v.is_empty() {
                    sub.push_str(&format!(" · {v}"));
                }
                ui.label(RichText::new(sub).size(12.5).color(pal.dim));
                if rep.community {
                    ui.add_space(6.0);
                    hint_text(
                        ui,
                        "A community theme converted by Lodestar — it may not look exactly like the original.",
                        pal,
                    );
                }
                ui.add_space(10.0);
                if rep.groups.is_empty() {
                    flat_lines(ui, rep, pal);
                } else {
                    let busy = t.busy();
                    if let Some(job) = &t.job {
                        ui.horizontal(|ui| {
                            lazy_spinner(ui, 14.0, pal.dim);
                            ui.label(RichText::new(&job.label).size(12.5).color(pal.dim));
                        });
                        ui.add_space(4.0);
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("ls_theme_report_groups")
                        .max_height((ctx.content_rect().height() * 0.55).max(220.0))
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            if let Some(o) = groups(ui, rep, pal, busy) {
                                rerun = Some(o);
                            }
                        });
                }
                for h in &rep.needs {
                    ui.add_space(10.0);
                    ui.label(RichText::new("To get everything").strong().color(pal.text));
                    ui.label(RichText::new(&h.text).size(12.5).color(pal.text));
                    if let Some(c) = &h.command {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(c).monospace().size(12.5).color(pal.text));
                            if ui
                                .small_button("Copy")
                                .on_hover_text("Copy the command, then paste it in a terminal")
                                .clicked()
                            {
                                ui.ctx().copy_text(c.clone());
                            }
                        });
                    }
                }
                ui.add_space(12.0);
                let page = if rep.controller { "Controller themes" } else { "Desktop themes" };
                hint_text(ui, &format!("It's listed under {page}."), pal);
                let in_use = if rep.controller {
                    st.config.controller.theme == rep.key
                } else {
                    st.config.desktop_theme == rep.key
                };
                let label = match (in_use, rep.controller) {
                    (true, _) => "In use",
                    (false, false) => "Use this theme",
                    (false, true) => "Use it in controller mode",
                };
                let f = footer(ui, pal, Buttons::primary(label, !in_use).cancel("Close"), |_| {});
                if f.primary {
                    use_it = Some((rep.key.clone(), rep.controller));
                }
                close = f.cancel;
            }
            Err(e) => {
                let title = if from_list { "Couldn't install the theme" } else { "Couldn't add the theme" };
                ui.label(RichText::new(title).size(19.0).strong().color(pal.text));
                ui.add_space(6.0);
                ui.add(egui::Label::new(RichText::new(e).color(pal.danger)).wrap());
                ui.add_space(6.0);
                let hint = if from_list {
                    "Check the internet connection and try again later. If it keeps failing, the theme's page may say \
                     more (its author may have moved or removed it)."
                } else {
                    "Choose a Playnite theme's .pthm package, its folder, or the theme.yaml inside it."
                };
                hint_text(ui, hint, pal);
                close = footer(ui, pal, Buttons::default().cancel("Close"), |_| {}).cancel;
            }
        }
    });
    if let Some((key, controller)) = use_it {
        if controller {
            st.set_controller_theme(&key);
        } else {
            st.set_desktop_theme(&key);
        }
        st.notify("Theme changed");
        close = true;
    }
    if let (Some(opts), Some(Ok(prev))) = (rerun, reports.first())
        && !t.busy()
    {
        t.start_reimport(st, prev.clone(), opts);
    }
    if (close || dismissed) && !t.busy() {
        for r in t.reports.drain(..).flatten() {
            install::discard(&r);
        }
    }
}

/// A report without areas: what came along, what couldn't.
fn flat_lines(ui: &mut Ui, rep: &Report, pal: &Pal) {
    ui.label(RichText::new("Carried over").strong().color(pal.text));
    for l in &rep.carried {
        line(ui, "✓", pal.accent, l, pal.text);
    }
    if !rep.missed.is_empty() {
        ui.add_space(8.0);
        ui.label(RichText::new("Couldn't be carried over").strong().color(pal.text));
        let shown = 4;
        for l in rep.missed.iter().take(shown) {
            line(ui, "–", pal.dim, l, pal.dim);
        }
        if rep.missed.len() > shown {
            egui::CollapsingHeader::new(
                RichText::new(format!("{} more", rep.missed.len() - shown)).size(12.5).color(pal.dim),
            )
            .id_salt("ls_theme_report_more")
            .show(ui, |ui| {
                for l in &rep.missed[shown..] {
                    line(ui, "–", pal.dim, l, pal.dim);
                }
            });
        }
    }
}

/// A Playnite report by area, with the layout choice and the videos-and-sounds switch.
/// Returns the choices to import it again with, when one was clicked.
fn groups(ui: &mut Ui, rep: &Report, pal: &Pal, busy: bool) -> Option<playnite::ImportOptions> {
    let mut out = None;
    let rr = rep.playnite.as_ref();
    for (gi, g) in rep.groups.iter().enumerate() {
        if gi > 0 {
            ui.add_space(8.0);
        }
        ui.label(RichText::new(&g.title).strong().color(pal.text));
        let not_possible = g.lines.iter().all(|l| !l.ok);
        let shown = if not_possible { 3 } else { usize::MAX };
        for l in g.lines.iter().take(shown) {
            if l.ok {
                line(ui, "✓", pal.accent, &l.text, pal.text);
            } else {
                line(ui, "–", pal.dim, &l.text, pal.dim);
            }
        }
        if g.lines.len() > shown {
            egui::CollapsingHeader::new(
                RichText::new(format!("{} more", g.lines.len() - shown)).size(12.5).color(pal.dim),
            )
            .id_salt(("ls_theme_report_more", gi))
            .show(ui, |ui| {
                for l in &g.lines[shown..] {
                    line(ui, "–", pal.dim, &l.text, pal.dim);
                }
            });
        }
        let Some(rr) = rr else { continue };
        if g.title == "Layout" && rr.fullscreen {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new("Looks wrong? Try:").size(12.5).color(pal.dim));
                for (i, f) in playnite::Family::ALL.into_iter().filter(|f| rr.offers(*f)).enumerate() {
                    if i > 0 {
                        ui.label(RichText::new("·").size(12.5).color(pal.dim));
                    }
                    let current = rr.family == Some(f);
                    let mut label = f.label().to_string();
                    if rr.suggested == Some(f) && !current {
                        label.push_str(" (suggested)");
                    }
                    let text = if current {
                        RichText::new(label).size(12.5).strong().color(pal.text)
                    } else {
                        RichText::new(label).size(12.5).color(pal.selection)
                    };
                    let r = ui.add_enabled(!current && !busy, egui::Button::new(text).frame(false));
                    let r = if current {
                        r.on_disabled_hover_text("In use now")
                    } else {
                        r.on_hover_text("Import it again with this layout")
                    };
                    if r.clicked() {
                        out = Some(playnite::ImportOptions { family: Some(f), media: Some(rr.media_included) });
                    }
                }
            });
        }
        if g.title == "Videos and sounds" && rr.fullscreen && rr.media_bytes > 0 {
            let mut on = rr.media_included;
            let text = format!("Include its videos and sounds ({})", playnite::report::size_text(rr.media_bytes));
            let r = ui.add_enabled(!busy, egui::Checkbox::new(&mut on, RichText::new(text).size(12.5).color(pal.text)));
            if r.changed() {
                out = Some(playnite::ImportOptions { family: rr.family, media: Some(on) });
            }
        }
    }
    out
}

fn version_text(rep: &Report) -> String {
    if rep.version.is_empty() {
        return String::new();
    }
    if rep.version.len() == 40 && rep.version.chars().all(|c| c.is_ascii_hexdigit()) {
        format!("version {}", &rep.version[..7])
    } else {
        format!("version {}", rep.version)
    }
}

fn line(ui: &mut Ui, mark: &str, mark_color: egui::Color32, text: &str, color: egui::Color32) {
    ui.horizontal_top(|ui| {
        ui.add_sized(vec2(14.0, 16.0), egui::Label::new(RichText::new(mark).color(mark_color).strong()));
        ui.add(egui::Label::new(RichText::new(text).size(13.0).color(color)).wrap());
    });
}

#[cfg(test)]
mod tests {
    use super::{Page, builtin_blurb};

    /// Remove confirmed while another theme job runs: nothing changes (the theme stays in use,
    /// the running job stays), and a message says why.
    #[test]
    fn removing_while_a_job_runs_changes_nothing_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = crate::app::AppState::for_test(dir.path());
        st.config.desktop_theme = "mine".into();
        st.config.controller.theme = "mine".into();
        let (_tx, rx) = crossbeam_channel::bounded(1);
        let mut t = super::Themes {
            job: Some(super::Job { label: "Installing…".into(), progress: Default::default(), rx, entries: vec![] }),
            ..Default::default()
        };
        t.start_remove(&mut st, "mine".into(), "Mine".into());
        assert_eq!(st.config.desktop_theme, "mine");
        assert_eq!(st.config.controller.theme, "mine");
        assert_eq!(t.job.as_ref().map(|j| j.label.as_str()), Some("Installing…"));
        assert_eq!(t.banner.as_ref().map(|b| b.0.as_str()), Some(super::BUSY));
    }

    #[test]
    fn desktop_page_describes_built_ins_by_what_the_desktop_uses() {
        let shelf = "Dense cover wall with a slim info strip.";
        assert_eq!(builtin_blurb("Shelf", shelf, Page::Controller), shelf);
        let d = builtin_blurb("Shelf", shelf, Page::Desktop);
        assert!(d.contains("colours") && !d.contains("cover wall"), "{d}");
        assert!(builtin_blurb("Lodestar Ambient", "x", Page::Desktop).contains("art blended behind"));
        assert!(builtin_blurb("Lodestar Ambient", "x", Page::Controller).starts_with("The Lodestar layout"));
        assert_eq!(builtin_blurb("Classic", "", Page::Controller), "Part of Lodestar");
    }
}
