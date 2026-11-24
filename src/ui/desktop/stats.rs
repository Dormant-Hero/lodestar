//! The Statistics page (menu → Statistics): totals, completion, most played, sources and
//! platforms — for the whole library or the current filter, with or without hidden games.
//! Shown in place of the games, so the sidebar keeps working next to it (pick a filter, the
//! numbers follow). Charts are drawn here with the painter in the theme's colours. Esc or
//! "Back to games" closes it; a bar or a game opens the games it stands for.

use super::{Action, DesktopUi, FilterValue, LibSel, Pal};
use crate::app::AppState;
use crate::images::ImgSize;
use crate::model::GameId;
use crate::ui::widgets::{mix, name_tint, paint_cover, truncated, with_alpha};
use egui::{Color32, Rect, RichText, Sense, Ui, pos2, vec2};

pub(crate) mod data;
use data::{Row, Stats, games, hours, sort_rows, thousands};

/// The page's switches and its cached numbers.
#[derive(Default)]
pub(crate) struct StatsPage {
    /// The current filter (else the whole library).
    pub filtered: bool,
    /// Count hidden games too.
    pub hidden: bool,
    /// Source / platform bars by play time (else by number of games).
    pub by_time: bool,
    cache: Option<(Key, Stats)>,
}

/// What the numbers were computed for: library generation, filter fingerprint, switches.
type Key = (u64, u64, bool, bool);

impl StatsPage {
    /// Testing hook (`LODESTAR_DESKTOP_OPEN=stats[-filter][-hidden][-time]`).
    pub(crate) fn debug(rest: &str) -> StatsPage {
        let has = |w: &str| rest.split('-').any(|x| x == w);
        StatsPage { filtered: has("filter"), hidden: has("hidden"), by_time: has("time"), cache: None }
    }
}

/// Something clicked on the page (applied after drawing).
enum Click {
    Close,
    Filter(FilterValue),
    Game(GameId),
}

