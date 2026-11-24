//! Playnite Fullscreen-style elements: the navigation `rail` and the home page's cover `rows`.
//!
//! Both get their data from `Scene::extra` (the controller UI decides which rows and sections
//! exist); the elements only describe the look, so any theme (also a converted one) can use
//! them in any view.

use super::*;
use crate::library::CollectionKind;
use crate::theme::model::RailEntry;
use std::sync::OnceLock;

/// One cover row of a `rows` element.
#[derive(Clone, Copy)]
pub struct RowScene<'a> {
    pub collection: &'a CollectionItem,
    /// Indices into `SceneExtra::games`.
    pub indices: &'a [usize],
    /// Selected item of the row (kept per row, so coming back to a row returns to it).
    pub selected: usize,
}

/// What a `rail` element shows this frame.
#[derive(Clone, Copy, Default)]
pub struct RailScene<'a> {
    /// Indices into the element's entries, in order (sections without games are left out).
    /// Empty = every entry.
    pub shown: &'a [usize],
    /// The open section (index into `shown`).
    pub current: Option<usize>,
    /// The highlighted section while the rail has the focus (index into `shown`).
    pub selected: usize,
    /// The rail has the focus (D-pad moves through its sections).
    pub focused: bool,
}

/// Data for the rail / rows elements (all empty for views that don't use them).
#[derive(Clone, Copy, Default)]
pub struct SceneExtra<'a> {
    /// The library's games (`RowScene::indices` point into it).
    pub games: &'a [Game],
    pub rows: &'a [RowScene<'a>],
    /// The focused row.
    pub row: usize,
    pub rail: Option<RailScene<'a>>,
    /// Name of the open rail section, for `{section.name}`.
    pub section: Option<&'a str>,
}

/// A rail's entries: its own, or the default set when it lists none.
pub fn rail_entries(entries: &[RailEntry]) -> &[RailEntry] {
    static DEFAULTS: OnceLock<Vec<RailEntry>> = OnceLock::new();
    if entries.is_empty() { DEFAULTS.get_or_init(RailEntry::defaults) } else { entries }
}

/// Which collections a `rows` element shows (indices into `colls`), following its `rows`
/// list: named groups ("recent", "platforms" …; "platforms:2" = at most two of them) or
/// collection keys. Empty collections and duplicates are skipped; at most `max` rows.
pub fn pick_rows(specs: &[String], colls: &[CollectionItem], max: usize) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    let kinds = |pred: &dyn Fn(&CollectionKind) -> bool, by_count: bool| {
        let mut v: Vec<usize> = (0..colls.len()).filter(|&i| pred(&colls[i].collection.kind)).collect();
        if by_count {
            // Stable: equal counts keep the library's (alphabetical) order.
            v.sort_by_key(|&i| std::cmp::Reverse(colls[i].count));
        }
        v
    };
    for spec in specs {
        if out.len() >= max {
            break;
        }
        let raw = spec.trim();
        let mut s = raw.to_ascii_lowercase();
        let mut limit = usize::MAX;
        if let Some((group, n)) = s.split_once(':')
            && matches!(group, "platforms" | "sources" | "collections")
            && let Ok(n) = n.trim().parse::<usize>()
        {
            limit = n;
            s = group.to_string();
        }
        let picked: Vec<usize> = match s.as_str() {
            "recent" | "recently_played" | "lastplayed" => kinds(&|k| matches!(k, CollectionKind::Recent), false),
            "favorites" | "favourites" => kinds(&|k| matches!(k, CollectionKind::Favorites), false),
            "installed" => kinds(&|k| matches!(k, CollectionKind::Installed), false),
            "controller" => kinds(&|k| matches!(k, CollectionKind::Controller), false),
            "all" => kinds(&|k| matches!(k, CollectionKind::All), false),
            "platforms" => kinds(&|k| matches!(k, CollectionKind::Platform(_)), true),
            "sources" => kinds(&|k| matches!(k, CollectionKind::Source(_)), true),
            "collections" => kinds(&|k| matches!(k, CollectionKind::User(_)), false),
            _ => colls.iter().position(|c| c.collection.key().eq_ignore_ascii_case(raw)).into_iter().collect(),
        };
        let mut taken = 0;
        for i in picked {
            if out.len() >= max || taken >= limit {
                break;
            }
            if colls[i].count > 0 && !out.contains(&i) {
                out.push(i);
                taken += 1;
            }
        }
    }
    out
}

