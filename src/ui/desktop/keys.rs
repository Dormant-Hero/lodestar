//! Keyboard shortcuts of the library (the explorer's own keys are in explorer/nav.rs).

use super::*;

impl DesktopUi {
    /// A dialog or a picker has the keyboard: the library's keys wait.
    pub(super) fn keyboard_taken(&self, ctx: &egui::Context, st: &AppState) -> bool {
        self.dlg.any_open()
            || self.coll_dlg.is_some()
            || crate::ui::launching::switch_ask(st).is_some()
            // "Choose what to play" / "Add to another game as a ROM": the arrows move between
            // their lines, Enter starts the focused one, nothing acts on the library behind.
            || emulation_ui::choose::is_open(ctx)
            || emulation_ui::addrom::is_open(ctx)
    }

    pub(super) fn handle_keys(&mut self, ctx: &egui::Context, st: &mut AppState) {
        if self.keyboard_taken(ctx, st) {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::F)) {
            self.focus_search = true;
        }
        if ctx.text_edit_focused() || ctx.any_popup_open() {
            return;
        }
        // Esc = Done in Arrange mode.
        if let Some(a) = &mut self.arrange
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            a.done = true;
        }
        // ← to the explorer and the keys inside it (Esc there returns to the games first).
        if self.sidebar_keys(ctx, st) {
            return;
        }
        // The Statistics page: Esc goes back to the games; the games' keys wait until then.
        if self.stats.is_some() {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                self.stats = None;
            }
            return;
        }
        self.library_keys(ctx, st);
        if self.groups.items.is_empty() {
            return;
        }
        self.move_keys(ctx, st);
        self.selected_game_keys(ctx);
    }

    /// Insert (Add game), Esc (multi-selection, then search), Ctrl+A, Delete (hide the
    /// multi-selection), Ctrl+R (random game).
    fn library_keys(&mut self, ctx: &egui::Context, st: &AppState) {
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Insert)) {
            self.open_add_menu = true;
        }
        // Esc clears a multi-selection first, then an active search.
        if !self.multi.is_empty() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.multi.clear();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::A)) && self.view.len() > 1 {
            self.multi = self.view.iter().map(|&i| st.library.games[i].id).collect();
        }
        // Delete hides the whole multi-selection.
        if self.multi.len() > 1 && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Delete)) {
            let ids = self.multi_ids(st);
            self.actions.push(Action::HideMany(ids));
        }
        // Esc outside the search box clears an active search.
        if !self.filter.search.is_empty() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.filter.search.clear();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::R)) {
            self.actions.push(Action::RandomGame);
        }
    }

    /// Arrows, Page Up / Down, Home / End: move the selection (across group headers: the same
    /// column in the group above / below).
    fn move_keys(&mut self, ctx: &egui::Context, st: &AppState) {
        let n = self.groups.items.len();
        let cur = self.sel_pos.unwrap_or(0);
        let spans = &self.groups.spans;
        let grid = st.config.desktop.view == DesktopView::Grid;
        let cols = if grid { self.grid_cols.max(1) } else { 1 };
        let page = if grid { cols * self.page_rows.max(1) } else { self.page_rows.max(1) };
        let mut target: Option<usize> = None;
        ctx.input_mut(|i| {
            let m = Modifiers::NONE;
            if grid && i.consume_key(m, Key::ArrowLeft) {
                target = Some(cur.saturating_sub(1));
            }
            if grid && i.consume_key(m, Key::ArrowRight) {
                target = Some((cur + 1).min(n - 1));
            }
            if i.consume_key(m, Key::ArrowUp)
                && let Some(t) = grouping::grid_move(spans, n, cols, cur, false)
            {
                target = Some(t);
            }
            if i.consume_key(m, Key::ArrowDown)
                && let Some(t) = grouping::grid_move(spans, n, cols, cur, true)
            {
                target = Some(t);
            }
            if i.consume_key(m, Key::PageUp) {
                target = Some(cur.saturating_sub(page));
            }
            if i.consume_key(m, Key::PageDown) {
                target = Some((cur + page).min(n - 1));
            }
            if i.consume_key(m, Key::Home) {
                target = Some(0);
            }
            if i.consume_key(m, Key::End) {
                target = Some(n - 1);
            }
        });
        let Some(t) = target else {
            return;
        };
        self.groups.touched = true;
        // Moving with the keyboard leaves a multi-selection (like a plain click).
        self.multi.clear();
        self.select_pos(st, t);
        // A keyboard-focused button (after Tab) would otherwise also follow the arrow keys
        // (egui moves focus in that direction and scrolls to it), fighting our selection.
        ctx.memory_mut(|m| {
            if let Some(f) = m.focused() {
                m.surrender_focus(f);
            }
        });
    }

    /// Enter (play), F2 (rename), F3 (edit), Delete (hide) on the selected game.
    fn selected_game_keys(&mut self, ctx: &egui::Context) {
        let Some(id) = self.selected else { return };
        // A keyboard-focused widget (after Tab) gets Enter as a click; don't also launch.
        let widget_focused = ctx.memory(|m| m.focused()).is_some();
        ctx.input_mut(|i| {
            if !widget_focused && i.consume_key(Modifiers::NONE, Key::Enter) {
                self.actions.push(Action::Play(id));
            }
            if i.consume_key(Modifiers::NONE, Key::F2) {
                self.actions.push(Action::Rename(id));
            }
            if i.consume_key(Modifiers::NONE, Key::F3) {
                self.actions.push(Action::Edit(id, edit::Tab::General));
            }
            if i.consume_key(Modifiers::NONE, Key::Delete) {
                self.actions.push(Action::Hide(id));
            }
        });
    }
}