impl DesktopUi {
    /// Fingerprint of everything that decides the current filter's games.
    fn filter_fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.explorer_hash().hash(&mut h);
        self.filter.search.trim().hash(&mut h);
        self.filter.installed_only.hash(&mut h);
        self.filter.show_hidden.hash(&mut h);
        h.finish()
    }

    /// No narrowing at all: the current filter is the whole library.
    fn nothing_filtered(&self) -> bool {
        self.no_extra_filters()
            && self.lib_sel == LibSel::All
            && self.filter.search.trim().is_empty()
            && !self.filter.installed_only
    }

    /// "Steam · RPG · Installed", for the line under the title.
    fn filter_summary(&self) -> String {
        let mut parts: Vec<String> = vec![];
        match self.lib_sel {
            LibSel::All => {}
            LibSel::Installed => parts.push("Installed".into()),
            LibSel::NotInstalled => parts.push("Not installed".into()),
            LibSel::Favorites => parts.push("Favourites".into()),
            LibSel::Recent => parts.push("Recently played".into()),
            LibSel::Hidden => parts.push("Hidden".into()),
        }
        if self.filter.installed_only && self.lib_sel != LibSel::Installed {
            parts.push("Installed only".into());
        }
        for (label, _) in self.active_filters() {
            parts.push(label);
        }
        let q = self.filter.search.trim();
        if !q.is_empty() {
            parts.push(format!("“{q}”"));
        }
        parts.join(" · ")
    }

    /// The numbers for the page's switches (cached until the library or the filter changes).
    fn stats_numbers(&mut self, st: &AppState) -> Stats {
        let filtered = self.stats.as_ref().is_some_and(|p| p.filtered);
        // On the Hidden entry every game is hidden: counting them is the point.
        let hidden = self.stats.as_ref().is_some_and(|p| p.hidden) || (filtered && self.lib_sel == LibSel::Hidden);
        let fp = if filtered { self.filter_fingerprint() } else { 0 };
        let key: Key = (st.library.generation, fp, filtered, hidden);
        if let Some(p) = &self.stats
            && let Some((k, s)) = &p.cache
            && *k == key
        {
            return s.clone();
        }
        let scope: Vec<usize> =
            if filtered { self.filtered_indices(&st.library, true) } else { (0..st.library.games.len()).collect() };
        let s = Stats::compute(&st.library, &scope, hidden);
        if let Some(p) = &mut self.stats {
            p.cache = Some((key, s.clone()));
        }
        s
    }

    pub(super) fn stats_page(&mut self, ui: &mut Ui, st: &mut AppState) {
        let pal = self.pal;
        let s = self.stats_numbers(st);
        let nothing_filtered = self.nothing_filtered();
        let summary = self.filter_summary();
        let Some(page) = self.stats.as_mut() else { return };
        let mut click: Option<Click> = None;
        egui::ScrollArea::vertical().id_salt("ls_stats_scroll").auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin::symmetric(24, 18)).show(ui, |ui| {
                let w = ui.available_width();
                // Title, back button.
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Statistics").family(self.fonts.bold.clone()).size(24.0).strong().color(pal.text),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(egui::Button::new(RichText::new("Back to games").size(13.5)))
                            .on_hover_text("Esc")
                            .clicked()
                        {
                            click = Some(Click::Close);
                        }
                    });
                });
                ui.add_space(8.0);
                // Scope switches.
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if let Some(v) = segmented(
                        ui,
                        "ls_stats_scope",
                        &["Whole library", "Current filter"],
                        page.filtered as usize,
                        &pal,
                    ) {
                        page.filtered = v == 1;
                    }
                    ui.add_space(6.0);
                    ui.checkbox(&mut page.hidden, RichText::new("Include hidden games").size(13.0));
                });
                ui.add_space(4.0);
                let line = if !page.filtered {
                    "Every game in your library".to_string()
                } else if nothing_filtered {
                    "No filter picked: every game in your library. Pick one in the sidebar.".to_string()
                } else {
                    summary.clone()
                };
                ui.label(RichText::new(line).size(13.0).color(pal.dim));
                ui.add_space(14.0);

                if s.games == 0 {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("No games here").size(16.0).color(pal.dim));
                    });
                    return;
                }

                tiles(ui, &s, page.hidden, &pal);
                ui.add_space(16.0);

                let two = w >= 860.0;
                let col_w = if two { (w - 16.0) / 2.0 } else { w };
                let by_time = &mut page.by_time;
                let mut left = |ui: &mut Ui, click: &mut Option<Click>| {
                    card(ui, "Completion", col_w, &pal, None, |ui| {
                        let max = s.completion.iter().map(|r| r.games).max().unwrap_or(1);
                        for r in &s.completion {
                            let text = format!("{}  ·  {}", games(r.games), pct(r.games, s.games));
                            if bar_row(ui, &r.label, r.games as f64 / max as f64, &text, r.filter.is_some(), &pal)
                                .clicked()
                                && let Some(f) = &r.filter
                            {
                                *click = Some(Click::Filter(f.clone()));
                            }
                        }
                    });
                    ui.add_space(16.0);
                    card(ui, "Most played", col_w, &pal, None, |ui| {
                        if s.top.is_empty() {
                            ui.label(RichText::new("Nothing played yet").size(13.0).color(pal.dim));
                        }
                        let max = s.top.first().map_or(1, |t| t.1.max(1));
                        for (rank, &(id, secs)) in s.top.iter().enumerate() {
                            if let Some(g) = st.library.get(id)
                                && top_row(ui, &mut st.images, g, rank + 1, secs, max, &pal).clicked()
                            {
                                *click = Some(Click::Game(id));
                            }
                        }
                    });
                };
                let mut right = |ui: &mut Ui, click: &mut Option<Click>| {
                    let mut toggled = None;
                    for (title, rows) in [("Sources", &s.sources), ("Platforms", &s.platforms)] {
                        let mut rows: Vec<Row> = rows.clone();
                        sort_rows(&mut rows, *by_time);
                        let cur = *by_time as usize;
                        card(ui, title, col_w, &pal, Some((&mut toggled, cur, title)), |ui| {
                            rows_chart(ui, &rows, *by_time, &pal, click);
                        });
                        ui.add_space(16.0);
                    }
                    if let Some(t) = toggled {
                        *by_time = t;
                    }
                };
                if two {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 16.0;
                        ui.vertical(|ui| {
                            ui.set_width(col_w);
                            left(ui, &mut click);
                        });
                        ui.vertical(|ui| {
                            ui.set_width(col_w);
                            right(ui, &mut click);
                        });
                    });
                } else {
                    left(ui, &mut click);
                    ui.add_space(16.0);
                    right(ui, &mut click);
                }
            });
        });
        let in_filter = page.filtered;
        if let Some(c) = click {
            self.stats_click(st, c, in_filter);
        }
    }

    /// Apply a click on the page: close it, show the games a bar stands for, or select a game
    /// (making sure it is in the view).
    fn stats_click(&mut self, st: &AppState, click: Click, in_filter: bool) {
        match click {
            Click::Close => self.stats = None,
            Click::Filter(v) => {
                // From the current filter: narrow it to that value (it replaces other values
                // picked in the same field, which would otherwise show those too); else start
                // from that value alone, like the bar was counted (whole library, no search).
                self.stats = None;
                if in_filter {
                    match &v {
                        FilterValue::Facet(f, _) => self.facet_sel[f.index()].clear(),
                        FilterValue::Source(_) => self.src_sel.clear(),
                        _ => {}
                    }
                } else {
                    self.set_lib_sel(LibSel::All);
                    self.filter.search.clear();
                }
                self.push(Action::FilterBy(v, in_filter));
            }
            Click::Game(id) => {
                self.stats = None;
                let in_view = self.view.iter().any(|&i| st.library.games[i].id == id);
                if !in_view {
                    let g = st.library.get(id);
                    let sel = if g.is_some_and(|g| g.hidden) {
                        LibSel::Hidden
                    } else if g.is_some_and(|g| !g.installed)
                        && st.config.desktop.preset == crate::config::QuickPreset::Installed
                    {
                        // The top bar's Installed toggle would hide it.
                        LibSel::NotInstalled
                    } else {
                        LibSel::All
                    };
                    self.set_lib_sel(sel);
                    self.filter.search.clear();
                }
                self.push(Action::Select(id));
                self.scroll_to_sel = true;
            }
        }
    }
}

