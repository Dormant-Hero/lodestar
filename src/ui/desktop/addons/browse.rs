//! Add-ons → Browse themes and Updates: Playnite's community theme list
//! (`theme::catalog`), cached on disk and refreshed in the background, with screenshots,
//! Install (download + convert on a helper thread, then the report window) and update checks
//! for themes installed from them.

use super::super::Pal;
use super::super::dialogs::{hint_text, primary_button};
use super::card_frame;
use super::kind_chip;
use super::themes::{Themes, banner, job_line};
use crate::app::AppState;
use crate::images::ImgSize;
use crate::theme::catalog::installed::From;
use crate::theme::catalog::{self, Catalog, Check, Endpoints, Entry, Index, Kind, Update};
use crate::ui::widgets::{Icon, hash_str, initials_tile, lazy_spinner, mix, paint_cover, with_alpha};
use crossbeam_channel::{Receiver, Sender};
use egui::{Align, Layout, RichText, Sense, Ui, vec2};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Filter {
    #[default]
    All,
    Desktop,
    Controller,
}

enum Msg {
    Cached(Catalog),
    Fetched(Catalog),
}

enum Shot {
    Pending,
    Ready(PathBuf),
    Failed,
}

#[derive(Default)]
pub(crate) struct Browse {
    pub catalog: Option<Catalog>,
    rx: Option<Receiver<Msg>>,
    /// A download of the list is running.
    fetching: bool,
    started: bool,
    pub filter: Filter,
    /// Only the themes installed from the list (with Remove); combines with the kind and
    /// list filters like they combine with each other.
    installed_only: bool,
    search: String,
    shots: HashMap<String, Shot>,
    shot_tx: Option<Sender<String>>,
    shot_rx: Option<Receiver<(String, Option<PathBuf>)>>,
    // ---- updates
    pub updates: Option<Vec<Update>>,
    update_problems: Vec<String>,
    checks: Vec<Check>,
    update_rx: Option<Receiver<(Vec<Update>, Vec<String>)>>,
    update_started: bool,
    seen_generation: u64,
    /// "Remove …?" asked for this installed theme (key, name).
    confirm_remove: Option<(String, String)>,
}

fn endpoints(st: &AppState) -> Endpoints {
    Endpoints::for_run(st.no_launch)
}

fn ago(secs: i64) -> String {
    match secs {
        s if s < 90 => "just now".into(),
        s if s < 3600 => format!("{} minutes ago", s / 60),
        s if s < 2 * 3600 => "an hour ago".into(),
        s if s < 86400 => format!("{} hours ago", s / 3600),
        s if s < 2 * 86400 => "yesterday".into(),
        s => format!("{} days ago", s / 86400),
    }
}

