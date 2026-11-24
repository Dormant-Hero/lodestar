//! The artwork editor's gallery: slot tabs, source chips, the candidate grid and the preview.

use super::*;

/// One slot tab: label with an underline when active and a dot when the slot has an image.
pub(super) fn slot_tab(ui: &mut Ui, label: &str, active: bool, has: bool, pal: &Pal) -> egui::Response {
    let font = egui::FontId::proportional(14.0);
    let fg = if active { pal.text } else { pal.dim };
    let gal = ui.painter().layout_no_wrap(label.to_string(), font, fg);
    let w = gal.size().x + 34.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 34.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() && !active {
        p.rect_filled(rect.shrink2(vec2(0.0, 3.0)), 5.0, pal.hover);
    }
    let dot_c = pos2(rect.left() + 13.0, rect.center().y);
    if has {
        p.circle_filled(dot_c, 3.2, if active { pal.accent } else { mix(pal.dim, pal.bg, 0.2) });
    } else {
        p.circle_stroke(dot_c, 3.0, Stroke::new(1.0, with_alpha(pal.dim, 160)));
    }
    p.galley(pos2(rect.left() + 22.0, rect.center().y - gal.size().y / 2.0), gal, fg);
    if active {
        p.rect_filled(
            Rect::from_min_max(pos2(rect.left() + 6.0, rect.bottom() - 2.5), pos2(rect.right() - 6.0, rect.bottom())),
            1.5,
            pal.accent,
        );
    }
    let tip = if has { "Has an image" } else { "No image yet" };
    resp.on_hover_text(tip)
}

/// Source filter chips ("All", then each source that can have this slot, with its result
/// count) and one line about sources that need a key. True when "Set up…" was clicked.
#[allow(clippy::too_many_arguments)]
pub(super) fn source_chips(
    ui: &mut Ui,
    sources: &[SourceInfo],
    search: Option<&SearchState>,
    slot: ArtSlot,
    console: bool,
    filter: &mut Option<String>,
    pal: &Pal,
) -> bool {
    let mut open = false;
    let usable: Vec<&SourceInfo> = sources
        .iter()
        .filter(|s| s.available && s.kind.has_slot(slot))
        .filter(|s| console || s.kind != crate::art::SourceKind::Libretro)
        .collect();
    let count = |name: Option<&str>| -> usize {
        search.map_or(0, |s| s.items.iter().filter(|c| name.is_none_or(|n| c.source == n)).count())
    };
    // A filter for a source that is gone (switched off meanwhile) shows everything.
    if filter.as_ref().is_some_and(|f| !usable.iter().any(|s| s.name == *f)) {
        *filter = None;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        if usable.is_empty() {
            ui.label(RichText::new("No online source has this kind of image.").size(12.0).color(pal.dim));
        }
        // Counts once there are results (a 0 only when the search is over).
        let done = search.is_some_and(|s| s.done);
        let chip = |ui: &mut Ui, label: &str, n: Option<usize>, on: bool, tip: &str| {
            let text = match n {
                Some(n) if n > 0 || done => format!("{label}  {n}"),
                _ => label.to_string(),
            };
            let empty = done && n == Some(0);
            let fg = if on {
                pal.accent_text
            } else if empty {
                mix(pal.dim, pal.bg, 0.3)
            } else {
                pal.text
            };
            let b = egui::Button::new(RichText::new(text).size(12.0).color(fg))
                .fill(if on { pal.accent } else { mix(pal.surface, pal.bg, 0.3) })
                .corner_radius(11)
                .min_size(vec2(0.0, 22.0));
            ui.add(b).on_hover_text(tip).clicked()
        };
        if chip(ui, "All", Some(count(None)), filter.is_none(), "Results from every source") {
            *filter = None;
        }
        for s in &usable {
            let on = filter.as_deref() == Some(s.name.as_str());
            if chip(ui, &s.name, Some(count(Some(&s.name))), on, s.kind.blurb()) {
                *filter = if on { None } else { Some(s.name.clone()) };
            }
        }
        // Switched on in Settings → Metadata but missing the user's key: shown (dimmed, with a
        // key mark) instead of silently left out, so it's clear why they bring nothing.
        for s in sources.iter().filter(|s| {
            s.enabled
                && !s.available
                && s.kind.keyed()
                && s.kind.has_slot(slot)
                && (console || s.kind != crate::art::SourceKind::Libretro)
        }) {
            let refused = s.reason.as_deref().is_some_and(|r| r.contains("rejected"));
            let (label, tip) = if refused {
                (format!("{}  ⚠ key refused", s.name), format!("{} refused the key — click to check it", s.name))
            } else {
                (format!("{}  · needs a key", s.name), format!("{} needs your own key — click to set it up", s.name))
            };
            let b = egui::Button::new(RichText::new(label).size(12.0).color(mix(pal.dim, pal.bg, 0.15)))
                .fill(egui::Color32::TRANSPARENT)
                .stroke(egui::Stroke::new(1.0, mix(pal.dim, pal.bg, 0.55)))
                .corner_radius(11)
                .min_size(vec2(0.0, 22.0));
            if ui.add(b).on_hover_text(tip).clicked() {
                open = true;
            }
        }
    });
    open
}

