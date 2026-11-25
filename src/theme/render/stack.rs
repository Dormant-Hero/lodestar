//! The `stack` container (children flowing down or across, measured with the real fonts and
//! pictures) and item templates (`Common::item_elements`, elements drawn over every item of a
//! grid / carousel / rows).
//!
//! A stack is laid out every frame it is drawn: children are measured (hidden / empty ones
//! take no space), placed, and drawn as copies whose `pos` / `size` are their place. Text is
//! laid out by egui's galley cache, so measuring and then drawing it costs one layout.

use super::*;

/// What a child may be: its own size, the size a `stretch` stack gives it across the flow,
/// and the most room it has (pixels; None = unbounded / not set).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Constraint {
    pub own: [Option<f32>; 2],
    pub stretch: [Option<f32>; 2],
    pub limit: [Option<f32>; 2],
    /// The axis a `stretch` stack of automatic size widens its children along once they are
    /// all measured (a rule with only its thickness given takes the stack's width).
    pub stretch_later: Option<usize>,
}

/// A measured child.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Sized {
    pub size: Vec2,
    /// Text: rows it was limited to by the room left (drawn with that `max_lines`).
    pub lines: Option<u32>,
    /// Text / dates: the height comes from the text (drawn with an automatic height);
    /// pictures: the height (or width) follows from the other one and the picture's aspect
    /// (drawn the same way, so the picture loads at the size it is measured at).
    pub auto_h: bool,
    pub auto_w: bool,
}

impl Sized {
    fn of(size: Vec2) -> Sized {
        Sized { size, lines: None, auto_h: false, auto_w: false }
    }
}

/// A laid-out stack: its outer size and its children's places (relative to its top-left).
pub(super) struct StackLayout {
    pub size: Vec2,
    pub children: Vec<(usize, Rect, Sized)>,
}

/// A `chips` text's parts: each "|"-separated template part resolved on its own (an empty
/// binding drops its chip; a binding giving several values gives one pill each).
pub(super) fn chip_parts(b: &Binder, text: &str) -> Vec<(String, Option<ChipIcon>)> {
    let game = b.game;
    text.split('|')
        .flat_map(|p| {
            let p = p.trim();
            // Achievements get a bar under their text (how far along, page or library count).
            let icon = if p.contains("game.achievements") {
                b.resolve("{game.achievementspercent}", false).trim().parse::<f32>().ok().map(ChipIcon::Progress)
            } else {
                ChipIcon::of(p, game)
            };
            let v = b.resolve(p, false);
            v.split('|').map(|x| (x.trim().to_string(), icon)).collect::<Vec<_>>()
        })
        .filter(|(p, _)| !p.is_empty())
        .collect()
}

fn pos_px(v: f32, len: f32) -> Option<f32> {
    let px = v * len;
    (px.is_finite() && px > 0.0).then_some(px)
}

/// Elements whose box may stretch across a stack (pictures keep their aspect instead).
fn stretchable(el: &Element) -> bool {
    matches!(el, Element::Text { .. } | Element::Rect { .. } | Element::Stack { .. } | Element::DateTime { .. })
}

impl Renderer {
    pub(super) fn draw_stack(&mut self, f: &mut Frame, key: usize, el: &Element) {
        let Element::Stack { common, children, background, corner_radius, border_color, border_width, .. } = el else {
            return;
        };
        let own = [pos_px(common.size[0], f.sw()), pos_px(common.size[1], f.sh())];
        let ts = f.ts;
        let Some(lay) = self.fitted_layout(f, key, el, own) else {
            f.ts = ts;
            return;
        };
        let r = Self::text_rect(f, common, lay.size);
        let a = common.opacity;
        let rad = corner_radius * f.sh();
        match background {
            Some(Brush::Solid(c)) => {
                f.painter.rect_filled(r, cr(rad), fade(c.to_egui(), a));
            }
            Some(Brush::Gradient(g)) => self.gradients.paint(&f.painter, r, [rad; 4], g, a),
            None => {}
        }
        if let Some(bc) = border_color {
            let w = (border_width * f.sh()).max(1.0 / f.ppp);
            f.painter.rect_stroke(r, cr(rad), Stroke::new(w, fade(bc.to_egui(), a)), StrokeKind::Inside);
        }
        let saved = (a < 1.0).then(|| {
            let s = f.painter.clone();
            f.painter.multiply_opacity(a.max(0.0));
            s
        });
        for (i, rel, sized) in lay.children {
            let Some(ch) = children.get(i) else { continue };
            let rect = rel.translate(r.min.to_vec2());
            let placed = place(f, ch, rect, sized);
            self.draw_fx(f, child_key(key, i), &placed, false, true);
        }
        if let Some(s) = saved {
            f.painter = s;
        }
        f.ts = ts;
    }

