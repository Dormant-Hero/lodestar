//! Menu → About Lodestar → Credits: thanks, the online services, fonts and built-in data, the
//! open-source libraries by licence and (folded away) every notice and licence text. The content
//! comes from the embedded THIRD_PARTY_NOTICES.md (`crate::ui::credits`).

use super::dialogs::{modal, plain_button, primary_button, title};
use super::{DesktopUi, Pal};
use crate::app::AppState;
use crate::ui::credits::{self as data, NOTICES};
use egui::{RichText, Ui};

/// The open Credits page.
#[derive(Default)]
pub(crate) struct CreditsDlg {
    /// Testing hook (`credits-all`): every folded section open.
    pub expand: bool,
}

impl DesktopUi {
    pub(super) fn credits_dialog(&mut self, ctx: &egui::Context, st: &mut AppState) {
        let Some(d) = self.dlg.credits.as_ref() else { return };
        let expand = d.expand;
        let pal = self.pal;
        let fam = self.fonts.bold.clone();
        let mut open_url: Option<String> = None;
        let mut keep = true;
        let mut copy = false;
        let max_h = (ctx.content_rect().height() - 200.0).clamp(240.0, 720.0);
        let (_, close) = modal(ctx, "ls_credits_dlg", &pal, 720.0, |ui| {
            title(
                ui,
                "Credits",
                Some(&format!("Lodestar {} · made with the work of many people", env!("CARGO_PKG_VERSION"))),
                &pal,
                &fam,
            );
            egui::ScrollArea::vertical()
                .id_salt("ls_credits_scroll")
                .max_height(max_h)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.set_max_width(690.0);
                    body(ui, &pal, expand, &mut open_url);
                });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if primary_button(ui, "Close", &pal, true).clicked() {
                    keep = false;
                }
                if plain_button(ui, "Copy all").on_hover_text("Every notice and licence text, as plain text").clicked()
                {
                    copy = true;
                }
            });
        });
        if copy {
            ctx.copy_text(data::readable().to_string());
            st.notify("Credits and licences copied to the clipboard");
        }
        if let Some(u) = open_url {
            st.open_url(&u);
        }
        if !keep || close {
            self.dlg.credits = None;
        }
    }
}

fn heading(ui: &mut Ui, text: &str, pal: &Pal) {
    ui.add_space(12.0);
    ui.label(RichText::new(text).size(16.0).strong().color(pal.text));
    ui.add_space(4.0);
}

/// A name that opens its site (hover shows the address).
fn link(ui: &mut Ui, name: &str, url: &str, pal: &Pal, open: &mut Option<String>) {
    let r = ui
        .add(egui::Label::new(RichText::new(name).strong().color(pal.accent)).sense(egui::Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(url);
    if r.clicked() {
        *open = Some(url.to_string());
    }
}

fn dim(ui: &mut Ui, text: &str, pal: &Pal) {
    ui.add(egui::Label::new(RichText::new(text).size(12.5).color(pal.dim)).wrap());
}

fn body(ui: &mut Ui, pal: &Pal, expand: bool, open: &mut Option<String>) {
    ui.label(
        RichText::new(
            "Lodestar stands on the shoulders of open-source projects and the communities that \
             collect game information. Thank you.",
        )
        .color(pal.text),
    );

    heading(ui, "Thanks to", pal);
    for (who, what, url) in data::THANKS {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            link(ui, who, url, pal, open);
            ui.label(RichText::new(format!("— {what}")).color(pal.text));
        });
    }

    heading(ui, "Game information and artwork", pal);
    dim(
        ui,
        "Each can be switched off in Settings → Metadata; the ones that need a key are asked only \
         once you add one. Pictures and texts belong to these services and their contributors.",
        pal,
    );
    ui.add_space(2.0);
    egui::Grid::new("ls_credits_services").num_columns(2).spacing([14.0, 4.0]).show(ui, |ui| {
        for row in data::table(NOTICES, "Online services") {
            let [name, what, site] = &row[..] else { continue };
            match data::link_of(site) {
                Some(url) => link(ui, name, url, pal, open),
                None => {
                    ui.label(RichText::new(name.as_str()).strong().color(pal.text));
                }
            }
            ui.label(RichText::new(what.as_str()).color(pal.text));
            ui.end_row();
        }
    });
    ui.add_space(4.0);
    dim(
        ui,
        "Game names, console names, store names and logos are trademarks of their owners. Lodestar \
         is not affiliated with or endorsed by any of them.",
        pal,
    );

    heading(ui, "Fonts", pal);
    for row in data::table(NOTICES, "Fonts built into Lodestar") {
        let [font, _, lic] = &row[..] else { continue };
        let short = data::short(lic);
        ui.horizontal_wrapped(|ui| {
            let name = data::plain(font);
            ui.label(RichText::new(name.split(" (").next().unwrap_or(&name)).color(pal.text));
            ui.label(RichText::new(format!("· {}", data::plain(&short))).size(12.5).color(pal.dim));
        });
    }

    heading(ui, "Built-in data", pal);
    for row in data::table(NOTICES, "Data built into Lodestar") {
        let [what, from, lic] = &row[..] else { continue };
        let short = data::short(lic);
        let what = data::plain(what);
        ui.label(RichText::new(what.split(" (").next().unwrap_or(&what)).color(pal.text));
        dim(ui, &format!("{} · {}", data::plain(from), data::plain(&short)), pal);
        ui.add_space(3.0);
    }

    heading(ui, "Open-source libraries", pal);
    let groups = data::built_in();
    let n: usize = groups.iter().map(|g| g.1.len()).sum();
    dim(
        ui,
        &format!(
            "{n} Rust libraries are built into Lodestar, each under the licence it is listed with. \
             The licence texts are below."
        ),
        pal,
    );
    ui.add_space(2.0);
    for (lic, libs) in &groups {
        egui::CollapsingHeader::new(RichText::new(format!("{lic} · {}", libs.len())).color(pal.text))
            .id_salt(("ls_credits_lic", lic.as_str()))
            .default_open(expand)
            .show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(libs.join(", ")).size(12.5).color(pal.dim)).wrap());
            });
    }

    heading(ui, "Licence texts and notices", pal);
    dim(ui, "The full THIRD_PARTY_NOTICES.md that comes with Lodestar.", pal);
    egui::CollapsingHeader::new(RichText::new("Show everything").color(pal.text))
        .id_salt("ls_credits_full")
        .default_open(expand)
        .show(ui, |ui| {
            egui::Frame::new()
                .fill(ui.visuals().extreme_bg_color)
                .stroke(egui::Stroke::new(1.0, pal.border))
                .corner_radius(5)
                .inner_margin(egui::Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        egui::Label::new(RichText::new(data::readable()).monospace().size(11.0).color(pal.dim))
                            .wrap_mode(egui::TextWrapMode::Wrap),
                    );
                });
        });
}
