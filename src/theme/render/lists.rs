//! Drawing the list elements: carousel, grid and text list, and their layout helpers.

use super::*;

/// Slots (positions along a carousel, item `slot mod n`) to draw around the animated position
/// `pos`, `reach` slots each way. Without `wrap` they stay inside 0..n; with it they extend
/// past both ends, but never so far that an item would be shown twice.
pub(super) fn carousel_slots(pos: f32, reach: f32, n: usize, wrap: bool) -> (i64, i64) {
    if !wrap {
        return ((pos - reach).floor().max(0.0) as i64, ((pos + reach).ceil() as i64).min(n as i64 - 1));
    }
    let (mut lo, mut hi) = ((pos - reach).floor() as i64, (pos + reach).ceil() as i64);
    while hi - lo + 1 > n as i64 {
        if (hi as f32 - pos) >= (pos - lo as f32) {
            hi -= 1;
        } else {
            lo += 1;
        }
    }
    (lo, hi)
}

/// Rows a text list shows: whole rows only (at least one).
pub(super) fn whole_rows(height: f32, row_h: f32) -> f32 {
    (height / row_h + 1e-3).floor().max(1.0)
}

/// Largest font size (≤ `base_px`) at which a word `longest` px wide at `base_px` fits `width`.
pub(super) fn word_fit_px(base_px: f32, longest: f32, width: f32) -> f32 {
    if longest > width && longest > 0.0 { base_px * width / longest * 0.97 } else { base_px }
}

/// Font size for a label that doesn't fit its card even at the readable minimum `min_px`: the
/// largest size (≤ `base_px`) at which its longest word (`longest` px wide at `base_px`) stays
/// whole, going below `min_px` down to `floor_px` for one long word ("Brawlhalla" in a narrow
/// card) rather than breaking it. True = the word is too wide even at `floor_px`: cut it with
/// "…" (`elide_words`), never in the middle onto a second row.
pub(super) fn whole_word_px(base_px: f32, min_px: f32, floor_px: f32, longest: f32, width: f32) -> (f32, bool) {
    let word_px = word_fit_px(base_px, longest, width);
    if word_px >= min_px {
        return (word_px.min(base_px), false);
    }
    let floor = floor_px.min(min_px);
    (word_px.max(floor), word_px < floor)
}

/// `text` with every word wider than `max_w` (as `measure` says) cut short with "…".
pub(super) fn elide_words(text: &str, max_w: f32, measure: impl Fn(&str) -> f32) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    for w in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        if measure(w) <= max_w {
            out.push_str(w);
            continue;
        }
        let chars: Vec<char> = w.chars().collect();
        let mut n = chars.len().saturating_sub(1);
        let cut = loop {
            let cand: String = chars[..n].iter().chain(std::iter::once(&'…')).collect();
            if n <= 1 || measure(&cand) <= max_w {
                break cand;
            }
            n -= 1;
        };
        out.push_str(&cut);
    }
    out
}

/// The textlist selector bar for a row at `y`: the whole row, or `width` (fraction of screen
/// width, clamped to the list) starting `offset` (fraction of screen width) from its left edge.
pub(super) fn selector_rect(list: Rect, y: f32, row_h: f32, width: Option<f32>, offset: f32, sw: f32) -> Rect {
    let x = (list.left() + offset * sw).clamp(list.left(), (list.right() - 1.0).max(list.left()));
    let w = match width {
        Some(w) if w > 0.0 => (w * sw).max(1.0),
        _ => list.width(),
    };
    Rect::from_min_size(pos2(x, y), vec2(w.min(list.right() - x).max(1.0), row_h))
}

/// The focus transition of a list element (`transitions.focus`), if it has one.
pub(super) fn focus_transition(common: &Common) -> Option<(&Transitions, &Transition)> {
    let t = common.transitions.as_deref()?;
    Some((t, t.focus.as_ref()?))
}

/// The focused item's scale: the focus transition's, else the element's `selected_scale`.
pub(super) fn focus_scale(common: &Common, selected_scale: f32) -> f32 {
    focus_transition(common).and_then(|(_, t)| t.scale).unwrap_or(selected_scale).max(0.1)
}

