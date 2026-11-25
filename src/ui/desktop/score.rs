//! User score from the right-click menus: five stars mapped to 20..100 (Edit Game Details
//! keeps the exact 0–100 value), for one game or a multi-selection, with one Undo.

use super::*;
use crate::ui::widgets::{mix, star, star_outline};

/// Stars shown for a 0..=100 score (nearest of 1..=5; 0 for not set).
pub(crate) fn stars_of(score: Option<u8>) -> u8 {
    score.map_or(0, |s| ((s as u32 + 10) / 20).clamp(1, 5) as u8)
}

/// The score a click on star `k` (1..=5) gives: k × 20, or not set when that star was lit.
pub(crate) fn score_for_click(cur: Option<u8>, k: u8) -> Option<u8> {
    (stars_of(cur) != k).then_some(k * 20)
}

/// The stars after `steps` presses of ← / → from `from` (0: not set).
pub(crate) fn step_stars(from: u8, steps: i32) -> u8 {
    (i32::from(from) + steps).clamp(0, 5) as u8
}

/// The common score of `ids` (None inside: not set; outer None: they differ).
pub(crate) fn common_score(lib: &crate::library::Library, ids: &[GameId]) -> Option<Option<u8>> {
    let mut it = ids.iter().filter_map(|id| lib.get(*id)).map(|g| g.edits.user_score);
    let first = it.next()?;
    it.all(|s| s == first).then_some(first)
}

/// "User score  ☆☆☆☆☆" menu row for `ids` whose score is `cur` (outer None: they differ).
/// Hovering lights the stars up to the pointer. The controller steps the stars the way a screen
/// reader steps a slider (pad.rs) and sets them with A.
pub(crate) fn score_row(
    ui: &mut egui::Ui,
    cur: Option<Option<u8>>,
    ids: &[GameId],
    pal: &Pal,
    actions: &mut Vec<Action>,
) {
    let lit = cur.map_or(0, stars_of);
    let star_w = 20.0;
    let h = ui.spacing().interact_size.y.max(22.0);
    // The menu's width so far (its widest entry): `available_width` would grow the menu.
    let w = ui.min_rect().width().max(200.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click());
    let stars_left = rect.right() - 5.0 * star_w - 4.0;
    let hover_k = resp
        .hover_pos()
        .and_then(|p| (p.x >= stars_left).then(|| (((p.x - stars_left) / star_w).floor() as u8 + 1).clamp(1, 5)));
    // The stars stepped to while the row has the focus (forgotten when it loses it).
    let pick_id = resp.id.with("pick");
    if !ui.memory(|m| m.has_focus(resp.id)) {
        ui.data_mut(|d| d.remove::<u8>(pick_id));
    }
    let mut pick: Option<u8> = ui.data(|d| d.get_temp(pick_id));
    let steps = ui.input(|i| {
        use egui::accesskit::Action;
        i.num_accesskit_action_requests(resp.id, Action::Increment) as i32
            - i.num_accesskit_action_requests(resp.id, Action::Decrement) as i32
    });
    if steps != 0 {
        let k = step_stars(pick.unwrap_or(lit), steps);
        ui.data_mut(|d| d.insert_temp(pick_id, k));
        pick = Some(k);
    }
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let text = ui.visuals().text_color();
    let label = match cur {
        Some(Some(s)) => format!("User score  {s}"),
        Some(None) => "User score".to_string(),
        None => "User score (mixed)".to_string(),
    };
    let gal = p.layout_no_wrap(label, egui::FontId::proportional(14.0), text);
    p.galley(egui::pos2(rect.left() + 4.0, rect.center().y - gal.size().y / 2.0), gal, text);
    let shown = hover_k.or(pick).unwrap_or(lit);
    for k in 1..=5u8 {
        let c = egui::pos2(stars_left + (k as f32 - 0.5) * star_w, rect.center().y);
        if k <= shown {
            let col = if hover_k.is_some() { mix(pal.fav, pal.text, 0.25) } else { pal.fav };
            star(p, c, 7.0, col);
        } else {
            star_outline(p, c, 7.0, egui::Stroke::new(1.2, pal.dim));
        }
    }
    resp.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(shown), "User score"));
    let resp = resp.on_hover_text("Click a star to rate (★ = 20 … ★★★★★ = 100); the same star again clears it");
    let new = match (hover_k, pick) {
        (Some(k), _) if cur.is_none() => Some(Some(k * 20)),
        (Some(k), _) => Some(score_for_click(cur.flatten(), k)),
        (None, Some(k)) => Some((k > 0).then_some(k * 20)),
        (None, None) => None,
    };
    if resp.clicked()
        && let Some(new) = new
    {
        actions.push(Action::SetScore(ids.to_vec(), new));
        ui.close();
    }
}

impl DesktopUi {
    /// One score for several games, with an Undo toast that puts each one back.
    pub(super) fn set_score_many(
        &mut self,
        ctx: &egui::Context,
        st: &mut AppState,
        ids: Vec<GameId>,
        score: Option<u8>,
    ) {
        let before: Vec<(GameId, Option<u8>)> = ids
            .iter()
            .filter_map(|id| st.library.get(*id))
            .filter(|g| g.edits.user_score != score)
            .map(|g| (g.id, g.edits.user_score))
            .collect();
        let Some(&(first, _)) = before.first() else { return };
        let set: Vec<(GameId, Option<u8>)> = before.iter().map(|(id, _)| (*id, score)).collect();
        st.set_user_score_many(&set);
        let who = match before.len() {
            1 => format!("“{}”", st.library.get(first).map(|g| g.display_name()).unwrap_or("")),
            n => format!("{n} games"),
        };
        let text = match score {
            Some(s) => {
                let n = stars_of(Some(s));
                format!("Rated {who} {n} star{}", if n == 1 { "" } else { "s" })
            }
            None => format!("Cleared the score of {who}"),
        };
        let focus = self.selected.filter(|s| ids.contains(s));
        self.show_undo(ctx, UndoWhat::Score(before), text, focus);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stars_map_to_twenty_steps() {
        assert_eq!(stars_of(None), 0);
        assert_eq!(stars_of(Some(0)), 1, "a set score shows at least one star");
        assert_eq!(stars_of(Some(20)), 1);
        assert_eq!(stars_of(Some(73)), 4);
        assert_eq!(stars_of(Some(100)), 5);
        assert_eq!(score_for_click(None, 4), Some(80));
        assert_eq!(score_for_click(Some(80), 4), None, "the lit star again clears");
        assert_eq!(score_for_click(Some(75), 4), None);
        assert_eq!(score_for_click(Some(80), 5), Some(100));
    }

    #[test]
    fn stepped_stars_stay_between_none_and_five() {
        assert_eq!(step_stars(0, 1), 1);
        assert_eq!(step_stars(3, -1), 2);
        assert_eq!(step_stars(1, -2), 0);
        assert_eq!(step_stars(5, 1), 5);
    }
}