impl Browse {
    /// Load the saved lists, then download fresh ones when they're old (or `force`).
    pub fn start(&mut self, st: &AppState, force: bool) {
        if self.fetching {
            return;
        }
        self.started = true;
        self.fetching = true;
        let dir = catalog::cache_dir(&st.paths.cache_dir);
        let ep = endpoints(st);
        let ctx = st.ctx().clone();
        let have = self.catalog.is_some();
        let (tx, rx) = crossbeam_channel::unbounded();
        let spawned = std::thread::Builder::new().name("theme-catalog".into()).spawn(move || {
            let cached = catalog::load_cached(&dir);
            let fresh = cached.as_ref().is_some_and(|c| c.age(catalog::now()) < catalog::MAX_AGE_SECS);
            if let Some(c) = cached
                && !have
            {
                let _ = tx.send(Msg::Cached(c));
                ctx.request_repaint();
            }
            if force || !fresh {
                let _ = tx.send(Msg::Fetched(catalog::fetch(&ep, &dir)));
            } else {
                let _ = tx.send(Msg::Fetched(catalog::load_cached(&dir).unwrap_or_default()));
            }
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => self.rx = Some(rx),
            Err(_) => self.fetching = false,
        }
    }

    pub fn poll(&mut self, themes: &Themes) {
        if let Some(rx) = &self.rx {
            while let Ok(m) = rx.try_recv() {
                match m {
                    Msg::Cached(c) => self.catalog = Some(c),
                    Msg::Fetched(c) => {
                        self.catalog = Some(c);
                        self.fetching = false;
                    }
                }
            }
            if !self.fetching {
                self.rx = None;
            }
        }
        if let Some(rx) = &self.shot_rx {
            while let Ok((url, path)) = rx.try_recv() {
                self.shots.insert(url, path.map_or(Shot::Failed, Shot::Ready));
            }
        }
        if let Some(rx) = &self.update_rx
            && let Ok((ups, problems)) = rx.try_recv()
        {
            self.updates = Some(ups);
            self.update_problems = problems;
            self.update_rx = None;
        }
        // Installed list changed (an update / install / removal finished): drop updates that
        // are done, without asking the servers again.
        if themes.generation != self.seen_generation {
            self.seen_generation = themes.generation;
            if let Some(list) = &themes.list {
                self.checks = catalog::checks(list, self.catalog.as_ref());
                if let Some(ups) = &mut self.updates {
                    let checks = &self.checks;
                    ups.retain(|u| {
                        checks.iter().any(|c| c.key == u.key && catalog::is_newer(c.index, &c.version, &u.latest))
                    });
                }
            }
        }
    }

    /// Ask the servers for each installed list theme's newest version.
    pub fn check_updates(&mut self, st: &AppState, themes: &Themes) {
        if self.update_rx.is_some() {
            return;
        }
        let Some(list) = &themes.list else { return };
        self.update_started = true;
        self.checks = catalog::checks(list, self.catalog.as_ref());
        // Screenshot runs never ask the servers (only local `file://` sources).
        let offline = st.no_launch && self.checks.iter().any(|c| !c.source.starts_with("file://"));
        if self.checks.is_empty() || offline {
            self.updates = Some(vec![]);
            self.update_problems = if offline { vec!["Update checks are off in this run.".into()] } else { vec![] };
            return;
        }
        let checks = self.checks.clone();
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = st.ctx().clone();
        if std::thread::Builder::new()
            .name("theme-updates".into())
            .spawn(move || {
                let _ = tx.send(catalog::find_updates(&checks));
                ctx.request_repaint();
            })
            .is_ok()
        {
            self.update_rx = Some(rx);
        }
    }

    /// A screenshot's local file (downloading it in the background the first time).
    fn shot(&mut self, st: &AppState, url: &str) -> Option<PathBuf> {
        match self.shots.get(url) {
            Some(Shot::Ready(p)) => return Some(p.clone()),
            Some(_) => return None,
            None => {}
        }
        let dir = catalog::cache_dir(&st.paths.cache_dir).join("shots");
        let ext = url
            .rsplit('.')
            .next()
            .map(|e| e.to_ascii_lowercase())
            .filter(|e| ["jpg", "jpeg", "png", "webp"].contains(&e.as_str()))
            .unwrap_or_else(|| "img".into());
        let path = dir.join(format!("{:016x}.{ext}", hash_str(url)));
        if path.is_file() {
            self.shots.insert(url.to_string(), Shot::Ready(path.clone()));
            return Some(path);
        }
        // Screenshot runs only read local files (fixtures), never the network.
        if st.no_launch && !url.starts_with("file://") {
            self.shots.insert(url.to_string(), Shot::Failed);
            return None;
        }
        if self.shot_tx.is_none() {
            let (tx, rx) = crossbeam_channel::unbounded::<String>();
            let (back_tx, back_rx) = crossbeam_channel::unbounded();
            let ctx = st.ctx().clone();
            let dir2 = dir.clone();
            let spawned = std::thread::Builder::new().name("theme-shots".into()).spawn(move || {
                let mut queue: Vec<String> = vec![];
                loop {
                    if queue.is_empty() {
                        match rx.recv() {
                            Ok(u) => queue.push(u),
                            Err(_) => break,
                        }
                    }
                    while let Ok(u) = rx.try_recv() {
                        queue.push(u);
                    }
                    // Newest first: the cards on screen now.
                    let Some(u) = queue.pop() else { continue };
                    let ext = u
                        .rsplit('.')
                        .next()
                        .map(|e| e.to_ascii_lowercase())
                        .filter(|e| ["jpg", "jpeg", "png", "webp"].contains(&e.as_str()))
                        .unwrap_or_else(|| "img".into());
                    let path = dir2.join(format!("{:016x}.{ext}", hash_str(&u)));
                    let r = std::fs::create_dir_all(&dir2)
                        .ok()
                        .and_then(|_| catalog::net::get(&u, catalog::net::MAX_SHOT_BYTES).ok())
                        .and_then(|b| std::fs::write(&path, b).ok())
                        .map(|_| path);
                    if back_tx.send((u, r)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            });
            if spawned.is_err() {
                self.shots.insert(url.to_string(), Shot::Failed);
                return None;
            }
            self.shot_tx = Some(tx);
            self.shot_rx = Some(back_rx);
        }
        if let Some(tx) = &self.shot_tx {
            let _ = tx.send(url.to_string());
        }
        self.shots.insert(url.to_string(), Shot::Pending);
        None
    }
}

/// (index, id) of every theme installed from a list (Playnite imports by their Playnite id),
/// with its installed key and name (for Remove).
fn installed_ids(themes: &Themes) -> HashMap<(Index, String), (String, String)> {
    let mut out = HashMap::new();
    for t in themes.list.iter().flatten() {
        let id = match &t.from {
            From::Catalog(r) => r.index.map(|i| (i, r.id.clone())),
            From::Playnite { playnite_id, .. } if !playnite_id.is_empty() => {
                Some((Index::Playnite, playnite_id.clone()))
            }
            _ => None,
        };
        if let Some(id) = id {
            out.insert(id, (t.key.clone(), t.name.clone()));
        }
    }
    out
}

fn community_note(ui: &mut Ui, pal: &Pal) {
    egui::Frame::new()
        .fill(with_alpha(pal.accent, 18))
        .stroke(egui::Stroke::new(1.0, with_alpha(pal.accent, 60)))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "Community themes from Playnite's theme list, made for Playnite. Lodestar converts them \
                         when you install one, so they may not look exactly like the original.",
                    )
                    .size(12.5)
                    .color(pal.text),
                )
                .wrap(),
            );
        });
    ui.add_space(8.0);
}