/// The name under a grid item drawn with the focused item's style.
#[allow(clippy::too_many_arguments)]
fn focused_label(
    f: &mut Frame,
    rect: Rect,
    b: &Binder,
    item_text: &str,
    style: &TextStyle,
    label_px: f32,
    fam: &egui::FontFamily,
    alpha: f32,
) {
    let t = b.resolve(item_text, false);
    let lr = Rect::from_min_size(pos2(rect.left(), rect.bottom() + label_px * 0.3), vec2(rect.width(), label_px * 1.3));
    let gl = layout_line(
        &f.painter,
        t.trim(),
        FontId::new(label_px, fam.clone()),
        fade(style.color.to_egui(), alpha),
        rect.width(),
    );
    let shadow = style.shadow.map(|c| fade(c.to_egui(), alpha));
    let lp = line_pos(lr, &gl, style.align, VAlign::Top);
    paint_galley(&f.painter, lp, &gl, Color32::WHITE, shadow, 0.0);
}

/// Where a carousel's items go: along the strip, around a wheel, or around an arc wheel's
/// pivot.
struct CarouselStrip {
    r: Rect,
    vertical: bool,
    /// Lodestar's own wheel (`Orientation::Wheel`).
    wheel: bool,
    /// Arc wheels: the selection's centre and the point the items turn around.
    arc_geo: Option<(Pos2, Pos2)>,
    /// Degrees between neighbouring items on a wheel.
    wheel_angle: f32,
    radius_px: f32,
    /// The strip's start and length along it, px.
    start: f32,
    len: f32,
    spacing: f32,
    /// How far the neighbours are pushed away from a scaled-up selection, px.
    extra: f32,
    selected_position: f32,
    item_align: f32,
    selected_margins: Option<[f32; 2]>,
    /// Screen size, px.
    screen: Vec2,
}

impl CarouselStrip {
    /// Centre and rotation (degrees) of an item `d` slots from the scroll position, `size`
    /// big.
    fn place(&self, d: f32, size: Vec2) -> (Pos2, f32) {
        let (r, vertical) = (self.r, self.vertical);
        if let Some((c0, pivot)) = self.arc_geo {
            let th = d * self.wheel_angle;
            return (rotate_around(c0, pivot, th), th);
        }
        if self.wheel {
            let th = d * self.wheel_angle.to_radians();
            let c = r.center();
            let centre = if vertical {
                pos2(c.x - self.radius_px, c.y) + vec2(th.cos(), th.sin()) * self.radius_px
            } else {
                pos2(c.x, c.y + self.radius_px) + vec2(th.sin(), -th.cos()) * self.radius_px
            };
            return (centre, d * self.wheel_angle);
        }
        let margin = self.selected_margins.map_or(0.0, |[before, after]| {
            let axis = if vertical { self.screen.y } else { self.screen.x };
            if d < 0.0 { -before * axis } else { after * axis }
        });
        let along = self.start
            + self.selected_position * self.len
            + d * self.spacing
            + (d.signum() * self.extra + margin) * d.abs().min(1.0);
        let across_len = if vertical { r.width() } else { r.height() };
        let across_item = if vertical { size.x } else { size.y };
        let across = (if vertical { r.left() } else { r.top() })
            + across_item / 2.0
            + self.item_align.clamp(0.0, 1.0) * (across_len - across_item);
        (if vertical { pos2(across, along) } else { pos2(along, across) }, 0.0)
    }
}