/// "12 %" of a total (no decimals; "<1 %" for a sliver).
fn pct(n: usize, total: usize) -> String {
    if total == 0 {
        return "0 %".into();
    }
    let p = n as f64 * 100.0 / total as f64;
    if p > 0.0 && p < 1.0 { "<1 %".into() } else { format!("{:.0} %", p) }
}

/// The row of number tiles at the top.
fn tiles(ui: &mut Ui, s: &Stats, with_hidden: bool, pal: &Pal) {
    let size = if s.no_size > 0 && s.size_bytes > 0 {
        format!("{} without a size", games(s.no_size))
    } else if s.size_bytes == 0 {
        "not known yet".into()
    } else {
        "installed games".into()
    };
    let hidden_sub = match (s.hidden, with_hidden) {
        (0, _) => "",
        (_, true) => "counted",
        (_, false) => "not counted",
    };
    let items: [(String, &str, String); 8] = [
        (thousands(s.games as u64), "Games", String::new()),
        (thousands(s.installed as u64), "Installed", pct(s.installed, s.games)),
        (thousands(s.favorites as u64), "Favourites", String::new()),
        (thousands(s.hidden as u64), "Hidden", hidden_sub.into()),
        (hours(s.total_secs), "Play time", format!("{} played", games(s.played))),
        (hours(s.average_secs()), "Average play time", "per played game".into()),
        (if s.size_bytes > 0 { super::details::format_size(s.size_bytes) } else { "—".into() }, "Install size", size),
        (pct(s.played, s.games), "Played", "of these games".into()),
    ];
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
        let w = ui.available_width();
        // 4 per row when wide, 2 when narrow; always the full width.
        let per = if w >= 760.0 { 4.0 } else { 2.0 };
        let tw = ((w - 12.0 * (per - 1.0)) / per).floor();
        for (value, label, sub) in &items {
            let (rect, _) = ui.allocate_exact_size(vec2(tw, 78.0), Sense::hover());
            let p = ui.painter();
            p.rect_filled(rect, 8.0, pal.surface);
            p.rect_stroke(rect, 8.0, egui::Stroke::new(1.0, with_alpha(pal.border, 140)), egui::StrokeKind::Inside);
            let x = rect.left() + 14.0;
            let lg = p.layout_no_wrap(label.to_string(), egui::FontId::proportional(12.5), pal.dim);
            p.galley(pos2(x, rect.top() + 10.0), lg, pal.dim);
            let vg = truncated(p, value, egui::FontId::proportional(22.0), pal.text, tw - 28.0);
            p.galley(pos2(x, rect.top() + 28.0), vg, pal.text);
            if !sub.is_empty() {
                let sg = truncated(p, sub, egui::FontId::proportional(11.5), with_alpha(pal.dim, 200), tw - 28.0);
                p.galley(pos2(x, rect.bottom() - 20.0), sg, pal.dim);
            }
        }
    });
}

