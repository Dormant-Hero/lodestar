//! List view (sortable table) and the compact list used by the Details view. Both virtualised.

use super::grouping::{Blocks, Shown, Span, run_of};
use super::{Action, DesktopUi, Pal, game_menu, multi_menu};
use crate::app::AppState;
use crate::config::SortBy;
use crate::images::{ImageCache, ImgSize};
use crate::model::{Game, format_last_played, format_playtime};
use crate::ui::widgets::{Icon, mix, name_tint, paint_contain, paint_cover, paint_icon, star, truncated};
use egui::{Color32, Rect, Sense, Ui, pos2, vec2};

const ROW_H: f32 = 32.0;
const COMPACT_ROW_H: f32 = 30.0;
/// Group header rows (List / Details list).
const GROUP_H: f32 = 34.0;
const COMPACT_GROUP_H: f32 = 30.0;

/// Content y range that shows row `p` (with its group header when it is the group's first),
/// `margin` around it.
fn row_range(b: &Blocks, spans: &[Span], n: usize, p: usize, row_h: f32, margin: f32) -> (f32, f32) {
    let (_, y) = b.item_pos(p);
    let first = run_of(spans, n, p).0 == p;
    (y - margin - if first { b.header_h } else { 0.0 }, y + row_h + margin)
}

/// Widget id of a row; `group` keeps the copies of a game shown in several groups apart.
fn row_id(ui: &Ui, base: (&str, crate::model::GameId), group: Option<&str>) -> egui::Id {
    let id = ui.scope_id().with(base);
    group.map_or(id, |k| id.with(k))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Col {
    Icon,
    Name,
    Source,
    /// Device icon + short name ("PS2", "Windows").
    Platform,
    Playtime,
    LastPlayed,
    /// "31 / 54" with a bar (only while the library shows achievement counts).
    Achievements,
    Installed,
}

impl Col {
    fn label(self) -> &'static str {
        match self {
            Col::Icon => "",
            Col::Name => "Name",
            Col::Source => "Source",
            Col::Platform => "Platform",
            Col::Playtime => "Play time",
            Col::LastPlayed => "Last played",
            Col::Achievements => "Achievements",
            Col::Installed => "Installed",
        }
    }
    fn sort(self) -> Option<SortBy> {
        match self {
            Col::Name => Some(SortBy::Name),
            Col::Source => Some(SortBy::Source),
            Col::Playtime => Some(SortBy::Playtime),
            Col::LastPlayed => Some(SortBy::LastPlayed),
            Col::Achievements => Some(SortBy::Achievements),
            _ => None,
        }
    }
}

/// Visible columns and their x-ranges for a table width (pure; the name column takes the slack).
/// When the table is narrow, the least important columns are dropped (Installed, then
/// Achievements, Last played, Playtime, Platform, Source) instead of overflowing past the panel
/// edge.
#[derive(Clone, Copy, Debug)]
struct Columns {
    cols: [(Col, f32, f32); 8],
    n: usize,
}

impl Columns {
    fn iter(&self) -> impl Iterator<Item = (Col, f32, f32)> + '_ {
        self.cols[..self.n].iter().copied()
    }
}

const MIN_NAME_W: f32 = 200.0;

/// Width of the Achievements column.
const ACH_W: f32 = 104.0;

/// `achievements`: the library shows achievement counts (the column is there).
fn columns(w: f32, achievements: bool) -> Columns {
    // In display order; the drop order is from the end of this list (Name is never dropped).
    let mut all = vec![
        (Col::Icon, 40.0),
        (Col::Name, 0.0),
        (Col::Source, 110.0),
        (Col::Platform, 110.0),
        (Col::Playtime, 110.0),
        (Col::LastPlayed, 130.0),
    ];
    if achievements {
        all.push((Col::Achievements, ACH_W));
    }
    all.push((Col::Installed, 90.0));
    // Installed goes first when room is short (an uninstalled game's row is dimmed anyway).
    let drop_first = |n: usize| -> Vec<(Col, f32)> {
        if n == all.len() {
            all.clone()
        } else {
            all.iter().copied().filter(|c| c.0 != Col::Installed).take(n).collect()
        }
    };
    let pad = 16.0;
    let mut n = all.len();
    let fixed = |n: usize| drop_first(n).iter().map(|c| c.1).sum::<f32>();
    while n > 2 && w - fixed(n) - pad < MIN_NAME_W {
        n -= 1;
    }
    let name_w = (w - fixed(n) - pad).max(140.0);
    let mut cols = [(Col::Icon, 0.0, 0.0); 8];
    let mut x = 8.0;
    for (i, &(c, cw)) in drop_first(n).iter().enumerate() {
        let cw = if c == Col::Name { name_w } else { cw };
        cols[i] = (c, x, x + cw);
        x += cw;
    }
    Columns { cols, n }
}

