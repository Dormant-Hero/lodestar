//! Drawing one game or collection item of a list, with its caption and label.

use super::*;

impl Renderer {
    /// Draw one carousel/grid item into `rect`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_item<'b>(
        &mut self,
        f: &mut Frame<'_, 'b>,
        rect: Rect,
        b: &Binder<'b>,
        spec: &ItemSpec,
        alpha: f32,
        focus: f32,
        angle: f32,
    ) {
        let path = self.pick_image_adjusted(f.theme, f.images, b, spec.image, spec.fallbacks, spec.size, spec.adjust);
        let tex = path.as_deref().and_then(|p| f.images.get_adjusted(p, spec.size, spec.adjust));
        let loading = path.as_deref().is_some_and(|p| f.images.is_loading_adjusted(p, spec.size, spec.adjust));
        let sh = f.sh();
        if spec.radius > 0.0 && (tex.is_some() || spec.fill.is_some()) && angle == 0.0 {
            drop_shadow(&f.painter, rect, spec.radius, alpha * (0.45 + 0.4 * focus));
        }
        // The focus frame's corners (`selected_radius`, else the item's own).
        let sel_radius = spec.look.radius.unwrap_or(spec.radius);
        match (spec.look.glow, spec.border, focus > 0.01) {
            (Some((c, reach)), _, true) => {
                // Like WPF's DropShadowEffect on the focus border: the blurred frame reaches
                // `reach` past the border's outer edge.
                if c.a() > 0 && reach >= 0.5 && angle == 0.0 {
                    let bw = spec.border.map_or(0.0, |_| spec.look.border_width.unwrap_or((sh * 0.004).max(2.0)));
                    glow(
                        &f.painter,
                        rect.expand(bw),
                        sel_radius + bw,
                        fade(c, focus * alpha),
                        (reach * 2.0).min(255.0),
                    );
                }
            }
            (None, Some(c), true) => {
                glow(&f.painter, rect, sel_radius, fade(c, 0.45 * focus * alpha), (sh * 0.03).clamp(8.0, 40.0))
            }
            _ => {}
        }
        // Not installed: the cover is dimmed (Settings → Visuals), or the theme's scrim laid
        // over it (`uninstalled_scrim`).
        let dim = self.opts.dim_not_installed && b.game.is_some_and(|g| !g.installed);
        let scrim = spec.look.scrim.filter(|_| dim && angle == 0.0);
        let white = if dim && scrim.is_none() { Color32::from_gray(120) } else { Color32::WHITE };
        let white = match spec.tint {
            Some(t) => mul_color(white, t),
            None => white,
        };
        let drawn = match &tex {
            Some(t) => {
                let r = paint_image(&f.painter, rect, t, spec.fit, fade(white, alpha), spec.radius, angle);
                if let (Some(cap), 0.0, true) = (spec.caption, angle, self.opts.show_titles) {
                    self.item_caption(f, r, b, spec, cap, alpha);
                }
                r
            }
            None => {
                let fill = spec.fill.unwrap_or_else(|| f.theme.file.palette.surface_alt.to_egui());
                let top = lerp_color(fill, Color32::WHITE, 0.06);
                if fill.a() == 0 {
                    // Transparent card: text only.
                } else if angle == 0.0 {
                    gradient_rect(&f.painter, rect, spec.radius, [fade(top, alpha), fade(fill, alpha)], false);
                } else {
                    let shape = egui::epaint::RectShape::filled(rect, cr(spec.radius), fade(fill, alpha))
                        .with_angle(angle.to_radians());
                    f.painter.add(shape);
                }
                if !loading
                    && spec.labelled
                    && angle == 0.0
                    && let Some(g) = b.game
                {
                    // The name is under the card already: the console's outline, not the
                    // name twice.
                    let side = (rect.width().min(rect.height()) * 0.42).max(8.0);
                    crate::ui::platform_icons::paint(
                        &f.painter,
                        Rect::from_center_size(rect.center(), vec2(side, side)),
                        crate::ui::platform_icons::game_shape(g),
                        fade(spec.style.color.to_egui(), alpha * 0.35),
                    );
                } else if !loading {
                    // Consoles / the user's collections without art: their device icon.
                    let label_rect = crate::ui::platform_icons::theme_card_icon(
                        &f.painter,
                        rect,
                        b.collection.filter(|_| b.game.is_none()).map(|c| &c.collection),
                        fade(spec.style.color.to_egui(), alpha * 0.85),
                        angle,
                    );
                    self.item_label(f, label_rect, b, spec, alpha, angle);
                }
                rect
            }
        };
        if let Some(c) = scrim
            && c.a() > 0
        {
            f.painter.rect_filled(drawn, cr(spec.radius), fade(c, alpha));
        }
        if let (Some(c), true) = (spec.border, focus > 0.01) {
            let w = spec.look.border_width.map_or((sh * 0.004).max(2.0), |w| w.max(1.0 / f.ppp));
            if angle == 0.0 && w > 0.0 {
                f.painter.rect_stroke(
                    drawn,
                    cr(sel_radius),
                    Stroke::new(w, fade(c, focus * alpha)),
                    StrokeKind::Outside,
                );
            }
        }
        let h = (drawn.width() * 0.15).clamp(sh * 0.018, sh * 0.036);
        let inset = h * 0.34;
        // Favourites: a ★ disc in the top-right corner (a bit larger than the gamepad pill's
        // height, so the star reads at a glance).
        let mut fav_bottom = None;
        if spec.fav_badge && angle == 0.0 && b.game.is_some_and(|g| g.favorite) {
            let accent = f.theme.file.palette.accent.to_egui();
            let r = favorite_badge(&f.painter, drawn, h * 1.25, accent, alpha);
            fav_bottom = Some(r.bottom());
        }
        if spec.badge && angle == 0.0 && b.game.is_some_and(|g| g.controller() == Some(ControllerSupport::Full)) {
            // Bottom-right like a sticker; top-right when a caption runs along the bottom
            // (under the favourite star when there is one).
            let corner = if spec.caption.is_some() && tex.is_some() {
                let top = fav_bottom.map_or(drawn.top() + inset, |y| y + inset * 0.6);
                pos2(drawn.right() - inset, top + h)
            } else {
                drawn.right_bottom() - vec2(inset, inset)
            };
            controller_badge(&f.painter, corner, h, alpha);
        }
        if !spec.templates.is_empty() && angle == 0.0 {
            self.draw_item_template(f, drawn, b, spec, alpha, focus);
        }
    }

    /// Caption over the bottom of an image item, on a scrim.
    pub(super) fn item_caption(
        &mut self,
        f: &mut Frame,
        rect: Rect,
        b: &Binder,
        spec: &ItemSpec,
        cap: &str,
        alpha: f32,
    ) {
        let text = b.resolve(cap, false);
        if text.trim().is_empty() {
            return;
        }
        let sh = f.sh();
        let scale = (rect.height() / spec.base_h.max(1.0)).clamp(0.3, 3.0);
        let px = (spec.style.font_size * sh * f.ts * scale * 0.8).max(6.0);
        let fam = f.fonts.family(f.theme, spec.style.font.as_deref());
        let pad = vec2(px * 0.8, px * 0.6);
        let mut style = spec.style.clone();
        style.align = Align::Left;
        let color = fade(spec.style.color.to_egui(), alpha);
        let g =
            layout(&f.painter, text.trim(), fam.clone(), px, color, &style, Some(rect.width() - pad.x * 2.0), Some(1));
        let sub = spec.subtext.map(|s| b.resolve(s, false)).filter(|s| !s.trim().is_empty());
        let sub_g = sub.map(|s| {
            let st2 = TextStyle { uppercase: false, letter_spacing: 0.0, ..style.clone() };
            layout(
                &f.painter,
                s.trim(),
                fam,
                px * 0.62,
                fade(color, 0.75),
                &st2,
                Some(rect.width() - pad.x * 2.0),
                Some(1),
            )
        });
        let block = g.size().y + sub_g.as_ref().map(|s| s.size().y + px * 0.15).unwrap_or(0.0);
        let shadow = match spec.look.caption_bg {
            // The theme's own strip behind the text (Playnite's title strip).
            Some(bg) => {
                let h = (block + pad.y * 2.0).min(rect.height());
                let strip = Rect::from_min_max(pos2(rect.left(), rect.bottom() - h), rect.max);
                let r = spec.radius.min(h / 2.0);
                match bg {
                    Brush::Solid(c) => {
                        let rr = egui::CornerRadius { nw: 0, ne: 0, sw: cr(r).sw, se: cr(r).se };
                        f.painter.rect_filled(strip, rr, fade(c.to_egui(), alpha));
                    }
                    Brush::Gradient(g) => {
                        self.gradients.paint(&f.painter, strip, [0.0, 0.0, r, r], g, alpha);
                    }
                }
                spec.style.shadow.map(|c| fade(c.to_egui(), alpha))
            }
            None => {
                let scrim_h = (block + pad.y * 2.0) * 1.9;
                let scrim = Rect::from_min_max(pos2(rect.left(), rect.bottom() - scrim_h.min(rect.height())), rect.max);
                // Rounded like the image; the top corners are transparent anyway.
                gradient_rect(
                    &f.painter,
                    scrim,
                    spec.radius,
                    [Color32::TRANSPARENT, fade(Color32::from_black_alpha(210), alpha)],
                    false,
                );
                Some(fade(Color32::from_black_alpha(160), alpha))
            }
        };
        let mut y = rect.bottom() - pad.y - block;
        paint_galley(&f.painter, pos2(rect.left() + pad.x, y), &g, color, shadow, 0.0);
        y += g.size().y + px * 0.15;
        if let Some(sg) = sub_g {
            paint_galley(&f.painter, pos2(rect.left() + pad.x, y), &sg, fade(color, 0.75), None, 0.0);
        }
    }

    /// Text-styled card content: name centred, optional subtext below.
    pub(super) fn item_label(
        &mut self,
        f: &mut Frame,
        rect: Rect,
        b: &Binder,
        spec: &ItemSpec,
        alpha: f32,
        angle: f32,
    ) {
        let text = b.resolve(spec.text, false);
        if text.trim().is_empty() {
            return;
        }
        let sh = f.sh();
        let scale = (rect.height() / spec.base_h.max(1.0)).clamp(0.3, 3.0);
        let mut px = (spec.style.font_size * sh * f.ts * scale).max(6.0);
        let fam = f.fonts.family(f.theme, spec.style.font.as_deref());
        let color = fade(spec.style.color.to_egui(), alpha);
        let pad = rect.width() * 0.08;
        let inner = rect.shrink(pad);
        let mut style = spec.style.clone();
        style.align = Align::Center;
        // Fit the label inside the item: as many rows as fit (max 3), shrinking the font when
        // that makes the whole label fit (a big placeholder font in a short card) — but never
        // below a size readable from the couch; a label too long even then keeps its full size
        // and ends in "…".
        let room = (inner.height() - if spec.subtext.is_some() { px * 0.9 } else { 0.0 }).max(inner.height() * 0.5);
        let base_px = px;
        let min_px = (base_px * 0.4).max(sh * 0.018).min(base_px);
        let lay = |px: f32| {
            let rows = ((room / line_height(px, style.line_spacing)).floor() as u32).clamp(1, 3);
            layout(&f.painter, text.trim(), fam.clone(), px, color, &style, Some(inner.width()), Some(rows))
        };
        // Rows may only break between words: the font must be small enough for the longest
        // word to fit the width ("Continue" must not become "Continu" / "e"). Text width scales
        // with the font size, so one measurement at the base size gives that limit.
        let longest = text
            .split_whitespace()
            .map(|w| layout(&f.painter, w, fam.clone(), base_px, color, &style, None, None).size().x)
            .fold(0.0f32, f32::max);
        let word_px = word_fit_px(base_px, longest, inner.width());
        let fits = |g: &egui::Galley, px: f32| !g.elided && g.size().y <= room + 0.5 && px <= word_px;
        let mut g = lay(px);
        if !fits(&g, px) {
            let mut found = false;
            while px > min_px {
                px = (px * 0.85).max(min_px);
                if px > word_px {
                    continue;
                }
                g = lay(px);
                if fits(&g, px) {
                    found = true;
                    break;
                }
            }
            if !found {
                // Too long even at the smallest size: the largest size that keeps words whole
                // (ending in "…"). One long word in a narrow card may go below the readable
                // minimum to stay whole; one too long even then is cut with "…" — never broken
                // in the middle onto the next row ("Brawlh" / "alla").
                let floor_px = (sh * 0.013).max(6.0);
                let (wpx, cut) = whole_word_px(base_px, min_px, floor_px, longest, inner.width());
                px = wpx;
                g = if cut {
                    let short = elide_words(text.trim(), inner.width(), |w| {
                        layout(&f.painter, w, fam.clone(), px, color, &style, None, None).size().x
                    });
                    let rows = ((room / line_height(px, style.line_spacing)).floor() as u32).clamp(1, 3);
                    layout(&f.painter, &short, fam.clone(), px, color, &style, Some(inner.width()), Some(rows))
                } else {
                    lay(px)
                };
            }
        }
        let sub = spec.subtext.map(|s| b.resolve(s, false)).filter(|s| !s.trim().is_empty());
        let sub_g = sub.map(|s| {
            let mut st2 = style.clone();
            st2.uppercase = false;
            st2.letter_spacing = 0.0;
            layout(&f.painter, s.trim(), fam, px * 0.55, fade(color, 0.7), &st2, Some(inner.width()), Some(1))
        });
        let gap = px * 0.35;
        let total = g.size().y + sub_g.as_ref().map(|s| s.size().y + gap).unwrap_or(0.0);
        let y0 = inner.center().y - total / 2.0;
        let pos = pos2(inner.center().x, y0);
        let shadow = spec.style.shadow.map(|c| fade(c.to_egui(), alpha));
        let pivot = rect.center();
        let rot = |p: Pos2| {
            if angle != 0.0 { rotate_around(p, pivot, angle) } else { p }
        };
        paint_galley(&f.painter, rot(pos), &g, color, shadow, angle);
        if let Some(sg) = sub_g {
            paint_galley(
                &f.painter,
                rot(pos2(inner.center().x, y0 + g.size().y + gap)),
                &sg,
                fade(color, 0.7),
                None,
                angle,
            );
        }
    }
}

