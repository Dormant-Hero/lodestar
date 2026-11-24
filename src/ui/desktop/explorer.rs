//! Left "Explorer" panel: Library / Collections / Sources / Platforms (with device icons) /
//! Tags / Genres / … / Developers / Publishers filters with counts (counts are cached per
//! library generation, see `Counts`).
//! Sections collapse; the selection and collapsed sections persist (DB kv `desktop.explorer`).

use super::{Action, DesktopUi, Facet, FilterValue, LibSel, Pal, SrcKey};
use crate::app::AppState;
use crate::ui::widgets::{truncated, with_alpha};
use egui::{Color32, Response, Sense, Ui, pos2, vec2};

pub(super) mod nav;
use nav::Entry;

pub(super) mod colls;

use crate::ui::platform_icons::{self, Shape};

const ROW_H: f32 = 26.0;

impl DesktopUi {
    pub(super) fn explorer(&mut self, ui: &mut Ui, st: &mut AppState) {
        let pal = self.pal;
        egui::ScrollArea::vertical().id_salt("ls_explorer_scroll").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            colls::drag_ghost(ui.ctx(), st, &pal);
            if self.section(ui, "LIBRARY", "library", 0, &pal) {
                self.library_rows(ui, st);
            }
            self.collections_section(ui, st, &pal);
            if !self.counts.sources.is_empty()
                && self.section(ui, "SOURCES", "sources", self.counts.sources.len(), &pal)
            {
                self.source_rows(ui, st);
            }
            self.platforms_section(ui, st, &pal, multi_key(ui));
            if self.section(ui, "TAGS", "tags", self.counts.tags.len(), &pal) {
                self.tag_rows(ui, st);
            }
            for f in Facet::ALL {
                // Platforms have their own section (with icons) above.
                if !self.counts.shows(f) || f == Facet::Platform {
                    continue;
                }
                let n = self.counts.facets[f.index()].len();
                if self.section(ui, f.title(), f.key(), n, &pal) {
                    self.facet_rows(ui, st, f);
                }
            }
            ui.add_space(8.0);
            if !self.no_extra_filters() {
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    if ui.add(egui::Button::new(egui::RichText::new("Clear selection").size(12.0)).small()).clicked() {
                        self.set_lib_sel(LibSel::All);
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                ui.label(
                    egui::RichText::new("Ctrl+click to pick several")
                        .size(11.0)
                        .color(super::super::widgets::with_alpha(pal.dim, 150)),
                );
            });
            ui.add_space(4.0);
        });
    }

    /// LIBRARY: All games, Installed, Not installed, Favourites, Recently played, Hidden.
    fn library_rows(&mut self, ui: &mut Ui, st: &AppState) {
        let pal = self.pal;
        let no_extra = self.no_extra_filters();
        let c = &self.counts;
        let mut lib_rows = vec![(LibSel::All, "All games", c.all), (LibSel::Installed, "Installed", c.installed)];
        if c.not_installed > 0 {
            lib_rows.push((LibSel::NotInstalled, "Not installed", c.not_installed));
        }
        lib_rows.push((LibSel::Favorites, "Favourites", c.favorites));
        lib_rows.push((LibSel::Recent, "Recently played", c.recent));
        if c.hidden > 0 {
            lib_rows.push((LibSel::Hidden, "Hidden", c.hidden));
        }
        let mut clicked_lib = None;
        let mut art_for: Option<(LibSel, &str)> = None;
        for (sel, label, n) in lib_rows {
            let foc = self.side_nav.is(|e| *e == Entry::Lib(sel));
            let r = row(ui, label, n, self.lib_sel == sel && no_extra, foc, &pal, None);
            if foc {
                self.side_nav.reveal(&r);
            }
            if r.clicked() {
                clicked_lib = Some(sel);
            }
            if art_menu(&r, n) {
                art_for = Some((sel, label));
            }
        }
        if let Some((sel, label)) = art_for {
            let recent =
                crate::library::Collection { kind: crate::library::CollectionKind::Recent, name: String::new() };
            let ids = section_games(st, |g| match sel {
                LibSel::Hidden => g.hidden,
                _ if g.hidden => false,
                LibSel::All => true,
                LibSel::Installed => g.installed,
                LibSel::NotInstalled => !g.installed,
                LibSel::Favorites => g.favorite,
                LibSel::Recent => recent.contains(g),
            });
            self.bulk_art_for(label, ids);
        }
        if let Some(sel) = clicked_lib {
            self.set_lib_sel(sel);
        }
    }

    /// SOURCES: one row per store / launcher (Ctrl / Shift: several).
    fn source_rows(&mut self, ui: &mut Ui, st: &AppState) {
        let pal = self.pal;
        let mut clicked: Option<(SrcKey, bool)> = None;
        let mut art_for = None;
        for &(s, n) in &self.counts.sources {
            let selected = self.src_sel.contains(&s);
            let foc = self.side_nav.is(|e| *e == Entry::Source(s));
            let r = row(ui, s.label(), n, selected, foc, &pal, None);
            if foc {
                self.side_nav.reveal(&r);
            }
            if r.clicked() {
                clicked = Some((s, multi_key(ui)));
            }
            if art_menu(&r, n) {
                art_for = Some(s);
            }
        }
        if let Some(s) = art_for {
            let ids = section_games(st, |g| !g.hidden && s.matches(g));
            self.bulk_art_for(s.label(), ids);
        }
        if let Some((s, multi)) = clicked {
            if !multi {
                self.clear_extras();
            }
            toggle(&mut self.src_sel, s, multi);
            self.lib_sel = LibSel::All;
        }
    }

    /// TAGS: the user's tags (or how to add one).
    fn tag_rows(&mut self, ui: &mut Ui, st: &AppState) {
        let pal = self.pal;
        if self.counts.tags.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                ui.label(egui::RichText::new("Right-click a game, then Edit tags…").size(12.0).color(pal.dim));
            });
            return;
        }
        let mut clicked: Option<(String, bool)> = None;
        let mut art_for: Option<String> = None;
        for (t, n) in &self.counts.tags {
            let selected = self.filter.tags.contains(t);
            let foc = self.side_nav.is(|e| matches!(e, Entry::Tag(x) if x == t));
            let r = row(ui, t, *n, selected, foc, &pal, None);
            if foc {
                self.side_nav.reveal(&r);
            }
            if r.clicked() {
                clicked = Some((t.clone(), multi_key(ui)));
            }
            if art_menu(&r, *n) {
                art_for = Some(t.clone());
            }
        }
        if let Some(t) = art_for {
            let ids = section_games(st, |g| !g.hidden && g.tags.iter().any(|x| x.to_lowercase() == t.to_lowercase()));
            self.bulk_art_for(&t, ids);
        }
        if let Some((t, multi)) = clicked {
            if !multi {
                self.clear_extras();
            }
            toggle(&mut self.filter.tags, t, multi);
            self.lib_sel = LibSel::All;
        }
    }

    /// One field's values (Genre, Developer, Year …).
    fn facet_rows(&mut self, ui: &mut Ui, st: &AppState, f: Facet) {
        let pal = self.pal;
        let fi = f.index();
        let mut clicked: Option<(String, bool)> = None;
        let mut art_for: Option<(String, String)> = None;
        for (key, label, n) in &self.counts.facets[fi] {
            let selected = self.facet_sel[fi].contains(key);
            let foc = self.side_nav.is(|e| matches!(e, Entry::Facet(i, k) if *i == fi && k == key));
            let r = row(ui, label, *n, selected, foc, &pal, None);
            if foc {
                self.side_nav.reveal(&r);
            }
            if r.clicked() {
                clicked = Some((key.clone(), multi_key(ui)));
            }
            if art_menu(&r, *n) {
                art_for = Some((key.clone(), label.clone()));
            }
        }
        if let Some((key, label)) = art_for {
            let want = [key];
            let ids = section_games(st, |g| !g.hidden && f.matches(g, &want));
            self.bulk_art_for(&label, ids);
        }
        if let Some((k, multi)) = clicked {
            if !multi {
                self.clear_extras();
            }
            toggle(&mut self.facet_sel[fi], k, multi);
            self.lib_sel = LibSel::All;
        }
    }

    /// A row's right-click → Download artwork…: the bulk dialog for the games under it.
    fn bulk_art_for(&mut self, label: &str, ids: Vec<crate::model::GameId>) {
        self.push(Action::BulkArt(super::artwork::BulkScope::Section(format!("“{label}”"), ids)));
    }

    /// Filter by a value clicked on a game page: like a click in the explorer (Ctrl: add).
    pub(crate) fn filter_by(&mut self, v: FilterValue, add: bool) {
        if !add {
            self.clear_extras();
        }
        if !matches!(self.lib_sel, LibSel::Installed | LibSel::NotInstalled) {
            self.lib_sel = LibSel::All;
        }
        match v {
            FilterValue::Facet(f, d) => {
                let k = d.to_lowercase();
                let list = &mut self.facet_sel[f.index()];
                if !list.contains(&k) {
                    list.push(k);
                }
                // Show the section the value is in (a collapsed one would hide the selection).
                self.collapsed.retain(|c| c != f.key());
            }
            FilterValue::Source(s) => {
                if !self.src_sel.contains(&s) {
                    self.src_sel.push(s);
                }
            }
            FilterValue::Collection(id) => self.coll_sel = Some(id),
            FilterValue::Tag(t) => {
                // The explorer's spelling (tags match case-insensitively).
                let t = self
                    .counts
                    .tags
                    .iter()
                    .find(|(x, _)| x.to_lowercase() == t.to_lowercase())
                    .map_or(t, |(x, _)| x.clone());
                if !self.filter.tags.contains(&t) {
                    self.filter.tags.push(t);
                }
            }
        }
    }

    pub(crate) fn unfilter(&mut self, v: &FilterValue) {
        match v {
            FilterValue::Facet(f, d) => {
                let k = d.to_lowercase();
                self.facet_sel[f.index()].retain(|x| *x != k);
            }
            FilterValue::Source(s) => self.src_sel.retain(|x| x != s),
            FilterValue::Tag(t) => self.filter.tags.retain(|x| x != t),
            FilterValue::Collection(_) => self.coll_sel = None,
        }
    }

    /// The explorer's narrowing on top of `Library::filter` (which leaves tags to this): the
    /// Hidden / Not installed entries, tags and the field selections, each field with any of
    /// its picked values, or with Match all every one (tags, and the fields a game can have
    /// several values of). Across fields, all must match.
    pub(crate) fn extra_match(&self, g: &crate::model::Game) -> bool {
        match self.lib_sel {
            LibSel::Hidden if !g.hidden => return false,
            LibSel::NotInstalled if g.installed => return false,
            _ => {}
        }
        if !self.src_sel.is_empty() && !self.src_sel.iter().any(|s| s.matches(g)) {
            return false;
        }
        let tags = &self.filter.tags;
        if !tags.is_empty() {
            let has = |t: &String| crate::library::has_tag(g, t);
            let ok = if self.match_all { tags.iter().all(has) } else { tags.iter().any(has) };
            if !ok {
                return false;
            }
        }
        Facet::ALL.iter().all(|f| {
            let want = &self.facet_sel[f.index()];
            want.is_empty() || f.matches_mode(g, want, self.match_all)
        })
    }

    /// The games the current filter shows (in view order), with or without the hidden ones.
    pub(crate) fn filtered_indices(&self, lib: &crate::library::Library, with_hidden: bool) -> Vec<usize> {
        let mut f = self.filter.clone();
        f.tags.clear();
        f.show_hidden |= with_hidden;
        let mut v = lib.filter(&f);
        v.retain(|&i| self.extra_match(&lib.games[i]));
        v
    }

    /// Several values are picked where Match all / Match any makes a difference.
    pub(crate) fn can_match_all(&self) -> bool {
        self.filter.tags.len() > 1 || Facet::ALL.iter().any(|f| f.multi_valued() && self.facet_sel[f.index()].len() > 1)
    }

    /// The active source / tag / field filters, for the chips above the games.
    pub(crate) fn active_filters(&self) -> Vec<(String, FilterValue)> {
        let mut out = vec![];
        if let Some(id) = self.coll_sel
            && let Some((_, name, _, _)) = self.counts.colls.iter().find(|c| c.0 == id)
        {
            out.push((format!("Collection: {name}"), FilterValue::Collection(id)));
        }
        for &s in &self.src_sel {
            out.push((format!("Source: {}", s.label()), FilterValue::Source(s)));
        }
        for t in &self.filter.tags {
            out.push((format!("Tag: {t}"), FilterValue::Tag(t.clone())));
        }
        for f in Facet::ALL {
            for k in &self.facet_sel[f.index()] {
                // Display spelling from the counts (selection keys are lowercase); platforms
                // by their short name, like the explorer shows them.
                let shown = self.counts.facets[f.index()]
                    .iter()
                    .find(|(key, _, _)| key == k)
                    .map_or(k.as_str(), |(_, d, _)| d.as_str());
                // Platforms by the name running text uses ("PlayStation 2"); the sidebar's
                // short form ("PS2") is only there because the sidebar is narrow.
                let (label, shown) = if f == Facet::Platform {
                    (super::facets::platform_display(shown), platform_icons::short_label(shown))
                } else {
                    (shown.to_string(), shown)
                };
                out.push((format!("{}: {label}", f.noun()), FilterValue::Facet(f, shown.to_string())));
            }
        }
        out
    }

    /// Removable chips for the active filters above the games (nothing when there are none),
    /// so a filter set from a game page is visible even with the explorer hidden.
    pub(super) fn filter_chips(&mut self, ui: &mut Ui) {
        if self.no_extra_filters() {
            return;
        }
        let pal = self.pal;
        let active = self.active_filters();
        egui::Frame::new().inner_margin(egui::Margin { left: 14, right: 14, top: 8, bottom: 2 }).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for (label, v) in active {
                    if filter_chip(ui, &label, &pal).on_hover_text("Remove this filter").clicked() {
                        self.push(Action::Unfilter(v));
                    }
                }
                if self.can_match_all() {
                    ui.add_space(4.0);
                    if let Some(all) = match_switch(ui, self.match_all, &pal) {
                        self.match_all = all;
                    }
                }
                if ui
                    .add(egui::Button::new(egui::RichText::new("Clear all").size(12.5).color(pal.dim)).frame(false))
                    .clicked()
                {
                    self.clear_extras();
                }
            });
        });
    }

    /// Clear source / tag / facet selections (a plain click picks exactly one thing).
    fn clear_extras(&mut self) {
        self.coll_sel = None;
        self.src_sel.clear();
        self.filter.tags.clear();
        self.facet_sel.iter_mut().for_each(Vec::clear);
    }

    /// Collapsible section header; returns whether the section is open. A collapsed section
    /// with an active selection shows a dot so the filter is never invisible.
    fn section(&mut self, ui: &mut Ui, title: &str, key: &str, n: usize, pal: &Pal) -> bool {
        self.section_resp(ui, title, key, n, pal).0
    }

    /// [`Self::section`], also returning the header (for a right-click menu or a drop).
    fn section_resp(&mut self, ui: &mut Ui, title: &str, key: &str, n: usize, pal: &Pal) -> (bool, Response) {
        let open = !self.collapsed.iter().any(|k| k == key);
        let active = match key {
            "sources" => !self.src_sel.is_empty(),
            "collections" => self.coll_sel.is_some(),
            "tags" => !self.filter.tags.is_empty(),
            "library" => false,
            k => Facet::ALL.iter().find(|f| f.key() == k).is_some_and(|f| !self.facet_sel[f.index()].is_empty()),
        };
        ui.add_space(if key == "library" { 2.0 } else { 9.0 });
        let w = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(vec2(w, 22.0), Sense::click());
        if ui.is_rect_visible(rect) {
            let p = ui.painter();
            let hovered = resp.hovered();
            let col = if hovered { pal.text } else { pal.dim };
            // Chevron: ▾ open, ▸ closed (drawn, not a glyph).
            let c = pos2(rect.left() + 12.0, rect.center().y);
            let s = 3.5;
            let pts = if open {
                vec![c + vec2(-s, -s * 0.5), c + vec2(s, -s * 0.5), c + vec2(0.0, s * 0.7)]
            } else {
                vec![c + vec2(-s * 0.5, -s), c + vec2(-s * 0.5, s), c + vec2(s * 0.7, 0.0)]
            };
            p.add(egui::Shape::convex_polygon(pts, with_alpha(col, 200), egui::Stroke::NONE));
            let mut job = egui::text::LayoutJob::default();
            job.append(
                title,
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::proportional(11.0),
                    color: col,
                    extra_letter_spacing: 1.0,
                    ..Default::default()
                },
            );
            let g = p.layout_job(job);
            let ty = rect.center().y - g.size().y / 2.0;
            let tw = g.size().x;
            p.galley(pos2(rect.left() + 22.0, ty), g, col);
            if !open && n > 0 {
                let cg = p.layout_no_wrap(n.to_string(), egui::FontId::proportional(11.0), with_alpha(pal.dim, 170));
                let cx = rect.right() - 14.0 - cg.size().x;
                p.galley(pos2(cx, rect.center().y - cg.size().y / 2.0), cg, pal.dim);
            }
            if active && !open {
                p.circle_filled(pos2(rect.left() + 22.0 + tw + 8.0, rect.center().y), 3.0, pal.accent);
            }
        }
        let resp = resp.on_hover_text(if open { "Collapse" } else { "Expand" });
        if resp.clicked() {
            if open {
                self.collapsed.push(key.to_string());
            } else {
                self.collapsed.retain(|k| k != key);
            }
        }
        ui.add_space(2.0);
        // The click applies next frame's layout; draw this frame as it was.
        (open, resp)
    }

    /// "PLATFORMS": PCs, then each console by its short name ("PS2", "Switch") with a device
    /// icon. Same filter as the other field sections (`Facet::Platform`).
    fn platforms_section(&mut self, ui: &mut Ui, st: &mut AppState, pal: &Pal, multi: bool) {
        let f = Facet::Platform;
        let fi = f.index();
        if self.counts.facets[fi].is_empty() {
            return;
        }
        let n = self.counts.facets[fi].len();
        if !self.section(ui, f.title(), f.key(), n, pal) {
            return;
        }
        let mut clicked: Option<String> = None;
        let mut art_for: Option<(String, String)> = None;
        let has_pc = self.counts.facets[fi].first().is_some_and(|(k, _, _)| k == super::facets::PC_KEY);
        for (key, label, n) in &self.counts.facets[fi] {
            let selected = self.facet_sel[fi].contains(key);
            let foc = self.side_nav.is(|e| matches!(e, Entry::Facet(i, k) if *i == fi && k == key));
            let short = platform_icons::short_label(label);
            let shape = if key == super::facets::PC_OTHER_KEY {
                platform_icons::Shape::Monitor
            } else {
                platform_icons::shape_for_label(label)
            };
            // Linux / Windows / macOS sit under "PC", a step in.
            let other = key == super::facets::PC_OTHER_KEY;
            let under_pc = has_pc && key != super::facets::PC_KEY && (other || platform_icons::is_pc_label(label));
            let r = if under_pc {
                ui.horizontal(|ui| {
                    ui.add_space(PC_INDENT);
                    row(ui, short, *n, selected, foc, pal, Some(shape))
                })
                .inner
            } else {
                row(ui, short, *n, selected, foc, pal, Some(shape))
            };
            if foc {
                self.side_nav.reveal(&r);
            }
            let r = if other {
                r.on_hover_text("PC games without a platform of their own (often store games that aren't installed)")
            } else if short != label.as_str() {
                r.on_hover_text(super::facets::platform_display(label))
            } else {
                r
            };
            if r.clicked() {
                clicked = Some(key.clone());
            }
            if art_menu(&r, *n) {
                art_for = Some((key.clone(), label.clone()));
            }
        }
        if let Some((key, label)) = art_for {
            let want = [key];
            let ids = section_games(st, |g| !g.hidden && f.matches(g, &want));
            self.push(super::Action::BulkArt(super::artwork::BulkScope::Section(format!("“{label}”"), ids)));
        }
        if let Some(k) = clicked {
            if !multi {
                self.clear_extras();
            }
            toggle(&mut self.facet_sel[fi], k, multi);
            self.lib_sel = LibSel::All;
        }
    }
}