/// Add-ons → Browse themes.
pub(super) fn browse_page(ui: &mut Ui, st: &mut AppState, b: &mut Browse, t: &mut Themes, pal: &Pal, max_h: f32) {
    if !b.started {
        b.start(st, false);
    }
    community_note(ui, pal);
    let entries: Vec<Entry> = b.catalog.as_ref().map(|c| c.entries.clone()).unwrap_or_default();
    let count = |k: Option<Kind>| entries.iter().filter(|e| k.is_none_or(|k| e.kind == k)).count();
    let installed = installed_ids(t);
    let installed_count = entries.iter().filter(|e| installed.contains_key(&(e.index, e.id.clone()))).count();
    // ---- toolbar: search and Refresh on one row, the filters on the next (they don't fit beside
    // each other in a normal-size window).
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut b.search).hint_text("Search themes").desired_width(240.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let r = ui.add_enabled(!b.fetching, egui::Button::new("Refresh"));
            if r.on_hover_text("Download the theme list again").clicked() {
                b.start(st, true);
            }
            if b.fetching {
                lazy_spinner(ui, 16.0, pal.dim);
            } else if let Some(c) = &b.catalog {
                let newest = c.fetched_playnite;
                if newest > 0 {
                    ui.label(
                        RichText::new(format!("Updated {}", ago(catalog::now() - newest))).size(12.0).color(pal.dim),
                    );
                }
            }
        });
    });
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for (f, label, k) in [
            (Filter::All, "All", None),
            (Filter::Desktop, "Desktop", Some(Kind::Desktop)),
            (Filter::Controller, "Controller", Some(Kind::Controller)),
        ] {
            let text = if entries.is_empty() { label.to_string() } else { format!("{label} {}", count(k)) };
            if ui.selectable_label(b.filter == f, text).clicked() {
                b.filter = f;
            }
        }
        ui.separator();
        // A switch on top of the kind filter (Desktop + Installed …).
        let n = if entries.is_empty() { installed.len() } else { installed_count };
        if ui
            .selectable_label(b.installed_only, format!("Installed {n}"))
            .on_hover_text("Only the themes you installed from the list: update or remove them here")
            .clicked()
        {
            b.installed_only = !b.installed_only;
        }
    });
    ui.add_space(8.0);
    job_line(ui, t, pal);
    // A removal confirmed here that couldn't start says so here, not only on the theme pages.
    if let Some((text, err)) = t.banner.clone()
        && banner(ui, &text, err, pal)
    {
        t.banner = None;
    }
    if let Some(c) = &b.catalog
        && !b.fetching
        && !c.problems.is_empty()
        && !entries.is_empty()
    {
        let text = c.problems.join(" ");
        if banner(ui, &text, false, pal)
            && let Some(c) = &mut b.catalog
        {
            c.problems.clear();
        }
    }

    if entries.is_empty() && b.installed_only && !b.fetching && b.catalog.is_some() {
        // No lists (offline): what is installed from them is known here all the same.
        hint_text(
            ui,
            "The theme list couldn't be loaded, so these are the themes installed from it on this computer.",
            pal,
        );
        ui.add_space(6.0);
        confirm_remove_frame(ui, st, b, t, pal);
        installed_offline(ui, b, t, pal, max_h - 150.0);
        return;
    }
    if entries.is_empty() {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            if b.fetching || b.catalog.is_none() {
                lazy_spinner(ui, 24.0, pal.dim);
                ui.add_space(8.0);
                ui.label(RichText::new("Getting the theme list…").size(14.0).color(pal.dim));
            } else {
                ui.label(RichText::new("Couldn't load the theme list").size(16.0).strong().color(pal.text));
                ui.add_space(4.0);
                for p in b.catalog.as_ref().map(|c| c.problems.clone()).unwrap_or_default() {
                    ui.label(RichText::new(p).size(12.5).color(pal.dim));
                }
                ui.add_space(4.0);
                hint_text(ui, "Check the internet connection and try again. Installed themes keep working.", pal);
                ui.add_space(8.0);
                if primary_button(ui, "Try again", pal, true).clicked() {
                    b.start(st, true);
                }
            }
        });
        return;
    }

    let q = b.search.trim().to_lowercase();
    let shown: Vec<&Entry> = entries
        .iter()
        .filter(|e| match b.filter {
            Filter::All => true,
            Filter::Desktop => e.kind == Kind::Desktop,
            Filter::Controller => e.kind == Kind::Controller,
        })
        .filter(|e| !b.installed_only || installed.contains_key(&(e.index, e.id.clone())))
        .filter(|e| {
            q.is_empty()
                || e.name.to_lowercase().contains(&q)
                || e.author.to_lowercase().contains(&q)
                || e.tags.iter().any(|t| t.to_lowercase().contains(&q))
        })
        .collect();
    if shown.is_empty() {
        let none = if b.installed_only && q.is_empty() && installed.is_empty() {
            "No themes installed from the list yet. Themes you imported yourself are under Desktop themes and Controller themes."
        } else {
            "No theme matches."
        };
        hint_text(ui, none, pal);
        return;
    }
    confirm_remove_frame(ui, st, b, t, pal);
    let updates: HashSet<(Index, String)> = b.updates.iter().flatten().map(|u| (u.index, u.id.clone())).collect();
    let row_h = 140.0;
    let mut install: Option<Entry> = None;
    egui::ScrollArea::vertical()
        .id_salt("ls_theme_browse")
        .max_height(max_h - 90.0)
        .auto_shrink([false, false])
        .show_rows(ui, row_h, shown.len(), |ui, range| {
            for e in &shown[range] {
                let key = (e.index, e.id.clone());
                let state = if t.installing(e.index, &e.id) {
                    Card::Installing
                } else if updates.contains(&key) {
                    Card::Update
                } else if installed.contains_key(&key) {
                    Card::Installed
                } else {
                    Card::New
                };
                match entry_card(ui, st, b, e, state, t.busy(), pal, row_h - 8.0) {
                    CardAction::Install => install = Some((*e).clone()),
                    CardAction::Remove => b.confirm_remove = installed.get(&key).cloned(),
                    CardAction::None => {}
                }
            }
        });
    if let Some(e) = install {
        t.start_install(st, vec![e]);
    }
}

