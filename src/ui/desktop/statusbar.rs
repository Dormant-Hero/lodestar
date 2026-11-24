//! Bottom status bar: counts, scan / metadata activity, controllers, running game.

use super::{Action, DesktopUi};
use crate::app::AppState;
use crate::ui::widgets::{Icon, lazy_spinner, paint_icon};
use egui::{Align, Layout, RichText, Sense, Ui, vec2};

impl DesktopUi {
    pub(super) fn status_bar(&mut self, ui: &mut Ui, st: &mut AppState, view: &[usize]) {
        let pal = self.pal;
        ui.set_height(20.0);
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let n = view.len();
            let total = self.counts.all;
            let text = if n == total || self.lib_sel == super::LibSel::Hidden {
                format!("{n} game{} · {} installed", if n == 1 { "" } else { "s" }, self.view_installed)
            } else {
                format!("{n} of {total} games · {} installed", self.view_installed)
            };
            ui.label(RichText::new(text).size(12.0).color(pal.dim));
            if self.multi.len() > 1 {
                let r = ui.label(
                    RichText::new(format!("· {} selected", self.multi.len())).size(12.0).strong().color(pal.text),
                );
                r.on_hover_text("Ctrl+click / Shift+click to change, Esc to clear");
            }
            if st.scan.running {
                lazy_spinner(ui, 12.0, pal.accent);
                ui.label(RichText::new("Scanning library…").size(12.0).color(pal.dim));
            }
            let pending = st.metadata.pending();
            if pending > 0 {
                lazy_spinner(ui, 12.0, pal.dim);
                ui.label(RichText::new(format!("Fetching metadata ({pending})")).size(12.0).color(pal.dim));
            }
            if let Some(job) = self.dlg.theme_job.as_ref().and_then(|t| t.job.as_ref()) {
                lazy_spinner(ui, 12.0, pal.dim);
                ui.label(RichText::new(&job.label).size(12.0).color(pal.dim));
            }
            self.bulk_status(ui, st);

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if let Some(id) = crate::ui::launching::running(st) {
                    let status = crate::ui::launching::status(st, id);
                    let (stop, stop_tip) = if status.is_some() {
                        ("Cancel", "Stop waiting for it")
                    } else {
                        ("Stop", "Ask the game to quit")
                    };
                    if ui
                        .add(egui::Button::new(RichText::new(stop).size(12.0)).small())
                        .on_hover_text(stop_tip)
                        .clicked()
                    {
                        self.push(Action::Stop(id));
                    }
                    if let Some(text) = status {
                        // Right to left: the text, then its spinner on the left.
                        let r = ui.add(egui::Label::new(RichText::new(&text).size(12.0).color(pal.text)).truncate());
                        r.on_hover_text(text);
                        lazy_spinner(ui, 12.0, pal.accent);
                    } else {
                        let name = st.library.get(id).map(|g| g.display_name()).unwrap_or("game");
                        ui.label(RichText::new(format!("Playing {name}")).size(12.0).strong().color(pal.text));
                        let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 4.0, pal.accent);
                    }
                    ui.add_space(8.0);
                }
                let pads = st.input.gamepads();
                if !pads.is_empty() {
                    let label = if pads.len() == 1 { pads[0].clone() } else { format!("{} controllers", pads.len()) };
                    let resp = ui.label(RichText::new(label).size(12.0).color(pal.dim));
                    let (r, _) = ui.allocate_exact_size(vec2(18.0, 16.0), Sense::hover());
                    paint_icon(ui.painter(), r, Icon::Gamepad, pal.dim);
                    if pads.len() > 1 {
                        resp.on_hover_text(pads.join("\n"));
                    }
                }
            });
        });
    }
}
