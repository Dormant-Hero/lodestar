//! "Choose what to play" on the desktop: a small list under the Play button (or where the game
//! was started from) when Play has several ways — several Play actions, a "Choose when playing"
//! emulator action, a game with several ROMs (`emulation::roms::entries`). One click starts that
//! line; "Always use this for this game" remembers it (Play ▾ → Ask every time forgets it).
//! Play ▾ lists the same lines.

use super::super::Action;
use crate::app::AppState;
use crate::emulation::roms::Entry;
use crate::model::{Game, GameId};
use egui::{Pos2, RichText};

#[derive(Clone)]
struct Open {
    id: GameId,
    pos: Pos2,
    entries: Vec<Entry>,
    /// The pass it opened in (that pass's click is the one that opened it).
    pass: u64,
    focused: bool,
}

fn state_id() -> egui::Id {
    egui::Id::unique("ls_play_chooser")
}

fn anchor_id() -> egui::Id {
    egui::Id::unique("ls_play_anchor")
}

fn always_id(id: GameId) -> egui::Id {
    egui::Id::NULL.with(("ls_play_choose_always", id))
}

/// The Play button was clicked: the list opens under it (`pos` = its bottom-left corner).
pub(crate) fn set_anchor(ctx: &egui::Context, pos: Pos2) {
    ctx.data_mut(|d| d.insert_temp(anchor_id(), pos));
}

/// Play for `id` from anywhere on the desktop: starts it, or opens the list when Play has to ask.
pub(crate) fn play(ctx: &egui::Context, st: &mut AppState, id: GameId) {
    let anchor = ctx.data_mut(|d| d.remove_temp::<Pos2>(anchor_id()));
    if let Some(entries) = st.play_or_choose(id) {
        let pos = anchor.or_else(|| ctx.pointer_latest_pos()).unwrap_or_else(|| ctx.content_rect().center());
        let pass = ctx.cumulative_pass_nr();
        ctx.data_mut(|d| {
            d.remove::<bool>(always_id(id));
            d.insert_temp(state_id(), Open { id, pos, entries, pass, focused: false });
        });
    }
}

/// Is the list open (for keyboard shortcuts that shouldn't act behind it)?
pub(crate) fn is_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<Open>(state_id())).is_some()
}

/// Testing hook (`LODESTAR_DESKTOP_OPEN=choose-play`): open the list for `id` at `pos`.
pub(crate) fn open_at(ctx: &egui::Context, st: &AppState, id: GameId, pos: Pos2) -> bool {
    let Some(g) = st.library.get(id) else { return false };
    let entries = crate::emu_app::play_entries_now(g);
    if entries.len() < 2 {
        return false;
    }
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(state_id(), Open { id, pos, entries, pass, focused: false }));
    true
}

/// Draw the list when it is open (every frame, over everything).
pub(crate) fn show(ctx: &egui::Context, st: &mut AppState, pal: &super::super::Pal) {
    let Some(mut open) = ctx.data(|d| d.get_temp::<Open>(state_id())) else { return };
    let close = |ctx: &egui::Context| ctx.data_mut(|d| d.remove::<Open>(state_id()));
    if st.library.get(open.id).is_none() || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close(ctx);
        return;
    }
    let mut picked: Option<(crate::model::PlayChoice, bool)> = None;
    let screen = ctx.content_rect();
    let pos =
        egui::pos2(open.pos.x.min(screen.right() - 300.0).max(screen.left()), open.pos.y.min(screen.bottom() - 60.0));
    let resp = egui::Area::new(egui::Id::unique("ls_play_chooser_area"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(300.0);
                ui.label(RichText::new("Choose what to play").size(12.5).color(pal.dim));
                ui.add_space(2.0);
                let mut always = ui.data(|d| d.get_temp::<bool>(always_id(open.id))).unwrap_or(false);
                for (i, e) in open.entries.iter().enumerate() {
                    let b = ui.add(
                        egui::Button::new(RichText::new(&e.label).size(14.0))
                            .min_size(egui::vec2(ui.available_width(), 28.0)),
                    );
                    if i == 0 && !open.focused {
                        b.request_focus();
                    }
                    if b.clicked() {
                        picked = Some((e.choice.clone(), always));
                    }
                }
                ui.separator();
                if ui
                    .checkbox(&mut always, "Always use this for this game")
                    .on_hover_text("Play starts the one you pick from now on (Play ▾ lists the others)")
                    .changed()
                {
                    ui.data_mut(|d| d.insert_temp(always_id(open.id), always));
                }
            });
        })
        .response;
    if !open.focused {
        open.focused = true;
        ctx.data_mut(|d| d.insert_temp(state_id(), open.clone()));
    }
    if let Some((c, always)) = picked {
        close(ctx);
        ctx.data_mut(|d| d.remove::<bool>(always_id(open.id)));
        st.launch_choice(open.id, c, always);
        return;
    }
    // A click elsewhere closes it (not the click that opened it).
    let outside =
        ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !resp.rect.contains(p)));
    if outside && ctx.cumulative_pass_nr() > open.pass {
        close(ctx);
    }
}

/// Play ▾ (and the right-click menu): one entry per line of "Choose what to play", then "Ask
/// every time" when a line is remembered. False (nothing drawn) when Play has one way only.
pub(crate) fn menu_entries(ui: &mut egui::Ui, g: &Game, enabled: bool, acts: &mut Vec<Action>) -> bool {
    let entries = crate::emu_app::play_entries_now(g);
    if entries.len() < 2 {
        return false;
    }
    let remembered = crate::emulation::roms::remembered(g, &entries);
    for e in &entries {
        let current = remembered.as_ref() == Some(&e.choice);
        let text = if current { RichText::new(format!("{}  ✓", e.label)).strong() } else { RichText::new(&e.label) };
        let r = ui.add_enabled(enabled, egui::Button::new(text));
        let r = if current { r.on_hover_text("What Play starts (you chose Always use this)") } else { r };
        if r.clicked() {
            acts.push(Action::PlayChoice(g.id, e.choice.clone()));
        }
    }
    if remembered.is_some()
        && ui
            .add_enabled(enabled, egui::Button::new("Ask every time"))
            .on_hover_text("Play asks what to play again")
            .clicked()
    {
        acts.push(Action::ForgetPlayChoice(g.id));
    }
    ui.separator();
    true
}
