//! The explorer's "Collections" section (Steam-style): the user's manual collections (games
//! added by hand: right-click → Add to collection, or dragged onto one) and smart ones (a saved
//! filter that updates by itself). Rename / move / delete from a collection's right-click menu;
//! deleting and removing games can be undone. Stored with `library::user`.

use super::super::dialogs::{modal, plain_button, primary_button, title};
use super::super::{Action, DesktopUi, Facet, LibSel, Pal, artwork};
use super::nav::Entry;
use super::{row, section_games};
use crate::app::AppState;
use crate::library::UserCollection;
use crate::library::user::{self, SmartRule};
use crate::model::GameId;
use crate::ui::platform_icons::Shape;
use crate::ui::widgets::with_alpha;
use egui::{Key, Ui};

/// What the name prompt is for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DlgKind {
    /// A manual collection with these games in it.
    New(Vec<GameId>),
    /// A smart collection with this rule.
    Smart(SmartRule),
    Rename(u64),
}

/// The collection name prompt.
#[derive(Clone, Debug)]
pub(crate) struct CollDlg {
    pub kind: DlgKind,
    pub name: String,
    /// Frames left to (re)claim the keyboard: the right-click menu it was opened from can take
    /// the focus away while it closes.
    focus: u8,
}

/// Games being dragged from the grid / list (dropped on a collection).
#[derive(Clone, Debug)]
pub(crate) struct DragGames(pub Vec<GameId>);

/// Drag-and-drop payload for `id`: the multi-selection when the game is in it, else the game.
pub(crate) fn drag_ids(ui: &DesktopUi, lib: &crate::library::Library, view: &[usize], id: GameId) -> DragGames {
    if ui.in_multi(id) && ui.multi.len() > 1 { DragGames(ui.multi_ids_in(lib, view)) } else { DragGames(vec![id]) }
}

/// "Add to collection ▸" for `ids` (one game or a multi-selection): the manual collections
/// as check boxes (all of `ids` in it: checked; clicking takes them out again), then "New
/// collection…". `cur`: the manual collection being shown, which gets a "Remove from" entry.
pub(crate) fn collection_menu(
    ui: &mut Ui,
    colls: &[UserCollection],
    ids: &[GameId],
    cur: Option<u64>,
    actions: &mut Vec<Action>,
) {
    if let Some(c) = cur.and_then(|id| colls.iter().find(|c| c.id == id && !c.is_smart()))
        && ids.iter().any(|g| c.games.contains(g))
        && ui.button(format!("Remove from “{}”", short(&c.name))).clicked()
    {
        actions.push(Action::RemoveFromCollection(c.id, ids.to_vec()));
    }
    ui.menu_button("Add to collection", |ui| {
        ui.set_min_width(200.0);
        let mut any = false;
        for c in colls.iter().filter(|c| !c.is_smart()) {
            any = true;
            let mut all_in = ids.iter().all(|g| c.games.contains(g));
            if ui.checkbox(&mut all_in, short(&c.name)).clicked() {
                actions.push(if all_in {
                    Action::AddToCollection(c.id, ids.to_vec())
                } else {
                    Action::RemoveFromCollection(c.id, ids.to_vec())
                });
            }
        }
        if any {
            ui.separator();
        }
        if ui.button("New collection…").clicked() {
            actions.push(Action::NewCollection(ids.to_vec()));
            ui.close();
        }
    });
}

/// A collection name short enough for a menu entry.
fn short(name: &str) -> String {
    const MAX: usize = 32;
    if name.chars().count() <= MAX {
        name.to_string()
    } else {
        let mut s: String = name.chars().take(MAX - 1).collect();
        s.push('…');
        s
    }
}