impl Renderer {
    pub(super) fn draw_carousel(&mut self, f: &mut Frame, idx: usize, el: &Element, _focused: bool) {
        let Element::Carousel {
            common,
            orientation,
            item_size,
            visible_items,
            item_image,
            item_fallbacks,
            item_text,
            item_text_style,
            selected_scale,
            unselected_opacity,
            background,
            item_align,
            selected_position,
            wheel_angle,
            selected_border,
            wheel_origin,
            wheel_align,
            cross_offset,
            items_around,
            item_tint,
            selected_margins,
        } = el
        else {
            return;
        };
        let r = Self::elem_rect(f, common, Vec2::ZERO);
        if let Some(bg) = background {
            f.painter.rect_filled(r, cr(common.radius.unwrap_or(0.0) * f.sh()), fade(bg.to_egui(), common.opacity));
        }
        f.out.focus_rect = Some(r);
        let items = f.scene.items;
        let n = items.len();
        if n == 0 {
            return;
        }
        let sel = f.scene.selected.min(n - 1);
        let (sw, sh) = (f.sw(), f.sh());
        let vertical = match orientation {
            Orientation::Vertical | Orientation::VerticalWheel => true,
            Orientation::Horizontal | Orientation::HorizontalWheel => false,
            Orientation::Wheel => r.height() > r.width(),
        };
        let arc_wheel = orientation.is_arc_wheel();
        let mut iw = item_size[0] * sw;
        let mut ih = item_size[1] * sh;
        if iw <= 0.0 {
            iw = ih * 1.5;
        }
        if ih <= 0.0 {
            ih = iw / 1.5;
        }
        let vis = match (arc_wheel, items_around) {
            (true, Some([b, a])) => (b + a + 1) as f32,
            (true, None) => 17.0,
            _ => visible_items.max(1.0),
        };
        let len = if vertical { r.height() } else { r.width() };
        let spacing = len / vis;
        let s_scale = focus_scale(common, *selected_scale);
        let ft = focus_transition(common);
        let item_main = if vertical { ih } else { iw };
        let gap = sh * 0.012;
        // Push neighbours away so a scaled-up selection never overlaps them — unless the items
        // overlap by design (themes with full-screen items a few slots apart).
        // The theme's own room around the selection (`selected_margins`) replaces this push.
        let extra = if selected_margins.is_some() {
            0.0
        } else if item_main <= spacing {
            (item_main * (s_scale + 1.0) / 2.0 + gap - spacing).max(0.0)
        } else {
            0.0
        };
        // The collections carousel loops (as its navigation does: Left on the first collection
        // goes to the last).
        let wrap = f.scene.kind == ViewKind::Collections && n > 1;
        let pos = self.carousel_position(f, idx, sel, n, vis, wrap, ft.map(|(_, t)| t));
        let reach = match (arc_wheel, items_around) {
            (true, Some([b, a])) => (*b).max(*a) as f32 + 1.0,
            (true, None) => 9.0,
            _ => vis * selected_position.max(1.0 - selected_position) + 2.0,
        };
        let (lo, hi) = carousel_slots(pos, reach, n, wrap);
        let item_at = |slot: i64| slot.rem_euclid(n as i64) as usize;
        let mut order: Vec<i64> = (lo..=hi).collect();
        order.sort_by(|a, b| {
            let da = (*a as f32 - pos).abs();
            let db = (*b as f32 - pos).abs();
            db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal)
        });
        let fit = common.item_fit.unwrap_or(Fit::Contain);
        let radius = common.radius.unwrap_or(0.0) * sh;
        let size = size_class(iw.max(ih) * s_scale * f.ppp);
        self.item_sizes.insert((f.scene.kind, idx), size);
        let spec = ItemSpec {
            tint: item_tint.map(|c| c.to_egui()),
            image: item_image,
            fallbacks: item_fallbacks,
            fit,
            radius,
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
        let wheel = *orientation == Orientation::Wheel;
        // Arc wheels: the selection's (unscaled) box, and the point the items turn around.
        let arc_geo = arc_wheel.then(|| {
            let a = wheel_align.unwrap_or(0.5).clamp(0.0, 1.0);
            let along = selected_position - 0.5;
            let (x, y) = if vertical {
                (
                    (iw * s_scale - iw) / 2.0 + a * (r.width() - iw * s_scale) + cross_offset * r.width(),
                    (r.height() - ih) / 2.0 + along * r.height(),
                )
            } else {
                (
                    (r.width() - iw) / 2.0 + along * r.width(),
                    (ih * s_scale - ih) / 2.0 + a * (r.height() - ih * s_scale) + cross_offset * r.height(),
                )
            };
            let o = wheel_origin.unwrap_or([-3.0, 0.5]);
            let min = r.min + vec2(x, y);
            (min + vec2(iw, ih) / 2.0, min + vec2(o[0] * iw, o[1] * ih))
        });
        let strip = CarouselStrip {
            r,
            vertical,
            wheel,
            arc_geo,
            wheel_angle: *wheel_angle,
            radius_px: spacing / wheel_angle.to_radians().max(0.01),
            start: if vertical { r.top() } else { r.left() },
            len,
            spacing,
            extra,
            selected_position: *selected_position,
            item_align: *item_align,
            selected_margins: *selected_margins,
            screen: vec2(sw, sh),
        };
        let focus_alpha = ft.and_then(|(_, t)| t.opacity).unwrap_or(1.0).clamp(0.0, 1.0);
        let shift = ft.and_then(|(_, t)| t.translate).map_or(Vec2::ZERO, |t| vec2(t[0] * sw, t[1] * sh));
        for slot in order {
            let i = item_at(slot);
            let d = slot as f32 - pos;
            let close = (1.0 - d.abs()).max(0.0);
            let scale = 1.0 + (s_scale - 1.0) * close;
            let a = (unselected_opacity + (focus_alpha - unselected_opacity) * close).clamp(0.0, 1.0) * common.opacity;
            let size_v = vec2(iw, ih) * scale;
            let (center, angle) = strip.place(d, size_v);
            let rect = Rect::from_center_size(center + shift * close, size_v);
            if !rect.intersects(f.bounds.expand(rect.width())) {
                continue;
            }
            let b = items.binder(i, f.base);
            let focus = if i == sel { close } else { 0.0 };
            self.draw_item(f, rect, &b, &spec, a, focus, angle);
            if f.scene.interactive {
                f.hits.push((i, rect));
            }
        }
        // Prefetch just beyond the visible range.
        for slot in [hi + 1, hi + 2, lo - 1, lo - 2] {
            if wrap || (0..n as i64).contains(&slot) {
                self.prefetch_item(f, item_at(slot), &spec);
            }
        }
    }

