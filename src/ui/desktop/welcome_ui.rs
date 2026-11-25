//! The first start's welcome step in desktop mode (see welcome_app.rs): why a SteamGridDB key
//! helps, where to get one, a field to paste it into and whether it works.

use super::Pal;
use super::dialogs::{Buttons, footer, hint_text, modal_esc_only};
use super::emulation_ui::{ChipTone, tone};
use crate::app::AppState;
use crate::art::KeyCheck;
use crate::welcome_app::{KEY_LATER, SGDB_KEY_PAGE, key_check_text};
use egui::{RichText, Ui};

const WIDTH: f32 = 540.0;
/// The steps' text and controls sit right of their number.
const STEP_INDENT: f32 = 22.0;
/// The key check's line under the field.
const STATUS_SIZE: f32 = 12.5;

/// Drawn every frame by the desktop; nothing once the step is answered.
pub(super) fn show(ctx: &egui::Context, st: &mut AppState, pal: &Pal) {
    if !st.welcome.open {
        return;
    }
    let mut answer: Option<bool> = None;
    // Only Esc and the buttons answer this one-time step, never a click beside it.
    let (_, close) = modal_esc_only(ctx, "ls_welcome", pal, WIDTH, |ui| {
        ui.label(RichText::new("Welcome to Lodestar").size(19.0).strong().color(pal.text));
        // The line keeps its place while it has nothing to say, so nothing below it moves.
        ui.label(RichText::new(games_line(st).unwrap_or_default()).size(13.0).color(pal.dim));
        ui.add_space(10.0);
        para(
            ui,
            "Steam and GOG games bring their own artwork. Covers, backgrounds and logos for emulated games, \
             shortcuts and launchers without art of their own come from SteamGridDB, a free community site.",
            pal,
        );
        ui.add_space(4.0);
        para(ui, "Add your free SteamGridDB key now, and artwork shows up for all your games from the start.", pal);
        ui.add_space(10.0);

        step(ui, "1", "Sign in at SteamGridDB (a Steam account works) and open the API page.", pal);
        indented(ui, |ui| {
            if ui.button("Open SteamGridDB's API page ↗").on_hover_text(SGDB_KEY_PAGE).clicked() {
                st.open_url(SGDB_KEY_PAGE);
            }
        });
        ui.add_space(8.0);
        step(ui, "2", "Copy the API key shown there and paste it here (Ctrl+V).", pal);
        // Hidden like the key field in Settings, with the same Show / Hide.
        let show_id = egui::Id::unique("ls_welcome_show_key");
        let mut show_key = ui.data(|d| d.get_temp::<bool>(show_id).unwrap_or(false));
        indented(ui, |ui| {
            // A fixed width for Show / Hide, so the row (and the dialog) keeps its width when
            // the label changes.
            let toggle_w = 56.0;
            let field_w = WIDTH - STEP_INDENT - toggle_w - ui.spacing().item_spacing.x;
            let field = ui.add(
                egui::TextEdit::singleline(&mut st.welcome.key)
                    .password(!show_key)
                    .hint_text("SteamGridDB API key")
                    .desired_width(field_w)
                    .font(egui::TextStyle::Monospace),
            );
            let toggle = egui::Button::new(if show_key { "Hide" } else { "Show" }).small();
            if ui.add_sized([toggle_w, field.rect.height()], toggle).clicked() {
                show_key = !show_key;
                ui.data_mut(|d| d.insert_temp(show_id, show_key));
            }
        });
        let key = st.welcome.key.clone();
        let check = st.sgdb_key_check(&key);
        // Room for the check's answer (up to two lines) is kept while there is none, so the
        // dialog doesn't grow, and its buttons don't move, when the answer appears.
        let line_h = ui.ctx().fonts_mut(|f| f.row_height(&egui::FontId::proportional(STATUS_SIZE)));
        let status_h = 2.0 * line_h + 2.0;
        let status_w = WIDTH - STEP_INDENT;
        indented(ui, |ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(status_w, status_h),
                egui::Layout::left_to_right(egui::Align::Min),
                |ui| {
                    ui.set_min_size(egui::vec2(status_w, status_h));
                    if let Some(c) = &check {
                        key_status(ui, c, pal);
                    }
                },
            );
        });
        ui.add_space(4.0);
        hint_text(
            ui,
            "Descriptions and genres of emulated games come from IGDB, which needs a free Twitch app: \
             Settings → Metadata.",
            pal,
        );

        let can_continue = !key.trim().is_empty() && check != Some(KeyCheck::Rejected);
        let buttons = Buttons::primary("Continue", can_continue)
            .cancel("Skip for now")
            .disabled_hint("Paste a key first, or skip for now");
        let f = footer(ui, pal, buttons, |ui| hint_text(ui, &format!("You can add it later in {KEY_LATER}."), pal));
        if f.primary {
            answer = Some(true);
        } else if f.cancel {
            answer = Some(false);
        }
    });
    // Escape: not now (the toast says where the key goes later).
    if let Some(use_key) = answer.or(close.then_some(false)) {
        st.finish_welcome(use_key);
    }
}

