//! The creator's start: a new theme from a template, one of your themes, or a shared .zip.

use super::super::Pal;
use super::super::dialogs::{hint_text, primary_button};
use super::files::Template;
use super::{CreatorDlg, Done, PickFor};
use crate::app::AppState;
use crate::theme::{Palette, ThemeInfo, builtin};
use egui::{RichText, Ui, vec2};

#[derive(Default)]
pub struct Start {
    pub template: Option<Template>,
    pub name: String,
    pub author: String,
    /// Re-read the list of themes.
    pub refresh: bool,
    palettes: Vec<(String, Palette, String)>,
    /// The user themes' palettes (read when the list is).
    user_palettes: std::collections::HashMap<String, Palette>,
    /// Live pictures of the templates.
    thumbs: super::preview::Thumbs,
}

fn user_themes(st: &AppState) -> Vec<ThemeInfo> {
    st.themes.list().iter().filter(|i| i.dir.is_some() && i.meta.controller).cloned().collect()
}

pub fn show(ui: &mut Ui, st: &mut AppState, d: &mut CreatorDlg, pal: &Pal, fam: &egui::FontFamily) {
    if std::mem::take(&mut d.start.refresh) || d.start.palettes.is_empty() {
        st.themes.refresh();
        d.start.palettes =
            builtin::all().into_iter().map(|(k, f)| (k.to_string(), f.palette, f.theme.description)).collect();
        d.start.user_palettes =
            user_themes(st).into_iter().map(|u| (u.key.clone(), st.themes.load(&u.key).file.palette.clone())).collect();
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Theme creator").family(fam.clone()).size(22.0).strong().color(pal.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new("Close").min_size(vec2(80.0, 30.0))).clicked() {
                d.close = true;
            }
        });
    });
    // The templates' live pictures: load each theme once, register its fonts.
    for (k, _, _) in &d.start.palettes {
        d.start.thumbs.ensure(k, || builtin::all().into_iter().find(|(b, _)| b == k).map(|(_, f)| (f, None)));
    }
    for u in user_themes(st) {
        if let Some(dir) = u.dir.clone() {
            let id = dir.display().to_string();
            let themes = &st.themes;
            d.start.thumbs.ensure(&id, || Some((themes.load(&u.key).file.clone(), Some(dir.clone()))));
        }
    }
    {
        let mut all = vec![st.desktop_theme.clone(), st.controller_theme.clone()];
        all.extend(d.start.thumbs.themes());
        let refs: Vec<&crate::theme::Theme> = all.iter().map(|t| t.as_ref()).collect();
        st.fonts.ensure(ui.ctx(), &refs);
    }
    hint_text(
        ui,
        "Make your own look for controller mode. Start from any theme — Lodestar's own are never changed: you work on a copy.",
        pal,
    );
    if let Some(b) = &d.banner {
        ui.label(RichText::new(&b.text).size(12.5).color(if b.error { pal.danger } else { pal.accent }));
    }
    ui.add_space(10.0);
    let users = user_themes(st);
    let ctx = ui.ctx().clone();
    ui.horizontal_top(|ui| {
        let left_w = (ui.available_width() * 0.62).max(420.0);
        ui.allocate_ui_with_layout(
            vec2(left_w, ui.available_height()),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(left_w);
                ui.label(RichText::new("New theme").size(15.0).strong().color(pal.text));
                hint_text(ui, "Pick a theme to start from.", pal);
                ui.add_space(6.0);
                let card_w = 190.0;
                let cols = ((left_w / (card_w + 10.0)).floor() as usize).max(1);
                egui::ScrollArea::vertical()
                    .id_salt("ls_creator_templates")
                    .max_height(ui.available_height() - 140.0)
                    .show(ui, |ui| {
                        egui::Grid::new("ls_creator_template_grid").spacing([10.0, 10.0]).show(ui, |ui| {
                            let mut i = 0;
                            let mut cards: Vec<(Template, String, Palette, String)> = d
                                .start
                                .palettes
                                .iter()
                                .map(|(k, p, desc)| (Template::Builtin(k.clone()), k.clone(), p.clone(), desc.clone()))
                                .collect();
                            for u in &users {
                                if let Some(dir) = &u.dir {
                                    let pal_u = d.start.user_palettes.get(&u.key).cloned().unwrap_or_default();
                                    cards.push((
                                        Template::Folder(dir.clone()),
                                        u.meta.name.clone(),
                                        pal_u,
                                        format!("Yours · {}", u.meta.description),
                                    ));
                                }
                            }
                            for (tpl, name, p, desc) in cards {
                                let selected = d.start.template.as_ref() == Some(&tpl);
                                let frame = egui::Frame::new()
                                    .fill(if selected { pal.sel_bg } else { pal.surface_alt })
                                    .stroke(egui::Stroke::new(
                                        if selected { 2.0 } else { 1.0 },
                                        if selected { pal.accent } else { pal.border },
                                    ))
                                    .corner_radius(8)
                                    .inner_margin(8);
                                let r = frame
                                    .show(ui, |ui| {
                                        ui.vertical(|ui| {
                                            ui.set_width(card_w - 16.0);
                                            let tid = match &tpl {
                                                Template::Builtin(k) => k.clone(),
                                                Template::Folder(dir) => dir.display().to_string(),
                                            };
                                            let w = card_w - 16.0;
                                            let (r, _) =
                                                ui.allocate_exact_size(vec2(w, w * 9.0 / 16.0), egui::Sense::hover());
                                            ui.painter().rect_filled(r, 4.0, p.background.to_egui());
                                            d.start.thumbs.draw(ui, st, &tid, r);
                                            ui.add(
                                                egui::Label::new(RichText::new(&name).strong().color(pal.text))
                                                    .truncate(),
                                            );
                                            let mut job = egui::text::LayoutJob::simple(
                                                desc.clone(),
                                                egui::FontId::proportional(11.5),
                                                pal.dim,
                                                card_w - 16.0,
                                            );
                                            job.wrap.max_rows = 2;
                                            job.wrap.overflow_character = Some('…');
                                            ui.allocate_ui_with_layout(
                                                vec2(card_w - 16.0, 30.0),
                                                egui::Layout::top_down(egui::Align::Min),
                                                |ui| {
                                                    ui.set_min_height(30.0);
                                                    ui.add(egui::Label::new(job));
                                                },
                                            );
                                        });
                                    })
                                    .response
                                    .interact(egui::Sense::click());
                                if r.clicked() {
                                    if d.start.name.trim().is_empty() || d.start.name.starts_with("My ") {
                                        d.start.name = format!("My {name}");
                                    }
                                    d.start.template = Some(tpl);
                                }
                                i += 1;
                                if i % cols == 0 {
                                    ui.end_row();
                                }
                            }
                        });
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Name").color(pal.dim));
                    ui.add(egui::TextEdit::singleline(&mut d.start.name).hint_text("My theme").desired_width(200.0));
                    ui.label(RichText::new("Your name").color(pal.dim));
                    ui.add(egui::TextEdit::singleline(&mut d.start.author).hint_text("optional").desired_width(140.0));
                    let busy = d.jobs.iter().any(|j| j.label.starts_with("Making"));
                    let can = d.start.template.is_some() && !d.start.name.trim().is_empty() && !busy;
                    if primary_button(ui, "Create", pal, can).clicked()
                        && let Some(tpl) = d.start.template.clone()
                    {
                        let (dir, name, author) =
                            (st.themes.themes_dir().to_path_buf(), d.start.name.clone(), d.start.author.clone());
                        d.spawn(&ctx, "Making the theme…", move || {
                            Done::Created(
                                super::files::create(&dir, &tpl, &name, &author).map_err(|e| format!("{e:#}")),
                            )
                        });
                    }
                });
            },
        );
        ui.separator();
        ui.vertical(|ui| {
            ui.label(RichText::new("Your themes").size(15.0).strong().color(pal.text));
            if users.is_empty() {
                hint_text(ui, "Themes you make or add show here.", pal);
            }
            let mut open = None;
            egui::ScrollArea::vertical().id_salt("ls_creator_yours").max_height(ui.available_height() - 80.0).show(
                ui,
                |ui| {
                    for u in &users {
                        ui.horizontal(|ui| {
                            ui.add(egui::Label::new(RichText::new(&u.meta.name).color(pal.text)).truncate());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let imported =
                                    u.dir.as_ref().is_some_and(|d| d.join("playnite-options.toml").is_file());
                                if imported {
                                    if ui
                                        .button("Edit a copy")
                                        .on_hover_text("Its Playnite options would undo changes to the original")
                                        .clicked()
                                        && let Some(dir) = &u.dir
                                    {
                                        d.start.template = Some(Template::Folder(dir.clone()));
                                        d.start.name = format!("{} (mine)", u.meta.name);
                                    }
                                } else if ui.button("Edit").clicked() {
                                    open = Some(u.key.clone());
                                }
                            });
                        });
                    }
                },
            );
            ui.add_space(8.0);
            if ui.button("Add a theme .zip…").on_hover_text("A Lodestar theme someone shared").clicked() {
                d.pick(&ctx, PickFor::ImportZip, "Choose a Lodestar theme .zip");
            }
            if let Some(k) = open {
                d.open_key(st, &k);
            }
        });
    });
}