    /// The carousel's scroll position (in item slots) this frame, moving towards the selection:
    /// with the focus transition's time and easing when it has one, else springing. A looping
    /// (`wrap`) carousel takes the short way round.
    #[allow(clippy::too_many_arguments)] // the scroll state's inputs, all per frame
    fn carousel_position(
        &mut self,
        f: &mut Frame,
        idx: usize,
        sel: usize,
        n: usize,
        vis: f32,
        wrap: bool,
        transition: Option<&Transition>,
    ) -> f32 {
        let (dt, list_id) = (f.dt, f.scene.list_id);
        let (now, instant) = (f.now, self.opts.reduce_motion);
        let st = self.state(f.scene.kind, idx);
        let nf = n as f32;
        if let Some(tr) = transition {
            // `transitions.focus`: the scroll takes the transition's time and easing.
            if !st.init || st.list_id != list_id {
                st.init = true;
                st.list_id = list_id;
                st.pos = sel as f32;
                st.top = sel as f32;
                st.tween = None;
            }
            if (st.top.rem_euclid(nf) - sel as f32).abs() > 0.5 {
                let to = if wrap {
                    let d = (sel as f32 - st.top).rem_euclid(nf);
                    st.top + if d > nf / 2.0 { d - nf } else { d }
                } else {
                    sel as f32
                };
                st.tween = Some((st.pos, to, now));
                st.top = to;
            }
            if (st.pos - st.top).abs() > vis * 3.0 {
                st.tween = None;
            }
            match st.tween {
                Some((from, to, t0)) if !instant && !tr.done(now - t0) => {
                    st.pos = from + (to - from) * tr.progress(now - t0);
                    f.out.animating = true;
                }
                _ => {
                    st.tween = None;
                    st.pos = sel as f32;
                    st.top = sel as f32;
                }
            }
            return st.pos;
        }
        if wrap && st.init && st.list_id == list_id {
            // Animate the short way round: Left from the first item slides one slot left
            // to the last one instead of scrolling back through every item.
            let d = (sel as f32 - st.pos).rem_euclid(nf);
            st.pos = sel as f32 - if d > nf / 2.0 { d - nf } else { d };
        }
        if !st.init || st.list_id != list_id || (st.pos - sel as f32).abs() > vis * 3.0 {
            st.init = true;
            st.list_id = list_id;
            st.pos = sel as f32;
        }
        if approach(&mut st.pos, sel as f32, dt, 13.0) {
            f.out.animating = true;
        }
        st.pos
    }

