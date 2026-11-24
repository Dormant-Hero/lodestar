//! Hiding games and "Not a game" with their Undo toast.

use super::*;

/// What an Undo toast takes back.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum UndoWhat {
    Hidden(Vec<GameId>),
    /// "Not a game" marks (and the marks they replaced).
    Marked(crate::exclude_app::MarkUndo),
    /// A rule added by "Not a game ▸ Everything named like …".
    Rule(crate::config::ExcludeRule),
    /// Completion statuses before "Set completion status" on a multi-selection.
    Completion(Vec<(GameId, Option<CompletionStatus>)>),
    /// User scores before right-click → User score.
    Score(Vec<(GameId, Option<u8>)>),
    /// The user's collections before a delete / remove-from-collection.
    Collections(Vec<crate::library::UserCollection>),
    /// A game's logo and background placements before Arrange mode.
    Arrange(GameId, Option<LogoPlacement>, Option<BackgroundPlacement>),
    /// Missing games removed from the library (Remove from library…).
    Removed(crate::missing_app::RemovedGames),
}

/// "Hid X — Undo" toast.
pub(crate) struct Undo {
    pub what: UndoWhat,
    pub text: String,
    /// The game to select again after Undo.
    pub focus: Option<GameId>,
    pub until: Instant,
}

impl DesktopUi {
    /// `gone` leave the view: keep the user's place (the next game that stays, else the one
    /// before) and drop them from the multi-selection.
    pub(super) fn step_off(&mut self, st: &AppState, gone: &[GameId]) {
        let gone: std::collections::HashSet<GameId> = gone.iter().copied().collect();
        // `get`: the view can be a frame behind the library (a settings page changed it).
        let id_at = |i: usize| st.library.games.get(i).map(|g| g.id);
        if let Some(cur) = self.selected
            && gone.contains(&cur)
            && let Some(p) = self.view.iter().position(|&i| id_at(i) == Some(cur))
        {
            let stays = |i: &&usize| id_at(**i).is_some_and(|id| !gone.contains(&id));
            let next = self.view[p + 1..].iter().find(stays).or_else(|| self.view[..p].iter().rev().find(stays));
            self.selected = next.and_then(|&i| id_at(i));
        }
        self.multi.retain(|id| !gone.contains(id));
        if self.multi.len() < 2 {
            self.multi.clear();
        }
    }

    pub(super) fn show_undo(&mut self, ctx: &egui::Context, what: UndoWhat, text: String, focus: Option<GameId>) {
        self.undo = Some(Undo { what, text, focus, until: Instant::now() + Duration::from_secs(7) });
        ctx.request_repaint_after(Duration::from_secs(7));
    }

    /// Hide games (the ones already hidden stay so) with an Undo toast.
    pub(super) fn hide_games(&mut self, ctx: &egui::Context, st: &mut AppState, ids: Vec<GameId>) {
        let ids: Vec<GameId> = ids.into_iter().filter(|&id| st.library.get(id).is_some_and(|g| !g.hidden)).collect();
        let Some(&first) = ids.first() else { return };
        // Undo selects the game that was focused (else the first one).
        let focus = self.selected.filter(|s| ids.contains(s)).unwrap_or(first);
        let text = match ids.len() {
            1 => format!("Hid “{}”", st.library.get(first).map(|g| g.display_name()).unwrap_or("")),
            n => format!("Hid {n} games"),
        };
        // Hidden games leave the view unless hidden ones are shown.
        if !st.config.desktop.show_hidden && self.lib_sel != LibSel::Hidden {
            self.step_off(st, &ids);
        }
        st.set_hidden_many(&ids, true);
        self.show_undo(ctx, UndoWhat::Hidden(ids), text, Some(focus));
    }

    /// "Not a game" with an Undo toast.
    pub(super) fn not_a_game(&mut self, ctx: &egui::Context, st: &mut AppState, ids: Vec<GameId>) {
        let Some(&first) = ids.first() else { return };
        let focus = self.selected.filter(|s| ids.contains(s)).unwrap_or(first);
        let text = match ids.len() {
            1 => format!(
                "Removed “{}” from the library (not a game)",
                st.library.get(first).map(|g| g.display_name()).unwrap_or("")
            ),
            n => format!("Removed {n} entries from the library (not games)"),
        };
        self.step_off(st, &ids);
        let undo = st.exclude_games(&ids);
        if !undo.is_empty() {
            self.show_undo(ctx, UndoWhat::Marked(undo), text, Some(focus));
        }
    }