/// A row's title, one line `width` wide at `at`.
#[allow(clippy::too_many_arguments)] // the title's text, look and place
fn row_title(
    f: &Frame,
    text: &str,
    style: &TextStyle,
    family: &egui::FontFamily,
    px: f32,
    alpha: f32,
    at: Pos2,
    width: f32,
) {
    let color = fade(style.color.to_egui(), alpha);
    let gl = layout(&f.painter, text, family.clone(), px, color, style, Some(width), Some(1));
    let shadow = style.shadow.map(|c| fade(c.to_egui(), alpha));
    paint_galley(&f.painter, at, &gl, color, shadow, 0.0);
}

impl Renderer {
    pub(super) fn draw_rows(&mut self, f: &mut Frame, idx: usize, el: &Element, focused: bool) {
        let Element::Rows {
            common,
            item_size,
            spacing,
            row_spacing,
            title,
            title_style,
            item_image,
            item_fallbacks,
            item_text,
            item_text_style,
            selected_scale,
            selected_border,
            unselected_opacity,
            ..
        } = el
        else {
            return;
        };
        let r = Self::elem_rect(f, common, Vec2::ZERO);
        f.out.focus_rect = Some(r);
        let ex = f.scene.extra;
        let rows = ex.rows;
        if rows.is_empty() || r.width() <= 0.0 || r.height() <= 0.0 {
            return;
        }
        let (sw, sh) = (f.sw(), f.sh());
        let kind = f.scene.kind;
        let s_scale = super::lists::focus_scale(common, *selected_scale);
        let ft = super::lists::focus_transition(common);
        let grow = (s_scale - 1.0).max(0.0);
        let iw = (item_size[0] * sw).max(4.0);
        let ih = (item_size[1] * sh).max(4.0);
        let g = spacing * sw;
        let title_px = (title_style.font_size * sh * f.ts).max(6.0);
        let has_title = !title.trim().is_empty();
        let title_h = if has_title { title_px * 1.75 } else { 0.0 };
        // Room for the scaled-up selection (and its glow) inside the box.
        let pad_x = iw * grow / 2.0 + (sh * 0.006).max(3.0);
        let pad_y = ih * grow / 2.0 + (sh * 0.004).max(2.0);
        let pitch = title_h + ih + pad_y * 2.0 + row_spacing * sh;
        let vis = (r.height() / pitch).max(1.0);
        let vis_full = vis.floor().max(1.0);
        let n = rows.len();
        let cur = ex.row.min(n - 1);
        let list_id = f.scene.list_id;
        // The focused row moves to the top of the box; the last rows don't leave it half empty.
        let pos = self.rows_scroll(f, idx, (cur as f32).min((n as f32 - vis_full).max(0.0)));
        let glow_room = (sh * 0.03).clamp(8.0, 40.0);
        let clip = Rect::from_min_max(pos2(r.left() - glow_room, r.top()), pos2(f.bounds.right(), r.bottom()));
        let clipped = f.painter.with_clip_rect(clip.intersect(f.bounds));
        let painter = std::mem::replace(&mut f.painter, clipped);
        let size = size_class(iw.max(ih) * s_scale * f.ppp);
        let spec = ItemSpec {
            tint: None,
            image: item_image,
            fallbacks: item_fallbacks,
            fit: common.item_fit.unwrap_or(Fit::Cover),
            radius: common.radius.unwrap_or(0.0) * sh,
            fill: common.item_fill.map(|c| c.to_egui()),
            text: item_text,
            subtext: common.item_subtext.as_deref(),
            caption: common.item_caption.as_deref(),
            style: item_text_style,
            border: selected_border.map(|c| c.to_egui()),
            base_h: ih,
            size,
            badge: common.controller_badge,
            fav_badge: common.favorite_badge,
            labelled: false,
            look: FocusLook::of(common, sh),
            templates: &common.item_elements,
            key: idx,
            adjust: picture_adjust(common),
        };
        let per_row = ((r.width() - pad_x + g) / (iw + g)).max(1.0);
        let per_full = per_row.floor().max(1.0);
        let first_row = (pos.floor() as usize).min(n - 1);
        let last_row = ((pos + vis).ceil() as usize + 1).min(n);
        let title_fam = f.fonts.family(f.theme, title_style.font.as_deref());
        let mut sel_item: Option<(Rect, Binder, usize, usize)> = None;
        // `transitions.focus`: the focus grows onto the new item and off the old one (also
        // across rows, and off / back on when the rail takes / gives back the focus).
        let row_id = |ri: usize, i: usize| ((ri as u64) << 32) | i as u64;
        let amounts = ft.map(|(t, _)| {
            let sel_now = rows.get(cur).map(|row| row_id(cur, row.selected.min(row.indices.len().saturating_sub(1))));
            let (now, instant) = (f.now, self.opts.reduce_motion);
            let (a, moving) = self.state(kind, idx).focus.step(list_id, sel_now.filter(|_| focused), now, t, instant);
            f.out.animating |= moving;
            a
        });
        let prev_id = amounts.and_then(|a| a.prev).map(|(p, _)| p);
        let mut prev_item: Option<(Rect, Binder, usize, usize, f32)> = None;
        for (ri, row) in rows.iter().enumerate().take(last_row).skip(first_row) {
            let y0 = r.top() + (ri as f32 - pos) * pitch;
            let row_focus = ri == cur;
            let rb = f.base.with_collection(Some(row.collection));
            let row_alpha = if row_focus { 1.0 } else { unselected_opacity.clamp(0.0, 1.0) };
            if has_title {
                let t = rb.resolve(title, false);
                let a = common.opacity * if row_focus { 1.0 } else { 0.75 };
                let width = r.width() - pad_x * 2.0;
                row_title(f, t.trim(), title_style, &title_fam, title_px, a, pos2(r.left() + pad_x, y0), width);
            }
            let m = row.indices.len();
            if m == 0 {
                continue;
            }
            let sel = row.selected.min(m - 1);
            let hx = self.row_scroll(f, idx, row, sel, m, per_full);
            let iy = y0 + title_h + pad_y;
            let lo = (hx.floor() as usize).saturating_sub(1);
            let hi = ((hx + per_row).ceil() as usize + 1).min(m);
            for (i, &gi) in row.indices.iter().enumerate().take(hi).skip(lo) {
                let x = r.left() + pad_x + (i as f32 - hx) * (iw + g);
                let rect = Rect::from_min_size(pos2(x, iy), vec2(iw, ih));
                let b = rb.with_game(ex.games.get(gi));
                if row_focus && i == sel {
                    sel_item = Some((rect, b, ri, i));
                    continue;
                }
                if prev_id == Some(row_id(ri, i)) {
                    prev_item = Some((rect, b, ri, i, row_alpha * common.opacity));
                    continue;
                }
                self.draw_item(f, rect, &b, &spec, row_alpha * common.opacity, 0.0, 0.0);
                if f.scene.interactive && clip.intersects(rect) {
                    f.row_hits.push(((ri, i), rect));
                }
            }
            // Prefetch the covers just past the right edge.
            for &gi in row.indices.iter().take(hi + 3).skip(hi) {
                let b = rb.with_game(ex.games.get(gi));
                if let Some(p) = self.pick_image(f.theme, f.images, &b, item_image, item_fallbacks, size) {
                    f.images.prefetch(&p, size);
                }
            }
        }
        if let (Some((_, tr)), Some(am)) = (ft, amounts) {
            let full = tr.opacity.unwrap_or(1.0).clamp(0.0, 1.0) * common.opacity;
            let shift = tr.translate.map_or(Vec2::ZERO, |t| vec2(t[0] * sw, t[1] * sh));
            let sel_item = sel_item.map(|(r, b, ri, i)| (r, b, ri, i, common.opacity));
            for (rect, b, ri, i, rest) in [prev_item, sel_item].into_iter().flatten() {
                let amount = am.of(row_id(ri, i));
                let (rect, a) = focus_item(rect, rest, full, s_scale, shift, amount);
                self.draw_item(f, rect, &b, &spec, a, amount, 0.0);
                if f.scene.interactive {
                    f.row_hits.push(((ri, i), rect));
                }
            }
        } else if let Some((rect, b, ri, i)) = sel_item {
            // While the rail has the focus the selection stays visible, unscaled and softer.
            let (rect, fo) =
                if focused { (Rect::from_center_size(rect.center(), rect.size() * s_scale), 1.0) } else { (rect, 0.0) };
            self.draw_item(f, rect, &b, &spec, common.opacity, fo, 0.0);
            if f.scene.interactive {
                f.row_hits.push(((ri, i), rect));
            }
        }
        f.painter = painter;
    }

