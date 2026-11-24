//! Drawing single elements: images, text, chips and ratings.

use super::*;

/// Drawn icon in front of a chip (fonts lack these glyphs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ChipIcon {
    Gamepad,
    Deck(crate::model::DeckRating),
    /// A dot in the ProtonDB tier's colour.
    ProtonDb([u8; 3]),
    /// Not an icon: a thin bar under the chip's text, this far along (achievements, 0..1).
    Progress(f32),
}

/// Room a progress chip adds under its text: (bar height, gap above it).
pub(super) fn progress_room(px: f32) -> (f32, f32) {
    ((px * 0.15).max(2.0).round(), (px * 0.2).round())
}

impl ChipIcon {
    /// The icon for a chip template part: game.controller* bindings get the gamepad, game.deck*
    /// the Steam Deck mark (coloured by the rating).
    pub(super) fn of(template: &str, game: Option<&Game>) -> Option<ChipIcon> {
        if !template.contains('{') {
            return None;
        }
        if template.contains("game.controller") {
            Some(ChipIcon::Gamepad)
        } else if template.contains("game.deck") {
            game.and_then(|g| g.meta.deck).map(ChipIcon::Deck)
        } else if template.contains("game.protondb") {
            game.and_then(|g| g.meta.protondb.as_deref())
                .and_then(crate::metadata::protondb::tier_rgb)
                .map(ChipIcon::ProtonDb)
        } else {
            None
        }
    }
}

impl Renderer {
    /// First usable image among `tpl` + `fallbacks` for this binder.
    pub(super) fn pick_image(
        &mut self,
        theme: &Theme,
        images: &mut ImageCache,
        b: &Binder,
        tpl: &str,
        fallbacks: &[String],
        size: ImgSize,
    ) -> Option<PathBuf> {
        self.pick_image_adjusted(theme, images, b, tpl, fallbacks, size, Adjust::NONE)
    }