    pub(super) fn draw_grid(&mut self, f: &mut Frame, idx: usize, el: &Element, _focused: bool) {
        let Element::Grid {
            common,
            columns,
            item_aspect,
            spacing,
            item_image,
            item_fallbacks,
            item_text,
            item_text_style,
            show_labels,
            selected_scale,
            selected_border,
            item_background,
            corner_radius,
            unselected_opacity,
            tiles,
        } = el
        else {
            return;
        };
        let tiles = tiles.as_deref();
        // "Show game titles" off hides the names under the covers too (a game without a
        // cover then shows its name on the card).
        let show_labels = *show_labels && self.opts.show_titles;
        let r = Self::elem_rect(f, common, Vec2::ZERO);
        f.out.focus_rect = Some(r);
        let items = f.scene.items;
        let n = items.len();
        if n == 0 || r.width() <= 0.0 || r.height() <= 0.0 {
            return;
        }
        let sh = f.sh();
        let sel = f.scene.selected.min(n - 1);
        let cols = (*columns).max(1) as usize;
        let g = spacing * sh;
        let s_scale = focus_scale(common, *selected_scale);
        let iw0 = ((r.width() - (cols as f32 - 1.0) * g) / cols as f32).max(1.0);
        // Leave room inside the box for the scaled-up selection.
        let grow = (s_scale - 1.0).max(0.0);
        let pad_x = iw0 * grow / 2.0;
        let iw = ((r.width() - 2.0 * pad_x - (cols as f32 - 1.0) * g) / cols as f32).max(1.0);
        let ih = iw * item_aspect.max(0.05);
        let pad_y = ih * grow / 2.0 + (sh * 0.004).max(2.0);
        let label_px = item_text_style.font_size * sh * f.ts;
        let label_h = if show_labels { label_px * 1.7 } else { 0.0 };
        let row_h = ih + label_h + g;
        let content =
            Rect::from_min_max(pos2(r.left() + pad_x, r.top() + pad_y), pos2(r.right() - pad_x, r.bottom() - pad_y));
        let vis = ((content.height() + g) / row_h).max(1.0);
        let rows = n.div_ceil(cols) as f32;
        let pos = self.grid_scroll(f, idx, (sel / cols) as f32, vis.floor().max(1.0), rows);
        // `transitions.focus`: the focus grows onto the new item, and off the old one.
        let ft = focus_transition(common);
        let amounts = ft.map(|(t, _)| {
            let (now, instant, list_id) = (f.now, self.opts.reduce_motion, f.scene.list_id);
            let (a, moving) = self.state(f.scene.kind, idx).focus.step(list_id, Some(sel as u64), now, t, instant);
            f.out.animating |= moving;
            a
        });
        let prev_id = amounts.and_then(|a| a.prev).map(|(p, _)| p as usize);
        let mut prev_item: Option<(Rect, usize)> = None;
        let clip =
            Rect::from_min_max(pos2(r.left() - pad_x, r.top()), pos2(r.right() + pad_x, r.bottom() + label_h * 0.2));
        let clipped = f.painter.with_clip_rect(clip);
        let painter = std::mem::replace(&mut f.painter, clipped);
        let size = size_class(iw.max(ih) * s_scale * f.ppp);
        self.item_sizes.insert((f.scene.kind, idx), size);
        let spec = ItemSpec {
            tint: tiles.and_then(|t| t.image_color).map(|c| c.to_egui()),
            image: item_image,
            fallbacks: item_fallbacks,
            fit: common.item_fit.unwrap_or(Fit::Contain),
            radius: corner_radius * sh,
            fill: item_background.map(|c| c.to_egui()),
            text: item_text,
            subtext: common.item_subtext.as_deref(),
            caption: common.item_caption.as_deref(),
            style: item_text_style,
            border: selected_border.map(|c| c.to_egui()),
            base_h: ih,
            size,
            badge: common.controller_badge,
            fav_badge: common.favorite_badge,
            labelled: show_labels,
            look: FocusLook::of(common, sh),
            templates: &common.item_elements,
            key: idx,
            adjust: picture_adjust(common),
        };
        // Tiles: the focused item's own picture tint, name colour and card.
        let sel_style =
            tiles.and_then(|t| t.selected_text_color).map(|color| TextStyle { color, ..item_text_style.clone() });
        let sel_spec = match tiles {
            Some(t) => ItemSpec {
                tint: t.selected_image_color.or(t.image_color).map(|c| c.to_egui()),
                fill: t.selected_text_background.map(|c| c.to_egui()).or(spec.fill),
                style: sel_style.as_ref().unwrap_or(item_text_style),
                ..spec
            },
            None => spec,
        };
        let first_row = (pos.floor() - 1.0).max(0.0) as usize;
        let last_row = ((pos + vis).ceil() as usize + 1).min(rows as usize);
        let mut sel_item: Option<(Rect, usize)> = None;
        let label_fam = f.fonts.family(f.theme, item_text_style.font.as_deref());
        for row in first_row..last_row {
            for col in 0..cols {
                let i = row * cols + col;
                if i >= n {
                    break;
                }
                let x = content.left() + col as f32 * (iw + g);
                let y = content.top() + (row as f32 - pos) * row_h;
                let rect = Rect::from_min_size(pos2(x, y), vec2(iw, ih));
                if i == sel {
                    sel_item = Some((rect, i));
                    continue;
                }
                if prev_id == Some(i) {
                    prev_item = Some((rect, i));
                    continue;
                }
                let b = items.binder(i, f.base);
                let a = unselected_opacity.clamp(0.0, 1.0) * common.opacity;
                self.draw_tile(f, rect, &b, &spec, tiles, a, 0.0);
                if show_labels {
                    let t = b.resolve(item_text, false);
                    let lr = Rect::from_min_size(pos2(x, y + ih + label_px * 0.35), vec2(iw, label_px * 1.3));
                    let gl = layout_line(
                        &f.painter,
                        t.trim(),
                        FontId::new(label_px, label_fam.clone()),
                        fade(item_text_style.color.to_egui(), a * 0.85),
                        iw,
                    );
                    let shadow = item_text_style.shadow.map(|c| fade(c.to_egui(), a));
                    let lp = line_pos(lr, &gl, item_text_style.align, VAlign::Top);
                    paint_galley(&f.painter, lp, &gl, Color32::WHITE, shadow, 0.0);
                }
                if f.scene.interactive && clip.intersects(rect) {
                    f.hits.push((i, rect));
                }
            }
        }
        // The focused item (and, while the focus moves, the one losing it) over the others.
        let focus_items: Vec<(Rect, usize, f32, f32)> = if let (Some((_, tr)), Some(am)) = (ft, amounts) {
            let rest = unselected_opacity.clamp(0.0, 1.0) * common.opacity;
            let full = tr.opacity.unwrap_or(1.0).clamp(0.0, 1.0) * common.opacity;
            let shift = tr.translate.map_or(Vec2::ZERO, |t| vec2(t[0] * f.sw(), t[1] * sh));
            [prev_item, sel_item]
                .into_iter()
                .flatten()
                .map(|(rect, i)| {
                    let amount = am.of(i as u64);
                    let (rect, a) = focus_item(rect, rest, full, s_scale, shift, amount);
                    (rect, i, a, amount)
                })
                .collect()
        } else {
            sel_item
                .map(|(rect, i)| (Rect::from_center_size(rect.center(), rect.size() * s_scale), i, common.opacity, 1.0))
                .into_iter()
                .collect()
        };
        for (rect, i, a, amount) in focus_items {
            let b = items.binder(i, f.base);
            let item_spec = if i == sel { &sel_spec } else { &spec };
            self.draw_tile(f, rect, &b, item_spec, tiles, a, amount);
            if show_labels {
                focused_label(f, rect, &b, item_text, item_text_style, label_px, &label_fam, a);
            }
            if f.scene.interactive {
                f.hits.push((i, rect));
            }
        }
        // Prefetch the next rows below and above.
        for row in [last_row, last_row + 1, first_row.wrapping_sub(1)] {
            for col in 0..cols {
                let i = row.wrapping_mul(cols).wrapping_add(col);
                if row < rows as usize && i < n {
                    self.prefetch_item(f, i, &spec);
                }
            }
        }
        f.painter = painter;
    }

