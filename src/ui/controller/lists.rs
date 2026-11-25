//! The controller's collections and game lists: building them, the hidden collection, and
//! letter jumps.

use super::*;
use crate::theme::model::{Element, ThemeFile};

/// Short controller-support label for menu values ("" = unknown).
pub(super) fn controller_short(c: Option<ControllerSupport>) -> &'static str {
    match c {
        Some(ControllerSupport::Full) => "Full support",
        Some(ControllerSupport::Partial) => "Partial support",
        Some(ControllerSupport::None) => "No support",
        None => "",
    }
}

pub(super) fn hidden_collection() -> Collection {
    Collection { kind: CollectionKind::Tag(HIDDEN_TAG.into()), name: "Hidden Games".into() }
}

pub(super) fn is_hidden_collection(c: &Collection) -> bool {
    matches!(&c.kind, CollectionKind::Tag(t) if t == HIDDEN_TAG)
}

/// The filter tabs a theme draws over its game list, as the collection keys they stand for, in
/// order. Translated Playnite themes draw each tab as a text shown or highlighted by a rule
/// `{collection.key} == <key>` (theme/playnite/compile.rs, `tabs_table`); no such rules: no tabs.
pub(super) fn theme_tab_keys(theme: &ThemeFile) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    each_games_element(theme, &mut |e| {
        let c = e.common();
        for rule in [&c.visible_if, &c.hidden_if].into_iter().flatten() {
            if let Some(key) = rule.trim().strip_prefix("{collection.key} == ").map(str::trim)
                && !key.is_empty()
                && !key.contains(' ')
                && !out.iter().any(|k| k == key)
            {
                out.push(key.to_string());
            }
        }
    });
    out
}

/// The theme draws its own LB / RB pictures on the game list (Playnite themes put them beside
/// their filter tabs), so the hint bar needn't say what those buttons do.
pub(super) fn theme_prompts_shoulders(theme: &ThemeFile) -> bool {
    let mut found = false;
    each_games_element(theme, &mut |e| {
        if let Element::HelpBar { buttons, glyphs: true, .. } = e {
            found |= buttons.iter().any(|b| b.eq_ignore_ascii_case("lb") || b.eq_ignore_ascii_case("rb"));
        }
    });
    found
}

/// Every element of the theme's game list, stacks' children included.
fn each_games_element(theme: &ThemeFile, f: &mut impl FnMut(&Element)) {
    fn walk(e: &Element, f: &mut impl FnMut(&Element)) {
        f(e);
        if let Element::Stack { children, .. } = e {
            children.iter().for_each(|ch| walk(ch, f));
        }
    }
    theme.games.elements.iter().for_each(|e| walk(e, f));
}

/// First letter used for letter jumps ('#' for digits/symbols).
pub(super) fn letter_of(g: &Game) -> char {
    match g.sort_name.chars().next() {
        Some(c) if c.is_alphabetic() => c.to_uppercase().next().unwrap_or(c),
        _ => '#',
    }
}

/// Next/previous index whose first letter differs from the current one.
pub(super) fn jump_letter(games: &[Game], list: &[usize], sel: usize, forward: bool) -> usize {
    if list.is_empty() {
        return 0;
    }
    let letter = |i: usize| games.get(list[i]).map(letter_of).unwrap_or('#');
    let sel = sel.min(list.len() - 1);
    let cur = letter(sel);
    if forward {
        (sel + 1..list.len()).find(|&i| letter(i) != cur).unwrap_or(list.len() - 1)
    } else {
        // Go to the start of the current letter; if already there, the start of the previous one.
        let start = (0..=sel).rev().take_while(|&i| letter(i) == cur).last().unwrap_or(sel);
        if start < sel {
            return start;
        }
        if start == 0 {
            return 0;
        }
        let prev = letter(start - 1);
        (0..start).rev().take_while(|&i| letter(i) == prev).last().unwrap_or(start - 1)
    }
}

impl ControllerUi {
    pub(super) fn refresh_collections(&mut self, lib: &Library) {
        let memo = (lib.generation, self.show_hidden);
        if self.colls_gen == Some(memo) {
            return;
        }
        self.colls_gen = Some(memo);
        let cur_key = self.cur.and_then(|i| self.colls.get(i)).map(|c| c.collection.key());
        let mut used: HashSet<GameId> = HashSet::new();
        self.colls.clear();
        self.colls_games.clear();
        let mut colls = lib.collections();
        if self.show_hidden && lib.games.iter().any(|g| g.hidden) {
            colls.push(hidden_collection());
        }
        for c in colls {
            let games = if is_hidden_collection(&c) {
                let mut v: Vec<usize> = (0..lib.games.len()).filter(|&i| lib.games[i].hidden).collect();
                lib.sort(&mut v, SortBy::Name, false);
                v
            } else {
                lib.collection_games(&c)
            };
            // Representative art: most recently played game with a hero not used yet.
            let mut by_recent: Vec<usize> = games.clone();
            by_recent.sort_by_key(|&i| std::cmp::Reverse(lib.games[i].last_played.unwrap_or(0)));
            let pick = by_recent
                .iter()
                .find(|&&i| lib.games[i].art.hero.is_some() && !used.contains(&lib.games[i].id))
                .or_else(|| by_recent.iter().find(|&&i| lib.games[i].art.hero.is_some()))
                .or_else(|| by_recent.iter().find(|&&i| lib.games[i].art.cover.is_some()))
                .copied();
            let mut art = Art::default();
            if let Some(i) = pick {
                used.insert(lib.games[i].id);
                art = lib.games[i].art.clone();
                // The card's cover: prefer a cover of the same game, else any game's cover.
                if art.cover.is_none() {
                    art.cover = by_recent.iter().find_map(|&j| lib.games[j].art.cover.clone());
                }
            }
            let name = match &c.kind {
                _ if is_hidden_collection(&c) => c.name.clone(),
                CollectionKind::Tag(t) => format!("#{t}"),
                CollectionKind::Platform(id) => console_label(id),
                _ => c.name.clone(),
            };
            self.colls.push(CollectionItem { count: games.len(), name, collection: c, art });
            self.colls_games.push(games);
        }
        // Keep selections by key.
        if let Some(k) = &self.coll_key
            && let Some(i) = self.colls.iter().position(|c| &c.collection.key() == k)
        {
            self.coll_sel = i;
        }
        self.coll_sel = self.coll_sel.min(self.colls.len().saturating_sub(1));
        self.cur = cur_key.and_then(|k| self.colls.iter().position(|c| c.collection.key() == k));
        if self.cur.is_none() {
            // The open collection is gone (its last game was hidden, or a rescan removed it).
            self.list.clear();
            self.list_ids.clear();
            self.list_key = None;
            self.game_sel = 0;
            if !matches!(self.screen, Screen::Collections | Screen::Home) {
                self.screen = Screen::Collections;
                self.details = None;
            }
        }
    }