/// A titled card of width `w`. `toggle`: a "Games | Play time" switch in the header (the
/// clicked choice goes into the `Option<bool>`).
fn card(
    ui: &mut Ui,
    title: &str,
    w: f32,
    pal: &Pal,
    toggle: Option<(&mut Option<bool>, usize, &str)>,
    add: impl FnOnce(&mut Ui),
) {
    egui::Frame::new()
        .fill(pal.surface)
        .stroke(egui::Stroke::new(1.0, with_alpha(pal.border, 140)))
        .corner_radius(8)
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(w - 28.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).size(15.0).strong().color(pal.text));
                if let Some((out, cur, id)) = toggle {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(v) = segmented(ui, &format!("ls_stats_by_{id}"), &["Games", "Play time"], cur, pal)
                        {
                            *out = Some(v == 1);
                        }
                    });
                }
            });
            ui.add_space(8.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            add(ui);
        });
}

/// The source / platform bars (the longest ten, then "Other" for the rest).
fn rows_chart(ui: &mut Ui, rows: &[Row], by_time: bool, pal: &Pal, click: &mut Option<Click>) {
    const SHOWN: usize = 10;
    let val = |r: &Row| if by_time { r.secs as f64 } else { r.games as f64 };
    let mut shown: Vec<Row> = rows.iter().take(SHOWN).cloned().collect();
    if rows.len() > SHOWN {
        let rest = &rows[SHOWN..];
        shown.push(Row {
            label: format!("{} more", rest.len()),
            filter: None,
            games: rest.iter().map(|r| r.games).sum(),
            secs: rest.iter().map(|r| r.secs).sum(),
        });
    }
    let max = shown.iter().map(val).fold(0.0, f64::max).max(1.0);
    for r in &shown {
        let text = format!("{}  ·  {}", games(r.games), hours(r.secs));
        if bar_row(ui, &r.label, val(r) / max, &text, r.filter.is_some(), pal).clicked()
            && let Some(f) = &r.filter
        {
            *click = Some(Click::Filter(f.clone()));
        }
    }
}

/// One horizontal bar: label, bar (`frac` of the space), value text. Clickable rows highlight
/// on hover and say what a click does.
fn bar_row(ui: &mut Ui, label: &str, frac: f64, text: &str, clickable: bool, pal: &Pal) -> egui::Response {
    let w = ui.available_width();
    let h = 26.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), if clickable { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let hov = clickable && resp.hovered();
        if hov {
            p.rect_filled(rect.expand2(vec2(4.0, 0.0)), 4.0, pal.hover);
        }
        let label_w = (w * 0.26).clamp(90.0, 170.0);
        let text_w = (w * 0.30).clamp(110.0, 190.0);
        let fg = if hov { pal.text } else { mix(pal.text, pal.dim, 0.15) };
        let lg = truncated(p, label, egui::FontId::proportional(13.0), fg, label_w - 8.0);
        p.galley(pos2(rect.left(), rect.center().y - lg.size().y / 2.0), lg, fg);
        let track = Rect::from_min_max(
            pos2(rect.left() + label_w, rect.center().y - 6.0),
            pos2(rect.right() - text_w, rect.center().y + 6.0),
        );
        if track.width() > 4.0 {
            p.rect_filled(track, 3.0, with_alpha(pal.text, 16));
            let fw = (track.width() * frac.clamp(0.0, 1.0) as f32).max(if frac > 0.0 { 3.0 } else { 0.0 });
            let fill = Rect::from_min_size(track.min, vec2(fw, track.height()));
            p.rect_filled(fill, 3.0, if hov { mix(pal.accent, pal.text, 0.2) } else { pal.accent });
        }
        let tg = truncated(p, text, egui::FontId::proportional(12.5), pal.dim, text_w - 10.0);
        p.galley(pos2(rect.right() - tg.size().x, rect.center().y - tg.size().y / 2.0), tg, pal.dim);
    }
    if clickable { resp.on_hover_text("Show these games") } else { resp }
}