    /// The grid's scroll position (in rows) this frame: the first visible row keeps the
    /// selection's row (`sel_row`) in view, and the position springs towards it.
    fn grid_scroll(&mut self, f: &mut Frame, idx: usize, sel_row: f32, vis_full: f32, rows: f32) -> f32 {
        let (dt, list_id) = (f.dt, f.scene.list_id);
        let st = self.state(f.scene.kind, idx);
        let mut top = st.top;
        if sel_row < top {
            top = sel_row;
        }
        if sel_row > top + vis_full - 1.0 {
            top = sel_row - vis_full + 1.0;
        }
        top = top.clamp(0.0, (rows - vis_full).max(0.0));
        st.top = top;
        if !st.init || st.list_id != list_id {
            st.init = true;
            st.list_id = list_id;
            st.pos = top;
        }
        if approach(&mut st.pos, top, dt, 14.0) {
            f.out.animating = true;
        }
        st.pos
    }

    /// Load item `i`'s picture ahead of time (it is just off screen).
    fn prefetch_item(&mut self, f: &mut Frame, i: usize, spec: &ItemSpec) {
        let b = f.scene.items.binder(i, f.base);
        if let Some(p) =
            self.pick_image_adjusted(f.theme, f.images, &b, spec.image, spec.fallbacks, spec.size, spec.adjust)
        {
            f.images.prefetch_adjusted(&p, spec.size, spec.adjust);
        }
    }