    /// The stack laid out as it is drawn. A horizontal one wider than its `max_width` is laid
    /// out again with its text smaller (`f.ts`, which the caller puts back once the children
    /// are drawn): only the text gives way; gaps, padding and pictures keep their size.
    fn fitted_layout(&mut self, f: &mut Frame, key: usize, el: &Element, own: [Option<f32>; 2]) -> Option<StackLayout> {
        let mut lay = self.stack_layout(f, key, el, own, [None, None])?;
        let Element::Stack { direction: Direction::Horizontal, max_width, children, .. } = el else {
            return Some(lay);
        };
        let room = max_width * f.sw();
        // Text widths don't follow the size exactly (hinting): a second pass catches the rest.
        for _ in 0..2 {
            if own[0].is_some() || room <= 0.0 || lay.size.x <= room {
                break;
            }
            let text: f32 = lay
                .children
                .iter()
                .filter(|(i, ..)| matches!(children.get(*i), Some(Element::Text { .. } | Element::DateTime { .. })))
                .map(|(_, r, _)| r.width())
                .sum();
            if text <= 0.0 {
                break;
            }
            let fixed = lay.size.x - text;
            f.ts *= ((room - fixed) / text).clamp(0.25, 1.0) * 0.995;
            lay = self.stack_layout(f, key, el, own, [None, None])?;
        }
        Some(lay)
    }