impl DesktopUi {
    /// The current explorer filter as a smart-collection rule (empty: nothing to save).
    pub(crate) fn current_rule(&self, st: &AppState) -> SmartRule {
        let mut r = self
            .coll_sel
            .and_then(|id| st.library.user_collection(id))
            .and_then(|c| c.rule.clone())
            .unwrap_or_default();
        let not_installed = self.lib_sel == LibSel::NotInstalled;
        r.installed |= self.lib_sel == LibSel::Installed
            || (st.config.desktop.preset == crate::config::QuickPreset::Installed && !not_installed);
        r.not_installed |= not_installed;
        if self.can_match_all() {
            r.match_all = Some(self.match_all);
        }
        r.favorites |= self.lib_sel == LibSel::Favorites;
        r.played |= self.lib_sel == LibSel::Recent;
        for s in &self.src_sel {
            let k = s.0.to_string();
            if !r.sources.contains(&k) {
                r.sources.push(k);
            }
        }
        for t in &self.filter.tags {
            if !r.tags.iter().any(|x| crate::library::tag_eq(x, t)) {
                r.tags.push(t.clone());
            }
        }
        for f in Facet::ALL {
            let sel = &self.facet_sel[f.index()];
            if !sel.is_empty() {
                let e = r.fields.entry(f.key().to_string()).or_default();
                for v in sel {
                    if !e.contains(v) {
                        e.push(v.clone());
                    }
                }
            }
        }
        let q = self.filter.search.trim();
        if !q.is_empty() {
            r.search = q.to_string();
        }
        r
    }

    /// Show collection `id` (None, or the one already shown: back to All games).
    fn select_collection(&mut self, id: Option<u64>) {
        let again = id.is_some() && id == self.coll_sel;
        self.set_lib_sel(LibSel::All);
        if !again {
            self.coll_sel = id;
        }
    }

    /// Store a new list of collections (memory + DB).
    fn commit_collections(st: &mut AppState, colls: Vec<UserCollection>) {
        user::save(&st.db, &colls);
        st.library.set_user_collections(colls);
    }

    /// Undo: put the collections back as they were.
    pub(crate) fn restore_collections(&mut self, st: &mut AppState, colls: Vec<UserCollection>) {
        Self::commit_collections(st, colls);
    }

    pub(crate) fn collection_action(&mut self, ctx: &egui::Context, st: &mut AppState, a: Action) {
        let mut colls = st.library.user.clone();
        let name_of = |colls: &[UserCollection], id: u64| {
            colls.iter().find(|c| c.id == id).map(|c| c.name.clone()).unwrap_or_default()
        };
        match a {
            Action::SelectCollection(id) => self.select_collection(id),
            Action::AddToCollection(id, ids) => {
                let n = user::add_games(&mut colls, id, &ids);
                if n > 0 {
                    let name = name_of(&colls, id);
                    Self::commit_collections(st, colls);
                    st.notify(format!("Added {} to “{name}”", super::super::games_word(n)));
                } else {
                    st.notify("Already in that collection");
                }
            }
            Action::RemoveFromCollection(id, ids) => {
                let before = colls.clone();
                let n = user::remove_games(&mut colls, id, &ids);
                if n > 0 {
                    let text = format!("Removed {} from “{}”", super::super::games_word(n), name_of(&colls, id));
                    if self.coll_sel == Some(id) {
                        self.step_off(st, &ids);
                    }
                    Self::commit_collections(st, colls);
                    self.show_undo(ctx, super::super::UndoWhat::Collections(before), text, self.selected);
                }
            }
            Action::NewCollection(ids) => {
                self.coll_dlg = Some(CollDlg {
                    name: user::unique_name(&colls, "New collection", None),
                    kind: DlgKind::New(ids),
                    focus: 4,
                });
            }
            Action::NewSmartCollection => {
                let rule = self.current_rule(st);
                if rule.is_empty() {
                    st.notify("Pick a filter first (a source, platform, genre, search …)");
                } else {
                    self.coll_dlg = Some(CollDlg {
                        name: user::unique_name(&colls, &self.suggest_name(&rule), None),
                        kind: DlgKind::Smart(rule),
                        focus: 4,
                    });
                }
            }
            Action::RenameCollection(id) => {
                self.coll_dlg = Some(CollDlg { name: name_of(&colls, id), kind: DlgKind::Rename(id), focus: 4 });
            }
            Action::DeleteCollection(id) => {
                let before = colls.clone();
                let name = name_of(&colls, id);
                colls.retain(|c| c.id != id);
                if colls.len() != before.len() {
                    if self.coll_sel == Some(id) {
                        self.set_lib_sel(LibSel::All);
                    }
                    Self::commit_collections(st, colls);
                    self.show_undo(
                        ctx,
                        super::super::UndoWhat::Collections(before),
                        format!("Deleted the collection “{name}”"),
                        self.selected,
                    );
                }
            }
            Action::MoveCollection(id, delta) => {
                if user::move_by(&mut colls, id, delta) {
                    Self::commit_collections(st, colls);
                }
            }
            Action::UpdateSmartCollection(id) => {
                // The rule as shown right now (a smart collection being shown is its base).
                let rule = self.current_rule(st);
                if rule.is_empty() {
                    st.notify("Pick a filter first (a source, platform, genre, search …)");
                } else if let Some(c) = colls.iter_mut().find(|c| c.id == id && c.is_smart()) {
                    c.rule = Some(rule);
                    let name = c.name.clone();
                    Self::commit_collections(st, colls);
                    // Show it (never toggled off, even when it is the one shown already).
                    self.set_lib_sel(LibSel::All);
                    self.coll_sel = Some(id);
                    st.notify(format!("“{name}” now shows this filter"));
                }
            }
            _ => {}
        }
    }