    /// The rows' scroll position (in rows) this frame, springing towards `target`.
    fn rows_scroll(&mut self, f: &mut Frame, idx: usize, target: f32) -> f32 {
        let (dt, list_id) = (f.dt, f.scene.list_id);
        let st = self.state(f.scene.kind, idx);
        if !st.init || st.list_id != list_id {
            st.init = true;
            st.list_id = list_id;
            st.pos = target;
        }
        if approach(&mut st.pos, target, dt, 12.0) {
            f.out.animating = true;
        }
        st.pos
    }

    /// One row's scroll position (in items) this frame: it scrolls at the edges, keeping one
    /// item of look-ahead on either side of the selection (`sel` of `m`, `per_full` whole
    /// items in view).
    fn row_scroll(&mut self, f: &mut Frame, idx: usize, row: &RowScene, sel: usize, m: usize, per_full: f32) -> f32 {
        let dt = f.dt;
        let st = self.row_states.entry((f.scene.kind, idx, hash_of(&row.collection.collection))).or_default();
        let mut first = st.top;
        let lead = if per_full >= 3.0 { 1.0 } else { 0.0 };
        if (sel as f32) < first + lead {
            first = (sel as f32 - lead).max(0.0);
        }
        if sel as f32 > first + per_full - 1.0 - lead {
            first = sel as f32 + 1.0 + lead - per_full;
        }
        first = first.clamp(0.0, (m as f32 - per_full).max(0.0));
        st.top = first;
        if !st.init {
            st.init = true;
            st.pos = first;
        }
        if approach(&mut st.pos, first, dt, 14.0) {
            f.out.animating = true;
        }
        st.pos
    }