    /// Lay out a stack whose outer box is `boxed` (definite sizes) within `limit`.
    pub(super) fn stack_layout(
        &mut self,
        f: &mut Frame,
        key: usize,
        el: &Element,
        boxed: [Option<f32>; 2],
        limit: [Option<f32>; 2],
    ) -> Option<StackLayout> {
        let Element::Stack { common, direction, gap, align, justify, children, .. } = el else {
            return None;
        };
        let (sw, sh) = (f.sw(), f.sh());
        let (m, x) = match direction {
            Direction::Vertical => (1usize, 0usize),
            Direction::Horizontal => (0, 1),
        };
        let pad = common.padding.map(|p| [p[0] * sh, p[1] * sh]).unwrap_or([0.0, 0.0]);
        let inner_box = [0, 1].map(|a| boxed[a].map(|v| (v - 2.0 * pad[a]).max(0.0)));
        let inner_lim = [0, 1].map(|a| inner_box[a].or(limit[a].map(|v| (v - 2.0 * pad[a]).max(0.0))));
        let gap = (gap * sh).max(0.0);
        let mut used = 0.0f32;
        let mut cross_max = 0.0f32;
        // (child, main position, sized, margins [lead main, trail main, lead cross, trail cross])
        let mut placed: Vec<(usize, f32, Sized, [f32; 4])> = Vec::new();
        for (i, ch) in children.iter().enumerate() {
            let c = ch.common();
            if !f.base.visible(c) {
                continue;
            }
            let mg = c.margin.map(|v| v.map(|x| (x * sh).max(0.0))).unwrap_or([0.0; 4]);
            // [left, top, right, bottom] → along / across the flow.
            let mgs = if m == 1 { [mg[1], mg[3], mg[0], mg[2]] } else { [mg[0], mg[2], mg[1], mg[3]] };
            let g = if placed.is_empty() { 0.0 } else { gap };
            let mut con = Constraint { own: [pos_px(c.size[0], sw), pos_px(c.size[1], sh)], ..Default::default() };
            // `shrink`: the room the children after this one take is kept for them.
            let reserve = match inner_lim[m] {
                Some(room) if c.shrink => self.room_after(f, key, el, i, room - used - g),
                _ => 0.0,
            };
            con.limit[m] = inner_lim[m].map(|v| (v - used - g - mgs[0] - mgs[1] - reserve).max(0.0));
            con.limit[x] = inner_lim[x].map(|v| (v - mgs[2] - mgs[3]).max(0.0));
            if *align == StackAlign::Stretch {
                con.stretch[x] = inner_box[x].map(|v| (v - mgs[2] - mgs[3]).max(0.0));
                con.stretch_later = Some(x);
            }
            let Some(sized) = self.measure(f, child_key(key, i), ch, con) else {
                continue;
            };
            let main = [sized.size.x, sized.size.y][m];
            let cross = [sized.size.x, sized.size.y][x];
            placed.push((i, used + g + mgs[0], sized, mgs));
            used += g + mgs[0] + main + mgs[1];
            cross_max = cross_max.max(cross + mgs[2] + mgs[3]);
        }
        if placed.is_empty() && !(boxed[0].is_some() && boxed[1].is_some()) {
            return None;
        }
        let mut content = [0.0f32; 2];
        content[m] = used;
        content[x] = cross_max;
        let size = [0, 1].map(|a| boxed[a].unwrap_or(content[a] + 2.0 * pad[a]));
        let room_main = inner_box[m].unwrap_or(used);
        let shift = justify.offset(room_main, used);
        let room_cross = inner_box[x].unwrap_or(cross_max);
        let mut out = Vec::with_capacity(placed.len());
        for (i, main_at, mut sized, mgs) in placed {
            let ch = &children[i];
            let own_cross = pos_px(ch.common().size[x], if x == 0 { sw } else { sh });
            let mut sz = [sized.size.x, sized.size.y];
            if *align == StackAlign::Stretch && own_cross.is_none() && stretchable(ch) {
                sz[x] = (room_cross - mgs[2] - mgs[3]).max(sz[x]);
            }
            let cross_at = mgs[2] + align.offset(room_cross - mgs[2] - mgs[3], sz[x]);
            let mut min = [0.0f32; 2];
            min[m] = pad[m] + shift + main_at;
            min[x] = pad[x] + cross_at;
            sized.size = vec2(sz[0], sz[1]);
            out.push((i, Rect::from_min_size(pos2(min[0], min[1]), sized.size), sized));
        }
        Some(StackLayout { size: vec2(size[0], size[1]), children: out })
    }

    /// The room (along the flow, with gaps and margins) the visible children after child `i`
    /// of stack `el` take out of `room`.
    fn room_after(&mut self, f: &mut Frame, key: usize, el: &Element, i: usize, room: f32) -> f32 {
        let Element::Stack { direction, gap, children, .. } = el else { return 0.0 };
        let (sw, sh) = (f.sw(), f.sh());
        let m = if *direction == Direction::Vertical { 1usize } else { 0 };
        let gap = (gap * sh).max(0.0);
        let mut taken = 0.0;
        for (j, ch) in children.iter().enumerate().skip(i + 1) {
            let c = ch.common();
            if !f.base.visible(c) {
                continue;
            }
            let mg = c.margin.map(|v| v.map(|x| (x * sh).max(0.0))).unwrap_or([0.0; 4]);
            let lead_trail = if m == 1 { mg[1] + mg[3] } else { mg[0] + mg[2] };
            let mut con = Constraint { own: [pos_px(c.size[0], sw), pos_px(c.size[1], sh)], ..Default::default() };
            con.limit[m] = Some((room - taken - gap - lead_trail).max(0.0));
            if let Some(sz) = self.measure(f, child_key(key, j), ch, con) {
                taken += gap + lead_trail + [sz.size.x, sz.size.y][m];
            }
        }
        taken
    }

