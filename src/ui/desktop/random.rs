//! The quick presets' extra narrowing (Recent, Most played) and the random game button with
//! its "Pick another" toast.

use super::*;
use crate::library::Library;
use std::sync::atomic::{AtomicU64, Ordering};

/// The toast after a random pick.
pub(crate) struct RandomPick {
    pub id: GameId,
    pub until: Instant,
}

const TOAST_FOR: Duration = Duration::from_secs(10);

/// Recent: played games only; Most played: games with play time (both are then sorted by
/// `effective_sort`). The other presets are plain filter flags.
pub(crate) fn preset_retain(p: QuickPreset, lib: &Library, v: &mut Vec<usize>) {
    match p {
        QuickPreset::Recent => v.retain(|&i| lib.games[i].last_played.is_some()),
        QuickPreset::MostPlayed => v.retain(|&i| lib.games[i].playtime_secs > 0),
        _ => {}
    }
}

/// A random number below `n` (xorshift seeded from the clock; no crate needed).
fn random_below(n: usize) -> usize {
    static STATE: AtomicU64 = AtomicU64::new(0);
    let mut x = STATE.load(Ordering::Relaxed);
    if x == 0 {
        x = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9E37_79B9_7F4A_7C15, |d| d.as_nanos() as u64)
            | 1;
    }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    STATE.store(x, Ordering::Relaxed);
    (x % n.max(1) as u64) as usize
}

/// Pick from `view` (indices into the library), never the game already selected when there
/// is another one.
pub(crate) fn pick(lib: &Library, view: &[usize], current: Option<GameId>, r: usize) -> Option<GameId> {
    let ids: Vec<GameId> = view
        .iter()
        .filter_map(|&i| lib.games.get(i).map(|g| g.id))
        .filter(|&id| view.len() < 2 || Some(id) != current)
        .collect();
    ids.get(r % ids.len().max(1)).copied()
}

impl DesktopUi {
    /// Select a random game of the view (opening its group), scroll to it, show the toast.
    pub(crate) fn random_game(&mut self, ctx: &egui::Context, st: &mut AppState) {
        let Some(id) = pick(&st.library, &self.view, self.selected, random_below(self.view.len())) else {
            st.notify("No games to pick from");
            return;
        };
        self.reveal(st, id);
        self.multi.clear();
        self.selected = Some(id);
        self.sel_pos = None;
        self.regroup_soon();
        self.scroll_to_sel = true;
        self.random = Some(RandomPick { id, until: Instant::now() + TOAST_FOR });
        ctx.request_repaint_after(TOAST_FOR);
    }

    pub(super) fn random_toast(&mut self, ctx: &egui::Context, st: &mut AppState) {
        let Some(r) = &self.random else { return };
        if Instant::now() >= r.until || self.selected != Some(r.id) {
            self.random = None;
            return;
        }
        let Some(name) = st.library.get(r.id).map(|g| g.display_name().to_string()) else {
            self.random = None;
            return;
        };
        let pal = self.pal;
        // Above the Undo toast when both show.
        let y = if self.undo.is_some() { -92.0 } else { -40.0 };
        let mut again = false;
        let mut close = false;
        egui::Area::new(egui::Id::unique("ls_random"))
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, y])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(pal.surface_alt)
                    .stroke(egui::Stroke::new(1.0, pal.border))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(14, 8))
                    .shadow(ui.visuals().popup_shadow)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (ir, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                            crate::ui::widgets::paint_icon(ui.painter(), ir, crate::ui::widgets::Icon::Dice, pal.dim);
                            ui.label(egui::RichText::new(format!("Picked “{name}”")).color(pal.text));
                            ui.add_space(8.0);
                            again = ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("Pick another").color(pal.selection).strong(),
                                    )
                                    .frame(false),
                                )
                                .on_hover_text("Ctrl+R")
                                .clicked();
                            close =
                                crate::ui::widgets::icon_button(ui, crate::ui::widgets::Icon::Close, false, 20.0, "")
                                    .clicked();
                        });
                    });
            });
        if again {
            self.push(Action::RandomGame);
        } else if close {
            self.random = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::desktop::tests::fake_game;

    #[test]
    fn random_pick_skips_the_selected_game() {
        let lib = Library::new((1..=3).map(|i| fake_game(i, &format!("G{i}"), Source::Steam)).collect());
        let view = vec![0, 1, 2];
        for r in 0..20 {
            assert_ne!(pick(&lib, &view, Some(2), r), Some(2));
        }
        assert_eq!(pick(&lib, &[1], Some(2), 5), Some(2), "the only game can be picked again");
        assert_eq!(pick(&lib, &[], None, 0), None);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            seen.insert(random_below(3));
        }
        assert_eq!(seen.len(), 3);
    }

    #[test]
    fn presets_narrow_to_played_games() {
        // Ids 15..=22: 16 and 20 were played lately, 17 has no play time.
        let lib = Library::new((15..=22).map(|i| fake_game(i, &format!("G{i}"), Source::Steam)).collect());
        let mut v: Vec<usize> = (0..8).collect();
        preset_retain(QuickPreset::Recent, &lib, &mut v);
        assert!(v.iter().all(|&i| lib.games[i].last_played.is_some()) && !v.is_empty());
        let mut v: Vec<usize> = (0..8).collect();
        preset_retain(QuickPreset::MostPlayed, &lib, &mut v);
        assert!(v.iter().all(|&i| lib.games[i].playtime_secs > 0) && v.len() < 8);
        let mut v: Vec<usize> = (0..8).collect();
        preset_retain(QuickPreset::Favorites, &lib, &mut v);
        assert_eq!(v.len(), 8, "flags are applied by the filter itself");
    }
}