/// One "Most played" row: rank, small cover, name with a thin bar, play time.
fn top_row(
    ui: &mut Ui,
    images: &mut crate::images::ImageCache,
    g: &crate::model::Game,
    rank: usize,
    secs: u64,
    max: u64,
    pal: &Pal,
) -> egui::Response {
    let w = ui.available_width();
    let h = 44.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    if ui.is_rect_visible(rect) {
        let hov = resp.hovered();
        if hov {
            ui.painter().rect_filled(rect.expand2(vec2(4.0, 0.0)), 4.0, pal.hover);
        }
        let p = ui.painter();
        let rg = p.layout_no_wrap(rank.to_string(), egui::FontId::proportional(13.0), pal.dim);
        p.galley(pos2(rect.left() + 18.0 - rg.size().x, rect.center().y - rg.size().y / 2.0), rg, pal.dim);
        let cover = Rect::from_min_size(pos2(rect.left() + 28.0, rect.top() + 3.0), vec2(28.0, h - 6.0));
        let tex = g.art.cover.as_ref().and_then(|c| images.get(c, ImgSize::Icon));
        match tex {
            Some(t) => paint_cover(p, cover, &t, 3, Color32::WHITE, vec2(0.5, 0.25)),
            None => {
                p.rect_filled(cover, 3.0, name_tint(g.display_name(), pal.surface_alt));
            }
        }
        let x = cover.right() + 12.0;
        let tg = p.layout_no_wrap(hours(secs), egui::FontId::proportional(12.5), pal.dim);
        // A fixed column for the time, so the bars line up whatever the text.
        let name_w = rect.right() - x - tg.size().x.max(84.0) - 16.0;
        let fg = if hov { pal.text } else { mix(pal.text, pal.dim, 0.1) };
        let ng = truncated(p, g.display_name(), egui::FontId::proportional(13.5), fg, name_w);
        p.galley(pos2(x, rect.top() + 7.0), ng, fg);
        let track = Rect::from_min_size(pos2(x, rect.bottom() - 13.0), vec2(name_w.max(10.0), 4.0));
        p.rect_filled(track, 2.0, with_alpha(pal.text, 16));
        let fw = track.width() * (secs as f32 / max.max(1) as f32).clamp(0.0, 1.0);
        p.rect_filled(Rect::from_min_size(track.min, vec2(fw.max(2.0), 4.0)), 2.0, pal.accent);
        p.galley(pos2(rect.right() - tg.size().x, rect.center().y - tg.size().y / 2.0), tg, pal.dim);
    }
    resp.on_hover_text("Show this game")
}