    /// How big `el` is inside a stack (None = it is empty and takes no space).
    pub(super) fn measure(&mut self, f: &mut Frame, key: usize, el: &Element, con: Constraint) -> Option<Sized> {
        let (sw, sh) = (f.sw(), f.sh());
        let boxed = [0, 1].map(|a| con.own[a].or(con.stretch[a]));
        let both = |b: [Option<f32>; 2]| match b {
            [Some(w), Some(h)] if w > 0.0 && h > 0.0 => Some(Sized::of(vec2(w, h))),
            _ => None,
        };
        match el {
            Element::Text { common, text, style, background, wrap, max_lines, scroll } => {
                if !self.opts.show_battery && is_battery_text(text) {
                    return None;
                }
                if common.chips {
                    return self.measure_chips(f, common, text, style, *background, con, boxed);
                }
                let t = f.base.resolve(text, false);
                if t.trim().is_empty() {
                    return None;
                }
                let t = t.trim_end().to_string();
                measure_text(f, common, &t, style, *wrap, *max_lines, *scroll, con, boxed)
            }
            Element::DateTime { common, value, style, clock, format, .. } => {
                if *clock && !self.opts.show_clock {
                    return None;
                }
                let text = if *clock {
                    let fmt = if format == "%d %b %Y" { "%H:%M" } else { format.as_str() };
                    bind::format_date(crate::model::now_unix(), fmt)
                } else {
                    let raw = f.base.resolve(value, true);
                    match raw.trim().parse::<i64>() {
                        Ok(ts) if ts > 0 => bind::format_date(ts, format),
                        _ => f.base.resolve(value, false).trim().to_string(),
                    }
                };
                if text.is_empty() {
                    return None;
                }
                measure_text(f, common, &text, style, false, Some(1), false, con, boxed)
            }
            Element::Image { common, path, fallbacks, fallback_text, video, .. } => {
                let own_any = con.own[0].is_some() || con.own[1].is_some();
                // Load at the size class the drawn copy will ask for (`place` keeps an automatic
                // side automatic); a stretched picture by the width it gets.
                let size = match con.stretch {
                    _ if own_any || common.blur > 0.0 => super::prefetch::image_size(common, true, sw, sh, f.ppp),
                    [Some(w), _] | [None, Some(w)] => size_class(w * f.ppp),
                    _ => super::prefetch::image_size(common, true, sw, sh, f.ppp),
                };
                let b = f.base;
                let chosen = self.pick_image(f.theme, f.images, &b, path, fallbacks, size);
                let Some(p) = chosen else {
                    // No picture: the fallback text, the stand-in or a video still needs a box.
                    let other = fallback_text.as_deref().is_some_and(|t| !b.resolve(t, false).trim().is_empty())
                        || (common.placeholder && b.game.is_some())
                        || (video.is_some() && self.opts.videos);
                    return if other { both(if own_any { con.own } else { boxed }) } else { None };
                };
                let Some(tex) = f.images.get(&p, size) else {
                    // Still loading: only a box of its own keeps its place.
                    return both(con.own);
                };
                let nat = tex.size_vec2() / f.ppp;
                if nat.x <= 0.0 || nat.y <= 0.0 {
                    return None;
                }
                let ar = nat.y / nat.x;
                let mut auto = [false, false];
                let mut s = match (con.own[0], con.own[1]) {
                    (Some(w), Some(h)) => vec2(w, h),
                    (Some(w), None) => {
                        auto[1] = true;
                        vec2(w, w * ar)
                    }
                    (None, Some(h)) => {
                        auto[0] = true;
                        vec2(h / ar, h)
                    }
                    (None, None) => match (con.stretch[0], con.stretch[1]) {
                        (Some(w), _) => vec2(w, w * ar),
                        (None, Some(h)) => vec2(h / ar, h),
                        _ => {
                            auto = [true, true];
                            nat
                        }
                    },
                };
                if !(con.own[0].is_some() && con.own[1].is_some()) {
                    if let Some(lw) = con.limit[0].filter(|lw| s.x > *lw) {
                        s = vec2(lw, lw * ar);
                        auto = [false, false];
                    }
                    if let Some(lh) = con.limit[1].filter(|lh| s.y > *lh && con.own[1].is_none()) {
                        s = vec2(lh / ar, lh);
                        auto = [false, false];
                    }
                }
                (s.x >= 0.5 && s.y >= 0.5).then_some(Sized { size: s, lines: None, auto_h: auto[1], auto_w: auto[0] })
            }
            // Only its thickness given: as wide (tall) as the stack, once that is known.
            Element::Rect { .. } => both(boxed).or_else(|| {
                let x = con.stretch_later?;
                let along = con.own[1 - x].filter(|v| *v > 0.0)?;
                Some(Sized::of(if x == 0 { vec2(0.0, along) } else { vec2(along, 0.0) }))
            }),
            Element::Stack { .. } => {
                let lay = self.stack_layout(f, key, el, boxed, con.limit)?;
                (lay.size.x > 0.0 || lay.size.y > 0.0).then_some(Sized::of(lay.size))
            }
            Element::Rating { .. } => {
                let h = match (con.own[0], con.own[1]) {
                    (_, Some(h)) => h,
                    (Some(w), None) => w / 5.0,
                    _ => boxed[1].or(boxed[0].map(|w| w / 5.0))?,
                };
                (h > 0.0).then_some(Sized::of(vec2(h * 5.0, h)))
            }
            Element::Badges { slots, .. } => {
                let g = f.base.game?;
                if !slots.iter().any(|s| badge_active(s, g)) {
                    return None;
                }
                both(boxed)
            }
            Element::HelpBar { glyphs: true, .. } => both(con.own),
            Element::HelpBar { style, .. } => {
                if f.scene.hints.is_empty() {
                    return None;
                }
                let px = style.font_size * sh * f.ts;
                let fam = f.fonts.family(f.theme, style.font.as_deref());
                let need = hints_width(&f.painter, f.scene.hints, f.scene.prompts, fam, px);
                Some(Sized::of(vec2(con.own[0].unwrap_or(need + px * 1.6), con.own[1].unwrap_or(px * 2.0))))
            }
            _ => both(con.own),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn measure_chips(
        &mut self,
        f: &mut Frame,
        common: &Common,
        text: &str,
        style: &TextStyle,
        background: Option<Color>,
        con: Constraint,
        boxed: [Option<f32>; 2],
    ) -> Option<Sized> {
        let parts = chip_parts(&f.base, text);
        if parts.is_empty() {
            return None;
        }
        let sh = f.sh();
        let px = (style.font_size * sh * f.ts).max(4.0);
        let pad = common.padding.map(|p| vec2(p[0] * sh, p[1] * sh)).unwrap_or(Vec2::ZERO);
        let fam = f.fonts.family(f.theme, style.font.as_deref());
        let st = TextStyle { align: Align::Left, ..style.clone() };
        let icon_w = px * 1.25 + px * 0.4;
        let gap = if background.is_some() { px * 0.5 } else { px * 1.1 };
        let mut total = 0.0f32;
        let mut h = 0.0f32;
        for (i, (p, icon)) in parts.iter().enumerate() {
            let g = layout(&f.painter, p, fam.clone(), px, Color32::WHITE, &st, None, None);
            let room = match icon {
                Some(ChipIcon::Progress(_)) | None => 0.0,
                Some(_) => icon_w,
            };
            total += g.size().x + room + pad.x * 2.0 + if i > 0 { gap } else { 0.0 };
            h = h.max(g.size().y);
        }
        if parts.iter().any(|(_, i)| matches!(i, Some(ChipIcon::Progress(_)))) {
            let (bar_h, bar_gap) = super::elements::progress_room(px);
            h += bar_h + bar_gap;
        }
        let w = boxed[0].unwrap_or_else(|| con.limit[0].map_or(total, |l| total.min(l)));
        Some(Sized {
            size: vec2(w, boxed[1].unwrap_or(h + pad.y * 2.0)),
            lines: None,
            auto_h: boxed[1].is_none(),
            auto_w: false,
        })
    }

    /// `Common::item_elements` over an item drawn in `rect` (the picture's box): the item's
    /// box is their screen, its binder their data.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_item_template<'b>(
        &mut self,
        f: &mut Frame<'_, 'b>,
        rect: Rect,
        b: &Binder<'b>,
        spec: &ItemSpec,
        alpha: f32,
        focus: f32,
    ) {
        if rect.width() < 2.0 || rect.height() < 2.0 || alpha <= 0.0 {
            return;
        }
        let id = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            b.game.map(|g| g.id).hash(&mut h);
            b.collection.map(|c| c.collection.key()).hash(&mut h);
            h.finish() as usize
        };
        let base_key = child_key(spec.key, id);
        let screen = std::mem::replace(&mut f.screen, rect);
        let base = std::mem::replace(&mut f.base, Binder { focused: focus > 0.5, ..*b });
        let saved = (alpha < 1.0).then(|| {
            let s = f.painter.clone();
            f.painter.multiply_opacity(alpha);
            s
        });
        for (i, el) in spec.templates.iter().enumerate() {
            if el.is_list() {
                continue;
            }
            let v = f.base.visible(el.common());
            self.draw_fx(f, child_key(base_key, i), el, false, v);
        }
        if let Some(s) = saved {
            f.painter = s;
        }
        f.screen = screen;
        f.base = base;
    }
}