/// "Remove …?" for an installed theme (asked first; the folder goes, Lodestar's own theme
/// takes over if it was in use).
fn confirm_remove_frame(ui: &mut Ui, st: &mut AppState, b: &mut Browse, t: &mut Themes, pal: &Pal) {
    if let Some((key, name)) = b.confirm_remove.clone() {
        egui::Frame::new()
            .fill(with_alpha(pal.danger, 18))
            .stroke(egui::Stroke::new(1.0, with_alpha(pal.danger, 70)))
            .corner_radius(6)
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(format!("Remove “{name}”?")).strong().color(pal.text));
                hint_text(
                    ui,
                    "Its folder is deleted from Lodestar's themes folder. If it's in use, Lodestar switches back to its own theme. You can install it again any time.",
                    pal,
                );
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!t.busy(), egui::Button::new(RichText::new("Remove").color(egui::Color32::WHITE).strong()).fill(pal.danger).min_size(vec2(90.0, 28.0)))
                        .on_disabled_hover_text(super::themes::BUSY)
                        .clicked()
                    {
                        b.confirm_remove = None;
                        t.start_remove(st, key.clone(), name.clone());
                    }
                    if ui.button("Cancel").clicked() {
                        b.confirm_remove = None;
                    }
                });
            });
        ui.add_space(8.0);
    }
}

