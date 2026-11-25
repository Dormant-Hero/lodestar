//! Settings → Categories: which categories controller mode lists, as a checklist, and "Hide
//! category" in the menu of a focused one. The rules (what is listed by default, All Games
//! always staying) are in library/categories.rs; the lists follow through `refresh_collections`.

use super::home::Group;
use super::*;
use crate::library::Category;

/// The checklist's groups, in order. All Games sits with the filters.
const GROUPS: [(&str, Group); 4] = [
    ("Filters", Group::Filters),
    ("Platforms", Group::Platforms),
    ("Sources", Group::Sources),
    ("Collections", Group::Collections),
];

/// PC's help line (here and in desktop Settings): hiding stores is what gives it a home row
/// (`added_categories`).
pub(crate) const PC_HELP: &str =
    "Every PC game, from any store. Hide the stores and PC takes their place on the home page.";

fn checklist_group(c: &Collection) -> Group {
    match Group::of(c) {
        Group::All => Group::Filters,
        g => g,
    }
}

/// Every category by checklist group (groups without one left out), each with its label
/// ("Steam (168)"). Desktop Settings → Controller shows the same list.
pub(crate) fn checklist(lib: &Library) -> Vec<(&'static str, Vec<(Category, String)>)> {
    let cats = lib.categories();
    GROUPS
        .iter()
        .map(|&(heading, group)| {
            let rows = cats
                .iter()
                .filter(|c| checklist_group(&c.collection) == group)
                .map(|c| {
                    (c.clone(), format!("{} ({})", card_name(&c.collection), lib.collection_games(&c.collection).len()))
                })
                .collect::<Vec<_>>();
            (heading, rows)
        })
        .filter(|(_, rows)| !rows.is_empty())
        .collect()
}

impl ControllerUi {
    /// The category under the focus, when the player can hide it: a card of the collections
    /// screen, or the focused row of a home page.
    pub(super) fn focused_category(&self) -> Option<Collection> {
        let i = match self.screen {
            Screen::Collections if !self.home.rail_focus => Some(self.coll_sel),
            Screen::Home if !self.home.rail_focus => self.home.rows.get(self.home.row).map(|(ci, _)| *ci),
            _ => None,
        }?;
        let c = &self.colls.get(i)?.collection;
        (c.kind != CollectionKind::All && !is_hidden_collection(c)).then(|| c.clone())
    }

    /// Settings → General → Categories' value: what the player changed ("" = nothing).
    pub(super) fn categories_summary(st: &AppState) -> String {
        let choice = &st.config.controller.categories;
        let cats = st.library.categories();
        let present = |keys: &[String]| keys.iter().filter(|k| cats.iter().any(|c| c.collection.key() == **k)).count();
        match (present(&choice.hidden), present(&choice.added)) {
            (0, 0) => String::new(),
            (h, 0) => format!("{h} hidden"),
            (0, a) => format!("{a} added"),
            (h, a) => format!("{h} hidden, {a} added"),
        }
    }

    pub(super) fn category_items(st: &AppState) -> Vec<Item> {
        let choice = &st.config.controller.categories;
        let mut v = vec![Item::info(
            "Choose what controller mode lists",
            "Hiding a category never hides its games: they stay in All Games.",
        )];
        for (heading, rows) in checklist(&st.library) {
            v.push(Item::heading(heading));
            for (c, label) in rows {
                let key = c.collection.key();
                v.push(if !c.hideable() {
                    Item::value(label, "Always shown", Act::ToggleCategory(key))
                        .with_help("Every game stays one press away here, and search looks through it.")
                } else {
                    let shown = choice.shows(&key, c.default);
                    let item = Item::value(label, if shown { "Shown" } else { "Hidden" }, Act::ToggleCategory(key));
                    if c.collection.kind == CollectionKind::Pc { item.with_help(PC_HELP) } else { item }
                });
            }
        }
        v
    }

    /// Settings → Categories: show or hide one.
    pub(super) fn toggle_category(&mut self, st: &mut AppState, key: &str) {
        let Some(c) = st.library.categories().into_iter().find(|c| c.collection.key() == key) else { return };
        if !c.hideable() {
            st.notify(format!("{} is always shown", c.collection.name));
            return;
        }
        let choice = &mut st.config.controller.categories;
        let shown = choice.shows(key, c.default);
        choice.set(key, c.default, !shown);
        self.categories_changed(st);
        let items = Self::category_items(st);
        if let Some(m) = self.menus.iter_mut().rev().find(|m| m.kind == MenuKind::Categories) {
            m.replace_items(items);
        }
    }