/// A text's size in a stack, laid out as `draw_text` will lay it out.
#[allow(clippy::too_many_arguments)]
fn measure_text(
    f: &Frame,
    common: &Common,
    text: &str,
    style: &TextStyle,
    wrap: bool,
    max_lines: Option<u32>,
    scroll: bool,
    con: Constraint,
    boxed: [Option<f32>; 2],
) -> Option<Sized> {
    let sh = f.sh();
    let px = (style.font_size * sh * f.ts).max(4.0);
    let pad = common.padding.map(|p| vec2(p[0] * sh, p[1] * sh)).unwrap_or(Vec2::ZERO);
    let family = f.fonts.family(f.theme, style.font.as_deref());
    let line_h = line_height(px, style.line_spacing);
    let multi = wrap || max_lines.is_some_and(|m| m > 1);
    let w = boxed[0].or(con.limit[0]);
    let mut limited = None;
    let (wrap_w, rows) = match w {
        Some(w) => {
            let inner = (w - pad.x * 2.0).max(1.0);
            if multi {
                let fit = match (boxed[1], con.limit[1]) {
                    (Some(h), _) if !scroll => Some(((h - pad.y * 2.0) / line_h).floor().max(1.0) as u32),
                    (None, Some(h)) => {
                        let n = ((h - pad.y * 2.0) / line_h + 1e-3).floor();
                        if n < 1.0 {
                            return None;
                        }
                        Some(n as u32)
                    }
                    _ => None,
                };
                let rows = match (max_lines, fit) {
                    (Some(m), Some(fr)) => Some(m.min(fr)),
                    (Some(m), None) => Some(m),
                    (None, fr) => fr,
                };
                if boxed[1].is_none() && fit.is_some() && rows == fit && max_lines != rows {
                    limited = rows;
                }
                (Some(inner), rows)
            } else {
                (Some(inner), Some(1))
            }
        }
        None => (None, None),
    };
    let color = Color32::WHITE;
    let galley = layout(&f.painter, text, family, px, color, style, wrap_w, rows);
    let natural = galley.size() + pad * 2.0;
    let width = match (boxed[0], con.limit[0]) {
        (Some(b), _) => b,
        (None, Some(l)) if galley.rows.len() > 1 || galley.elided => l,
        (None, Some(l)) => (natural.x + 0.5).min(l),
        (None, None) => natural.x + 0.5,
    };
    let height = boxed[1].unwrap_or(natural.y);
    Some(Sized { size: vec2(width, height), lines: limited, auto_h: boxed[1].is_none(), auto_w: false })
}

/// A copy of a stack child that draws in `rect` (its own `pos` / `origin` / `margin` dropped).
fn place(f: &Frame, el: &Element, rect: Rect, sized: Sized) -> Element {
    let (sw, sh) = (f.sw().max(1.0), f.sh().max(1.0));
    let mut out = el.clone();
    {
        let c = out.common_mut();
        c.pos = [(rect.min.x - f.screen.min.x) / sw, (rect.min.y - f.screen.min.y) / sh];
        c.origin = [0.0, 0.0];
        c.size =
            [if sized.auto_w { 0.0 } else { rect.width() / sw }, if sized.auto_h { 0.0 } else { rect.height() / sh }];
        c.margin = None;
    }
    if let (Element::Text { max_lines, .. }, Some(l)) = (&mut out, sized.lines) {
        *max_lines = Some(l);
    }
    out
}