/// Browse → Installed without the theme lists: the installed themes that came from them, from
/// this computer's own records, each with Remove.
fn installed_offline(ui: &mut Ui, b: &mut Browse, t: &Themes, pal: &Pal, max_h: f32) {
    let list: Vec<&crate::theme::catalog::installed::InstalledTheme> = t
        .list
        .iter()
        .flatten()
        .filter(|th| {
            let index = match &th.from {
                From::Catalog(r) => r.index,
                From::Playnite { playnite_id, .. } if !playnite_id.is_empty() => Some(Index::Playnite),
                _ => None,
            };
            index.is_some()
                && match b.filter {
                    Filter::All => true,
                    Filter::Desktop => !th.controller,
                    Filter::Controller => th.controller,
                }
        })
        .collect();
    if list.is_empty() {
        hint_text(
            ui,
            "No themes installed from the list yet. Themes you imported yourself are under Desktop themes and Controller themes.",
            pal,
        );
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("ls_theme_browse_offline")
        .max_height(max_h.max(120.0))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for th in list {
                card_frame(pal, false).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let r = ui.small_button(RichText::new("Remove").color(pal.danger));
                            if r.on_hover_text("Delete this theme from Lodestar (asks first)").clicked() && !t.busy() {
                                b.confirm_remove = Some((th.key.clone(), th.name.clone()));
                            }
                            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                ui.label(RichText::new(&th.name).size(14.5).strong().color(pal.text));
                                let origin = th.origin_text();
                                ui.add(egui::Label::new(RichText::new(&origin).size(12.0).color(pal.dim)).truncate())
                                    .on_hover_text(&origin);
                            });
                        });
                    });
                });
                ui.add_space(4.0);
            }
        });
}

#[derive(Clone, Copy, PartialEq)]
enum Card {
    New,
    Installed,
    Update,
    Installing,
}

