//! The Achievements section of a game's page: the service, "N of M" with a bar,
//! the latest unlocks, "Show all" with every achievement, and the service's page. A game
//! without achievements gets no section; something missing is one plain line.

use super::super::{Action, Pal};
use crate::achievements::{Achievement, Progress, State};
use crate::images::{ImageCache, ImgSize};
use crate::model::GameId;
use crate::ui::achievements::{
    count_line, fraction, hardcore_line, link_label, percent_label, shown_description, shown_name, when_line,
};
use crate::ui::widgets::{mix, with_alpha};
use egui::{Align, Color32, FontFamily, Rect, RichText, Sense, Ui, vec2};

/// Seconds a lookup may take before "Looking up achievements…" shows (a game without any
/// answers at once: no line flashing up and away).
const QUIET_LOADING_SECS: f64 = 0.6;

/// Sizes of one place the section is drawn in (the Details page, or the narrow side panel).
pub(crate) struct Look {
    pub heading_pt: f32,
    pub text_pt: f32,
    pub bold: FontFamily,
    /// Latest unlocks listed before "Show all".
    pub recent: usize,
    pub icon: f32,
}

/// "Show all" is open for this game (egui memory; the testing hook opens it too).
pub(crate) fn show_all_id(id: GameId) -> egui::Id {
    egui::Id::NULL.with(("ls_ach_all", id))
}

/// Draw the section for `state` (None: nothing). Spacing above it is added only when something
/// is drawn. Returns a link to open.
pub(crate) fn section(
    ui: &mut Ui,
    images: &mut ImageCache,
    id: GameId,
    state: Option<&State>,
    pal: &Pal,
    look: &Look,
    space_above: f32,
) -> Option<Action> {
    let state = state?;
    let loading_since = egui::Id::NULL.with(("ls_ach_loading", id));
    if let State::Loading = state {
        let now = ui.input(|i| i.time);
        let since = ui.data_mut(|d| *d.get_temp_mut_or(loading_since, now));
        if now - since < QUIET_LOADING_SECS {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(QUIET_LOADING_SECS));
            return None;
        }
    } else {
        ui.data_mut(|d| d.remove::<f64>(loading_since));
    }
    ui.add_space(space_above);
    heading(ui, look, pal);
    match state {
        State::None => None,
        State::Loading => {
            ui.label(RichText::new("Looking up achievements…").size(look.text_pt - 1.0).color(pal.dim));
            None
        }
        State::Hint(text) => {
            ui.add(egui::Label::new(RichText::new(text).size(look.text_pt - 1.0).color(pal.dim)).wrap());
            // Something to set up: one click away.
            let settings = text.contains("Settings → Achievements");
            let open = settings
                && ui
                    .add(
                        egui::Label::new(
                            RichText::new("Open Settings → Achievements").size(look.text_pt - 1.5).color(pal.accent),
                        )
                        .sense(Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked();
            open.then_some(Action::Settings(crate::ui::desktop::settings::Tab::Achievements))
        }
        State::Ready(p) => progress(ui, images, id, p, pal, look),
    }
}

fn heading(ui: &mut Ui, look: &Look, pal: &Pal) {
    ui.label(RichText::new("Achievements").family(look.bold.clone()).size(look.heading_pt).strong().color(pal.text));
    ui.add_space(6.0);
}

fn progress(ui: &mut Ui, images: &mut ImageCache, id: GameId, p: &Progress, pal: &Pal, look: &Look) -> Option<Action> {
    let mut open = None;
    // Service · link.
    ui.horizontal(|ui| {
        ui.label(RichText::new(p.service.label().to_uppercase()).size(10.5).color(pal.dim).extra_letter_spacing(0.8));
        if let Some(url) = &p.page_url {
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if crate::ui::desktop::dialogs::link_text(ui, &link_label(p.service), url, pal) {
                    open = Some(Action::OpenUrl(url.clone()));
                }
            });
        }
    });
    // N of M · percent.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.label(RichText::new(count_line(p)).family(look.bold.clone()).size(look.text_pt + 5.0).color(pal.text));
        if p.synced && p.total > 0 {
            ui.label(RichText::new(percent_label(p)).size(look.text_pt).color(pal.dim));
        }
        if let Some(hc) = hardcore_line(p) {
            ui.label(RichText::new(format!("· {hc}")).size(look.text_pt - 1.0).color(pal.dim));
        }
    });
    ui.add_space(4.0);
    if p.synced {
        bar(ui, fraction(p), pal);
        ui.add_space(4.0);
    }
    if let Some(note) = &p.note {
        ui.add(egui::Label::new(RichText::new(note).size(look.text_pt - 1.5).color(pal.dim)).wrap());
    }
    if p.list.is_empty() {
        return open;
    }
    let all_id = show_all_id(id);
    let mut all = ui.data(|d| d.get_temp::<bool>(all_id)).unwrap_or(false);
    let recent = p.recent();
    ui.add_space(10.0);
    if all {
        small_caps(ui, "All achievements", pal);
        for a in &p.list {
            row(ui, images, a, pal, look);
        }
    } else if !recent.is_empty() {
        small_caps(ui, "Recently unlocked", pal);
        for a in recent.iter().take(look.recent) {
            row(ui, images, a, pal, look);
        }
    }
    let more = p.list.len();
    let label = if all { "Show less".to_string() } else { format!("Show all {more}") };
    ui.add_space(2.0);
    let r =
        ui.add(egui::Label::new(RichText::new(label).size(look.text_pt - 1.0).color(pal.accent)).sense(Sense::click()));
    if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        all = !all;
        ui.data_mut(|d| d.insert_temp(all_id, all));
    }
    open
}

