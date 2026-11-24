//! Centered confirmation card for the launch prompts ("Another game is running — Close & start
//! / Cancel", "Quit Lodestar? — Minimise / Quit / Cancel"): A / X / B, or a click on a button.

use super::look::Look;
use crate::theme::render::draw::{self as rd, Button, button_glyph, fade};
use crate::ui::launching::{Prompt, PromptAnswer};
use egui::{Color32, Rect, pos2, vec2};

/// Draw the card over everything; returns the button clicked.
pub(super) fn draw_prompt(ui: &egui::Ui, screen: Rect, look: &Look, ask: &Prompt) -> Option<PromptAnswer> {
    let p = ui.painter();
    p.rect_filled(screen, 0.0, Color32::from_black_alpha(170));
    let w = (look.sw * 0.42).clamp(look.h(0.62), look.sw * 0.9);
    let pad = look.h(0.04);
    let inner_w = w - 2.0 * pad;

    let title = rd::layout_line(p, &ask.title, look.title(0.036), look.text, inner_w);
    let body = p.layout(ask.body.clone(), look.font(0.023), look.text, inner_w);
    let hint = p.layout(ask.hint.clone(), look.font(0.018), look.dim, inner_w);
    let btn_h = look.h(0.058);
    let h = pad
        + title.size().y
        + look.h(0.018)
        + body.size().y
        + look.h(0.012)
        + hint.size().y
        + look.h(0.04)
        + btn_h
        + pad;
    let card = Rect::from_center_size(screen.center(), vec2(w, h));
    rd::drop_shadow(p, card, look.h(0.016), 0.9);
    p.rect_filled(card, rd::cr(look.h(0.016)), look.surface);
    let mut y = card.top() + pad;
    let x = card.left() + pad;
    p.galley(pos2(x, y), title.clone(), look.text);
    y += title.size().y + look.h(0.018);
    p.galley(pos2(x, y), body.clone(), look.text);
    y += body.size().y + look.h(0.012);
    p.galley(pos2(x, y), hint.clone(), look.dim);
    y += hint.size().y + look.h(0.04);

    // Buttons, right-aligned: [A primary] [X other] [B Cancel]; the A one is the accent pill.
    let gap = look.h(0.016);
    let mut specs = vec![(Button::A, ask.primary, look.accent, look.accent_text, PromptAnswer::Primary)];
    if let Some(alt) = ask.alt {
        specs.push((Button::X, alt, look.surface_alt, look.text, PromptAnswer::Alt));
    }
    specs.push((Button::B, ask.cancel, look.surface_alt, look.text, PromptAnswer::Cancel));
    let label_font = look.bold(0.022);
    let glyph_h = btn_h * 0.52;
    let galleys: Vec<_> =
        specs.iter().map(|(_, t, _, fg, _)| p.layout_no_wrap((*t).into(), label_font.clone(), *fg)).collect();
    let widths: Vec<f32> =
        galleys.iter().map(|g| btn_h * 0.32 + glyph_h + btn_h * 0.22 + g.size().x + btn_h * 0.42).collect();
    let mut bx = card.right() - pad - widths.iter().sum::<f32>() - gap * (widths.len() - 1) as f32;
    let pointer = ui.input(|i| i.pointer.hover_pos().map(|p| (p, i.pointer.primary_clicked())));
    let mut clicked = None;
    for (((b, _, fill, fg, answer), g), bw) in specs.iter().zip(galleys).zip(widths) {
        let r = Rect::from_min_size(pos2(bx, y), vec2(bw, btn_h));
        let hover = pointer.is_some_and(|(pos, _)| r.contains(pos));
        p.rect_filled(r, rd::cr(btn_h / 2.0), if hover { fade(*fill, 0.85) } else { *fill });
        let gc = pos2(r.left() + btn_h * 0.32 + glyph_h / 2.0, r.center().y);
        // Glyph in the label colour with the button's fill cut out, so it reads on both pills.
        button_glyph(p, gc, glyph_h, *b, look.prompts, *fg, *fill, look.bold.clone());
        p.galley(pos2(gc.x + glyph_h / 2.0 + btn_h * 0.22, r.center().y - g.size().y / 2.0), g, *fg);
        if let Some((pos, true)) = pointer
            && r.contains(pos)
        {
            clicked = Some(*answer);
        }
        bx += bw + gap;
    }
    clicked
}