impl DesktopUi {
    /// Draw the group headers in the viewport and return its rows: (position, index within
    /// its group, content y, group key). The key goes into the row's widget id: a game shown
    /// in several groups is one widget per group (egui keeps only the last rect of an id).
    fn visible_rows(
        &mut self,
        ui: &mut Ui,
        st: &mut AppState,
        b: &Blocks,
        spans: &[Span],
        (vp, origin, w): (Rect, egui::Pos2, f32),
    ) -> Vec<(usize, usize, f32, Option<usize>)> {
        let mut rows = vec![];
        for (bi, rs) in b.visible(vp.min.y, vp.max.y) {
            let (span, start, _, top) = b.blocks[bi];
            if let Some(si) = span
                && top + b.header_h >= vp.min.y
            {
                let hr = Rect::from_min_size(origin + vec2(0.0, top), vec2(w, b.header_h));
                self.group_header(ui, st, hr, &spans[si]);
            }
            for r in rs {
                rows.push((start + r, r, top + b.header_h + r as f32 * b.row_h, span));
            }
        }
        rows
    }

    pub(super) fn list_view(&mut self, ui: &mut Ui, st: &mut AppState, shown: &Shown) {
        let (view, items, spans) = (shown.view, shown.items, shown.spans);
        self.note_input(ui);
        let pal = self.pal;
        let full = ui.available_rect_before_wrap();
        // The library's achievement counts (the column shows while there are any to show).
        let counts = st.achievements.counts().clone();
        let cols = columns(full.width(), !counts.is_empty());
        // Rows start right under the header (no item spacing between them).
        ui.spacing_mut().item_spacing.y = 0.0;
        self.list_header(ui, st, &cols, full.width());

        let running = st.running_game();
        let sel_pos = self.sel_pos;
        let row_look = super::style::RowLook::of(&st.desktop_theme.file.desktop, &pal);
        let scroll_to = std::mem::take(&mut self.scroll_to_sel);
        let stripe = self.fills.stripe;
        let b = Blocks::new(spans, items.len(), 1, ROW_H, GROUP_H, (0.0, 0.0));
        let content_h = b.total_h;
        let range = sel_pos.filter(|_| scroll_to).map(|p| row_range(&b, spans, items.len(), p, ROW_H, 0.0));
        let area = egui::ScrollArea::vertical().id_salt("ls_list").auto_shrink([false, false]);
        let out = self.list_keep.apply(area, ui.available_height(), range, content_h).show_viewport(ui, |ui, vp| {
            let w = ui.available_width();
            let (content, _) = ui.allocate_exact_size(vec2(w, content_h), Sense::hover());
            self.page_rows = ((vp.height() / ROW_H) as usize).max(1);
            let rows = self.visible_rows(ui, st, &b, spans, (vp, content.min, w));
            for (pos, local, y, span) in rows {
                let Some(g) = items.get(pos).map(|&i| &st.library.games[i]) else { continue };
                let rect = Rect::from_min_size(content.min + vec2(0.0, y), vec2(w, ROW_H));
                let resp = ui.interact(
                    rect,
                    row_id(ui, ("ls_row", g.id), span.map(|si| spans[si].key.as_str())),
                    Sense::click_and_drag(),
                );
                if resp.drag_started() {
                    resp.dnd_set_drag_payload(super::explorer::colls::drag_ids(self, &st.library, view, g.id));
                }
                let selected = sel_pos == Some(pos) || self.in_multi(g.id);
                let p = ui.painter();
                if selected {
                    row_look.paint_selected(p, rect, 0);
                    if self.multi.len() > 1 && sel_pos == Some(pos) {
                        // The focused game of a multi-selection (shown in the details).
                        p.rect_stroke(
                            rect.shrink(1.0),
                            0.0,
                            egui::Stroke::new(1.5, pal.selection),
                            egui::StrokeKind::Inside,
                        );
                    }
                } else if resp.hovered() {
                    p.rect_filled(rect, 0.0, pal.hover);
                } else if local % 2 == 1 {
                    p.rect_filled(rect, 0.0, stripe);
                }
                for (c, x0, x1) in cols.iter() {
                    let cell = Rect::from_x_y_ranges(rect.left() + x0..=rect.left() + x1, rect.y_range());
                    let ach = if c == Col::Achievements { counts.get(&g.id).copied() } else { None };
                    paint_cell(p, &mut st.images, g, c, cell, &pal, ach);
                }
                if running == Some(g.id) {
                    p.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, ROW_H)), 0.0, pal.accent);
                }
                self.row_clicks(ui, st, &resp, g, pos, view);
            }
        });
        if self.list_keep.update(out.state.clamped_offset().y, out.inner_rect.height()) {
            self.scroll_to_sel = true;
            ui.ctx().request_repaint();
        }
    }

    /// The column headers: a click sorts by that column (again: reverses), the sorted one
    /// shows the direction.
    fn list_header(&mut self, ui: &mut Ui, st: &mut AppState, cols: &Columns, full_w: f32) {
        let pal = self.pal;
        let (hrect, _) = ui.allocate_exact_size(vec2(full_w, 30.0), Sense::hover());
        ui.painter().rect_filled(hrect, 0.0, self.fills.header);
        ui.painter().hline(hrect.x_range(), hrect.bottom() - 0.5, egui::Stroke::new(1.0, pal.border));
        let (eff_sort, eff_rev, locked) = self.effective_sort(&st.config.desktop);
        let cfg = &mut st.config.desktop;
        let mut sort_changed = false;
        for (c, x0, x1) in cols.iter() {
            if c == Col::Icon {
                continue;
            }
            let r = Rect::from_x_y_ranges(hrect.left() + x0..=hrect.left() + x1, hrect.y_range());
            let resp = ui.interact(
                r,
                ui.scope_id().with(("ls_hdr", c as u8)),
                if c.sort().is_some() && !locked { Sense::click() } else { Sense::hover() },
            );
            let active = c.sort() == Some(eff_sort);
            let color = if active || resp.hovered() { pal.text } else { pal.dim };
            let g = ui.painter().layout_no_wrap(c.label().to_owned(), egui::FontId::proportional(12.5), color);
            let ty = r.center().y - g.size().y / 2.0;
            let gw = g.size().x;
            ui.painter().galley(pos2(r.left() + 6.0, ty), g, color);
            if active {
                let ir = Rect::from_min_size(pos2(r.left() + 10.0 + gw, r.center().y - 7.0), vec2(14.0, 14.0));
                paint_icon(ui.painter(), ir, if eff_rev { Icon::SortDesc } else { Icon::SortAsc }, pal.selection);
            }
            if locked && c.sort().is_some() {
                resp.on_hover_text("Recently played is always sorted by last played, newest first");
            } else if resp.clicked()
                && let Some(s) = c.sort()
            {
                if cfg.sort == s {
                    cfg.sort_descending = !cfg.sort_descending;
                } else {
                    cfg.sort = s;
                    cfg.sort_descending = false;
                }
                sort_changed = true;
            }
        }
        if sort_changed {
            self.mark_config_dirty();
        }
    }

    /// A row's clicks (select, Ctrl / Shift, double-click plays) and its right-click menu.
    fn row_clicks(&mut self, ui: &Ui, st: &AppState, resp: &egui::Response, g: &Game, pos: usize, view: &[usize]) {
        let pal = self.pal;
        let running = st.running_game();
        let in_multi = self.in_multi(g.id);
        if resp.clicked() {
            self.note_click(pos);
            self.push(Self::click_action(ui, g.id));
        } else if resp.secondary_clicked() && !in_multi {
            self.note_click(pos);
            self.push(Action::Select(g.id));
        }
        if resp.double_clicked() {
            self.push(Action::Play(g.id));
        }
        let caps = self.caps_for(g.id).cloned();
        let mut acts = vec![];
        let n_multi = if in_multi { self.multi.len() } else { 0 };
        let cur = self.coll_sel;
        self.game_menu_popup(resp, pos).show(|ui| {
            if n_multi > 1 {
                let ids = self.multi_ids_in(&st.library, view);
                multi_menu(ui, &st.library, &ids, cur, &pal, &mut acts)
            } else {
                let colls = (&st.library, cur);
                game_menu(ui, g, running == Some(g.id), caps.as_ref(), colls, &pal, &mut acts)
            }
        });
        self.actions.extend(acts);
    }

    /// Compact list (icon + name) for the Details view's left side.
    pub(super) fn compact_list(&mut self, ui: &mut Ui, st: &mut AppState, shown: &Shown) {
        let (view, items, spans) = (shown.view, shown.items, shown.spans);
        self.note_input(ui);
        let pal = self.pal;
        let running = st.running_game();
        let sel_pos = self.sel_pos;
        let row_look = super::style::RowLook::of(&st.desktop_theme.file.desktop, &pal);
        let scroll_to = std::mem::take(&mut self.scroll_to_sel);
        let b = Blocks::new(spans, items.len(), 1, COMPACT_ROW_H, COMPACT_GROUP_H, (6.0, 6.0));
        let content_h = b.total_h;
        let range = sel_pos.filter(|_| scroll_to).map(|p| row_range(&b, spans, items.len(), p, COMPACT_ROW_H, 6.0));
        let area = egui::ScrollArea::vertical().id_salt("ls_compact").auto_shrink([false, false]);
        let out = self.compact_keep.apply(area, ui.available_height(), range, content_h).show_viewport(ui, |ui, vp| {
            let w = ui.available_width();
            let (content, _) = ui.allocate_exact_size(vec2(w, content_h), Sense::hover());
            self.page_rows = ((vp.height() / COMPACT_ROW_H) as usize).max(1);
            let rows = self.visible_rows(ui, st, &b, spans, (vp, content.min, w));
            for (pos, _, y, span) in rows {
                let Some(g) = items.get(pos).map(|&i| &st.library.games[i]) else { continue };
                let rect = Rect::from_min_size(content.min + vec2(0.0, y), vec2(w, COMPACT_ROW_H));
                let resp = ui.interact(
                    rect,
                    row_id(ui, ("ls_crow", g.id), span.map(|si| spans[si].key.as_str())),
                    Sense::click_and_drag(),
                );
                if resp.drag_started() {
                    resp.dnd_set_drag_payload(super::explorer::colls::drag_ids(self, &st.library, view, g.id));
                }
                let selected = sel_pos == Some(pos) || self.in_multi(g.id);
                let p = ui.painter();
                let inner = rect.shrink2(vec2(6.0, 1.0));
                if selected {
                    row_look.paint_selected(p, inner, 4);
                    if self.multi.len() > 1 && sel_pos == Some(pos) {
                        // The focused game of a multi-selection (shown in the details).
                        p.rect_stroke(inner, 4.0, egui::Stroke::new(1.5, pal.selection), egui::StrokeKind::Inside);
                    }
                } else if resp.hovered() {
                    p.rect_filled(inner, 4.0, pal.hover);
                }
                let ir = Rect::from_min_size(pos2(inner.left() + 6.0, inner.center().y - 11.0), vec2(22.0, 22.0));
                paint_small_icon(p, &mut st.images, g, ir, &pal, !g.installed);
                let fg = row_look.text_for(selected, g.installed);
                // Consoles get their device icon at the right (PC games, the usual case,
                // stay unmarked).
                let console = g.emu.as_ref().and_then(|e| crate::emulation::platforms::by_id(&e.platform));
                let right_pad = if g.favorite { 22.0 } else { 6.0 } + if console.is_some() { 22.0 } else { 0.0 };
                if let Some(pl) = console {
                    let x = inner.right() - if g.favorite { 22.0 } else { 6.0 } - 10.0;
                    let ir = Rect::from_center_size(pos2(x, inner.center().y), vec2(16.0, 16.0));
                    crate::ui::platform_icons::paint(
                        p,
                        ir,
                        crate::ui::platform_icons::shape_for(pl.id),
                        mix(pal.dim, pal.bg, 0.1),
                    );
                }
                let gal = truncated(
                    p,
                    g.display_name(),
                    egui::FontId::proportional(13.5),
                    fg,
                    inner.right() - ir.right() - 10.0 - right_pad,
                );
                p.galley(pos2(ir.right() + 9.0, inner.center().y - gal.size().y / 2.0), gal, fg);
                if g.favorite {
                    star(p, pos2(inner.right() - 12.0, inner.center().y), 5.5, pal.fav);
                }
                if running == Some(g.id) {
                    p.rect_filled(
                        Rect::from_min_size(inner.min + vec2(0.0, 6.0), vec2(3.0, COMPACT_ROW_H - 14.0)),
                        1.5,
                        pal.accent,
                    );
                }
                let in_multi = self.in_multi(g.id);
                if resp.clicked() {
                    self.note_click(pos);
                    self.push(Self::click_action(ui, g.id));
                } else if resp.secondary_clicked() && !in_multi {
                    self.note_click(pos);
                    self.push(Action::Select(g.id));
                }
                if resp.double_clicked() {
                    self.push(Action::Play(g.id));
                }
                let caps = self.caps_for(g.id).cloned();
                let mut acts = vec![];
                let n_multi = if in_multi { self.multi.len() } else { 0 };
                let cur = self.coll_sel;
                self.game_menu_popup(&resp, pos).show(|ui| {
                    if n_multi > 1 {
                        let ids = self.multi_ids_in(&st.library, view);
                        multi_menu(ui, &st.library, &ids, cur, &pal, &mut acts)
                    } else {
                        let colls = (&st.library, cur);
                        game_menu(ui, g, running == Some(g.id), caps.as_ref(), colls, &pal, &mut acts)
                    }
                });
                self.actions.extend(acts);
            }
        });
        if self.compact_keep.update(out.state.clamped_offset().y, out.inner_rect.height()) {
            self.scroll_to_sel = true;
            ui.ctx().request_repaint();
        }
    }
}