    pub(super) fn draw_textlist(&mut self, f: &mut Frame, idx: usize, el: &Element, _focused: bool) {
        let Element::TextList {
            common,
            item_text,
            style,
            selected_color,
            selector_color,
            selector_image,
            row_height,
            margin,
            favorite_color,
            corner_radius,
            selector_width,
            selector_offset,
            selected_background,
            selected_background_margins,
        } = el
        else {
            return;
        };
        let r = Self::elem_rect(f, common, Vec2::ZERO);
        f.out.focus_rect = Some(r);
        let items = f.scene.items;
        let n = items.len();
        if n == 0 || r.height() <= 0.0 {
            return;
        }
        let (sw, sh) = (f.sw(), f.sh());
        let px = (style.font_size * sh * f.ts).max(4.0);
        let rh = (px * row_height.max(0.5)).max(1.0);
        // Whole rows only (floor(height / row)): a list whose height isn't a
        // multiple of the row height must not show half a row at the bottom.
        let vis = whole_rows(r.height(), rh);
        let r = Rect::from_min_size(r.min, vec2(r.width(), (vis * rh).min(r.height())));
        let sel = f.scene.selected.min(n - 1) as f32;
        let max_top = (n as f32 - vis).max(0.0);
        let target_top = (sel - (vis - 1.0) / 2.0).clamp(0.0, max_top).round().min(max_top.ceil());
        let dt = f.dt;
        let list_id = f.scene.list_id;
        let (top, sel_anim) = {
            let st = self.state(f.scene.kind, idx);
            if !st.init || st.list_id != list_id {
                st.init = true;
                st.list_id = list_id;
                st.pos = target_top;
                st.sel = sel;
            }
            let a = approach(&mut st.pos, target_top, dt, 16.0);
            let b = approach(&mut st.sel, sel, dt, 20.0);
            if a || b {
                f.out.animating = true;
            }
            (st.pos, st.sel)
        };
        let painter = f.painter.with_clip_rect(r);
        let bar = selector_rect(r, r.top() + (sel_anim - top) * rh, rh, *selector_width, *selector_offset, sw);
        let rad = (corner_radius * sh).min(bar.width() / 2.0);
        // A selector picture is drawn in the selector colour instead of the plain bar.
        let picture = selector_image.as_deref().and_then(|img| {
            let b = f.base;
            let p = self.pick_image(f.theme, f.images, &b, img, &[], ImgSize::Medium)?;
            f.images.get(&p, ImgSize::Medium)
        });
        match (&picture, selector_color) {
            (Some(t), c) => {
                let tint = c.map(|c| c.to_egui()).unwrap_or(Color32::WHITE);
                paint_image(&painter, bar, t, Fit::Stretch, fade(tint, common.opacity), rad, 0.0);
            }
            (None, Some(c)) => {
                painter.rect_filled(bar, cr(rad), fade(c.to_egui(), common.opacity));
            }
            (None, None) => {}
        }
        if let (Some(bg), None) = (selected_background, selected_background_margins) {
            // The selected row's own box over the bar, as wide as the bar.
            painter.rect_filled(bar, cr(rad), fade(bg.to_egui(), common.opacity));
        }
        let fam = f.fonts.family(f.theme, style.font.as_deref());
        let m = margin * sw;
        let lo = top.floor().max(0.0) as usize;
        let hi = ((top + vis).ceil() as usize + 1).min(n);
        for i in lo..hi {
            let y = r.top() + (i as f32 - top) * rh;
            let row = Rect::from_min_size(pos2(r.left() + m, y), vec2((r.width() - 2.0 * m).max(1.0), rh));
            let b = items.binder(i, f.base);
            let fav = b.game.is_some_and(|g| g.favorite);
            // Selected colour follows the (animated) bar: a jump far down the list must not
            // show the target row in the bar's text colour before the bar gets there.
            let is_sel = (i as f32 - sel_anim).abs() < 0.5;
            let c = if is_sel {
                selected_color.unwrap_or(style.color)
            } else if fav {
                favorite_color.unwrap_or(style.color)
            } else {
                style.color
            };
            let t = b.resolve(item_text, false);
            let text = if style.uppercase { t.trim().to_uppercase() } else { t.trim().to_string() };
            let badge =
                common.controller_badge && b.game.is_some_and(|g| g.controller() == Some(ControllerSupport::Full));
            let mut text_row = row;
            let star_r = px * 0.42;
            // Where the favourite ★ goes: the row's end, left of the gamepad glyph when that is
            // drawn too.
            let mut star_right = row.right();
            if badge {
                // Gamepad glyph at the row's end; the name truncates before it.
                let gw = px * 1.1;
                text_row.max.x -= gw + px * 0.7;
                let cut = if is_sel {
                    selector_color.unwrap_or(f.theme.file.palette.background)
                } else {
                    f.view_bg.unwrap_or(f.theme.file.palette.background)
                };
                // Dimmed by mixing towards the background, not by alpha: the glyph is made of
                // overlapping shapes and translucent overlaps would show.
                let cut = fade(cut.to_egui(), common.opacity);
                gamepad_glyph(
                    &painter,
                    pos2(row.right() - gw / 2.0, row.center().y),
                    gw,
                    lerp_color(cut, fade(c.to_egui(), common.opacity), if is_sel { 0.9 } else { 0.6 }),
                    cut,
                );
                star_right -= gw + px * 0.5;
            }
            if common.favorite_badge && fav {
                // In the row's own text colour (the favourite colour, or the selected colour on
                // the bar), so it reads wherever the list is drawn.
                let cx = star_right - star_r;
                star(&painter, pos2(cx, row.center().y), star_r, fade(c.to_egui(), common.opacity));
                text_row.max.x = text_row.max.x.min(cx - star_r - px * 0.6);
            }
            let gl = layout_line(
                &painter,
                &text,
                FontId::new(px, fam.clone()),
                fade(c.to_egui(), common.opacity),
                text_row.width(),
            );
            let pos = line_pos(text_row, &gl, style.align, VAlign::Center);
            if is_sel && let (Some(bg), Some([l, r])) = (selected_background, selected_background_margins) {
                // Around the selected text, at the bar's height.
                let bx = Rect::from_x_y_ranges((pos.x - l * sw)..=(pos.x + gl.size().x + r * sw), bar.y_range());
                painter.rect_filled(
                    bx,
                    cr((corner_radius * sh).min(bx.height() / 2.0)),
                    fade(bg.to_egui(), common.opacity),
                );
            }
            let shadow = style.shadow.map(|c| fade(c.to_egui(), common.opacity));
            paint_galley(&painter, pos, &gl, Color32::WHITE, shadow, 0.0);
            if f.scene.interactive {
                f.hits.push((i, row.intersect(r)));
            }
        }
    }
}