fn small_caps(ui: &mut Ui, text: &str, pal: &Pal) {
    ui.label(RichText::new(text.to_uppercase()).size(10.5).color(pal.dim).extra_letter_spacing(0.8));
    ui.add_space(4.0);
}

/// The progress bar: the accent over a quiet track.
fn bar(ui: &mut Ui, f: f32, pal: &Pal) {
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, 6.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, 3.0, mix(pal.surface_alt, pal.dim, 0.18));
    if f > 0.0 {
        let fill = Rect::from_min_size(r.min, vec2((r.width() * f).max(6.0), r.height()));
        p.rect_filled(fill, 3.0, pal.accent);
    }
}

/// One achievement: picture, name and description, when (or "Locked"). Locked ones are greyed.
fn row(ui: &mut Ui, images: &mut ImageCache, a: &Achievement, pal: &Pal, look: &Look) {
    let locked = a.unlocked_at.is_none();
    let s = look.icon;
    let when = when_line(a);
    let when_w = 96.0;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (r, _) = ui.allocate_exact_size(vec2(s, s), Sense::hover());
        icon(ui, images, r, a, pal);
        // Room left after the spacing before and after the text column.
        let text_w = (ui.available_width() - when_w - 24.0).max(80.0);
        ui.allocate_ui_with_layout(vec2(text_w, s), egui::Layout::top_down(Align::Min), |ui| {
            ui.set_width(text_w);
            ui.spacing_mut().item_spacing.y = 1.0;
            let name_c = if locked { mix(pal.text, pal.dim, 0.85) } else { pal.text };
            ui.add(egui::Label::new(RichText::new(shown_name(a)).size(look.text_pt).color(name_c)).wrap());
            let desc = shown_description(a);
            if !desc.is_empty() {
                let c = if locked { with_alpha(pal.dim, 150) } else { pal.dim };
                ui.add(egui::Label::new(RichText::new(desc).size(look.text_pt - 1.5).color(c)).wrap());
            }
        });
        ui.allocate_ui_with_layout(vec2(when_w, s), egui::Layout::right_to_left(Align::Min), |ui| {
            let c = if locked { with_alpha(pal.dim, 150) } else { pal.dim };
            ui.label(RichText::new(when).size(look.text_pt - 2.0).color(c));
        });
    });
    ui.add_space(8.0);
}

/// The achievement's picture (the locked one, dimmed, while locked); a quiet tile without one.
fn icon(ui: &Ui, images: &mut ImageCache, r: Rect, a: &Achievement, pal: &Pal) {
    let locked = a.unlocked_at.is_none();
    let path = if locked { a.icon_locked.as_ref().or(a.icon.as_ref()) } else { a.icon.as_ref() };
    let tex = path.and_then(|p| images.get(std::path::Path::new(p), ImgSize::Icon));
    let p = ui.painter();
    let radius = 6.0;
    match tex {
        Some(t) => {
            // Locked: faded into the page (the services' locked pictures are grey already).
            let tint = if locked { Color32::from_gray(150).gamma_multiply(0.55) } else { Color32::WHITE };
            crate::ui::widgets::paint_cover(p, r, &t, radius, tint, vec2(0.5, 0.5));
        }
        None => {
            p.rect_filled(r, radius, mix(pal.surface_alt, pal.dim, 0.12));
        }
    }
}