/// Empty gallery: a line of text and an optional big button; true when clicked.
pub(super) fn gallery_empty(ui: &mut Ui, pal: &Pal, text: &str, button: Option<&str>) -> bool {
    let mut clicked = false;
    let h = ui.available_height();
    ui.add_space((h * 0.5 - 48.0).max(8.0));
    ui.vertical_centered(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
        paint_icon(ui.painter(), r, Icon::Image, with_alpha(pal.dim, 170));
        ui.add_space(6.0);
        ui.label(RichText::new(text).color(pal.dim));
        if let Some(b) = button {
            ui.add_space(8.0);
            clicked = ui
                .add(
                    egui::Button::new(RichText::new(b).strong().color(pal.accent_text))
                        .fill(pal.accent)
                        .min_size(vec2(120.0, 32.0)),
                )
                .clicked();
        }
    });
    clicked
}

pub(super) enum Thumb {
    Ready(egui::TextureHandle),
    Failed,
    Loading,
}

/// Thumbnail grid of candidates (only visible rows are painted). Returns the URL clicked.
#[allow(clippy::too_many_arguments)]
pub(super) fn gallery(
    ui: &mut Ui,
    images: &mut ImageCache,
    art_state: &crate::art_app::ArtState,
    items: &[&Candidate],
    slot: ArtSlot,
    picked: Option<&str>,
    applying: bool,
    pal: &Pal,
) -> Option<String> {
    const GAP: f32 = 12.0;
    const LABEL_H: f32 = 36.0;
    let mut clicked = None;
    let n = items.len();
    egui::ScrollArea::vertical().id_salt(("ls_art_gallery", slot)).auto_shrink([false, false]).show_viewport(
        ui,
        |ui, vp| {
            let w = ui.available_width();
            let min_w = cell_min_w(slot);
            let cols = (((w + GAP) / (min_w + GAP)).floor() as usize).max(1);
            let cw = (w - GAP * (cols - 1) as f32) / cols as f32;
            let ih = (cw / slot_aspect(slot)).min(cw * 1.6);
            let row_h = ih + LABEL_H + GAP;
            let rows = n.div_ceil(cols);
            let (content, _) = ui.allocate_exact_size(vec2(w, rows as f32 * row_h - GAP), Sense::hover());
            let first = (vp.min.y / row_h).floor().max(0.0) as usize;
            let last = ((vp.max.y / row_h).ceil() as usize + 1).min(rows);
            for r in first.min(last)..last {
                for c in 0..cols {
                    let i = r * cols + c;
                    let Some(&cand) = items.get(i) else { break };
                    let cell = Rect::from_min_size(
                        content.min + vec2(c as f32 * (cw + GAP), r as f32 * row_h),
                        vec2(cw, ih + LABEL_H),
                    );
                    let img = Rect::from_min_size(cell.min, vec2(cw, ih));
                    let id = ui.scope_id().with(("ls_art_cand", i));
                    let resp = ui.interact(cell, id, Sense::click());
                    let hov = resp.hovered();
                    let is_picked = picked == Some(cand.url.as_str());
                    let p = ui.painter();
                    // image
                    let key = thumb_key(cand);
                    let tex = match art_state.thumbs.get(key) {
                        Some(path) => match images.get(path, ImgSize::Thumb) {
                            Some(t) => Thumb::Ready(t),
                            None if images.is_failed(path, ImgSize::Thumb) => Thumb::Failed,
                            None => Thumb::Loading,
                        },
                        None if art_state.thumbs_failed.contains(key) => Thumb::Failed,
                        None => Thumb::Loading,
                    };
                    let bg = mix(pal.surface_alt, pal.bg, 0.35);
                    match slot {
                        ArtSlot::Logo | ArtSlot::Icon => {
                            // Dark, like the backgrounds logos are shown on.
                            p.rect_filled(img, 6.0, mix(bg, Color32::BLACK, 0.4));
                        }
                        _ => {
                            p.rect_filled(img, 6.0, bg);
                        }
                    }
                    match tex {
                        Thumb::Ready(t) => match slot {
                            ArtSlot::Logo | ArtSlot::Icon => {
                                paint_contain(p, img.shrink(8.0), &t, Color32::WHITE, vec2(0.5, 0.5));
                            }
                            _ => paint_cover(p, img, &t, 6, Color32::WHITE, vec2(0.5, 0.5)),
                        },
                        Thumb::Failed => {
                            p.text(
                                img.center(),
                                egui::Align2::CENTER_CENTER,
                                "Can't show preview",
                                egui::FontId::proportional(11.5),
                                pal.dim,
                            );
                        }
                        Thumb::Loading => {
                            let ir = Rect::from_center_size(img.center(), vec2(22.0, 22.0));
                            paint_icon(p, ir, Icon::Image, with_alpha(pal.dim, 90));
                        }
                    }
                    if hov {
                        p.rect_filled(img, 6.0, Color32::from_white_alpha(14));
                    }
                    let border = if is_picked {
                        Some(Stroke::new(2.5, pal.selection))
                    } else if hov {
                        Some(Stroke::new(2.0, with_alpha(pal.accent, 220)))
                    } else {
                        None
                    };
                    if let Some(b) = border {
                        p.rect_stroke(img, 6.0, b, StrokeKind::Outside);
                    }
                    if is_picked {
                        let c = pos2(img.right() - 14.0, img.top() + 14.0);
                        p.circle_filled(c, 11.0, Color32::from_black_alpha(170));
                        if applying {
                            let t = ui.input(|i| i.time) as f32;
                            for k in 0..8 {
                                let a = k as f32 / 8.0 * std::f32::consts::TAU;
                                let head = ((t * 8.0).floor() as i32).rem_euclid(8);
                                let age = (head - k).rem_euclid(8) as u8;
                                let alpha = 255u8.saturating_sub(age * 28);
                                p.circle_filled(c + vec2(a.cos(), a.sin()) * 6.0, 1.4, with_alpha(pal.accent, alpha));
                            }
                            ui.ctx().request_repaint_after(Duration::from_millis(125));
                        } else {
                            paint_icon(p, Rect::from_center_size(c, vec2(14.0, 14.0)), Icon::Check, pal.accent);
                        }
                    }
                    // labels
                    let fg = if hov || is_picked { pal.text } else { mix(pal.text, pal.dim, 0.3) };
                    let l1 = truncated(p, &cand.label, egui::FontId::proportional(12.0), fg, cw);
                    p.galley(pos2(cell.left(), img.bottom() + 4.0), l1, fg);
                    let size = size_text(cand.width, cand.height);
                    let line2 = if cand.label.starts_with(&cand.source) {
                        size
                    } else if size.is_empty() {
                        cand.source.clone()
                    } else {
                        format!("{} · {size}", cand.source)
                    };
                    let l2 = truncated(p, &line2, egui::FontId::proportional(11.0), pal.dim, cw);
                    p.galley(pos2(cell.left(), img.bottom() + 20.0), l2, pal.dim);
                    if resp.clicked() {
                        clicked = Some(cand.url.clone());
                    }
                    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                    if hov {
                        resp.on_hover_text(format!("{}\n{}\nClick to use this image", cand.label, line_size(cand)));
                    }
                }
            }
        },
    );
    clicked
}