/// One theme of the list. True when Install / Update was clicked.
/// What a card's buttons asked for.
#[derive(Clone, Copy, PartialEq)]
enum CardAction {
    None,
    Install,
    Remove,
}

#[allow(clippy::too_many_arguments)]
fn entry_card(
    ui: &mut Ui,
    st: &mut AppState,
    b: &mut Browse,
    e: &Entry,
    state: Card,
    busy: bool,
    pal: &Pal,
    h: f32,
) -> CardAction {
    let mut clicked = false;
    let mut remove = false;
    let shot = e.shots.first().map(|s| s.thumb.clone());
    let tex = shot.and_then(|u| b.shot(st, &u)).and_then(|p| st.images.get(&p, ImgSize::Thumb));
    card_frame(pal, false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(h - 24.0);
        ui.horizontal_top(|ui| {
            let th = h - 24.0;
            let (r, _) = ui.allocate_exact_size(vec2(th * 16.0 / 9.0, th), Sense::hover());
            match &tex {
                Some(tex) => paint_cover(ui.painter(), r, tex, 6, egui::Color32::WHITE, vec2(0.5, 0.5)),
                None => {
                    initials_tile(ui.painter(), r, &e.name, pal.surface_alt, pal.text);
                }
            }
            ui.add_space(8.0);
            let text_w = (ui.available_width() - 120.0).max(140.0);
            ui.allocate_ui_with_layout(vec2(text_w, th), Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 7.0;
                    ui.add(egui::Label::new(RichText::new(&e.name).size(14.5).strong().color(pal.text)).truncate());
                    kind_chip(ui, if e.kind == Kind::Desktop { "Desktop" } else { "Controller" }, pal);
                    if e.new_entry {
                        kind_chip(ui, "New", pal);
                    }
                });
                let made = match (e.index, e.kind) {
                    (Index::Playnite, Kind::Desktop) => "made for Playnite (desktop)",
                    (Index::Playnite, Kind::Controller) => "made for Playnite (fullscreen)",
                };
                let by = if e.author.is_empty() { made.to_string() } else { format!("by {} · {made}", e.author) };
                ui.add(egui::Label::new(RichText::new(by).size(12.0).color(pal.dim)).truncate());
                if !e.description.is_empty() {
                    let d: String = e.description.chars().take(170).collect();
                    let d = if e.description.chars().count() > 170 { format!("{}…", d.trim_end()) } else { d };
                    ui.add(egui::Label::new(RichText::new(d).size(12.5).color(mix(pal.text, pal.dim, 0.3))).wrap());
                }
            });
            ui.with_layout(Layout::top_down(Align::Max), |ui| {
                match state {
                    Card::New => {
                        if primary_button(ui, "Install", pal, !busy)
                            .on_hover_text("Download it and convert it into a Lodestar theme")
                            .clicked()
                        {
                            clicked = true;
                        }
                    }
                    Card::Update => {
                        if primary_button(ui, "Update", pal, !busy)
                            .on_hover_text("A newer version is available")
                            .clicked()
                        {
                            clicked = true;
                        }
                    }
                    Card::Installed => {
                        ui.add_enabled(false, egui::Button::new("Installed").min_size(vec2(90.0, 30.0)));
                        if ui.small_button("Reinstall").on_hover_text("Download and convert it again").clicked()
                            && !busy
                        {
                            clicked = true;
                        }
                        let r = ui.small_button(RichText::new("Remove").color(pal.danger));
                        if r.on_hover_text("Delete this theme from Lodestar (asks first)").clicked() && !busy {
                            remove = true;
                        }
                    }
                    Card::Installing => {
                        ui.horizontal(|ui| {
                            lazy_spinner(ui, 16.0, pal.dim);
                            ui.label(RichText::new("Installing…").size(12.5).color(pal.dim));
                        });
                    }
                }
                if !e.page.is_empty() && e.page.starts_with("http") {
                    let r =
                        ui.add(egui::Button::new(RichText::new("Page ↗").size(12.0).color(pal.selection)).frame(false));
                    if r.on_hover_text(&e.page).clicked() {
                        let url = e.page.clone();
                        st.open_url(&url);
                    }
                }
            });
        });
    });
    if remove {
        CardAction::Remove
    } else if clicked {
        CardAction::Install
    } else {
        CardAction::None
    }
}