    /// A name for a smart collection made from `rule` ("RPG", "PS2", "Steam · Installed").
    fn suggest_name(&self, rule: &SmartRule) -> String {
        let mut parts: Vec<String> = vec![];
        for s in &self.src_sel {
            parts.push(s.label().to_string());
        }
        for (label, v) in self.active_filters() {
            if matches!(v, super::super::FilterValue::Facet(..) | super::super::FilterValue::Tag(_)) {
                let shown = label.split_once(": ").map_or(label.as_str(), |(_, x)| x);
                let shown = match v {
                    super::super::FilterValue::Facet(Facet::Platform, _) => {
                        crate::ui::platform_icons::short_label(shown).to_string()
                    }
                    _ => shown.to_string(),
                };
                parts.push(shown);
            }
        }
        if !rule.search.is_empty() {
            parts.push(format!("“{}”", rule.search));
        }
        if self.lib_sel == LibSel::Favorites {
            parts.push("Favourites".into());
        }
        if self.lib_sel == LibSel::Recent {
            parts.push("Played".into());
        }
        // ("Installed only" from the top bar is in the rule but not worth naming.)
        if self.lib_sel == LibSel::Installed {
            parts.push("Installed".into());
        }
        if self.lib_sel == LibSel::NotInstalled {
            parts.push("Not installed".into());
        }
        parts.truncate(3);
        if parts.is_empty() { "Smart collection".into() } else { parts.join(" · ") }
    }