/// How far Linux / Windows / macOS sit in under PLATFORMS → PC.
const PC_INDENT: f32 = 12.0;

/// Right-click menu on an explorer row; true when "Download artwork…" was chosen.
fn art_menu(r: &Response, n: usize) -> bool {
    let mut chosen = false;
    r.context_menu(|ui| {
        ui.set_min_width(220.0);
        let label = format!("Download artwork for {n} game{}…", if n == 1 { "" } else { "s" });
        if ui.button(label).clicked() {
            chosen = true;
        }
    });
    chosen
}

/// Ids of the library's games matching `pred`, in library order.
fn section_games(st: &AppState, pred: impl Fn(&crate::model::Game) -> bool) -> Vec<crate::model::GameId> {
    st.library.games.iter().filter(|g| pred(g)).map(|g| g.id).collect()
}

/// Toggle `v` in `list`: exclusive unless `multi`. Clicking the only selected item clears it.
fn toggle<T: PartialEq>(list: &mut Vec<T>, v: T, multi: bool) {
    let present = list.contains(&v);
    if multi {
        if present {
            list.retain(|x| *x != v);
        } else {
            list.push(v);
        }
    } else if present && list.len() == 1 {
        list.clear();
    } else {
        list.clear();
        list.push(v);
    }
}