    /// `pick_image` for pictures drawn with a colour adjustment (a candidate that failed to
    /// load with it is skipped).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn pick_image_adjusted(
        &mut self,
        theme: &Theme,
        images: &mut ImageCache,
        b: &Binder,
        tpl: &str,
        fallbacks: &[String],
        size: ImgSize,
        adjust: Adjust,
    ) -> Option<PathBuf> {
        let mut cands = std::mem::take(&mut self.candidates);
        cands.clear();
        b.path_candidates(tpl, &mut cands);
        for f in fallbacks {
            b.path_candidates(f, &mut cands);
        }
        let mut found = None;
        for c in &cands {
            let Some(p) = Self::to_path(theme, c) else {
                continue;
            };
            if !self.exists(&p) || images.is_failed_adjusted(&p, size, adjust) {
                continue;
            }
            found = Some(p);
            break;
        }
        self.candidates = cands;
        found
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_image(
        &mut self,
        f: &mut Frame,
        idx: usize,
        common: &Common,
        path: &str,
        fallbacks: &[String],
        fit: Fit,
        tint: Option<Color>,
        tile: bool,
        corner_radius: f32,
        gradient: Option<[Color; 2]>,
        extra: ImageExtra,
    ) {
        let common = &*self.look_switches(common);
        let sh = f.sh();
        let size = super::prefetch::image_size(common, true, f.sw(), sh, f.ppp);
        let adjust = picture_adjust(common);
        let b = f.base;
        let chosen = self.pick_image_adjusted(f.theme, f.images, &b, path, fallbacks, size, adjust);
        let bound = path.contains('{') || fallbacks.iter().any(|p| p.contains('{'));
        let fade_secs = common.fade.unwrap_or(if bound { 0.25 } else { 0.0 }).max(0.0) as f64;
        let now = f.now;
        let tex = chosen.as_deref().and_then(|p| f.images.get_adjusted(p, size, adjust));
        let (prev_tex, t, ready) = self.state(f.scene.kind, idx).fade.step(&chosen, tex.is_some(), now, fade_secs);
        let prev_tex = prev_tex.and_then(|p| f.images.get_adjusted(&p, size, adjust));
        // While the new picture is still decoding nothing moves (the image cache repaints
        // when it is ready); once it is, the cross-fade animates.
        if prev_tex.is_some() && t < 1.0 && ready {
            f.out.animating = true;
        }
        // A game's background placement for elements bound to its background (Arrange mode).
        let crop = chosen.as_deref().and_then(|c| b.image_crop(path, fallbacks, c));
        let opaque = common.blur > 0.0 || fit == Fit::Cover || fit == Fit::Stretch;
        let alpha = common.opacity;
        let tint = tint.map(|c| c.to_egui()).unwrap_or(Color32::WHITE);
        let ease = |x: f32| x * x * (3.0 - 2.0 * x);
        let te = ease(t);
        let mut drew_any = false;
        let paint = |f: &mut Frame, tex: &TextureHandle, a: f32, crop: Option<crate::model::BackgroundPlacement>| {
            let natural = tex.size_vec2() / f.ppp;
            let rect = f.bleed(Self::elem_rect(f, common, natural));
            if tile && (common.size[0] <= 0.0 || common.size[1] <= 0.0) {
                // A tiled picture fills its box: without one (a theme that sizes it only in
                // some layouts) there is nothing to fill.
                return rect;
            }
            let tint = fade(tint, a);
            if common.blur > 0.0 {
                paint_blurred(&f.painter, rect, tex, fit, tint, common.blur * sh, crop);
            } else if tile {
                paint_tiled(&f.painter, rect, tex, natural, tint);
            } else if let Some(end) = extra.tint_end {
                return paint_image_tinted(
                    &f.painter,
                    rect,
                    tex,
                    fit,
                    [tint, fade(end.to_egui(), a)],
                    common.gradient_horizontal,
                    common.rotation,
                    Vec2::from(common.origin),
                    crop,
                );
            } else if let Some(mask) = common.mask {
                return paint_image_masked(&f.painter, rect, tex, fit, tint, mask, Vec2::from(common.origin), crop);
            } else {
                return paint_image_aligned(
                    &f.painter,
                    rect,
                    tex,
                    fit,
                    tint,
                    corner_radius * sh,
                    common.rotation,
                    Vec2::from(common.origin),
                    crop,
                );
            }
            rect
        };
        if common.placeholder
            && chosen.is_none()
            && let Some(g) = b.game
        {
            Self::paint_placeholder(f, common, g);
        }
        if chosen.is_none()
            && let Some(tpl) = extra.fallback_text.filter(|t| !t.trim().is_empty())
        {
            let text = b.resolve(tpl, false).trim().to_string();
            if !text.is_empty() {
                Self::paint_fallback_text(f, common, &text, extra.fallback_style);
            }
        }
        let mut last_rect = None;
        if let Some(pt) = &prev_tex
            && t < 1.0
        {
            let a = if opaque { alpha } else { alpha * (1.0 - te) };
            last_rect = Some(paint(f, pt, a, None));
            drew_any = true;
        }
        if let Some(tx) = &tex {
            // Fade in when replacing a picture, or on first appearance when fading is on.
            let a = if (prev_tex.is_some() && t < 1.0) || fade_secs > 0.0 { alpha * te } else { alpha };
            last_rect = Some(paint(f, tx, a, crop));
            drew_any = true;
            if t < 1.0 {
                f.out.animating = true;
            }
        }
        if let (true, Some(r), Some(g)) = (drew_any, last_rect, common.gradient_spec(gradient)) {
            // N stops / an angle / radial (mesh, cached per size).
            self.gradients.paint(&f.painter, r, [corner_radius * sh; 4], &g, alpha);
        } else if let (true, Some(r), Some([g0, g1])) = (drew_any, last_rect, gradient) {
            gradient_rect(
                &f.painter,
                r,
                corner_radius * sh,
                [fade(g0.to_egui(), alpha), fade(g1.to_egui(), alpha)],
                common.gradient_horizontal,
            );
        }
    }

    /// `common` with the user's look switches applied: no blur, and full-screen pictures at
    /// full strength.
    fn look_switches<'c>(&self, common: &'c Common) -> std::borrow::Cow<'c, Common> {
        let full_screen = |c: &Common| c.size[0] >= 0.95 && c.size[1] >= 0.95;
        if !(!self.opts.blur_background && common.blur > 0.0 || !self.opts.darken_background && full_screen(common)) {
            return std::borrow::Cow::Borrowed(common);
        }
        let mut c = common.clone();
        if !self.opts.blur_background {
            c.blur = 0.0;
        }
        if !self.opts.darken_background && full_screen(&c) {
            c.opacity = 1.0;
        }
        std::borrow::Cow::Owned(c)
    }

    /// `Image::fallback_text`: the text, in the theme's font, as large as fits the image's box
    /// on one line (two for long names), centred like a logo would be.
    fn paint_fallback_text(f: &mut Frame, common: &Common, text: &str, style: Option<&TextStyle>) {
        let rect = Self::elem_rect(f, common, Vec2::ZERO);
        if rect.width() < 8.0 || rect.height() < 8.0 {
            return;
        }
        let sh = f.sh();
        let mut st =
            style.cloned().unwrap_or_else(|| TextStyle { color: f.theme.file.palette.text, ..Default::default() });
        st.align = Align::Center;
        st.line_spacing = st.line_spacing.min(1.1);
        let fam = f.fonts.family(f.theme, st.font.as_deref());
        let color = fade(st.color.to_egui(), common.opacity);
        let inner = rect.shrink2(vec2(rect.width() * 0.04, rect.height() * 0.06));
        // The theme's size when it set one, else what fills the box; never wider than the box.
        let wanted = if style.is_some_and(|s| s.font_size > 0.0) {
            (st.font_size * sh * f.ts).min(inner.height() * 0.8)
        } else {
            inner.height() * 0.5
        };
        let mut px = wanted.clamp(6.0, (sh * 0.2).max(6.0));
        let longest = text
            .split_whitespace()
            .map(|w| layout(&f.painter, w, fam.clone(), px, color, &st, None, None).size().x)
            .fold(0.0f32, f32::max);
        px = word_fit_px(px, longest, inner.width()).max(6.0);
        let rows = if line_height(px, st.line_spacing) * 2.0 <= inner.height() { 2 } else { 1 };
        let g = layout(&f.painter, text, fam, px, color, &st, Some(inner.width()), Some(rows));
        let pos = galley_pos(inner, &g, Align::Center, VAlign::Center);
        paint_galley(&f.painter, pos, &g, color, st.shadow.map(|c| c.to_egui()), common.rotation);
    }

    /// The stand-in for a game without the artwork an image asks for (`Common::placeholder`):
    /// a wash in its console's / store's colour with the console's outline, fading out where the
    /// image's mask would fade the picture.
    fn paint_placeholder(f: &mut Frame, common: &Common, g: &Game) {
        let rect = f.bleed(Self::elem_rect(f, common, Vec2::ZERO));
        if rect.width() < 2.0 || rect.height() < 2.0 {
            return;
        }
        let tint = placeholder_tint(g);
        let a = common.opacity;
        let fades_left = common.mask.is_some_and(|m| m[0] > 0.0);
        if fades_left {
            gradient_rect(&f.painter, rect, 0.0, [fade(tint, 0.0), fade(tint, 0.5 * a)], true);
        } else {
            gradient_rect(&f.painter, rect, 0.0, [fade(tint, 0.55 * a), fade(tint, 0.3 * a)], false);
        }
        let side = rect.height().min(rect.width()) * 0.4;
        let c = pos2(rect.right() - rect.width() * 0.3, rect.center().y - rect.height() * 0.06);
        crate::ui::platform_icons::paint(
            &f.painter,
            Rect::from_center_size(c, vec2(side, side)),
            crate::ui::platform_icons::game_shape(g),
            fade(Color32::WHITE, 0.12 * a),
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_text(
        &mut self,
        f: &mut Frame,
        key: (ViewKind, usize),
        common: &Common,
        text: &str,
        style: &TextStyle,
        background: Option<Color>,
        wrap: bool,
        max_lines: Option<u32>,
        scroll: bool,
    ) {
        let (sw, sh) = (f.sw(), f.sh());
        let px = (style.font_size * sh * f.ts).max(4.0);
        let pad = common.padding.map(|p| vec2(p[0] * sh, p[1] * sh)).unwrap_or(Vec2::ZERO);
        let family = f.fonts.family(f.theme, style.font.as_deref());
        let color = fade(style.color.to_egui(), common.opacity);
        let box_w = common.size[0] * sw;
        let box_h = common.size[1] * sh;
        let line_h = line_height(px, style.line_spacing);
        let (wrap_w, rows) = if box_w > 0.0 {
            let inner = (box_w - pad.x * 2.0).max(1.0);
            if wrap || max_lines.is_some_and(|m| m > 1) {
                let fit_rows = if box_h > 0.0 && !scroll {
                    Some(((box_h - pad.y * 2.0) / line_h).floor().max(1.0) as u32)
                } else {
                    None
                };
                let rows = match (max_lines, fit_rows) {
                    (Some(m), Some(fr)) => Some(m.min(fr)),
                    (Some(m), None) => Some(m),
                    (None, fr) => fr,
                };
                (Some(inner), rows)
            } else {
                (Some(inner), Some(1))
            }
        } else {
            (None, None)
        };
        let galley = layout(&f.painter, text, family, px, color, style, wrap_w, rows);
        let natural = galley.size() + pad * 2.0;
        let rect = Self::text_rect(f, common, natural);
        if let Some(bg) = background {
            let rad = common.radius.unwrap_or(0.0) * sh;
            f.painter.rect_filled(rect, cr(rad), fade(bg.to_egui(), common.opacity));
        }
        let inner = rect.shrink2(pad);
        let shadow = style.shadow.map(|c| fade(c.to_egui(), common.opacity));
        let overflow = galley.size().y - inner.height();
        if scroll && wrap && box_h > 0.0 && overflow > 1.0 {
            // Auto-scroll: wait, scroll down slowly, hold, fade back to the top.
            let mut h = std::collections::hash_map::DefaultHasher::new();
            text.hash(&mut h);
            let th = h.finish();
            let now = f.now;
            let st = self.state(key.0, key.1);
            let start = match st.text {
                Some((hh, s)) if hh == th => s,
                _ => {
                    st.text = Some((th, now));
                    now
                }
            };
            let speed = (px * 0.9) as f64;
            let (wait, hold, back) = (3.0, 3.0, 0.6);
            let scroll_t = overflow as f64 / speed;
            let cycle = wait + scroll_t + hold + back;
            let elapsed = now - start;
            let ctx = f.painter.ctx().clone();
            let after = |secs: f64| {
                // NaN or infinite (a theme's odd sizes) would panic in `from_secs_f64`.
                let secs = if secs.is_nan() { 0.01 } else { secs.clamp(0.01, 3600.0) };
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(secs))
            };
            let (offset, alpha) = if elapsed >= cycle * SCROLL_CYCLES {
                // Done: rest at the top (fading in after the last cycle), no more frames.
                let fade_in = ((elapsed - cycle * SCROLL_CYCLES) / 0.4).min(1.0) as f32;
                if fade_in < 1.0 {
                    f.out.animating = true;
                }
                (0.0, fade_in)
            } else {
                let t = elapsed % cycle;
                if t < wait {
                    after(wait - t);
                    let fade_in = if elapsed > cycle { (t / 0.4).min(1.0) as f32 } else { 1.0 };
                    if fade_in < 1.0 {
                        f.out.animating = true;
                    }
                    (0.0, fade_in)
                } else if t < wait + scroll_t {
                    // Slow scroll: one frame per physical pixel of movement, not one per vsync.
                    let ppp = f.ppp.max(0.5) as f64;
                    after(1.0 / (speed * ppp));
                    let off = (t - wait) * speed;
                    (((off * ppp).round() / ppp) as f32, 1.0)
                } else if t < wait + scroll_t + hold {
                    after(wait + scroll_t + hold - t);
                    (overflow, 1.0)
                } else {
                    f.out.animating = true;
                    (overflow, (1.0 - (t - wait - scroll_t - hold) / back).clamp(0.0, 1.0) as f32)
                }
            };
            let p = f.painter.with_clip_rect(inner);
            let pos = galley_pos(inner, &galley, style.align, VAlign::Top) - vec2(0.0, offset);
            let c = fade(color, alpha);
            paint_galley(&p, pos, &galley, c, shadow.map(|s| fade(s, alpha)), 0.0);
            return;
        }
        let pos = galley_pos(inner, &galley, style.align, style.valign);
        if common.rotation != 0.0 {
            let pivot = rect.min + rect.size() * Vec2::from(common.rotation_origin);
            let p = rotate_around(pos, pivot, common.rotation);
            paint_galley(&f.painter, p, &galley, color, shadow, common.rotation);
        } else {
            paint_galley(&f.painter, pos, &galley, color, shadow, 0.0);
        }
    }

    /// "|"-separated parts drawn as pills flowing left to right (or right to left for
    /// right-aligned text). `pos`/`origin` place the whole row.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_chips(
        &mut self,
        f: &mut Frame,
        common: &Common,
        parts: &[(String, Option<ChipIcon>)],
        style: &TextStyle,
        background: Option<Color>,
        px: f32,
        pad: Vec2,
        family: egui::FontFamily,
        color: Color32,
    ) {
        let sh = f.sh();
        // Room an icon takes in front of its chip's text.
        let icon_w = px * 1.25;
        let icon_gap = px * 0.4;
        let icon_room = |i: &Option<ChipIcon>| match i {
            Some(ChipIcon::Progress(_)) | None => 0.0,
            Some(_) => icon_w + icon_gap,
        };
        // A progress chip makes the row a little taller (only while it is there).
        let (bar_h, bar_gap) = progress_room(px);
        let extra =
            if parts.iter().any(|(_, i)| matches!(i, Some(ChipIcon::Progress(_)))) { bar_h + bar_gap } else { 0.0 };
        let galleys: Vec<_> = parts
            .iter()
            .map(|(p, _)| {
                layout(
                    &f.painter,
                    p,
                    family.clone(),
                    px,
                    color,
                    &TextStyle { align: Align::Left, ..style.clone() },
                    None,
                    None,
                )
            })
            .collect();
        let gap = if background.is_some() { px * 0.5 } else { px * 1.1 };
        let h = galleys.iter().map(|g| g.size().y).fold(0.0, f32::max) + pad.y * 2.0 + extra;
        let total: f32 =
            galleys.iter().zip(parts).map(|(g, (_, i))| g.size().x + icon_room(i) + pad.x * 2.0).sum::<f32>()
                + gap * (galleys.len() - 1) as f32;
        let max_w = if common.size[0] > 0.0 { common.size[0] * f.sw() } else { f32::INFINITY };
        let row = Self::text_rect(f, common, vec2(total.min(max_w), h));
        let rad = common.radius.map(|r| r * sh).unwrap_or(h / 2.0);
        let bg = background.map(|c| fade(c.to_egui(), common.opacity));
        let shadow = style.shadow.map(|c| fade(c.to_egui(), common.opacity));
        let mut x = match style.align {
            Align::Left => row.left(),
            Align::Center => row.center().x - total.min(max_w) / 2.0,
            Align::Right => row.right() - total.min(max_w),
        };
        for (g, (_, icon)) in galleys.into_iter().zip(parts) {
            let w = g.size().x + icon_room(icon) + pad.x * 2.0;
            if x + w > row.left() + max_w + 0.5 {
                break;
            }
            let chip = Rect::from_min_size(pos2(x, row.top()), vec2(w, h));
            match bg {
                Some(bg) => {
                    f.painter.rect_filled(chip, cr(rad), bg);
                }
                None if x > row.left() + 0.5 => {
                    // Plain text parts get a dot between them.
                    f.painter.circle_filled(
                        pos2(x - gap / 2.0, chip.center().y),
                        (px * 0.09).max(1.0),
                        fade(color, 0.6),
                    );
                }
                None => {}
            }
            let mut tx = chip.left() + pad.x;
            if let Some(icon) = icon {
                let c = pos2(tx + icon_w / 2.0, chip.center().y);
                // Cut-outs show the chip (or, without one, a dark tone that reads over art).
                let cut = match bg {
                    Some(bg) if bg.a() > 200 => bg,
                    _ => fade(Color32::from_rgb(24, 26, 31), common.opacity),
                };
                match *icon {
                    ChipIcon::Gamepad => gamepad_glyph(&f.painter, c, icon_w, color, cut),
                    ChipIcon::Deck(r) => {
                        deck_glyph(&f.painter, c, px * 0.48, r, fade(Color32::from_rgb(20, 22, 26), common.opacity))
                    }
                    ChipIcon::ProtonDb([r, g, b]) => {
                        f.painter.circle_filled(c, px * 0.3, fade(Color32::from_rgb(r, g, b), common.opacity));
                    }
                    ChipIcon::Progress(_) => {}
                }
                if !matches!(icon, ChipIcon::Progress(_)) {
                    tx += icon_w + icon_gap;
                }
            }
            let mut p = pos2(tx, chip.center().y - g.size().y / 2.0);
            if let Some(ChipIcon::Progress(v)) = *icon {
                // The text, then the bar under it in the theme's accent.
                p.y = chip.center().y - (g.size().y + extra) / 2.0;
                let track =
                    Rect::from_min_size(pos2(tx, p.y + g.size().y + bar_gap), vec2(g.size().x.max(px * 3.0), bar_h));
                f.painter.rect_filled(track, bar_h / 2.0, fade(color, 0.22));
                let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
                if v > 0.0 {
                    let fill = Rect::from_min_size(track.min, vec2((track.width() * v).max(bar_h), bar_h));
                    let accent = fade(f.theme.file.palette.accent.to_egui(), common.opacity);
                    f.painter.rect_filled(fill, bar_h / 2.0, accent);
                }
            }
            paint_galley(&f.painter, p, &g, color, shadow, 0.0);
            x += w + gap;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_rating(
        &mut self,
        f: &mut Frame,
        r: Rect,
        v: f32,
        filled: Option<&str>,
        unfilled: Option<&str>,
        color: Color,
        opacity: f32,
    ) {
        let s = (r.width() / 5.0).min(r.height());
        if s <= 0.0 {
            return;
        }
        let b = f.base;
        let size = size_class(s * f.ppp);
        let tf = filled
            .and_then(|p| self.pick_image(f.theme, f.images, &b, p, &[], size))
            .and_then(|p| f.images.get(&p, size));
        let tu = unfilled
            .and_then(|p| self.pick_image(f.theme, f.images, &b, p, &[], size))
            .and_then(|p| f.images.get(&p, size));
        let fill_w = r.left() + s * 5.0 * v;
        let col = fade(color.to_egui(), opacity);
        // Rating images are multiplied by `color` (white star SVGs tinted dark). Without an
        // explicit colour the images keep their own colours.
        let img_tint = if color == Color::RATING_DEFAULT { fade(Color32::WHITE, opacity) } else { col };
        for i in 0..5 {
            let cell = Rect::from_min_size(pos2(r.left() + s * i as f32, r.top()), vec2(s, s));
            match &tu {
                Some(t) => {
                    paint_image(&f.painter, cell, t, Fit::Contain, img_tint, 0.0, 0.0);
                }
                None => star(&f.painter, cell.center(), s * 0.48, fade(col, 0.28)),
            }
            if cell.left() < fill_w {
                let clip = Rect::from_min_max(cell.min, pos2(fill_w.min(cell.right()), cell.bottom()));
                let p = f.painter.with_clip_rect(clip);
                match &tf {
                    Some(t) => {
                        paint_image(&p, cell, t, Fit::Contain, img_tint, 0.0, 0.0);
                    }
                    None => star(&p, cell.center(), s * 0.48, col),
                }
            }
        }
    }
}

/// Whether badge `slot` applies to `g`. Badges Lodestar has no data for never show.
pub(super) fn badge_active(slot: &str, g: &Game) -> bool {
    match slot {
        "favorite" => g.favorite,
        "completed" => matches!(
            g.completion(),
            Some(crate::model::CompletionStatus::Completed | crate::model::CompletionStatus::Beaten)
        ),
        "controller" => g.controller() == Some(ControllerSupport::Full),
        _ => false,
    }
}

impl Renderer {
    /// `Element::Badges`: the badges that apply to the shown game, packed from the start; a
    /// theme picture where the theme has one, else a drawn mark on a dark disc.
    pub(super) fn draw_badges(&mut self, f: &mut Frame, el: &Element) {
        let Element::Badges { common, slots, column, lines, per_line, margin, align, icons, color } = el else {
            return;
        };
        let Some(g) = f.base.game else {
            return;
        };
        let active: Vec<&str> = slots.iter().map(String::as_str).filter(|s| badge_active(s, g)).collect();
        if active.is_empty() {
            return;
        }
        let r = Self::elem_rect(f, common, Vec2::ZERO);
        let (sw, sh) = (f.sw(), f.sh());
        let (mx, my) = (margin[0] * sw, margin[1] * sh);
        let per = (*per_line).max(1) as usize;
        let lines = (*lines).max(1) as usize;
        let (cols, rows) = if *column { (lines, per) } else { (per, lines) };
        let side = ((r.width() - mx * (cols as f32 - 1.0)) / cols as f32)
            .min((r.height() - my * (rows as f32 - 1.0)) / rows as f32);
        if side < 4.0 {
            return;
        }
        let a = common.opacity;
        let tint = color.map(|c| c.to_egui()).unwrap_or(Color32::WHITE);
        let size = size_class(side * f.ppp);
        for (i, slot) in active.iter().enumerate().take(cols * rows) {
            let (cx, cy) = if *column { (i / per, i % per) } else { (i % per, i / per) };
            let x = match align {
                Align::Right => r.right() - side - cx as f32 * (side + mx),
                _ => r.left() + cx as f32 * (side + mx),
            };
            let cell = Rect::from_min_size(pos2(x, r.top() + cy as f32 * (side + my)), vec2(side, side));
            let picture = icons
                .get(*slot)
                .and_then(|p| Self::to_path(f.theme, p))
                .filter(|p| self.exists(p) && !f.images.is_failed(p, size));
            if let Some(p) = picture {
                if let Some(tex) = f.images.get(&p, size) {
                    paint_image(&f.painter, cell, &tex, Fit::Contain, fade(tint, a), 0.0, 0.0);
                }
                continue;
            }
            let c = cell.center();
            let rad = side * 0.46;
            let disc = Color32::from_rgba_unmultiplied(12, 14, 18, 200);
            f.painter.circle_filled(c, rad, fade(disc, a));
            let mark = fade(Color32::WHITE, a);
            match *slot {
                "favorite" => {
                    let accent = f.theme.file.palette.accent.to_egui();
                    star(&f.painter, c, rad * 0.62, fade(favorite_star_color(accent), a));
                }
                "completed" => {
                    let w = (side * 0.09).max(1.5);
                    let pts = [
                        c + vec2(-rad * 0.45, 0.0),
                        c + vec2(-rad * 0.12, rad * 0.34),
                        c + vec2(rad * 0.48, -rad * 0.32),
                    ];
                    f.painter.line_segment([pts[0], pts[1]], Stroke::new(w, mark));
                    f.painter.line_segment([pts[1], pts[2]], Stroke::new(w, mark));
                }
                _ => gamepad_glyph(&f.painter, c, rad * 1.3, mark, fade(disc, a)),
            }
        }
    }
}

impl Renderer {
    /// The controller hints of the screen in a theme's help bar: the bar's `buttons` only (in
    /// the screen's order), fitted to its box and the screen (`fit_hints`).
    pub(super) fn draw_help_bar(&mut self, f: &mut Frame, el: &Element) {
        let Element::HelpBar {
            common,
            style,
            icon_color,
            background,
            buttons,
            entry_spacing,
            label_spacing,
            button_scale,
            glyphs,
        } = el
        else {
            return;
        };
        let shown: Vec<Hint>;
        let hints: &[Hint] = if *glyphs {
            // A theme's own prompt: the button whatever the screen's hints are, no label.
            shown = buttons.iter().filter_map(|b| button_named(b)).map(|b| Hint::new(b, "")).collect();
            &shown
        } else if buttons.is_empty() && !style.uppercase {
            f.scene.hints
        } else {
            let wanted: Vec<Button> = buttons.iter().filter_map(|b| button_named(b)).collect();
            // The first filtered bar drawn also carries the hints that must show which no bar
            // of the view lists (the way back from the in-game Library when only A and Start
            // are listed).
            let first = !std::mem::replace(&mut f.bar_drawn, true);
            let orphan = |h: &Hint| {
                first && h.always && f.bar_buttons.as_ref().is_some_and(|listed| !listed.contains(&h.button))
            };
            shown = f
                .scene
                .hints
                .iter()
                .filter(|h| buttons.is_empty() || wanted.contains(&h.button) || orphan(h))
                .map(|h| Hint {
                    label: if style.uppercase { h.label.to_uppercase() } else { h.label.clone() },
                    ..h.clone()
                })
                .collect();
            &shown
        };
        if hints.is_empty() {
            return;
        }
        let (sw, sh) = (f.sw(), f.sh());
        let px0 = style.font_size * sh * f.ts;
        let metrics = |px: f32| HintMetrics {
            glyph_h: px * button_scale.unwrap_or(1.35),
            gap: if *glyphs { 0.0 } else { label_spacing.map_or(px * 0.45, |s| s * sw) },
            spacing: entry_spacing.map_or(px * 1.5, |s| s * sw),
        };
        let fam = f.fonts.family(f.theme, style.font.as_deref());
        let width = |k: usize, px: f32| {
            hints_width_with(&f.painter, &hints[..k], f.scene.prompts, fam.clone(), px, metrics(px))
        };
        // Room around the hints: the theme's padding, else 0.8 × the label size at each end.
        let pad = |px: f32| common.padding.map_or(vec2(px * 0.8, 0.0), |p| vec2(p[0] * sh, p[1] * sh));
        let height = |px: f32| match common.padding {
            Some(_) => metrics(px).glyph_h.max(px * 1.2) + pad(px).y * 2.0,
            None => px * 2.0,
        };
        // Auto size (0) = the hints' own width, so `origin` places the bar correctly (a
        // screen-wide natural box pushed right-anchored bars off the left edge).
        let all = width(hints.len(), px0);
        let r = Self::text_rect(f, common, vec2(all + pad(px0).x * 2.0, height(px0)));
        // Too many / too long hints for the box (or the screen): smaller, then fewer, never
        // overflowing.
        let room = (r.intersect(f.screen).width() - pad(px0).x * 2.0).max(1.0);
        let (count, px) = fit_hints(hints.len(), px0, room, width);
        let fitted = (count, px) != (hints.len(), px0);
        let hints = &hints[..count];
        let r = if common.size[0] <= 0.0 && fitted {
            // An auto-sized bar hugs what it shows, still anchored by its origin.
            Self::text_rect(f, common, vec2(width(count, px) + pad(px).x * 2.0, height(px0)))
        } else {
            r
        };
        if let Some(bg) = background {
            let rad = common.radius.map_or(0.0, |r| r * sh).min(r.height() / 2.0);
            f.painter.rect_filled(r, cr(rad), fade(bg.to_egui(), common.opacity));
        }
        let text = fade(style.color.to_egui(), common.opacity);
        let icon = icon_color.map(|c| c.to_egui()).unwrap_or_else(|| fade(style.color.to_egui(), 0.92));
        let icon_text = if luminance(icon) > 0.5 { Color32::from_rgb(20, 22, 28) } else { Color32::WHITE };
        let inner = r.intersect(f.screen).shrink2(vec2(pad(px).x, 0.0));
        paint_hints_shadow(
            &f.painter,
            inner,
            hints,
            f.scene.prompts,
            fam,
            (px, metrics(px)),
            (text, style.shadow.map(|c| fade(c.to_egui(), common.opacity))),
            fade(icon, common.opacity),
            icon_text,
            style.align,
        );
    }
}