    /// The name prompt (New collection… / New smart collection… / Rename…).
    pub(crate) fn collection_dialog(&mut self, ctx: &egui::Context, st: &mut AppState) {
        let Some(d) = &mut self.coll_dlg else { return };
        let pal = self.pal;
        let fam = self.fonts.bold.clone();
        let (head, sub, ok) = match &d.kind {
            DlgKind::New(ids) if ids.is_empty() => ("New collection", None, "Create"),
            DlgKind::New(ids) => {
                ("New collection", Some(format!("With {}", super::super::games_word(ids.len()))), "Create")
            }
            DlgKind::Smart(_) => (
                "New smart collection",
                Some("Shows every game that matches the current filter, now and later".into()),
                "Create",
            ),
            DlgKind::Rename(_) => ("Rename collection", None, "Save"),
        };
        let mut done: Option<bool> = None;
        let (_, close) = modal(ctx, "ls_coll_dlg", &pal, 400.0, |ui| {
            title(ui, head, sub.as_deref(), &pal, &fam);
            let r = ui.add(
                egui::TextEdit::singleline(&mut d.name)
                    .desired_width(f32::INFINITY)
                    .margin(egui::Margin::symmetric(8, 6)),
            );
            if d.focus > 0 {
                d.focus -= 1;
            }
            if d.focus > 0 && !r.has_focus() {
                r.request_focus();
                let mut state = egui::widgets::text_edit::TextEditState::load(ui.ctx(), r.id).unwrap_or_default();
                state.cursor.set_char_range(Some(egui::text::CCursorRange::select_all_str(&d.name)));
                state.store(ui.ctx(), r.id);
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let valid = !d.name.trim().is_empty();
                if primary_button(ui, ok, &pal, valid).clicked() || (enter && valid) {
                    done = Some(true);
                }
                if plain_button(ui, "Cancel").clicked() {
                    done = Some(false);
                }
            });
        });
        if close {
            done = Some(false);
        }
        let Some(ok) = done else { return };
        let Some(d) = self.coll_dlg.take() else { return };
        if ok {
            self.finish_dialog(st, d);
        }
    }

    /// The name prompt's OK: create / rename.
    fn finish_dialog(&mut self, st: &mut AppState, d: CollDlg) {
        let mut colls = st.library.user.clone();
        match d.kind {
            DlgKind::Rename(id) => {
                let name = user::unique_name(&colls, &d.name, Some(id));
                if let Some(c) = colls.iter_mut().find(|c| c.id == id) {
                    c.name = name;
                    Self::commit_collections(st, colls);
                }
            }
            DlgKind::New(ids) => {
                let id = user::next_id(&colls);
                let name = user::unique_name(&colls, &d.name, None);
                let n = ids.len();
                colls.push(UserCollection { id, name: name.clone(), games: ids, rule: None });
                Self::commit_collections(st, colls);
                self.collapsed.retain(|k| k != "collections");
                if n > 0 {
                    st.notify(format!("Added {} to “{name}”", super::super::games_word(n)));
                }
            }
            DlgKind::Smart(rule) => {
                let id = user::next_id(&colls);
                colls.push(UserCollection {
                    id,
                    name: user::unique_name(&colls, &d.name, None),
                    games: vec![],
                    rule: Some(rule),
                });
                Self::commit_collections(st, colls);
                self.collapsed.retain(|k| k != "collections");
                self.select_collection(Some(id));
            }
        }
    }

    /// The "COLLECTIONS" explorer section (rows, drop targets, right-click menus).
    pub(super) fn collections_section(&mut self, ui: &mut Ui, st: &mut AppState, pal: &Pal) {
        let n = self.counts.colls.len();
        let (open, header) = self.section_resp(ui, "COLLECTIONS", "collections", n, pal);
        let mut acts: Vec<Action> = vec![];
        let can_smart = !self.current_rule(st).is_empty();
        header.context_menu(|ui| {
            ui.set_min_width(240.0);
            if ui.button("New collection…").clicked() {
                acts.push(Action::NewCollection(vec![]));
            }
            if ui
                .add_enabled(can_smart, egui::Button::new("New smart collection from this filter…"))
                .on_disabled_hover_text("Pick a source, platform, genre … or search first")
                .clicked()
            {
                acts.push(Action::NewSmartCollection);
            }
        });
        let dragging = egui::DragAndDrop::has_payload_of_type::<DragGames>(ui.ctx());
        if !open {
            // A collapsed section still takes a drop on its header.
            if let Some(p) = header.dnd_release_payload::<DragGames>() {
                acts.push(Action::NewCollection(p.0.clone()));
            }
            self.actions.extend(acts);
            return;
        }
        if n == 0 && !dragging {
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                ui.label(egui::RichText::new("Right-click a game, then Add to collection").size(12.0).color(pal.dim));
            });
        }
        let colls = self.counts.colls.clone();
        let last = colls.len().saturating_sub(1);
        for (i, (id, name, count, smart)) in colls.iter().enumerate() {
            let (id, count, smart) = (*id, *count, *smart);
            let foc = self.side_nav.is(|e| *e == Entry::Coll(id));
            let shape = if smart { Shape::Smart } else { Shape::Collection };
            let r = row(ui, name, count, self.coll_sel == Some(id), foc, pal, Some(shape));
            if foc {
                self.side_nav.reveal(&r);
            }
            // Manual collections take games dragged from the grid / list.
            if !smart {
                if r.dnd_hover_payload::<DragGames>().is_some() {
                    ui.painter().rect_stroke(
                        r.rect.shrink2(egui::vec2(6.0, 0.0)),
                        4.0,
                        egui::Stroke::new(2.0, pal.accent),
                        egui::StrokeKind::Inside,
                    );
                }
                if let Some(p) = r.dnd_release_payload::<DragGames>() {
                    acts.push(Action::AddToCollection(id, p.0.clone()));
                }
            }
            if r.clicked() {
                acts.push(Action::SelectCollection(Some(id)));
            }
            let r =
                if smart { r.on_hover_text("Smart collection: shows every game that matches its filter") } else { r };
            r.context_menu(|ui| {
                ui.set_min_width(220.0);
                if ui.button("Rename…").clicked() {
                    acts.push(Action::RenameCollection(id));
                }
                if smart
                    && ui
                        .add_enabled(can_smart, egui::Button::new("Use the current filter"))
                        .on_hover_text("Replace what this collection shows with the filter you have on now")
                        .clicked()
                {
                    acts.push(Action::UpdateSmartCollection(id));
                }
                if ui.add_enabled(i > 0, egui::Button::new("Move up")).clicked() {
                    acts.push(Action::MoveCollection(id, -1));
                }
                if ui.add_enabled(i < last, egui::Button::new("Move down")).clicked() {
                    acts.push(Action::MoveCollection(id, 1));
                }
                ui.separator();
                if count > 0
                    && ui.button(format!("Download artwork for {}…", super::super::games_word(count))).clicked()
                    && let Some(u) = st.library.user_collection(id)
                {
                    let ids = section_games(st, |g| !g.hidden && u.contains(g));
                    acts.push(Action::BulkArt(artwork::BulkScope::Section(format!("“{}”", u.name), ids)));
                }
                if ui.button("Delete collection").clicked() {
                    acts.push(Action::DeleteCollection(id));
                }
            });
        }
        if dragging {
            // Drop here to start a new collection with the dragged games.
            let r = row(ui, "New collection…", 0, false, false, pal, Some(Shape::Collection));
            if r.dnd_hover_payload::<DragGames>().is_some() {
                ui.painter().rect_stroke(
                    r.rect.shrink2(egui::vec2(6.0, 0.0)),
                    4.0,
                    egui::Stroke::new(2.0, pal.accent),
                    egui::StrokeKind::Inside,
                );
            }
            if let Some(p) = r.dnd_release_payload::<DragGames>() {
                acts.push(Action::NewCollection(p.0.clone()));
            }
        }
        self.actions.extend(acts);
    }
}