    pub(super) fn draw_rail(&mut self, f: &mut Frame, idx: usize, el: &Element) {
        let Element::Rail { common, entries, style, row_height, current_color, selector_color, background, compact } =
            el
        else {
            return;
        };
        let entries = rail_entries(entries);
        let rs = f.scene.extra.rail.unwrap_or_default();
        let all: Vec<usize>;
        let shown: &[usize] = if rs.shown.is_empty() {
            all = (0..entries.len()).collect();
            &all
        } else {
            rs.shown
        };
        let sh = f.sh();
        let r = Self::elem_rect(f, common, Vec2::ZERO);
        let px = (style.font_size * sh * f.ts).max(6.0);
        let row_h = px * row_height.max(1.0);
        let icon = px * 1.3;
        let inset = px * 0.5;
        let narrow = (inset * 2.0 + px * 1.1 * 2.0 + icon).min(r.width());
        let open = if *compact {
            let dt = f.dt;
            let target = if rs.focused { 1.0 } else { 0.0 };
            let st = self.state(f.scene.kind, idx);
            if !st.init {
                st.init = true;
                st.pos = target;
            }
            if approach(&mut st.pos, target, dt, 16.0) {
                f.out.animating = true;
            }
            st.pos
        } else {
            1.0
        };
        let w = narrow + (r.width() - narrow).max(0.0) * open;
        let panel = Rect::from_min_size(r.min, vec2(w, r.height()));
        let a = common.opacity;
        if let Some(bg) = background {
            f.painter.rect_filled(panel, 0.0, fade(bg.to_egui(), a));
        }
        let pal = &f.theme.file.palette;
        let text_c = style.color.to_egui();
        let cur_c = current_color.unwrap_or(pal.accent).to_egui();
        let sel_bg = selector_color.unwrap_or(pal.accent).to_egui();
        let sel_text = pal.accent_text.to_egui();
        let fam = f.fonts.family(f.theme, style.font.as_deref());
        let painter = f.painter.with_clip_rect(panel.intersect(f.bounds));
        let label = |e: &RailEntry| if style.uppercase { e.label().to_uppercase() } else { e.label().to_string() };
        // The labels fit the open rail (a bigger text size, a narrow screen): the longest one
        // sets a smaller label size rather than being cut off.
        let label_room = r.width() - (inset + px * 1.1 + icon + px * 0.8) - inset * 1.5;
        let longest = shown
            .iter()
            .filter_map(|&ei| entries.get(ei))
            .map(|e| painter.layout_no_wrap(label(e), FontId::new(px, fam.clone()), text_c).size().x)
            .fold(0.0f32, f32::max);
        let label_px =
            if longest > label_room && label_room > 0.0 { (px * label_room / longest).max(px * 0.6) } else { px };
        for (j, &ei) in shown.iter().enumerate() {
            let Some(e) = entries.get(ei) else { continue };
            let row = Rect::from_min_size(pos2(panel.left(), panel.top() + j as f32 * row_h), vec2(w, row_h));
            if row.top() > panel.bottom() {
                break;
            }
            let is_sel = rs.focused && j == rs.selected;
            let is_cur = rs.current == Some(j);
            let pill = row.shrink2(vec2(inset, row_h * 0.1));
            let rad = cr(pill.height() * 0.5);
            if is_sel {
                painter.rect_filled(pill, rad, fade(sel_bg, a));
            } else if is_cur {
                painter.rect_filled(pill, rad, fade(text_c, 0.09 * a));
                // A short accent bar on the panel's edge marks the open section.
                let bar = Rect::from_center_size(
                    pos2(panel.left() + inset * 0.35, row.center().y),
                    vec2((px * 0.16).max(2.0), pill.height() * 0.55),
                );
                painter.rect_filled(bar, cr(bar.width()), fade(cur_c, a));
            }
            let c = if is_sel {
                sel_text
            } else if is_cur {
                cur_c
            } else {
                fade(text_c, if rs.focused { 0.92 } else { 0.72 })
            };
            let c = fade(c, a);
            let ir =
                Rect::from_center_size(pos2(pill.left() + px * 1.1 + icon / 2.0, row.center().y), vec2(icon, icon));
            self.rail_icon(f, &painter, ir, e.icon(), c);
            if open > 0.02 {
                let gl = painter.layout_no_wrap(label(e), FontId::new(label_px, fam.clone()), fade(c, open));
                let p = pos2(ir.right() + px * 0.8, row.center().y - gl.size().y / 2.0);
                let shadow = style.shadow.map(|s| fade(s.to_egui(), a * open));
                paint_galley(&painter, p, &gl, c, shadow, 0.0);
            }
            if f.scene.interactive {
                f.rail_hits.push((j, row));
            }
        }
    }

