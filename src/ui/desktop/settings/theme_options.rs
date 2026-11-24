//! Settings → Appearance → Theme options: the options an imported Playnite theme offers
//! (its thememodifier.yaml) for the desktop theme and the controller theme. Switches, choices,
//! sliders, colours and texts; a change shows at once and the theme is rebuilt in the
//! background (ui/theme_options.rs).

use super::Pal;
use super::hint_text;
use crate::app::AppState;
use crate::theme::playnite::options::{Kind, OptionsSpec, ThemeOption, number_text, parse_color};
use crate::ui::theme_options as jobs;
use egui::{RichText, Ui};

/// The section for the desktop theme and, when it is another one, the controller theme.
pub(super) fn show(ui: &mut Ui, st: &mut AppState, pal: &Pal) {
    let mut keys = vec![(st.config.desktop_theme.clone(), "desktop theme")];
    if st.config.controller.theme != st.config.desktop_theme {
        keys.push((st.config.controller.theme.clone(), "controller theme"));
    }
    for (key, role) in keys {
        let Some(spec) = jobs::spec_for(st, &key) else { continue };
        let name =
            st.themes.list().iter().find(|i| i.key == key).map(|i| i.meta.name.clone()).unwrap_or_else(|| key.clone());
        ui.add_space(8.0);
        ui.label(RichText::new(format!("Theme options — {name}")).size(15.0).strong().color(pal.text));
        hint_text(ui, &format!("Choices the {role}'s author offers. Changes apply right away."), pal);
        ui.add_space(2.0);
        theme(ui, st, &key, &spec, pal);
    }
}

fn theme(ui: &mut Ui, st: &mut AppState, key: &str, spec: &OptionsSpec, pal: &Pal) {
    let values = jobs::values(key, spec);
    let mut set: Option<(String, String)> = None;
    let mut section = "";
    egui::Grid::new(("ls_theme_options", key)).num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
        for o in spec.effective() {
            if !o.section.is_empty() && o.section != section {
                section = &o.section;
                ui.label(RichText::new(&o.section).size(12.0).color(pal.dim));
                ui.end_row();
            }
            if let Some(v) = row(ui, key, o, &values) {
                set = Some((o.key.clone(), v));
            }
            ui.end_row();
        }
    });
    if let Some((k, v)) = set {
        jobs::set_one(st, key, spec, &k, v);
    }
    ui.horizontal(|ui| {
        let changed = !values.is_empty();
        if ui
            .add_enabled(changed, egui::Button::new("Use the theme's own settings"))
            .on_hover_text("Back to the values the theme comes with")
            .clicked()
        {
            jobs::reset(st, key);
        }
        if jobs::busy(key) {
            ui.add(egui::Spinner::new().size(14.0));
        }
        let others = spec.others();
        if others > 0 {
            hint_text(
                ui,
                &format!(
                    "{others} more of its options change parts Lodestar draws its own way, so they aren't listed."
                ),
                pal,
            );
        }
    });
}

/// One option's label and control; Some(new value) when it changed.
fn row(ui: &mut Ui, key: &str, o: &ThemeOption, values: &std::collections::BTreeMap<String, String>) -> Option<String> {
    let id = egui::Id::NULL.with(("ls_theme_option", key, &o.key));
    match &o.kind {
        Kind::Switch { on, off } => {
            let mut b = o.is_on(values);
            let r = ui.checkbox(&mut b, &o.label).changed().then(|| if b { on.clone() } else { off.clone() });
            ui.label("");
            r
        }
        Kind::Choice { choices } => {
            ui.label(&o.label);
            let cur = o.value(values).to_string();
            let mut out = None;
            egui::ComboBox::from_id_salt(id).selected_text(&cur).width(160.0).show_ui(ui, |ui| {
                for c in choices {
                    if ui.selectable_label(c.eq_ignore_ascii_case(&cur), c).clicked() && !c.eq_ignore_ascii_case(&cur) {
                        out = Some(c.clone());
                    }
                }
            });
            out
        }
        Kind::Range { min, max, step } => {
            ui.label(&o.label);
            // The slider's value while it is dragged (the theme is rebuilt when it is let go).
            let mut v = ui.data(|d| d.get_temp::<f64>(id)).unwrap_or_else(|| o.number(values));
            let mut s = egui::Slider::new(&mut v, min.min(*max)..=max.max(*min));
            s = if *step >= 1.0 { s.step_by(1.0).fixed_decimals(0) } else { s.fixed_decimals(2) };
            let r = ui.add(s);
            if r.dragged() {
                ui.data_mut(|d| d.insert_temp(id, v));
                None
            } else {
                ui.data_mut(|d| d.remove::<f64>(id));
                let new = number_text(v, *step);
                (r.drag_stopped() || r.changed()).then_some(new).filter(|n| n != &number_text(o.number(values), *step))
            }
        }
        Kind::Color => {
            ui.label(&o.label);
            let c = parse_color(o.value(values)).unwrap_or(crate::theme::Color([255, 255, 255, 255]));
            let [r, g, b, a] = c.0;
            let mut col = egui::Color32::from_rgba_unmultiplied(r, g, b, a);
            if ui.color_edit_button_srgba(&mut col).changed() {
                let [r, g, b, a] = col.to_srgba_unmultiplied();
                // Applied once the picker closes (each change rebuilds the theme).
                ui.data_mut(|d| d.insert_temp(id, format!("#{a:02X}{r:02X}{g:02X}{b:02X}")));
            }
            let open = egui::Popup::is_any_open(ui.ctx());
            if !open && let Some(v) = ui.data_mut(|d| d.remove_temp::<String>(id)) {
                return Some(v);
            }
            None
        }
        Kind::Text => {
            ui.label(&o.label);
            let mut t = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| o.value(values).to_string());
            let r = ui.add(egui::TextEdit::singleline(&mut t).desired_width(200.0));
            if r.changed() {
                ui.data_mut(|d| d.insert_temp(id, t.clone()));
            }
            if r.lost_focus() {
                ui.data_mut(|d| d.remove::<String>(id));
                return (t.trim() != o.value(values).trim()).then(|| t.trim().to_string());
            }
            None
        }
    }
}