/// While games are dragged: "N games" next to the pointer.
pub(crate) fn drag_ghost(ctx: &egui::Context, st: &AppState, pal: &Pal) {
    let Some(p) = egui::DragAndDrop::payload::<DragGames>(ctx) else {
        return;
    };
    let Some(pos) = ctx.pointer_latest_pos() else {
        return;
    };
    let text = match p.0.as_slice() {
        [one] => st.library.get(*one).map_or_else(|| "1 game".to_string(), |g| g.display_name().to_string()),
        ids => super::super::games_word(ids.len()),
    };
    egui::Area::new(egui::Id::unique("ls_drag_ghost"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pos + egui::vec2(14.0, 10.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(with_alpha(pal.surface_alt, 240))
                .stroke(egui::Stroke::new(1.0, pal.accent))
                .corner_radius(6)
                .inner_margin(egui::Margin::symmetric(10, 5))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(text).size(13.0).color(pal.text));
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ScannedGame, Source};

    fn setup(dir: &std::path::Path) -> (AppState, Vec<GameId>) {
        let mut st = AppState::for_test(dir);
        let scanned: Vec<ScannedGame> = ["A", "B", "C", "D"]
            .iter()
            .map(|n| ScannedGame::new(Source::Desktop, format!("{n}.desktop"), *n))
            .collect();
        crate::db::Db::open(&st.paths.db_file()).unwrap().merge_scan(&scanned, &[Source::Desktop]).unwrap();
        st.reload_library();
        let ids =
            ["A", "B", "C", "D"].iter().map(|n| st.library.games.iter().find(|g| g.name == *n).unwrap().id).collect();
        (st, ids)
    }

    fn view_names(ui: &DesktopUi, st: &AppState) -> Vec<String> {
        ui.view.iter().map(|&i| st.library.games[i].name.clone()).collect()
    }

    #[test]
    fn manual_collections_add_remove_delete_and_undo() {
        let dir = tempfile::tempdir().unwrap();
        let (mut st, ids) = setup(dir.path());
        let ctx = st.ctx().clone();
        let mut ui = DesktopUi::new();
        ui.push(Action::NewCollection(vec![ids[0], ids[1]]));
        ui.apply_actions(&ctx, &mut st);
        let d = ui.coll_dlg.take().expect("name prompt");
        assert_eq!(d.name, "New collection");
        ui.finish_dialog(&mut st, CollDlg { name: "Co-op".into(), ..d });
        let id = st.library.user[0].id;
        assert_eq!(st.library.user[0].games, [ids[0], ids[1]]);
        // Shown like a Library entry; the filter chip names it.
        ui.push(Action::SelectCollection(Some(id)));
        ui.push(Action::AddToCollection(id, vec![ids[3]]));
        ui.apply_actions(&ctx, &mut st);
        ui.sync_filter(&st);
        ui.refresh_view_for(&st.library);
        assert_eq!(view_names(&ui, &st), ["A", "B", "D"]);
        ui.counts = super::super::super::Counts::compute(&st.library);
        assert!(ui.active_filters().iter().any(|(l, _)| l == "Collection: Co-op"));
        // Remove with Undo.
        ui.push(Action::RemoveFromCollection(id, vec![ids[1]]));
        ui.apply_actions(&ctx, &mut st);
        assert_eq!(st.library.user[0].games, [ids[0], ids[3]]);
        let Some(super::super::super::Undo { what: super::super::super::UndoWhat::Collections(before), .. }) =
            ui.undo.take()
        else {
            panic!("undo")
        };
        ui.restore_collections(&mut st, before);
        assert_eq!(st.library.user[0].games, [ids[0], ids[1], ids[3]], "back in its place");
        // Stored in the DB.
        assert_eq!(crate::library::user::load(&st.db), st.library.user);
        // Delete: back to All games; Undo puts it back.
        ui.push(Action::DeleteCollection(id));
        ui.apply_actions(&ctx, &mut st);
        assert!(st.library.user.is_empty() && ui.coll_sel.is_none());
        assert!(crate::library::user::load(&st.db).is_empty());
        let Some(super::super::super::Undo { what: super::super::super::UndoWhat::Collections(before), .. }) =
            ui.undo.take()
        else {
            panic!("undo")
        };
        ui.restore_collections(&mut st, before);
        assert_eq!(st.library.user.len(), 1);
        // Controller mode lists it as a collection.
        assert!(st.library.collections().iter().any(|c| c.key() == format!("coll:{id}")));
    }

    #[test]
    fn smart_collection_from_the_filter_updates_by_itself() {
        let dir = tempfile::tempdir().unwrap();
        let (mut st, ids) = setup(dir.path());
        let ctx = st.ctx().clone();
        let mut ui = DesktopUi::new();
        ui.push(Action::NewSmartCollection);
        ui.apply_actions(&ctx, &mut st);
        assert!(ui.coll_dlg.is_none(), "no filter, nothing to save");
        ui.filter.search = "c".into();
        ui.push(Action::NewSmartCollection);
        ui.apply_actions(&ctx, &mut st);
        let d = ui.coll_dlg.take().expect("name prompt");
        assert!(matches!(&d.kind, DlgKind::Smart(r) if r.search == "c"));
        ui.finish_dialog(&mut st, d);
        let id = st.library.user[0].id;
        assert_eq!(ui.coll_sel, Some(id), "the new smart collection is shown");
        ui.filter.search.clear();
        ui.sync_filter(&st);
        ui.refresh_view_for(&st.library);
        assert_eq!(view_names(&ui, &st), ["C"]);
        // A new game matching the rule shows up without touching the collection.
        st.rename(ids[3], Some("Cello".into()));
        ui.sync_filter(&st);
        ui.refresh_view_for(&st.library);
        assert_eq!(view_names(&ui, &st), ["C", "D"], "D is now called Cello");
        // Smart collections take no games by hand.
        ui.push(Action::AddToCollection(id, vec![ids[0]]));
        ui.apply_actions(&ctx, &mut st);
        assert!(st.library.user[0].games.is_empty());
        // "Use the current filter" while it is shown: narrowed, and still shown.
        ui.filter.search = "cello".into();
        ui.push(Action::UpdateSmartCollection(id));
        ui.apply_actions(&ctx, &mut st);
        assert_eq!(ui.coll_sel, Some(id), "still shown");
        ui.filter.search.clear();
        ui.sync_filter(&st);
        ui.refresh_view_for(&st.library);
        assert_eq!(view_names(&ui, &st), ["D"]);
    }
}