/// Add-ons → Updates.
pub(super) fn updates_page(ui: &mut Ui, st: &mut AppState, b: &mut Browse, t: &mut Themes, pal: &Pal) {
    if !b.started {
        // The list gives Playnite imports their update source.
        b.start(st, false);
    }
    if !b.update_started && t.list.is_some() && !b.fetching {
        b.check_updates(st, t);
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Theme updates").size(15.0).strong().color(pal.text));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let checking = b.update_rx.is_some();
            if ui.add_enabled(!checking && !t.busy(), egui::Button::new("Check again")).clicked() {
                b.check_updates(st, t);
            }
            if checking {
                lazy_spinner(ui, 16.0, pal.dim);
            }
        });
    });
    ui.add_space(6.0);
    job_line(ui, t, pal);
    let ups = b.updates.clone();
    match &ups {
        None => {
            ui.horizontal(|ui| {
                lazy_spinner(ui, 16.0, pal.dim);
                ui.label(RichText::new("Checking the themes installed from the theme list…").color(pal.dim));
            });
        }
        Some(list) if list.is_empty() && !(b.checks.is_empty() || b.update_problems.is_empty()) => {}
        Some(list) if list.is_empty() => {
            let text = if b.checks.is_empty() {
                "No themes installed from the theme list yet — they get update checks here (Browse themes)."
            } else {
                "Every theme installed from the theme list is up to date."
            };
            ui.horizontal(|ui| {
                if !b.checks.is_empty() && b.update_problems.is_empty() {
                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    crate::ui::widgets::paint_icon(ui.painter(), r, Icon::CheckCircle, pal.accent);
                }
                ui.label(RichText::new(text).size(13.0).color(pal.text));
            });
        }
        Some(list) => {
            let mut go: Vec<Update> = vec![];
            for u in list {
                card_frame(pal, false).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.label(RichText::new(&u.name).size(14.0).strong().color(pal.text));
                            let what = match u.index {
                                Index::Playnite => {
                                    let from = if u.installed.is_empty() { "?".into() } else { u.installed.clone() };
                                    format!("Playnite theme · version {from} → {}", u.latest)
                                }
                            };
                            ui.label(RichText::new(what).size(12.0).color(pal.dim));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if t.installing(u.index, &u.id) {
                                lazy_spinner(ui, 16.0, pal.dim);
                            } else if primary_button(ui, "Update", pal, !t.busy()).clicked() {
                                go.push(u.clone());
                            }
                        });
                    });
                });
            }
            ui.add_space(4.0);
            if list.len() > 1 && primary_button(ui, &format!("Update all ({})", list.len()), pal, !t.busy()).clicked() {
                go = list.clone();
            }
            hint_text(ui, "Updating converts the new version again; the theme keeps its place in Lodestar.", pal);
            let entries: Vec<Entry> = go.iter().filter_map(|u| u.entry(b.catalog.as_ref(), &b.checks)).collect();
            if !entries.is_empty() {
                t.start_install(st, entries);
            }
        }
    }
    if !b.update_problems.is_empty() {
        ui.add_space(6.0);
        ui.label(RichText::new("Couldn't check").size(12.5).strong().color(pal.dim));
        for p in &b.update_problems {
            ui.label(RichText::new(p).size(12.0).color(pal.dim));
        }
    }
    ui.add_space(18.0);
    ui.label(RichText::new("Extensions").size(15.0).strong().color(pal.text));
    ui.add_space(4.0);
    hint_text(
        ui,
        "Extensions aren't in an online list yet. Installing a newer copy of one (same id and author) from a folder \
         or a .zip on the Extensions page updates it in place and keeps its settings.",
        pal,
    );
}