/// "Looking for your games… 12 found so far" while the first scan runs.
fn games_line(st: &AppState) -> Option<String> {
    let n = st.library.games.iter().filter(|g| !g.hidden).count();
    if st.scan.running {
        return Some(format!("Looking for your games… {n} found so far"));
    }
    match n {
        0 => None,
        1 => Some("Lodestar found 1 game".into()),
        n => Some(format!("Lodestar found {n} games")),
    }
}

fn para(ui: &mut Ui, text: &str, pal: &Pal) {
    ui.add(egui::Label::new(RichText::new(text).size(13.5).color(pal.text)).wrap());
}

fn step(ui: &mut Ui, number: &str, text: &str, pal: &Pal) {
    ui.horizontal_top(|ui| {
        ui.add_sized([STEP_INDENT - 6.0, 18.0], egui::Label::new(RichText::new(number).strong().color(pal.accent)));
        ui.add(egui::Label::new(RichText::new(text).size(13.5).color(pal.text)).wrap());
    });
    ui.add_space(4.0);
}

fn indented(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.add_space(STEP_INDENT - ui.spacing().item_spacing.x);
        add(ui);
    });
}

/// Whether the key works, in its colour (the step's and Settings' key fields).
pub(super) fn key_status(ui: &mut Ui, c: &KeyCheck, pal: &Pal) {
    let color = match c {
        KeyCheck::Checking => pal.dim,
        KeyCheck::Works => tone(ChipTone::Good, pal),
        KeyCheck::Rejected => pal.danger,
        KeyCheck::Unreachable(_) => tone(ChipTone::Warn, pal),
    };
    if *c == KeyCheck::Checking {
        crate::ui::widgets::lazy_spinner(ui, 12.0, pal.dim);
    }
    let label = ui.add(egui::Label::new(RichText::new(key_check_text(c)).size(STATUS_SIZE).color(color)).wrap());
    if let KeyCheck::Unreachable(why) = c {
        label.on_hover_text(why);
    }
}

#[cfg(test)]
mod tests {
    use crate::app::AppState;
    use egui::{Event, PointerButton, pos2};

    /// One frame of the step on a 1280×800 window with `events`.
    fn frame(st: &mut AppState, events: Vec<Event>) {
        let ctx = st.ctx().clone();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(1280.0, 800.0))),
            events,
            ..Default::default()
        };
        let pal = super::Pal::default();
        let _ = ctx.run_ui(input, |ui| super::show(&ui.ctx().clone(), st, &pal));
    }

    fn click(at: egui::Pos2) -> [Vec<Event>; 3] {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        [vec![Event::PointerMoved(at)], vec![button(true)], vec![button(false)]]
    }

    #[test]
    fn a_click_beside_the_step_does_not_answer_it_but_escape_does() {
        let tmp = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(tmp.path());
        st.welcome.open = true;
        frame(&mut st, vec![]);
        for events in click(pos2(20.0, 20.0)) {
            frame(&mut st, events);
        }
        frame(&mut st, vec![]);
        assert!(st.welcome.open, "a click on the backdrop skipped the step");
        let esc = Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        frame(&mut st, vec![esc]);
        assert!(!st.welcome.open, "Escape is Skip for now");
    }
}
