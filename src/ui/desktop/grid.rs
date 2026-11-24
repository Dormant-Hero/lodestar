//! Grid view: virtualised cover wall. Only visible rows are laid out.

use super::grouping::{Blocks, Shown, run_of};
use super::{Action, DesktopUi, Pal, game_menu, multi_menu};

/// Height of a group header above its covers.
const GROUP_H: f32 = 36.0;
use crate::app::AppState;
use crate::images::{ImageCache, ImgSize};
use crate::model::{ControllerSupport, Game};
use crate::ui::widgets::{
    self, gamepad_badge, mix, name_tint, paint_contain, paint_cover, pill, soft_shadow, star, with_alpha, wrapped,
};
use egui::{Align, Color32, Painter, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

/// Geometry of the cover grid for a given width (pure, unit-tested).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GridLayout {
    pub cols: usize,
    pub rows: usize,
    pub gap: f32,
    pub item_w: f32,
    pub cover_h: f32,
    pub name_h: f32,
    pub row_h: f32,
    pub total_h: f32,
}

impl GridLayout {
    #[cfg(test)]
    pub fn new(avail_w: f32, cover_w: f32, n: usize, show_names: bool) -> GridLayout {
        Self::styled(avail_w, cover_w, n, show_names, None)
    }

    /// `names`: room for a name under each cover; `gap`: the theme's gap (None = from the
    /// cover width).
    pub fn styled(avail_w: f32, cover_w: f32, n: usize, show_names: bool, gap: Option<f32>) -> GridLayout {
        let cover_w = cover_w.clamp(60.0, 600.0);
        let gap = gap.unwrap_or_else(|| (cover_w * 0.1).clamp(10.0, 22.0));
        let avail_w = avail_w.max(cover_w + 2.0 * gap);
        let cols = (((avail_w - gap) / (cover_w + gap)).floor() as usize).max(1);
        // Stretch items so the grid fills the width with even gaps.
        let item_w = ((avail_w - gap * (cols as f32 + 1.0)) / cols as f32).max(40.0);
        let cover_h = (item_w * 1.5).round();
        let name_h = if show_names { 24.0 } else { 0.0 };
        let row_h = cover_h + name_h + gap;
        let rows = n.div_ceil(cols);
        GridLayout { cols, rows, gap, item_w, cover_h, name_h, row_h, total_h: rows as f32 * row_h + gap }
    }

    /// Rect of item `i` relative to the content origin (without groups; see `Blocks`).
    #[cfg(test)]
    pub fn item_rect(&self, i: usize) -> Rect {
        let (r, c) = (i / self.cols, i % self.cols);
        Rect::from_min_size(
            pos2(self.gap + c as f32 * (self.item_w + self.gap), self.gap + r as f32 * self.row_h),
            vec2(self.item_w, self.cover_h + self.name_h),
        )
    }

    /// Rows intersecting the vertical range `[y0, y1]` (content coordinates), clamped.
    #[cfg(test)]
    pub fn visible_rows(&self, y0: f32, y1: f32) -> std::ops::Range<usize> {
        let first = ((y0 - self.gap) / self.row_h).floor().max(0.0) as usize;
        let last = ((y1 / self.row_h).ceil().max(0.0) as usize + 1).min(self.rows);
        first.min(last)..last
    }
}