impl Renderer {
    /// A grid item with its tile (`GridTiles`): the selector under, between or over the
    /// background and the item, the picture at its size in the box. `sel_spec`: how the
    /// focused item is drawn.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_tile<'b>(
        &mut self,
        f: &mut Frame<'_, 'b>,
        rect: Rect,
        b: &Binder<'b>,
        spec: &ItemSpec,
        tiles: Option<&GridTiles>,
        alpha: f32,
        focus: f32,
    ) {
        let Some(t) = tiles else {
            self.draw_item(f, rect, b, spec, alpha, focus, 0.0);
            return;
        };
        let radius = t.corner_radius * f.sh();
        let selector = (focus > 0.01 && (t.selector_image.is_some() || t.selector_color.is_some())).then_some((
            t.selector_image.as_deref(),
            t.selector_color,
            t.selector_scale,
            alpha * focus,
        ));
        let layer = |l: TileLayer| selector.filter(|_| t.selector_layer == l);
        if let Some((img, c, s, a)) = layer(TileLayer::Bottom) {
            self.tile_layer(f, rect, img, c, s, radius, a);
        }
        self.tile_layer(f, rect, t.background_image.as_deref(), t.background_color, t.background_scale, radius, alpha);
        if let Some((img, c, s, a)) = layer(TileLayer::Middle) {
            self.tile_layer(f, rect, img, c, s, radius, a);
        }
        let inner = Rect::from_center_size(rect.center(), rect.size() * t.image_scale.clamp(0.05, 4.0));
        self.draw_item(f, inner, b, spec, alpha, focus, 0.0);
        if let Some((img, c, s, a)) = layer(TileLayer::Top) {
            self.tile_layer(f, rect, img, c, s, radius, a);
        }
    }

    /// One tile layer: `image` stretched over the box (`scale` × `rect`) tinted by `color`, or
    /// without an image a box in `color`.
    #[allow(clippy::too_many_arguments)]
    fn tile_layer(
        &mut self,
        f: &mut Frame,
        rect: Rect,
        image: Option<&str>,
        color: Option<Color>,
        scale: f32,
        radius: f32,
        alpha: f32,
    ) {
        let r = Rect::from_center_size(rect.center(), rect.size() * scale.clamp(0.05, 4.0));
        match (image, color) {
            (Some(img), c) => {
                let size = size_class(r.width().max(r.height()) * f.ppp);
                let b = f.base;
                if let Some(p) = self.pick_image(f.theme, f.images, &b, img, &[], size)
                    && let Some(tex) = f.images.get(&p, size)
                {
                    let tint = c.map_or(Color32::WHITE, |c| c.to_egui());
                    paint_image(&f.painter, r, &tex, Fit::Stretch, fade(tint, alpha), radius, 0.0);
                }
            }
            (None, Some(c)) => {
                f.painter.rect_filled(r, cr(radius), fade(c.to_egui(), alpha));
            }
            (None, None) => {}
        }
    }
}
