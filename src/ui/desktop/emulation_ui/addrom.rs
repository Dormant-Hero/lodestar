//! Right-click → "Add to another game as a ROM…" (emulated games): the file becomes one more
//! ROM of a game of the same console (Pokémon Azul inside Pokémon Blue), and its own library
//! entry folds into that game (favourite, play time and tags move along; rescans don't list it
//! again). Edit → Installation of that game can take it out again.

use super::super::Pal;
use super::super::dialogs::{Buttons, footer, hint_text, modal, title};
use crate::app::AppState;
use crate::model::GameId;
use egui::RichText;

#[derive(Clone)]
struct Open {
    from: GameId,
    search: String,
}

fn state_id() -> egui::Id {
    egui::Id::unique("ls_add_as_rom")
}

pub(crate) fn open(ctx: &egui::Context, from: GameId) {
    ctx.data_mut(|d| d.insert_temp(state_id(), Open { from, search: String::new() }));
}

/// Is the dialog open (the library's keys wait behind it)?
pub(crate) fn is_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<Open>(state_id())).is_some()
}

/// Can `t` take `g`'s file as one more ROM? Another visible emulated game of the same console.
fn is_target(g: &crate::model::Game, t: &crate::model::Game) -> bool {
    g.emu
        .as_ref()
        .is_some_and(|e| t.id != g.id && !t.hidden && t.emu.as_ref().is_some_and(|x| x.platform == e.platform))
}

/// The games `from` can become a ROM of: emulated games of the same console, by name.
pub(crate) fn targets(st: &AppState, from: GameId) -> Vec<(GameId, String)> {
    let Some(g) = st.library.get(from) else { return vec![] };
    let mut v: Vec<(GameId, String, String)> = st
        .library
        .games
        .iter()
        .filter(|t| is_target(g, t))
        .map(|t| (t.id, t.display_name().to_string(), t.sort_name.clone()))
        .collect();
    v.sort_by(|a, b| a.2.cmp(&b.2));
    v.into_iter().map(|(id, n, _)| (id, n)).collect()
}

/// Is there any game `g` could be added to? The menu greys its entry out otherwise (the dialog
/// would only say there's nothing to pick).
pub(crate) fn has_target(lib: &crate::library::Library, g: &crate::model::Game) -> bool {
    lib.games.iter().any(|t| is_target(g, t))
}

/// Draws the dialog; returns the game the file went into (the desktop selects it, as the
/// entry that was selected is gone).
pub(crate) fn show(ctx: &egui::Context, st: &mut AppState, pal: &Pal, fam: &egui::FontFamily) -> Option<GameId> {
    let mut open = ctx.data(|d| d.get_temp::<Open>(state_id()))?;
    let Some(name) = st.library.get(open.from).map(|g| g.display_name().to_string()) else {
        ctx.data_mut(|d| d.remove::<Open>(state_id()));
        return None;
    };
    let all = targets(st, open.from);
    let mut keep = true;
    let mut chosen: Option<GameId> = None;
    let (_, close) = modal(ctx, "ls_add_as_rom_dlg", pal, 440.0, |ui| {
        title(ui, "Add to another game as a ROM", Some(&name), pal, fam);
        hint_text(
            ui,
            "Pick the game it belongs to. Play on that game then asks which ROM to start, and this entry \
             goes into it (its favourite, play time and tags move along).",
            pal,
        );
        ui.add_space(6.0);
        let r = ui.add(egui::TextEdit::singleline(&mut open.search).hint_text("Search").desired_width(f32::INFINITY));
        if open.search.is_empty() && !r.has_focus() {
            r.request_focus();
        }
        ui.add_space(4.0);
        let q = open.search.trim().to_lowercase();
        let shown: Vec<&(GameId, String)> =
            all.iter().filter(|(_, n)| q.is_empty() || n.to_lowercase().contains(&q)).collect();
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            if shown.is_empty() {
                hint_text(
                    ui,
                    if all.is_empty() { "No other game of this console in your library." } else { "No game matches." },
                    pal,
                );
            }
            for (id, n) in shown {
                if ui
                    .add(
                        egui::Button::new(RichText::new(n).color(pal.text))
                            .min_size(egui::vec2(ui.available_width(), 26.0)),
                    )
                    .clicked()
                {
                    chosen = Some(*id);
                }
            }
        });
        if footer(ui, pal, Buttons::default().cancel("Cancel"), |_| {}).cancel {
            keep = false;
        }
    });
    if let Some(to) = chosen {
        st.add_as_rom(open.from, to);
        keep = false;
    }
    if close || !keep {
        ctx.data_mut(|d| d.remove::<Open>(state_id()));
    } else {
        ctx.data_mut(|d| d.insert_temp(state_id(), open));
    }
    chosen
}

#[cfg(test)]
mod tests {
    use crate::model::{EmuRef, Game};

    fn emu(id: crate::model::GameId, platform: &str) -> Game {
        let mut g = Game::blank(&format!("G{id}"));
        g.id = id;
        g.emu = Some(EmuRef { platform: platform.into(), ..Default::default() });
        g
    }

    /// "Add to another game as a ROM…" is offered only when there is a game of the same
    /// console to add it to (hidden ones don't count).
    #[test]
    fn add_as_rom_needs_another_game_of_the_same_console() {
        let mut hidden = emu(3, "sony_playstation2");
        hidden.hidden = true;
        let lib = crate::library::Library::new(vec![emu(1, "sony_playstation2"), emu(2, "nintendo_snes"), hidden]);
        assert!(!super::has_target(&lib, &lib.games[0]));
        let lib = crate::library::Library::new(vec![emu(1, "sony_playstation2"), emu(2, "sony_playstation2")]);
        assert!(super::has_target(&lib, &lib.games[0]));
    }
}