    /// LB / RB on a game list whose theme draws filter tabs: the collection the next (or
    /// previous) tab opens, going round. Tabs whose collection doesn't exist (Favourites with no
    /// favourites) are skipped; the open collection, when it has no tab of its own, sits after
    /// them (that's where such themes show its name). None: the theme draws no tabs.
    pub(super) fn tab_step(&self, theme: &crate::theme::model::ThemeFile, forward: bool) -> Option<usize> {
        let keys = theme_tab_keys(theme);
        if keys.is_empty() {
            return None;
        }
        let mut tabs: Vec<usize> =
            keys.iter().filter_map(|k| self.colls.iter().position(|c| c.collection.key() == *k)).collect();
        let Some(cur) = self.cur.filter(|&i| i < self.colls.len()) else { return tabs.first().copied() };
        if !tabs.contains(&cur) {
            tabs.push(cur);
        }
        let at = tabs.iter().position(|&i| i == cur)?;
        let n = tabs.len();
        Some(tabs[if forward { (at + 1) % n } else { (at + n - 1) % n }])
    }

    pub(super) fn cur_key(&self) -> Option<String> {
        self.cur.and_then(|i| self.colls.get(i)).map(|c| c.collection.key())
    }

    pub(super) fn cur_is_hidden(&self) -> bool {
        self.cur.and_then(|i| self.colls.get(i)).is_some_and(|c| is_hidden_collection(&c.collection))
    }

    /// Bring `colls` and the Games list up to date with the library (cheap when nothing
    /// changed). Everything is rebuilt from the current `lib`; the previous list is only
    /// consulted through `list_ids`, so a rescan that shrinks or reorders `library.games`
    /// can't index out of range or move the selection to another game.
    pub(super) fn refresh_list(&mut self, lib: &Library) {
        // The list is built from `colls_games`: bring those up to date first, or a change made
        // by a nav this frame (hide, favourite) would be keyed with the new library generation
        // but built from the old collection contents, and never rebuilt.
        self.refresh_collections(lib);
        let Some(ci) = self.cur.filter(|&i| i < self.colls.len()) else {
            self.list.clear();
            self.list_ids.clear();
            self.list_key = None;
            return;
        };
        let key = ListKey {
            generation: lib.view_generation(self.sort.unwrap_or_default()),
            coll: self.colls[ci].collection.key(),
            sort: self.sort,
            descending: self.descending,
            search: self.search.trim().to_string(),
        };
        if self.list_key.as_ref() == Some(&key) {
            return;
        }
        let same_coll = self.list_key.as_ref().is_some_and(|k| k.coll == key.coll);
        let prev_id = if same_coll { self.sel_id() } else { None };
        let base = &self.colls_games[ci];
        let list: Vec<usize> = if key.search.is_empty() {
            let mut v = base.clone();
            if let Some(s) = self.sort {
                lib.sort(&mut v, s, self.descending);
            } else if self.descending {
                v.reverse();
            }
            v
        } else {
            let mut scored: Vec<(i32, usize)> = base
                .iter()
                .filter_map(|&i| crate::search::score(&key.search, lib.games[i].display_name()).map(|s| (s, i)))
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(lib.games[a.1].sort_name.cmp(&lib.games[b.1].sort_name)));
            scored.into_iter().map(|(_, i)| i).collect()
        };
        // A new search query starts on its best match; otherwise keep the selected game.
        let new_query =
            same_coll && !key.search.is_empty() && self.list_key.as_ref().is_some_and(|k| k.search != key.search);
        let want = if new_query { None } else { prev_id.or_else(|| self.remembered.get(&key.coll).copied()) };
        self.list_ids = list.iter().map(|&i| lib.games[i].id).collect();
        self.list = list;
        self.game_sel = want
            .and_then(|id| self.list_ids.iter().position(|&g| g == id))
            .unwrap_or(if same_coll && !new_query { self.game_sel.min(self.list.len().saturating_sub(1)) } else { 0 });
        let new_id = hash_of((&key.coll, key.sort.map(SortBy::label), key.descending, &key.search));
        if new_id != self.list_id {
            self.list_id = new_id;
        }
        self.list_key = Some(key);
    }
}

/// A console collection's card name ("PlayStation 2", "GameCube", "Nintendo 64").
pub(super) fn console_label(id: &str) -> String {
    crate::ui::platform_icons::console_name(id)
}