/// A small segmented switch ("Whole library | Current filter"); returns the clicked option.
fn segmented(ui: &mut Ui, id: &str, labels: &[&str], cur: usize, pal: &Pal) -> Option<usize> {
    let font = egui::FontId::proportional(13.0);
    let gals: Vec<_> =
        labels.iter().map(|l| ui.painter().layout_no_wrap(l.to_string(), font.clone(), pal.text)).collect();
    let h = 26.0;
    let ws: Vec<f32> = gals.iter().map(|g| g.size().x + 24.0).collect();
    let (rect, _) = ui.allocate_exact_size(vec2(ws.iter().sum(), h), Sense::hover());
    let mut out = None;
    let mut x = rect.left();
    let last = gals.len().saturating_sub(1);
    for (i, gal) in gals.into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(x, rect.top()), vec2(ws[i], h));
        x += ws[i];
        let on = i == cur;
        let resp = ui.interact(r, ui.scope_id().with((id, i)), Sense::click());
        if resp.clicked() && !on {
            out = Some(i);
        }
        let rad = 13u8;
        let cr = egui::CornerRadius {
            nw: if i == 0 { rad } else { 0 },
            sw: if i == 0 { rad } else { 0 },
            ne: if i == last { rad } else { 0 },
            se: if i == last { rad } else { 0 },
        };
        let fill = if on {
            with_alpha(pal.accent, 160)
        } else if resp.hovered() {
            pal.hover
        } else {
            pal.surface_alt
        };
        let p = ui.painter();
        p.rect_filled(r, cr, fill);
        let col = if on { pal.text } else { pal.dim };
        p.galley_with_override_text_color(r.center() - gal.size() / 2.0, gal, col);
    }
    ui.painter().rect_stroke(
        rect,
        h / 2.0,
        egui::Stroke::new(1.0, with_alpha(pal.border, 200)),
        egui::StrokeKind::Inside,
    );
    out
}

#[cfg(test)]
mod tests {
    use super::super::{Facet, SrcKey};
    use super::*;
    use crate::library::Library;
    use crate::model::{Game, Source};

    fn game(id: GameId, source: Source, installed: bool) -> Game {
        let mut g = Game::blank(&format!("G{id}"));
        g.id = id;
        g.source = source;
        g.installed = installed;
        g.playtime_secs = 3600 * id as u64;
        g
    }

    #[test]
    fn clicks_show_the_games_they_stand_for() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        st.library =
            Library::new(vec![game(1, Source::Steam, true), game(2, Source::Gog, true), game(3, Source::Steam, false)]);
        st.config.desktop.installed_only = true;
        st.config.desktop.preset = crate::config::QuickPreset::Installed;
        let mut ui = DesktopUi::new();
        ui.sync_filter(&st);
        // Current filter with Steam + GOG picked: a Steam bar narrows to Steam alone.
        ui.src_sel = vec![SrcKey::source(Source::Steam), SrcKey::source(Source::Gog)];
        ui.facet_sel[Facet::Completion.index()] = vec!["playing".into(), "beaten".into()];
        ui.stats_click(&st, Click::Filter(FilterValue::Source(SrcKey::source(Source::Steam))), true);
        ui.stats_click(&st, Click::Filter(FilterValue::Facet(Facet::Completion, "Beaten".into())), true);
        ui.apply_actions(&st.ctx().clone(), &mut st);
        assert_eq!(ui.src_sel, [SrcKey::source(Source::Steam)]);
        assert_eq!(ui.facet_sel[Facet::Completion.index()], ["beaten"]);
        // Whole library: the value alone, from every game (search and Library entry reset).
        ui.lib_sel = LibSel::Favorites;
        ui.filter.search = "g".into();
        ui.stats_click(&st, Click::Filter(FilterValue::Source(SrcKey::source(Source::Gog))), false);
        ui.apply_actions(&st.ctx().clone(), &mut st);
        assert_eq!(ui.lib_sel, LibSel::All);
        assert!(ui.filter.search.is_empty() && !ui.no_extra_filters());
        assert_eq!(ui.src_sel, [SrcKey::source(Source::Gog)]);
        assert!(ui.facet_sel.iter().all(Vec::is_empty));
        // Most played: an uninstalled game the top bar's Installed toggle hides → Not installed.
        ui.sync_filter(&st);
        ui.refresh_view_for(&st.library);
        ui.stats = Some(StatsPage::default());
        ui.stats_click(&st, Click::Game(3), false);
        assert!(ui.stats.is_none());
        assert_eq!(ui.lib_sel, LibSel::NotInstalled);
        ui.sync_filter(&st);
        ui.refresh_view_for(&st.library);
        assert!(ui.view.iter().any(|&i| st.library.games[i].id == 3), "now in view");
    }
}