pub(super) fn line_size(c: &Candidate) -> String {
    match size_text(c.width, c.height) {
        s if s.is_empty() => format!("{} · size unknown", c.source),
        s => format!("{} · {s}", c.source),
    }
}

/// The current image of `slot` in a fixed 300×200 box, at the slot's aspect.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_preview(
    p: &egui::Painter,
    images: &mut ImageCache,
    slot: ArtSlot,
    current: Option<&Path>,
    hero: Option<&Path>,
    bounds: Rect,
    pal: &Pal,
    name: &str,
    crop: Option<BackgroundPlacement>,
) -> Option<(Rect, egui::Vec2)> {
    let aspect = slot_aspect(slot);
    let mut shown = None;
    let target = if slot == ArtSlot::Icon {
        Rect::from_center_size(bounds.center(), vec2(128.0, 128.0))
    } else {
        contain_rect(vec2(aspect, 1.0) * 100.0, bounds)
    };
    let p = p.with_clip_rect(bounds.expand(4.0));
    let base = mix(pal.surface_alt, pal.bg, 0.3);
    let tex = current.and_then(|c| images.get(c, ImgSize::Medium));
    match slot {
        ArtSlot::Logo => {
            // Over the game's background (dimmed), the way the details show it.
            let bg_tex = hero.and_then(|h| images.get(h, ImgSize::Medium));
            match bg_tex {
                Some(t) => {
                    let uv = background_uv(crop, t.size_vec2(), target.size(), vec2(0.5, 0.3));
                    p.add(egui::epaint::RectShape::filled(target, 8, Color32::from_gray(110)).with_texture(t.id(), uv));
                }
                None => vgradient(&p, target, mix(base, Color32::BLACK, 0.2), mix(base, Color32::BLACK, 0.5)),
            }
        }
        ArtSlot::Icon => {
            p.rect_filled(target.expand(14.0), 10.0, mix(base, Color32::BLACK, 0.25));
        }
        _ => {}
    }
    match (&tex, current) {
        (Some(t), _) => match slot {
            ArtSlot::Logo => {
                paint_contain(&p, target.shrink(14.0), t, Color32::WHITE, vec2(0.5, 0.5));
            }
            ArtSlot::Icon => {
                paint_contain(&p, target, t, Color32::WHITE, vec2(0.5, 0.5));
            }
            _ => {
                crate::ui::widgets::soft_shadow(&p, target, 8, 14, 4, 110);
                if slot == ArtSlot::Hero {
                    // The whole picture, the part the game page shows framed (Arrange mode's
                    // crop; the frame can be dragged here too).
                    let drawn = contain_rect(t.size_vec2(), bounds);
                    paint_contain(&p, bounds, t, Color32::WHITE, vec2(0.5, 0.5));
                    let win = crop_window(drawn, t.size_vec2(), crop);
                    let dim = Color32::from_black_alpha(150);
                    for r in [
                        Rect::from_min_max(drawn.min, pos2(drawn.right(), win.top())),
                        Rect::from_min_max(pos2(drawn.left(), win.bottom()), drawn.max),
                        Rect::from_min_max(pos2(drawn.left(), win.top()), pos2(win.left(), win.bottom())),
                        Rect::from_min_max(pos2(win.right(), win.top()), pos2(drawn.right(), win.bottom())),
                    ] {
                        if r.is_positive() {
                            p.rect_filled(r, 0.0, dim);
                        }
                    }
                    p.rect_stroke(win, 2.0, Stroke::new(1.5, Color32::WHITE), StrokeKind::Inside);
                    shown = Some((drawn, t.size_vec2()));
                } else {
                    paint_cover(&p, target, t, 8, Color32::WHITE, vec2(0.5, 0.5));
                }
            }
        },
        (None, Some(c)) if !images.is_failed(c, ImgSize::Medium) => {
            if !matches!(slot, ArtSlot::Logo | ArtSlot::Icon) {
                p.rect_filled(target, 8.0, base);
            }
        }
        (None, _) => {
            if !matches!(slot, ArtSlot::Logo | ArtSlot::Icon) {
                p.rect_filled(target, 8.0, with_alpha(base, 160));
            }
            dashed_rect(&p, target.shrink(1.0), with_alpha(pal.dim, 120));
            let missing = current.is_some();
            let ir = Rect::from_center_size(target.center() - vec2(0.0, 12.0), vec2(26.0, 26.0));
            paint_icon(&p, ir, Icon::Image, with_alpha(pal.dim, 150));
            let text = if missing { "Can't load the image" } else { "No image" };
            p.text(
                target.center() + vec2(0.0, 14.0),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(12.5),
                pal.dim,
            );
            if slot == ArtSlot::Logo && !missing {
                // What shows instead: the name.
                let g = truncated(
                    &p,
                    name,
                    egui::FontId::proportional(13.0),
                    with_alpha(pal.text, 120),
                    target.width() - 20.0,
                );
                p.galley(pos2(target.center().x - g.size().x / 2.0, target.bottom() - 22.0), g, pal.text);
            }
        }
    }
    shown
}

/// Shape of the game page's header the Background preview frames (a typical Details view).
pub(super) const HEADER_SHAPE: egui::Vec2 = egui::Vec2::new(1000.0, 400.0);

/// Where the game page's crop lies on the picture drawn at `drawn`.
pub(super) fn crop_window(drawn: Rect, tex: egui::Vec2, crop: Option<BackgroundPlacement>) -> Rect {
    let uv = background_uv(crop, tex, HEADER_SHAPE, vec2(0.5, 0.3));
    Rect::from_min_size(drawn.min + uv.min.to_vec2() * drawn.size(), uv.size() * drawn.size())
}

pub(super) fn dashed_rect(p: &egui::Painter, r: Rect, color: Color32) {
    let st = Stroke::new(1.0, color);
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    for w in pts.windows(2) {
        p.extend(egui::Shape::dashed_line(&[w[0], w[1]], st, 5.0, 4.0));
    }
}
