//! Small editing widgets the creator's panels share: labelled rows, percentages, colours with
//! the palette's swatches, optional values, text with a value picker.

use super::super::Pal;
use crate::theme::{Color, Palette, SLOT_ORDER, palette_slot};
use egui::{RichText, Ui, vec2};

/// Width of the label column of a property row.
pub const LABEL_W: f32 = 100.0;

/// A property row: label on the left, the control on the right.
pub fn row<R>(ui: &mut Ui, label: &str, pal: &Pal, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.add_sized(vec2(LABEL_W, 20.0), egui::Label::new(RichText::new(label).size(12.5).color(pal.dim)).truncate());
        add(ui)
    })
    .inner
}

/// A section heading inside a panel.
pub fn heading(ui: &mut Ui, text: &str, pal: &Pal) {
    ui.add_space(8.0);
    ui.label(RichText::new(text).size(13.0).strong().color(pal.text));
    ui.add_space(2.0);
}

/// A fraction shown as a percentage (of the screen).
pub fn pct(ui: &mut Ui, v: &mut f32, range: std::ops::RangeInclusive<f32>) -> egui::Response {
    let mut p = *v * 100.0;
    let r = ui.add(
        egui::DragValue::new(&mut p)
            .speed(0.1)
            .range(*range.start() * 100.0..=*range.end() * 100.0)
            .fixed_decimals(1)
            .suffix(" %"),
    );
    if r.changed() {
        *v = super::geom::tidy(p / 100.0);
    }
    r
}

/// A plain number.
pub fn num(
    ui: &mut Ui,
    v: &mut f32,
    speed: f64,
    range: std::ops::RangeInclusive<f32>,
    decimals: usize,
) -> egui::Response {
    ui.add(egui::DragValue::new(v).speed(speed).range(range).fixed_decimals(decimals))
}

/// The palette colour a colour is (same red / green / blue), for "Accent" labels.
pub fn slot_of(p: &Palette, c: Color) -> Option<&'static str> {
    SLOT_ORDER.into_iter().find(|k| palette_slot(p, k).is_some_and(|s| s.0[..3] == c.0[..3]))
}

pub fn slot_label(k: &str) -> &'static str {
    match k {
        "accent" => "Accent",
        "selection" => "Selection",
        "text" => "Text",
        "text_dim" => "Dim text",
        "accent_text" => "Text on accent",
        "favorite" => "Favourite",
        "danger" => "Warning",
        "background" => "Background",
        "surface" => "Panels",
        "surface_alt" => "Cards",
        "border" => "Borders",
        _ => "",
    }
}

/// A colour: a swatch that opens a picker with the palette's colours (picking one keeps the
/// colour's transparency), the colour itself and a hex field. Shows which palette colour it is.
pub fn color(ui: &mut Ui, c: &mut Color, p: &Palette, pal: &Pal) -> bool {
    let before = *c;
    let id = ui.next_auto_id();
    let mut rgba = c.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let r = ui.color_edit_button_srgba_unmultiplied(&mut rgba);
        if r.changed() {
            *c = Color(rgba);
        }
        // Palette swatches.
        let btn = ui.add(egui::Button::new(RichText::new("▾").size(11.0)).min_size(vec2(18.0, 18.0)));
        let popup = egui::Popup::menu(&btn).id(id.with("pal"));
        popup.show(|ui| {
            ui.label(RichText::new("Palette colours").size(12.0).color(pal.dim));
            for k in SLOT_ORDER {
                let Some(s) = palette_slot(p, k) else { continue };
                ui.horizontal(|ui| {
                    let (rect, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::click());
                    ui.painter().rect_filled(rect, 3.0, s.to_egui());
                    let l = ui.selectable_label(false, slot_label(k));
                    if resp.clicked() || l.clicked() {
                        *c = Color([s.0[0], s.0[1], s.0[2], c.0[3]]);
                        ui.close();
                    }
                });
            }
        });
        let mut hex = c.to_hex();
        let r = ui.add(egui::TextEdit::singleline(&mut hex).desired_width(76.0).font(egui::TextStyle::Monospace));
        if r.changed()
            && let Some(n) = Color::parse(&hex)
        {
            *c = n;
        }
        if let Some(k) = slot_of(p, *c)
            && ui.available_width() > 50.0
        {
            ui.add(egui::Label::new(RichText::new(slot_label(k)).size(11.5).color(pal.dim)).truncate());
        }
    });
    *c != before
}

/// An optional colour: a switch and the colour.
pub fn opt_color(ui: &mut Ui, c: &mut Option<Color>, default: Color, p: &Palette, pal: &Pal) -> bool {
    let before = *c;
    ui.horizontal(|ui| {
        let mut on = c.is_some();
        if ui.checkbox(&mut on, "").changed() {
            *c = on.then_some(default);
        }
        if let Some(v) = c {
            color(ui, v, p, pal);
        } else {
            ui.label(RichText::new("none").size(12.0).color(pal.dim));
        }
    });
    *c != before
}

/// An optional number (a switch, then the value).
pub fn opt_num(
    ui: &mut Ui,
    v: &mut Option<f32>,
    default: f32,
    percent: bool,
    range: std::ops::RangeInclusive<f32>,
    pal: &Pal,
) -> bool {
    let before = *v;
    ui.horizontal(|ui| {
        let mut on = v.is_some();
        if ui.checkbox(&mut on, "").changed() {
            *v = on.then_some(default);
        }
        match v {
            Some(x) if percent => {
                pct(ui, x, range);
            }
            Some(x) => {
                num(ui, x, 0.01, range, 2);
            }
            None => {
                ui.label(RichText::new("automatic").size(12.0).color(pal.dim));
            }
        }
    });
    *v != before
}

/// One line of text.
pub fn line(ui: &mut Ui, s: &mut String, hint: &str, w: f32) -> egui::Response {
    ui.add(egui::TextEdit::singleline(s).hint_text(hint).desired_width(w))
}

/// A choice among `options` ((value, label)).
pub fn choice<T: PartialEq + Clone>(ui: &mut Ui, id: egui::Id, v: &mut T, options: &[(T, &str)]) -> bool {
    let before = v.clone();
    let cur = options.iter().find(|(o, _)| o == v).map(|(_, l)| *l).unwrap_or("Other");
    egui::ComboBox::from_id_salt(id).selected_text(cur).show_ui(ui, |ui| {
        for (o, l) in options {
            ui.selectable_value(v, o.clone(), *l);
        }
    });
    *v != before
}

/// Two numbers side by side ([x, y]), as percentages.
pub fn pct2(ui: &mut Ui, v: &mut [f32; 2], range: std::ops::RangeInclusive<f32>) -> bool {
    let before = *v;
    pct(ui, &mut v[0], range.clone());
    pct(ui, &mut v[1], range);
    *v != before
}

/// A small dim note.
pub fn note(ui: &mut Ui, text: &str, pal: &Pal) {
    ui.label(RichText::new(text).size(11.5).color(pal.dim));
}

/// The user pressed, released or typed something this frame. Edits from the panels' widgets
/// are taken only then: a widget that rounds or clamps a value on its own (a slider showing
/// whole pixels) never changes a theme nobody touched.
pub fn user_acted(ctx: &egui::Context) -> bool {
    ctx.input(|i| {
        i.pointer.any_down()
            || i.pointer.any_released()
            || i.events
                .iter()
                .any(|e| matches!(e, egui::Event::Key { .. } | egui::Event::Text(_) | egui::Event::Paste(_)))
    })
}