/// "Match any | Match all" for several picked values; returns the new choice when clicked.
fn match_switch(ui: &mut Ui, all: bool, pal: &Pal) -> Option<bool> {
    let font = egui::FontId::proportional(12.5);
    let labels = ["Match any", "Match all"];
    let gals: Vec<_> =
        labels.iter().map(|l| ui.painter().layout_no_wrap(l.to_string(), font.clone(), pal.text)).collect();
    let h = 24.0;
    let ws: Vec<f32> = gals.iter().map(|g| g.size().x + 20.0).collect();
    let (rect, _) = ui.allocate_exact_size(vec2(ws[0] + ws[1], h), Sense::hover());
    let mut out = None;
    let mut x = rect.left();
    for (i, gal) in gals.into_iter().enumerate() {
        let r = egui::Rect::from_min_size(pos2(x, rect.top()), vec2(ws[i], h));
        x += ws[i];
        let on = (i == 1) == all;
        let resp = ui.interact(r, ui.scope_id().with(("ls_match", i)), Sense::click()).on_hover_text(if i == 0 {
            "Show games that have any of the picked values"
        } else {
            "Show only games that have every picked tag, genre, feature, developer or \
                 publisher (a game has one source, platform or status, so those stay \
                 “any”)"
        });
        if resp.clicked() && !on {
            out = Some(i == 1);
        }
        let p = ui.painter();
        let cr = if i == 0 {
            egui::CornerRadius { nw: 12, sw: 12, ne: 0, se: 0 }
        } else {
            egui::CornerRadius { nw: 0, sw: 0, ne: 12, se: 12 }
        };
        let fill = if on {
            with_alpha(pal.accent, 150)
        } else if resp.hovered() {
            pal.hover
        } else {
            with_alpha(pal.surface_alt, 160)
        };
        p.rect_filled(r, cr, fill);
        let col = if on { pal.text } else { pal.dim };
        p.galley_with_override_text_color(r.center() - gal.size() / 2.0, gal, col);
    }
    ui.painter().rect_stroke(
        rect,
        h / 2.0,
        egui::Stroke::new(1.0, with_alpha(pal.accent, 120)),
        egui::StrokeKind::Inside,
    );
    out
}