    /// The menu of a focused category → Hide category.
    pub(super) fn hide_category(&mut self, st: &mut AppState, key: &str) {
        let Some(c) = st.library.categories().into_iter().find(|c| c.collection.key() == key) else { return };
        if !c.hideable() {
            return;
        }
        st.config.controller.categories.set(key, c.default, false);
        self.categories_changed(st);
        self.close_menus(st);
        let name = card_name(&c.collection);
        st.notify(format!("{name} is hidden · Settings → General → Categories shows it again"));
    }

    /// Save the choice and rebuild the lists with it now (not at the next frame), so this
    /// press's screen already shows them.
    fn categories_changed(&mut self, st: &mut AppState) {
        st.save_config();
        self.categories.clone_from(&st.config.controller.categories);
        self.refresh_list(&st.library);
        self.refresh_settings(st);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Game, Source};

    /// The user's case: an all-PC library of stores; hide the stores, add PC.
    fn st_with_stores(dir: &std::path::Path) -> AppState {
        let mut st = AppState::for_test(dir);
        let mut games = vec![];
        for (i, (src, plat)) in [
            (Source::Steam, "linux"),
            (Source::Steam, "windows"),
            (Source::Steam, "windows"),
            (Source::Gog, "windows"),
            (Source::Gog, "windows"),
            (Source::Gog, "linux"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut g = Game::blank(&format!("G{i}"));
            g.id = i as GameId + 1;
            g.source = src;
            g.platform = Some(plat.into());
            g.installed = false;
            games.push(g);
        }
        st.library.replace(games);
        st
    }

    fn keys(ui: &ControllerUi) -> Vec<String> {
        ui.colls.iter().map(|c| c.collection.key()).collect()
    }

    fn value_of(ui: &ControllerUi, label_start: &str) -> String {
        let m = ui.menus.last().expect("menu");
        let it = m.items.iter().find(|i| !i.info && i.label.starts_with(label_start)).expect(label_start);
        it.value.clone().unwrap_or_default()
    }

    #[test]
    fn the_checklist_hides_and_adds_and_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = st_with_stores(dir.path());
        let mut ui = ControllerUi::new();
        ui.refresh_collections(&st.library);
        assert_eq!(keys(&ui), ["all", "steam", "gog"]);
        ui.menu_act(&mut st, Act::CategoriesMenu, 0.0);
        assert_eq!(value_of(&ui, "PC Games (6)"), "Hidden", "PC counts every PC game");
        assert_eq!(value_of(&ui, "Steam"), "Shown");
        assert_eq!(value_of(&ui, "All Games"), "Always shown");
        for k in ["steam", "gog", "pc"] {
            ui.menu_act(&mut st, Act::ToggleCategory(k.into()), 0.0);
        }
        assert_eq!(keys(&ui), ["all", "pc"]);
        assert_eq!(value_of(&ui, "PC Games"), "Shown");
        assert_eq!(value_of(&ui, "Steam"), "Hidden");
        // All Games can't be hidden: the last category always stays.
        ui.menu_act(&mut st, Act::ToggleCategory("all".into()), 0.0);
        assert_eq!(keys(&ui), ["all", "pc"]);
        let saved = crate::config::Config::load(&st.paths.config_file());
        assert_eq!(saved.controller.categories.hidden, ["steam", "gog"]);
        assert_eq!(saved.controller.categories.added, ["pc"]);
        // No game was hidden by any of it.
        assert!(st.library.games.iter().all(|g| !g.hidden));
    }

    #[test]
    fn hide_category_from_the_menu_of_a_focused_card() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = st_with_stores(dir.path());
        let mut ui = ControllerUi::new();
        ui.refresh_collections(&st.library);
        ui.coll_sel = keys(&ui).iter().position(|k| k == "gog").unwrap();
        let items = ui.quick_items(&st);
        let hide = items.iter().find(|i| i.act == Some(Act::HideCategory("gog".into()))).expect("offered");
        assert_eq!(hide.value.as_deref(), Some("GOG"));
        ui.menu_act(&mut st, Act::HideCategory("gog".into()), 0.0);
        assert_eq!(keys(&ui), ["all", "steam"]);
        assert!(ui.coll_sel < ui.colls.len(), "the focus moved to a card that is still there");
        // All Games never offers it.
        ui.coll_sel = 0;
        assert!(ui.quick_items(&st).iter().all(|i| !matches!(i.act, Some(Act::HideCategory(_)))));
    }

    /// A key saved by an earlier library (a store that was removed since) changes nothing.
    #[test]
    fn categories_the_library_lacks_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = st_with_stores(dir.path());
        st.config.controller.categories.hidden = vec!["epic".into()];
        st.config.controller.categories.added = vec!["coll:42".into()];
        let mut ui = ControllerUi::new();
        ui.categories.clone_from(&st.config.controller.categories);
        ui.refresh_collections(&st.library);
        assert_eq!(keys(&ui), ["all", "steam", "gog"]);
        assert_eq!(ControllerUi::categories_summary(&st), "");
    }
}