/// Small square icon: the game's icon, else a cropped cover, else a tinted initial.
pub(crate) fn paint_small_icon(p: &egui::Painter, images: &mut ImageCache, g: &Game, rect: Rect, pal: &Pal, dim: bool) {
    let tint = if dim { Color32::from_gray(130) } else { Color32::WHITE };
    if let Some(icon) = &g.art.icon
        && let Some(tex) = images.get(icon, ImgSize::Icon)
    {
        paint_contain(p, rect, &tex, tint, vec2(0.5, 0.5));
        return;
    }
    if let Some(cover) = &g.art.cover
        && let Some(tex) = images.get(cover, ImgSize::Icon)
    {
        paint_cover(p, rect, &tex, 3, tint, vec2(0.5, 0.25));
        return;
    }
    let base = name_tint(g.display_name(), pal.surface_alt);
    p.rect_filled(rect, 4.0, if dim { mix(base, pal.bg, 0.4) } else { base });
    let initial: String =
        g.display_name().chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default();
    let gal = p.layout_no_wrap(initial, egui::FontId::proportional(rect.height() * 0.55), pal.text);
    p.galley(rect.center() - gal.size() / 2.0, gal, pal.text);
}

/// One cell of a game's row (dimmed when the game isn't installed).
fn paint_cell(
    p: &egui::Painter,
    images: &mut ImageCache,
    g: &Game,
    c: Col,
    cell: Rect,
    pal: &Pal,
    achievements: Option<(u32, u32)>,
) {
    let dimmed = !g.installed;
    let fg = if dimmed { pal.dim } else { pal.text };
    match c {
        Col::Icon => {
            let ir = Rect::from_center_size(cell.center(), vec2(24.0, 24.0));
            paint_small_icon(p, images, g, ir, pal, dimmed);
        }
        Col::Name => {
            let mut x = cell.left() + 6.0;
            let maxw = cell.width() - 12.0 - if g.favorite { 18.0 } else { 0.0 };
            let gal = truncated(p, g.display_name(), egui::FontId::proportional(14.0), fg, maxw);
            let gw = gal.size().x;
            p.galley(pos2(x, cell.center().y - gal.size().y / 2.0), gal, fg);
            x += gw + 10.0;
            if g.favorite {
                star(p, pos2(x, cell.center().y), 6.0, pal.fav);
            }
        }
        Col::Platform => {
            if let Some(pl) = crate::ui::platform_icons::platform_of(g) {
                let color = if dimmed { mix(pal.dim, pal.bg, 0.2) } else { pal.dim };
                let ir = Rect::from_center_size(pos2(cell.left() + 14.0, cell.center().y), vec2(16.0, 16.0));
                crate::ui::platform_icons::paint(p, ir, crate::ui::platform_icons::shape_for(pl.id), color);
                let gal = truncated(p, pl.short, egui::FontId::proportional(13.0), color, cell.width() - 34.0);
                p.galley(pos2(cell.left() + 28.0, cell.center().y - gal.size().y / 2.0), gal, color);
            }
        }
        Col::Achievements => {
            let Some((n, total)) = achievements else { return };
            let color = if dimmed { mix(pal.dim, pal.bg, 0.2) } else { pal.text };
            let text = format!("{} / {total}", n.min(total));
            let gal = truncated(p, &text, egui::FontId::proportional(12.5), color, cell.width() - 12.0);
            let top = cell.center().y - (gal.size().y + 6.0) / 2.0;
            p.galley(pos2(cell.left() + 6.0, top), gal.clone(), color);
            let track = Rect::from_min_size(
                pos2(cell.left() + 6.0, top + gal.size().y + 3.0),
                vec2((cell.width() - 30.0).max(20.0), 3.0),
            );
            p.rect_filled(track, 1.5, mix(pal.border, pal.bg, 0.2));
            let f = crate::achievements::summary::fraction((n, total));
            if f > 0.0 {
                let fill = Rect::from_min_size(track.min, vec2((track.width() * f).max(3.0), 3.0));
                p.rect_filled(fill, 1.5, if dimmed { mix(pal.accent, pal.bg, 0.4) } else { pal.accent });
            }
        }
        Col::Installed => {
            if g.installed {
                let ir = Rect::from_center_size(pos2(cell.left() + 16.0, cell.center().y), vec2(14.0, 14.0));
                paint_icon(p, ir, Icon::Check, mix(pal.accent, pal.text, 0.3));
            } else {
                let gal = truncated(p, "—", egui::FontId::proportional(13.0), pal.dim, cell.width());
                p.galley(pos2(cell.left() + 12.0, cell.center().y - gal.size().y / 2.0), gal, pal.dim);
            }
        }
        _ => {
            let text = match c {
                Col::Source => g.source_name().to_string(),
                Col::Playtime => format_playtime(g.playtime_secs),
                Col::LastPlayed => format_last_played(g.last_played),
                _ => String::new(),
            };
            let color = if dimmed { mix(pal.dim, pal.bg, 0.2) } else { pal.dim };
            let gal = truncated(p, &text, egui::FontId::proportional(13.0), color, cell.width() - 12.0);
            p.galley(pos2(cell.left() + 6.0, cell.center().y - gal.size().y / 2.0), gal, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Col, MIN_NAME_W, columns};

    #[test]
    fn columns_are_contiguous_and_name_flexes() {
        let c = columns(1100.0, false);
        assert_eq!(c.n, 7, "all columns fit at 1100 px");
        let v: Vec<_> = c.iter().collect();
        for w in v.windows(2) {
            assert_eq!(w[0].2, w[1].1);
        }
        let name = v.iter().find(|c| c.0 == Col::Name).unwrap();
        assert!(name.2 - name.1 > 400.0);
    }

    #[test]
    fn narrow_tables_drop_columns_instead_of_overflowing() {
        for w in [700.0, 560.0, 420.0, 330.0] {
            let c = columns(w, false);
            let last = c.iter().last().unwrap();
            assert!(last.2 <= w, "width {w}: columns end at {}", last.2);
            let name = c.iter().find(|c| c.0 == Col::Name).unwrap();
            assert!(name.2 - name.1 >= MIN_NAME_W - 0.5, "width {w}");
        }
        // Drop order: Installed goes first, Source last.
        let c = columns(600.0, false);
        assert!(c.iter().all(|c| c.0 != Col::Installed));
        assert!(c.iter().any(|c| c.0 == Col::Source));
        // Never fewer than icon + name, even when absurdly narrow.
        let tiny = columns(100.0, false);
        assert_eq!(tiny.n, 2);
        assert!(tiny.iter().any(|c| c.0 == Col::Name));
    }

    #[test]
    fn the_achievements_column_is_added_next_to_installed_and_dropped_after_it() {
        let names = |c: &super::Columns| c.iter().map(|c| c.0).collect::<Vec<_>>();
        let wide = columns(1400.0, true);
        assert_eq!(
            names(&wide),
            [
                Col::Icon,
                Col::Name,
                Col::Source,
                Col::Platform,
                Col::Playtime,
                Col::LastPlayed,
                Col::Achievements,
                Col::Installed
            ]
        );
        // A little short: Installed goes, Achievements stays.
        let fixed: f32 = wide.iter().filter(|c| c.0 != Col::Name).map(|c| c.2 - c.1).sum();
        let w = fixed + 16.0 + super::MIN_NAME_W - 10.0;
        let c = columns(w, true);
        assert!(c.iter().any(|c| c.0 == Col::Achievements) && c.iter().all(|c| c.0 != Col::Installed), "{w}");
        // Switched off: the table is as before.
        assert_eq!(names(&columns(1400.0, false)).len(), 7);
        assert!(columns(1400.0, false).iter().all(|c| c.0 != Col::Achievements));
    }
}