    /// A rail section's icon: a drawn one by name, else a picture (theme-relative path).
    fn rail_icon(&mut self, f: &mut Frame, p: &Painter, rect: Rect, name: &str, color: Color32) {
        use crate::ui::platform_icons::{Shape, paint};
        let side = rect.width().min(rect.height());
        let at = |x: f32, y: f32| rect.min + vec2(x * side, y * side);
        let stroke = Stroke::new((side * 0.09).max(1.2), color);
        match name {
            "console" => paint(p, rect, Shape::Console, color),
            "monitor" => paint(p, rect, Shape::Monitor, color),
            "folder" => paint(p, rect, Shape::Collection, color),
            "gamepad" => paint(p, rect, Shape::Gamepad, color),
            "home" => {
                p.add(egui::Shape::line(vec![at(0.1, 0.5), at(0.5, 0.14), at(0.9, 0.5)], stroke));
                p.add(egui::Shape::line(vec![at(0.24, 0.42), at(0.24, 0.86), at(0.76, 0.86), at(0.76, 0.42)], stroke));
                p.rect_filled(Rect::from_min_max(at(0.42, 0.6), at(0.58, 0.86)), 0.0, color);
            }
            "grid" => {
                for (x, y) in [(0.12, 0.12), (0.56, 0.12), (0.12, 0.56), (0.56, 0.56)] {
                    let sq = Rect::from_min_size(at(x, y), vec2(side * 0.32, side * 0.32));
                    p.rect_stroke(sq, cr(side * 0.06), stroke, StrokeKind::Inside);
                }
            }
            "filter" => {
                p.add(egui::Shape::closed_line(
                    vec![
                        at(0.12, 0.18),
                        at(0.88, 0.18),
                        at(0.58, 0.54),
                        at(0.58, 0.84),
                        at(0.42, 0.74),
                        at(0.42, 0.54),
                    ],
                    stroke,
                ));
            }
            "search" => {
                p.circle_stroke(at(0.43, 0.43), side * 0.27, stroke);
                p.line_segment([at(0.63, 0.63), at(0.88, 0.88)], stroke);
            }
            "menu" => {
                for y in [0.25, 0.5, 0.75] {
                    p.line_segment([at(0.14, y), at(0.86, y)], stroke);
                }
            }
            "star" => star(p, rect.center(), side * 0.46, color),
            "clock" => {
                p.circle_stroke(rect.center(), side * 0.4, stroke);
                p.line_segment([rect.center(), at(0.5, 0.24)], stroke);
                p.line_segment([rect.center(), at(0.7, 0.58)], stroke);
            }
            path => {
                let size = ImgSize::Thumb;
                if let Some(pb) = Self::to_path(f.theme, path)
                    && self.exists(&pb)
                    && let Some(tex) = f.images.get(&pb, size)
                {
                    paint_image(p, rect, &tex, Fit::Contain, color, 0.0, 0.0);
                }
            }
        }
    }
}