    /// One completion status for several games, with an Undo toast that puts each one back.
    pub(super) fn set_completion_many(
        &mut self,
        ctx: &egui::Context,
        st: &mut AppState,
        ids: Vec<GameId>,
        c: Option<CompletionStatus>,
    ) {
        let before: Vec<(GameId, Option<CompletionStatus>)> = ids
            .iter()
            .filter_map(|id| st.library.get(*id))
            .filter(|g| g.edits.completion != c)
            .map(|g| (g.id, g.edits.completion))
            .collect();
        let Some(&(first, _)) = before.first() else { return };
        let set: Vec<(GameId, Option<CompletionStatus>)> = before.iter().map(|(id, _)| (*id, c)).collect();
        st.set_completion_many(&set);
        let who = match before.len() {
            1 => format!("“{}”", st.library.get(first).map(|g| g.display_name()).unwrap_or("")),
            n => format!("{n} games"),
        };
        let text = match c {
            Some(c) => format!("Set {who} to {}", c.label()),
            None => format!("Cleared the completion status of {who}"),
        };
        let focus = self.selected.filter(|s| ids.contains(s));
        self.show_undo(ctx, UndoWhat::Completion(before), text, focus);
    }

    /// The pattern dialog's rule: added (applies at once), with an Undo toast that removes it.
    pub(crate) fn rule_added(
        &mut self,
        ctx: &egui::Context,
        st: &mut AppState,
        rule: crate::config::ExcludeRule,
        from: GameId,
    ) {
        let ids: Vec<GameId> =
            crate::exclude::rule_matches(&rule, st.library.games.iter()).into_iter().map(|g| g.id).collect();
        if st.config.exclude.rules.contains(&rule) {
            // Undo would remove the user's earlier rule: nothing to add, nothing to undo.
            st.notify(format!("There already is a rule “{}”", rule.pattern.trim()));
            return;
        }
        self.step_off(st, &ids);
        let n = st.add_exclude_rule(rule.clone());
        let text = match n {
            0 => format!("Rule added: “{}”", rule.pattern.trim()),
            1 => format!("Removed 1 entry named like “{}” (not a game)", rule.pattern.trim()),
            n => format!("Removed {n} entries named like “{}” (not games)", rule.pattern.trim()),
        };
        self.show_undo(ctx, UndoWhat::Rule(rule), text, Some(from));
    }

    pub(super) fn undo_toast(&mut self, ctx: &egui::Context, st: &mut AppState) {
        let Some(u) = &self.undo else { return };
        if Instant::now() >= u.until {
            self.undo = None;
            return;
        }
        let pal = self.pal;
        let mut undo_clicked = false;
        let mut close = false;
        egui::Area::new(egui::Id::unique("ls_undo"))
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -40.0])
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
                            ui.label(egui::RichText::new(&u.text).color(pal.text));
                            ui.add_space(8.0);
                            if ui
                                .add(
                                    egui::Button::new(egui::RichText::new("Undo").color(pal.selection).strong())
                                        .frame(false),
                                )
                                .clicked()
                            {
                                undo_clicked = true;
                            }
                            if crate::ui::widgets::icon_button(ui, crate::ui::widgets::Icon::Close, false, 20.0, "")
                                .clicked()
                            {
                                close = true;
                            }
                        });
                    });
            });
        if undo_clicked {
            let Some(u) = self.undo.take() else { return };
            match &u.what {
                UndoWhat::Hidden(ids) => st.set_hidden_many(ids, false),
                UndoWhat::Marked(u) => st.undo_marks(u),
                UndoWhat::Rule(rule) => st.remove_exclude_rule(rule),
                UndoWhat::Completion(before) => st.set_completion_many(before),
                UndoWhat::Score(before) => st.set_user_score_many(before),
                UndoWhat::Collections(before) => self.restore_collections(st, before.clone()),
                UndoWhat::Arrange(id, logo, bg) => details::save_placements(st, *id, *logo, *bg),
                UndoWhat::Removed(r) => st.restore_removed(r),
            }
            if let Some(id) = u.focus {
                self.selected = Some(id);
                // Recompute the view (and the selection's place in it) next frame.
                self.view_key = None;
            }
            self.scroll_to_sel = true;
        } else if close {
            self.undo = None;
        }
    }
}