impl DesktopUi {
    pub(super) fn grid_view(&mut self, ui: &mut Ui, st: &mut AppState, shown: &Shown) {
        let (view, items, spans) = (shown.view, shown.items, shown.spans);
        self.note_input(ui);
        // Ctrl + wheel resizes covers.
        let hovered = ui.rect_contains_pointer(ui.max_rect());
        if hovered {
            let zoom = ui.input(|i| i.zoom_delta());
            if zoom != 1.0 {
                let w = (st.config.desktop.cover_width * zoom).clamp(100.0, 320.0);
                if (w - st.config.desktop.cover_width).abs() > 0.01 {
                    st.config.desktop.cover_width = w;
                    self.scroll_to_sel = true;
                    self.mark_config_dirty();
                }
            }
        }
        let pal = self.pal;
        let running = st.running_game();
        let look = super::style::CoverLook::of(&st.desktop_theme.file.desktop, &pal);
        // Store / console banners of an imported Playnite theme (its themeExtras.yaml).
        let theme_dir = st.desktop_theme.dir.clone();
        let banners = theme_dir.as_deref().and_then(crate::theme::playnite::extras::Banners::cached);
        let banners = banners.as_deref().zip(theme_dir.as_deref());
        // The user's "Show game names" switch, then the theme's place for them.
        let title = if self.show_names { look.title } else { crate::theme::CoverTitle::None };
        let show_names = title == crate::theme::CoverTitle::Under;
        let cover_w = st.config.desktop.cover_width;
        let badge = st.config.desktop.controller_badge;
        // The library's achievement counts (empty when switched off): one lookup per cover.
        let counts = st.achievements.counts().clone();
        let sel_pos = self.sel_pos;
        let scroll_to = std::mem::take(&mut self.scroll_to_sel);
        // Rows of covers under group headers (one block without groups).
        let blocks_for =
            |lay: &GridLayout| Blocks::new(spans, items.len(), lay.cols, lay.row_h, GROUP_H, (lay.gap, 0.0));

        // Bring the selection into view by setting the scroll offset directly, computed from the
        // same layout the grid uses. (egui's `scroll_to_rect` animates through a stored target
        // that, repeated over the first frames while the panels settle, overshot to the end.)
        let avail = ui.available_size();
        let content_w = self.grid_view_size.map_or(avail.x, |s| s.x);
        let vp_h = self.grid_view_size.map_or(avail.y, |s| s.y);
        let mut wanted = None;
        let mut short = false;
        let mut area = egui::ScrollArea::vertical().id_salt("ls_grid").auto_shrink([false, false]);
        if scroll_to && let Some(p) = sel_pos {
            let lay = GridLayout::styled(content_w, cover_w, items.len(), show_names, look.gap);
            let b = blocks_for(&lay);
            let (_, y) = b.item_pos(p);
            // The first row of a group: show its header too.
            let first_row = p - run_of(spans, items.len(), p).0 < lay.cols;
            let top = y - lay.gap * 0.5 - if first_row { b.header_h } else { 0.0 };
            let bottom = y + lay.cover_h + lay.name_h + lay.gap * 0.5;
            if let Some(off) = super::offset_to_show(self.grid_offset, vp_h, top, bottom, b.total_h) {
                // Never past last frame's end: the scroll area would take that as overscroll and
                // spring back with a velocity that keeps drifting the grid. Taller content (just
                // regrouped) gets the rest next frame (`wanted`).
                let limit = (self.grid_content_h - vp_h).max(0.0);
                short = off > limit + 0.5;
                let off = off.min(limit);
                area = area.vertical_scroll_offset(off);
                wanted = Some(off);
            }
        }

        let out = area.show_viewport(ui, |ui, vp| {
            let w = ui.available_width();
            let lay = GridLayout::styled(w, cover_w, items.len(), show_names, look.gap);
            let b = blocks_for(&lay);
            // The column count / width changes while the side panels settle after startup (and
            // when they are resized or toggled): the selected item moves to another row, so
            // bring it back into view next frame, with the settled layout.
            if lay.cols != self.grid_cols || self.grid_view_size.is_some_and(|s| (s.x - w).abs() > 0.5) {
                self.scroll_to_sel = true;
                ui.ctx().request_repaint();
            }
            self.grid_cols = lay.cols;
            self.page_rows = ((vp.height() / lay.row_h).floor() as usize).max(1);
            let (content, _) = ui.allocate_exact_size(vec2(w, b.total_h), Sense::hover());
            let origin = content.min.to_vec2();
            for (bi, rows) in b.visible(vp.min.y, vp.max.y) {
                let (span, start, len, top) = b.blocks[bi];
                if let Some(si) = span
                    && top + b.header_h >= vp.min.y
                {
                    let hr =
                        Rect::from_min_size(pos2(lay.gap * 0.5, top), vec2(w - lay.gap, b.header_h)).translate(origin);
                    self.group_header(ui, st, hr, &spans[si]);
                }
                for r in rows {
                    for c in 0..lay.cols {
                        let local = r * lay.cols + c;
                        if local >= len {
                            break;
                        }
                        let pos = start + local;
                        let Some(&gi) = items.get(pos) else { break };
                        let g = &st.library.games[gi];
                        let y = top + b.header_h + r as f32 * lay.row_h;
                        let rect = Rect::from_min_size(
                            pos2(lay.gap + c as f32 * (lay.item_w + lay.gap), y),
                            vec2(lay.item_w, lay.cover_h + lay.name_h),
                        )
                        .translate(origin);
                        self.grid_item(
                            ui,
                            &mut st.images,
                            (&st.library, view, pos, span.map(|si| spans[si].key.as_str())),
                            g,
                            rect,
                            &lay,
                            sel_pos == Some(pos) || self.in_multi(g.id),
                            running == Some(g.id),
                            badge && g.controller() == Some(ControllerSupport::Full),
                            counts.get(&g.id).copied(),
                            (&pal, &look, title, banners),
                        );
                    }
                }
            }
            (b, w)
        });
        let (b, w) = out.inner;
        self.grid_content_h = b.total_h;
        let new_size = vec2(w, out.inner_rect.height());
        if self.grid_view_size.is_some_and(|s| (s.y - new_size.y).abs() > 0.5) {
            self.scroll_to_sel = true;
            ui.ctx().request_repaint();
        }
        self.grid_view_size = Some(new_size);
        self.grid_offset = out.state.clamped_offset().y;
        // The scroll area clamps to last frame's content height: when the content just grew
        // (a library update regrouped the games) the offset falls short, so ask again.
        // (A few frames at most: never a repaint loop when an offset can't be reached.)
        let short = short || wanted.is_some_and(|w| (w - self.grid_offset).abs() > 1.0);
        if short && self.grid_retries < 8 {
            self.grid_retries += 1;
            self.scroll_to_sel = true;
            ui.ctx().request_repaint();
        } else if wanted.is_some() && !short {
            self.grid_retries = 0;
        }
        // Prefetch the next screenful of covers below the viewport (cheap: only schedules decodes).
        let vp_bottom = out.state.clamped_offset().y + out.inner_rect.height();
        for (bi, rows) in b.visible(vp_bottom, vp_bottom + out.inner_rect.height()) {
            let (_, start, len, _) = b.blocks[bi];
            let from = (start + rows.start * b.cols).min(start + len);
            let to = (start + rows.end * b.cols).min(start + len);
            for &gi in &items[from.min(items.len())..to.min(items.len())] {
                if let Some(c) = &st.library.games[gi].art.cover {
                    st.images.prefetch(c, ImgSize::Thumb);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn grid_item(
        &mut self,
        ui: &mut Ui,
        images: &mut ImageCache,
        (lib, view, pos, group): (&crate::library::Library, &[usize], usize, Option<&str>),
        g: &Game,
        rect: Rect,
        lay: &GridLayout,
        selected: bool,
        running: bool,
        controller_badge: bool,
        achievements: Option<(u32, u32)>,
        (pal, look, title, banners): (
            &Pal,
            &super::style::CoverLook,
            crate::theme::CoverTitle,
            Option<(&crate::theme::playnite::extras::Banners, &std::path::Path)>,
        ),
    ) {
        // A game shown in several groups (genres, tags …) is one widget per group: with one id
        // egui would only keep the last rect, and the other copy couldn't be clicked.
        let id = ui.scope_id().with(("ls_cover", g.id));
        let id = group.map_or(id, |k| id.with(k));
        let resp = ui.interact(rect, id, Sense::click_and_drag());
        if resp.drag_started() {
            // Drag onto a collection in the explorer.
            resp.dnd_set_drag_payload(super::explorer::colls::drag_ids(self, lib, view, g.id));
        }
        let hover = ui.ctx().animate_bool_with_time(id, resp.hovered() || resp.has_focus(), 0.10);
        let lift = (hover * 3.0).round();
        let cover = Rect::from_min_size(rect.min - vec2(0.0, lift), vec2(lay.item_w, lay.cover_h));
        let p = ui.painter();
        let radius = look.radius;

        soft_shadow(
            p,
            cover,
            radius,
            (8.0 + hover * 8.0) as u8,
            (3.0 + hover * 3.0) as i8,
            (70.0 + hover * 40.0) as u8,
        );
        paint_game_cover(p, images, g, cover, radius, pal, ImgSize::Thumb, !g.installed);
        // A banner along the bottom lifts the badges and pills above it.
        let (mut banner_top, mut banner_h) = (0.0, 0.0);
        if let Some((b, dir)) = banners
            && let Some(rel) = b.image(g.source.key(), g.platform.as_deref())
        {
            (banner_top, banner_h) = banner(p, images, cover, radius, &dir.join(rel), b.placement);
        }
        if hover > 0.0 {
            let shade = match look.hover_darken {
                Some(d) => Color32::from_black_alpha((hover * d * 255.0) as u8),
                None => Color32::from_white_alpha((hover * 16.0) as u8),
            };
            p.rect_filled(cover, radius, shade);
        }
        // The name over the cover (theme `cover_title` overlay, or hover while pointed at).
        let over = match title {
            crate::theme::CoverTitle::Overlay => 1.0,
            crate::theme::CoverTitle::Hover => hover,
            _ => 0.0,
        };
        // Not over a stand-in card: it shows the name already.
        let over = if g.art.cover.is_some() { over } else { 0.0 };
        let strip_h = if over > 0.0 {
            // Above a banner along the bottom, not over it.
            let (area, r) = if banner_h > 0.0 {
                (Rect::from_min_max(cover.min, pos2(cover.right(), cover.bottom() - banner_h)), 0)
            } else {
                (cover, radius)
            };
            name_overlay(p, area, r, g.display_name(), pal, over)
        } else {
            0.0
        };
        if selected && look.sel_width > 0.0 {
            p.rect_stroke(
                cover.expand(1.0),
                radius.saturating_add(1),
                Stroke::new(look.sel_width, look.sel_color),
                StrokeKind::Outside,
            );
            // In a multi-selection the focused game (the one the details show) gets an inner ring.
            if self.multi.len() > 1 && self.selected == Some(g.id) {
                p.rect_stroke(cover, radius, Stroke::new(2.0, pal.text), StrokeKind::Inside);
            }
        }
        let s = (lay.item_w / 160.0).clamp(0.75, 1.3);
        if g.favorite {
            let c = pos2(cover.right() - 15.0 * s, cover.top() + 15.0 * s + banner_top);
            p.circle_filled(c, 11.0 * s, Color32::from_black_alpha(150));
            star(p, c, 7.5 * s, pal.fav);
        }
        if controller_badge {
            // Same row as the "Not installed" pill (bottom-left), mirrored on the right.
            let corner = pos2(cover.right() - 8.0, cover.bottom() - 8.0 - 3.0 * s - strip_h * over - banner_h);
            gamepad_badge(p, corner, 17.0 * s);
        }
        if running {
            pill(p, cover.left_top() + vec2(8.0, 8.0 + banner_top), "Running", 11.0 * s, pal.accent_text, pal.accent);
        }
        let mut low = cover.bottom() - 8.0 - 3.0 * s - strip_h * over - banner_h;
        if !g.installed {
            let r = pill(
                p,
                pos2(cover.left() + 8.0, cover.bottom() - 8.0 - 20.0 * s - strip_h * over - banner_h),
                "Not installed",
                10.5 * s,
                Color32::from_gray(225),
                Color32::from_black_alpha(175),
            );
            low = r.top() - 4.0;
        }
        if let (Some(count), Some(ps)) = (achievements, count_pill_scale(lay.item_w)) {
            // Bottom-left, above "Not installed"; never as far as the gamepad badge on the right.
            let room = cover.width() - 16.0 - if controller_badge { 26.0 * s } else { 0.0 };
            count_pill(p, pos2(cover.left() + 8.0, low), count, ps, room, pal.accent);
        }
        if lay.name_h > 0.0 {
            let color = if selected { pal.text } else { mix(pal.text, pal.dim, 0.35) };
            let gal = wrapped(
                p,
                g.display_name(),
                egui::FontId::proportional(13.0),
                color,
                lay.item_w - 4.0,
                1,
                Align::Center,
            );
            let y = cover.bottom() + lift + 5.0;
            // centre-aligned galleys are laid out around x = 0
            p.galley(pos2(rect.center().x, y), gal, color);
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
        resp.context_menu(|ui| {
            if n_multi > 1 {
                let ids = self.multi_ids_in(lib, view);
                multi_menu(ui, lib, &ids, cur, pal, &mut acts)
            } else {
                game_menu(ui, g, running, caps.as_ref(), (lib, cur), pal, &mut acts)
            }
        });
        self.actions.extend(acts);
        if lay.name_h == 0.0
            && title != crate::theme::CoverTitle::Overlay
            && title != crate::theme::CoverTitle::Hover
            && resp.hovered()
        {
            resp.on_hover_text_at_pointer(g.display_name());
        }
    }
}

/// Covers narrower than this show no achievement count (it would cover the picture).
pub(crate) const COUNT_PILL_MIN_W: f32 = 110.0;

/// The achievement pill's scale for a cover this wide (None: too small to show one).
pub(crate) fn count_pill_scale(item_w: f32) -> Option<f32> {
    (item_w >= COUNT_PILL_MIN_W).then(|| (item_w / 160.0).clamp(0.8, 1.3))
}

/// "31 / 54" with a thin bar under it, on a dark pill that reads on light and dark covers,
/// standing on `bottom_left`; at most `room` wide (else not drawn).
fn count_pill(p: &Painter, bottom_left: egui::Pos2, (n, total): (u32, u32), s: f32, room: f32, accent: Color32) {
    let text = format!("{} / {total}", n.min(total));
    let fg = Color32::from_gray(238);
    let gal = p.layout_no_wrap(text, egui::FontId::proportional(11.0 * s), fg);
    let pad = vec2(6.0 * s, 3.0 * s);
    let bar_h = (2.5 * s).round().max(2.0);
    let w = (gal.size().x + pad.x * 2.0).max(42.0 * s);
    if w > room {
        return;
    }
    let h = gal.size().y + pad.y * 2.0 + bar_h + 2.0 * s;
    let r = Rect::from_min_size(pos2(bottom_left.x, bottom_left.y - h), vec2(w, h));
    p.rect_filled(r, 4.0 * s, Color32::from_black_alpha(185));
    p.galley(r.min + pad, gal, fg);
    let track = Rect::from_min_size(pos2(r.left() + pad.x, r.bottom() - pad.y - bar_h), vec2(w - pad.x * 2.0, bar_h));
    p.rect_filled(track, bar_h / 2.0, Color32::from_white_alpha(46));
    let f = crate::achievements::summary::fraction((n, total));
    if f > 0.0 {
        let fill = Rect::from_min_size(track.min, vec2((track.width() * f).max(bar_h), bar_h));
        p.rect_filled(fill, bar_h / 2.0, accent);
    }
}

/// A theme's banner strip across the top (or bottom) of a cover, at the picture's own shape
/// (at most a quarter of the cover's height). Returns its height (along the top, along the
/// bottom) so badges move clear of it.
fn banner(
    p: &Painter,
    images: &mut ImageCache,
    cover: Rect,
    radius: u8,
    path: &std::path::Path,
    place: crate::theme::playnite::extras::Placement,
) -> (f32, f32) {
    use crate::theme::playnite::extras::Placement;
    let Some(tex) = images.get(path, ImgSize::Thumb) else { return (0.0, 0.0) };
    let [tw, th] = tex.size();
    if tw == 0 || th == 0 {
        return (0.0, 0.0);
    }
    let h = (cover.width() * th as f32 / tw as f32).min(cover.height() * 0.25);
    let (rect, corners) = match place {
        Placement::Bottom => (
            Rect::from_min_max(pos2(cover.left(), cover.bottom() - h), cover.max),
            egui::CornerRadius { nw: 0, ne: 0, sw: radius, se: radius },
        ),
        Placement::Above | Placement::Top => (
            Rect::from_min_max(cover.min, pos2(cover.right(), cover.top() + h)),
            egui::CornerRadius { nw: radius, ne: radius, sw: 0, se: 0 },
        ),
    };
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    p.add(egui::epaint::RectShape::filled(rect, corners, Color32::WHITE).with_texture(tex.id(), uv));
    if place == Placement::Bottom { (0.0, h) } else { (h, 0.0) }
}

/// A game's name over the bottom of its cover on a dark fade (`CoverTitle::Overlay` /
/// `Hover`), at `alpha`. Returns the strip's height.
fn name_overlay(p: &Painter, cover: Rect, radius: u8, name: &str, pal: &Pal, alpha: f32) -> f32 {
    let px = (cover.width() / 12.0).clamp(11.0, 15.0);
    // Light text on the dark strip, also with a light theme's dark text colour.
    let [r, g, b, _] = pal.text.to_array();
    let text = if r as u32 + g as u32 + b as u32 >= 3 * 140 { pal.text } else { Color32::from_gray(240) };
    let color = text.gamma_multiply(alpha);
    let gal = wrapped(p, name, egui::FontId::proportional(px), color, cover.width() - 12.0, 2, Align::Center);
    let h = gal.size().y + 14.0;
    let strip = Rect::from_min_max(pos2(cover.left(), cover.bottom() - h * 1.6), cover.max);
    crate::theme::render::draw::gradient_rect(
        p,
        strip,
        radius as f32,
        [Color32::TRANSPARENT, Color32::from_black_alpha((200.0 * alpha) as u8)],
        false,
    );
    // centre-aligned galleys are laid out around x = 0
    p.galley(pos2(cover.center().x, cover.bottom() - 7.0 - gal.size().y), gal, color);
    h
}

/// Paint a game's portrait cover into `rect`. Without a cover: a palette-tinted card with the
/// banner/icon letterboxed (if any) and the name. Never blocks; images appear when decoded.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_game_cover(
    p: &Painter,
    images: &mut ImageCache,
    g: &Game,
    rect: Rect,
    radius: u8,
    pal: &Pal,
    size: ImgSize,
    dim: bool,
) {
    let tint = if dim { Color32::from_gray(120) } else { Color32::WHITE };
    if let Some(path) = &g.art.cover {
        if let Some(tex) = images.get(path, size) {
            paint_cover(p, rect, &tex, radius, tint, vec2(0.5, 0.5));
            return;
        }
        if !images.is_failed(path, size) {
            // Still decoding: a quiet card (avoids flashing the name placeholder).
            p.rect_filled(rect, radius, mix(pal.surface_alt, pal.bg, 0.3));
            return;
        }
    }
    paint_placeholder_cover(p, images, g, rect, radius, pal, size, dim);
}

/// Cover for a game without one (Faugus games, shortcuts…): a name-tinted card with the game's
/// icon (or banner, or a monogram) as an emblem, the name under it and at the bottom the
/// console (emulated games: its icon and "PLAYSTATION 2") or else the source.
#[allow(clippy::too_many_arguments)]
fn paint_placeholder_cover(
    p: &Painter,
    images: &mut ImageCache,
    g: &Game,
    rect: Rect,
    radius: u8,
    pal: &Pal,
    size: ImgSize,
    dim: bool,
) {
    let name = g.display_name();
    let base = name_tint(name, pal.surface_alt);
    let (top, bottom) = (mix(base, Color32::WHITE, 0.06), mix(base, pal.bg, 0.55));
    let (top, bottom) = if dim { (mix(top, pal.bg, 0.45), mix(bottom, pal.bg, 0.45)) } else { (top, bottom) };
    widgets::vgradient_rounded(p, rect, radius, top, bottom);
    p.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, Color32::from_white_alpha(if dim { 8 } else { 16 })),
        StrokeKind::Inside,
    );
    let tint = if dim { Color32::from_gray(120) } else { Color32::WHITE };
    let w = rect.width();
    let inner_w = w * 0.84;
    let emblem_c = pos2(rect.center().x, rect.top() + rect.height() * 0.34);
    let emblem = w * 0.44;

    // Emblem: icon > banner > monogram. While an image decodes, leave the slot empty (no flash).
    let icon = g.art.icon.as_deref();
    let banner = g.art.banner.as_deref();
    let mut emblem_rect = Rect::from_center_size(emblem_c, vec2(emblem, emblem));
    let mut loading = false;
    let mut drawn = false;
    for (path, is_banner) in [(icon, false), (banner, true)] {
        let Some(path) = path else { continue };
        match images.get(path, size) {
            Some(tex) => {
                let bounds = if is_banner {
                    Rect::from_center_size(emblem_c, vec2(inner_w, inner_w * 0.47))
                } else {
                    emblem_rect
                };
                let r = widgets::contain_rect(tex.size_vec2(), bounds);
                soft_shadow(p, r.shrink(r.width() * 0.08), 8, 14, 3, if dim { 40 } else { 90 });
                emblem_rect = paint_contain(p, bounds, &tex, tint, vec2(0.5, 0.5));
                drawn = true;
                break;
            }
            None if !images.is_failed(path, size) => {
                loading = true;
                break;
            }
            None => {}
        }
    }
    if !drawn && !loading {
        let letters = widgets::initials(name);
        let fill = Color32::from_white_alpha(if dim { 10 } else { 22 });
        p.rect_filled(emblem_rect, (emblem * 0.22).min(255.0) as u8, fill);
        let font = egui::FontId::proportional(emblem * if letters.chars().count() > 1 { 0.42 } else { 0.52 });
        let color = with_alpha(if dim { pal.dim } else { pal.text }, 215);
        let gal = p.layout_no_wrap(letters, font, color);
        p.galley(emblem_rect.center() - gal.size() / 2.0, gal, color);
    }

    // Name under the emblem.
    let text = if dim { pal.dim } else { pal.text };
    let font = egui::FontId::proportional((w / 10.0).clamp(11.0, 21.0));
    let caption_h = (w / 17.0).clamp(8.0, 12.0);
    let name_top = emblem_rect.bottom() + rect.height() * 0.06;
    let console = g.emu.as_ref().and_then(|e| crate::emulation::platforms::by_id(&e.platform));
    // Emulated games keep room for the console icon above the caption.
    let icon_room = if console.is_some() && caption_h >= 8.5 { caption_h * 2.9 } else { 0.0 };
    let name_bottom = rect.bottom() - rect.height() * 0.06 - caption_h * 1.6 - icon_room;
    let rows = (((name_bottom - name_top) / (font.size * 1.25)).floor() as usize).clamp(1, 3);
    let gal = wrapped(p, name, font, text, inner_w, rows, Align::Center);
    p.galley(pos2(rect.center().x, name_top), gal, text);

    // Caption: the console of an emulated game (with its icon), else the source.
    if caption_h >= 8.5 {
        let color = with_alpha(text, 110);
        let caption = match console {
            Some(pl) => crate::ui::platform_icons::console_name(pl.id),
            None => g.source_name().to_string(),
        };
        let mut job = egui::text::LayoutJob::default();
        job.append(
            &caption.to_uppercase(),
            0.0,
            egui::TextFormat {
                font_id: egui::FontId::proportional(caption_h),
                color,
                extra_letter_spacing: caption_h * 0.12,
                ..Default::default()
            },
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(inner_w);
        let gal = p.layout_job(job);
        let y = rect.bottom() - rect.height() * 0.05 - gal.size().y;
        p.galley(pos2(rect.center().x - gal.size().x / 2.0, y), gal, color);
        if let Some(pl) = console {
            let side = (caption_h * 2.2).round();
            let ir = Rect::from_center_size(pos2(rect.center().x, y - side * 0.5 - caption_h * 0.45), vec2(side, side));
            crate::ui::platform_icons::paint(p, ir, crate::ui::platform_icons::shape_for(pl.id), with_alpha(text, 150));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::GridLayout;

    #[test]
    fn the_achievement_pill_hides_on_small_covers_and_grows_with_big_ones() {
        use super::{COUNT_PILL_MIN_W, count_pill_scale};
        assert_eq!(count_pill_scale(COUNT_PILL_MIN_W - 1.0), None);
        assert_eq!(count_pill_scale(60.0), None);
        let small = count_pill_scale(COUNT_PILL_MIN_W).unwrap();
        let big = count_pill_scale(300.0).unwrap();
        assert!(small < big && small >= 0.8 && big <= 1.3);
        // The usual cover size shows it; the smallest the zoom allows doesn't.
        let l = GridLayout::new(1000.0, 170.0, 100, true);
        assert!(count_pill_scale(l.item_w).is_some(), "{}", l.item_w);
        let l = GridLayout::new(1000.0, 100.0, 100, true);
        assert!(count_pill_scale(l.item_w).is_none(), "{}", l.item_w);
    }

    #[test]
    fn fills_width_with_even_gaps() {
        let l = GridLayout::new(1000.0, 170.0, 100, true);
        assert!(l.cols >= 4);
        let used = l.cols as f32 * l.item_w + (l.cols as f32 + 1.0) * l.gap;
        assert!((used - 1000.0).abs() < 0.5, "used {used}");
        assert!(l.item_w >= 170.0);
        assert_eq!(l.rows, 100usize.div_ceil(l.cols));
    }

    #[test]
    fn narrow_and_empty() {
        let l = GridLayout::new(50.0, 170.0, 0, false);
        assert_eq!(l.cols, 1);
        assert_eq!(l.rows, 0);
        assert!(l.visible_rows(0.0, 500.0).is_empty());
    }

    #[test]
    fn visible_rows_are_bounded() {
        let l = GridLayout::new(1200.0, 170.0, 3000, true);
        // A theme's gap replaces the automatic one.
        let g = GridLayout::styled(1200.0, 170.0, 10, true, Some(30.0));
        assert_eq!(g.gap, 30.0);
        assert!((g.cols as f32 * g.item_w + (g.cols as f32 + 1.0) * 30.0 - 1200.0).abs() < 0.5);
        let r = l.visible_rows(10_000.0, 10_900.0);
        assert!(r.len() <= (900.0 / l.row_h).ceil() as usize + 2);
        assert!(r.end <= l.rows);
        // every item in those rows intersects (or touches) the range
        let first = l.item_rect(r.start * l.cols);
        assert!(first.bottom() >= 10_000.0 - l.row_h);
        let r = l.visible_rows(1e9, 1e9 + 100.0);
        assert!(r.is_empty() || r.end == l.rows);
    }
}