fn hash_of(v: impl Hash) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Collection;
    use crate::model::Source;

    fn item(kind: CollectionKind, name: &str, count: usize) -> CollectionItem {
        CollectionItem {
            collection: Collection { kind, name: name.into() },
            name: name.into(),
            count,
            art: Default::default(),
        }
    }

    #[test]
    fn rows_follow_the_theme_list() {
        let colls = vec![
            item(CollectionKind::Recent, "Continue Playing", 4),
            item(CollectionKind::Favorites, "Favourites", 0),
            item(CollectionKind::All, "All Games", 40),
            item(CollectionKind::Source(Source::Steam), "Steam", 30),
            item(CollectionKind::Platform("nintendo_gamecube".into()), "GameCube", 3),
            item(CollectionKind::Platform("sony_playstation2".into()), "PlayStation 2", 7),
        ];
        let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // Empty rows are skipped; consoles come most games first.
        assert_eq!(pick_rows(&s(&["recent", "favorites", "platforms"]), &colls, 6), vec![0, 5, 4]);
        assert_eq!(pick_rows(&s(&["recent", "platforms"]), &colls, 2), vec![0, 5]);
        assert_eq!(pick_rows(&s(&["platforms:1", "sources:0", "all"]), &colls, 6), vec![5, 2]);
        // Collection keys; no duplicates.
        assert_eq!(pick_rows(&s(&["Platform:nintendo_gamecube", "steam", "pc", "nope"]), &colls, 6), vec![4, 3]);
        assert_eq!(pick_rows(&s(&["all", "all"]), &colls, 6), vec![2]);
        assert!(pick_rows(&s(&["recent"]), &colls, 0).is_empty());
    }

    #[test]
    fn rail_defaults_and_labels() {
        let d = rail_entries(&[]);
        let keys: Vec<&str> = d.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["home", "all", "platforms", "sources", "collections", "filters"]);
        assert_eq!(d[1].label(), "All games");
        assert_eq!(d[5].icon(), "filter");
        let own = [RailEntry { key: "menu".into(), label: " Settings ".into(), icon: Some("star".into()) }];
        assert_eq!(rail_entries(&own).len(), 1);
        assert_eq!((own[0].label(), own[0].icon()), ("Settings", "star"));
    }
}