/// One active-filter chip: accent outline, label and a ×; the whole chip removes the filter.
fn filter_chip(ui: &mut Ui, label: &str, pal: &Pal) -> Response {
    let font = egui::FontId::proportional(12.5);
    let gal = ui.painter().layout_no_wrap(label.to_owned(), font, pal.text);
    let h = 24.0;
    let size = vec2(10.0 + gal.size().x + 8.0 + 8.0 + 9.0, h);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let hov = resp.hovered();
        p.rect_filled(rect, h / 2.0, with_alpha(pal.accent, if hov { 70 } else { 40 }));
        p.rect_stroke(rect, h / 2.0, egui::Stroke::new(1.0, with_alpha(pal.accent, 120)), egui::StrokeKind::Inside);
        p.galley(pos2(rect.left() + 10.0, rect.center().y - gal.size().y / 2.0), gal, pal.text);
        let xr = egui::Rect::from_center_size(pos2(rect.right() - 13.0, rect.center().y), vec2(8.0, 8.0));
        crate::ui::widgets::paint_icon(p, xr, crate::ui::widgets::Icon::Close, if hov { pal.text } else { pal.dim });
    }
    resp
}

/// One entry; `focused`: the sidebar has the keyboard and this is its entry (accent ring);
/// `icon`: a small device / folder outline before the label. A zero count isn't shown.
fn row(
    ui: &mut Ui,
    label: &str,
    count: usize,
    selected: bool,
    focused: bool,
    pal: &Pal,
    icon: Option<Shape>,
) -> Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::CLICK);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let inner = rect.shrink2(vec2(6.0, 0.0));
        if selected {
            p.rect_filled(inner, 4.0, pal.sel_bg);
            p.rect_filled(
                egui::Rect::from_min_size(pos2(inner.left(), inner.top() + 5.0), vec2(3.0, ROW_H - 10.0)),
                1.5,
                pal.accent,
            );
        } else if resp.hovered() || resp.has_focus() {
            p.rect_filled(inner, 4.0, pal.hover);
        }
        if focused {
            p.rect_stroke(inner, 4.0, egui::Stroke::new(2.0, pal.accent), egui::StrokeKind::Inside);
        }
        let cx = if count > 0 {
            let cg = p.layout_no_wrap(count.to_string(), egui::FontId::proportional(12.0), pal.dim);
            let cx = inner.right() - 8.0 - cg.size().x;
            p.galley(pos2(cx, rect.center().y - cg.size().y / 2.0), cg, pal.dim);
            cx
        } else {
            inner.right() - 8.0
        };
        let fg = if selected { pal.text } else { crate::ui::widgets::mix(pal.text, pal.dim, 0.2) };
        let mut x = inner.left() + 12.0;
        if let Some(shape) = icon {
            let ir = egui::Rect::from_center_size(pos2(x + 8.0, rect.center().y), vec2(16.0, 16.0));
            platform_icons::paint(p, ir, shape, with_alpha(fg, if selected { 255 } else { 200 }));
            x += 24.0;
        }
        let g = truncated(p, label, egui::FontId::proportional(13.5), fg, cx - x - 10.0);
        p.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, Color32::PLACEHOLDER);
    }
    resp
}

/// Ctrl / Shift held: a click adds to the selection instead of replacing it.
fn multi_key(ui: &Ui) -> bool {
    ui.input(|i| i.modifiers.command || i.modifiers.shift)
}

#[cfg(test)]
mod tests {
    use super::toggle;

    #[test]
    fn toggle_semantics() {
        let mut v = vec![];
        toggle(&mut v, 1, false);
        assert_eq!(v, [1]);
        toggle(&mut v, 2, false);
        assert_eq!(v, [2]);
        toggle(&mut v, 3, true);
        assert_eq!(v, [2, 3]);
        toggle(&mut v, 2, true);
        assert_eq!(v, [3]);
        toggle(&mut v, 3, false);
        assert!(v.is_empty());
    }
}
