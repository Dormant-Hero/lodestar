//! Controller mode's categories: every collection the library can offer, which of them are
//! listed when the player chose nothing, and the player's choice (Settings → Categories).
//! The choice only changes which categories are listed; every game stays in All Games.

use super::*;
use crate::config::Categories;

/// One category controller mode can list.
#[derive(Clone, Debug, PartialEq)]
pub struct Category {
    pub collection: Collection,
    /// Listed when the player chose nothing (what [`Library::collections`] returns).
    pub default: bool,
}

impl Category {
    /// The player can leave it out. All Games always stays: it is where search, a random pick
    /// and a home page with nothing else to show go, and it keeps every game one press away.
    pub fn hideable(&self) -> bool {
        self.collection.kind != CollectionKind::All
    }
}

impl Library {
    /// Every category with at least one visible game, in display order: the filters, All Games,
    /// PC with its PC platforms, the user's collections, the stores, the consoles, the tags.
    /// Not listed by default: PC when every game is a PC game (it would repeat All Games), the
    /// PC platforms, and stores with fewer than [`MIN_STORE_GAMES`] games (a row of one-game
    /// stores was mostly noise; their games are in PC and All Games).
    pub fn categories(&self) -> Vec<Category> {
        let mut out: Vec<Category> = vec![];
        let mut push = |kind: CollectionKind, name: String, default: bool| {
            let collection = Collection { kind, name };
            let has_games = (0..self.games.len()).any(|i| !self.games[i].hidden && self.in_collection(&collection, i));
            if has_games && !out.iter().any(|c| c.collection.kind == collection.kind) {
                out.push(Category { collection, default });
            }
        };
        push(CollectionKind::Recent, "Continue Playing".into(), true);
        push(CollectionKind::Favorites, "Favourites".into(), true);
        push(CollectionKind::Installed, "Installed".into(), true);
        push(CollectionKind::Controller, "Controller-friendly".into(), true);
        push(CollectionKind::All, "All Games".into(), true);
        let visible = || self.games.iter().filter(|g| !g.hidden);
        let mixed = visible().any(in_pc_category) && visible().any(|g| !in_pc_category(g));
        push(CollectionKind::Pc, "PC Games".into(), mixed);
        // Emulated DOS games make PC (DOS) a console of its own, listed by default further down;
        // offering it here first (not listed) would take that away.
        let consoles = self.platforms_present();
        for id in self.pc_platforms_present().into_iter().filter(|id| !consoles.iter().any(|c| c == id)) {
            push(CollectionKind::Platform(id.into()), platform_label(id), false);
        }
        for u in &self.user {
            push(CollectionKind::User(u.clone()), u.name.clone(), true);
        }
        for s in self.sources_present() {
            let n = visible().filter(|g| g.source == s).count();
            let default = n >= MIN_STORE_GAMES || matches!(s, Source::Emulator | Source::Romm);
            push(CollectionKind::Source(s), s.display().into(), default);
        }
        for p in consoles {
            let name = platform_label(&p);
            push(CollectionKind::Platform(p), name, true);
        }
        for t in self.all_tags() {
            push(CollectionKind::Tag(t.clone()), t, true);
        }
        out
    }

    /// The categories controller mode lists with the player's choice applied, in display order.
    /// All Games is never left out.
    pub fn chosen_collections(&self, choice: &Categories) -> Vec<Collection> {
        self.categories()
            .into_iter()
            .filter(|c| !c.hideable() || choice.shows(&c.collection.key(), c.default))
            .map(|c| c.collection)
            .collect()
    }

    /// The PC platforms (Playnite ids "pc_linux", "pc_windows" …) the PC category's visible
    /// games name, in id order: what the desktop sidebar lists under PLATFORMS → PC.
    fn pc_platforms_present(&self) -> Vec<&'static str> {
        let mut v: Vec<&'static str> = (0..self.games.len())
            .filter(|&i| !self.games[i].hidden && in_pc_category(&self.games[i]))
            .filter_map(|i| self.named_platform.get(i).copied().flatten())
            .filter(|id| id.starts_with("pc_"))
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(id: GameId, source: Source, platform: Option<&str>) -> Game {
        let mut g = Game::blank(&format!("G{id}"));
        g.id = id;
        g.source = source;
        g.platform = platform.map(String::from);
        // Not installed: no Installed category, so the lists below are only the ones that matter.
        g.installed = false;
        g
    }

    /// An all-PC library like the user's: Steam and GOG with Linux and Windows games, one
    /// Shortcuts game, one game with no platform.
    fn pc_library() -> Library {
        let mut games = vec![];
        for i in 0..4 {
            games.push(game(i + 1, Source::Steam, Some(if i % 2 == 0 { "linux" } else { "windows" })));
        }
        for i in 0..3 {
            games.push(game(i + 10, Source::Gog, Some("windows")));
        }
        games.push(game(20, Source::Desktop, Some("linux")));
        games.push(game(21, Source::Steam, None));
        Library::new(games)
    }

    fn emulated(id: GameId, platform: &str) -> Game {
        let mut g = game(id, Source::Emulator, None);
        g.emu = Some(EmuRef { platform: platform.into(), ..Default::default() });
        g
    }

    fn keys(cs: &[Collection]) -> Vec<String> {
        cs.iter().map(Collection::key).collect()
    }

    #[test]
    fn nothing_chosen_lists_what_it_always_did() {
        let l = pc_library();
        assert_eq!(keys(&l.chosen_collections(&Categories::default())), keys(&l.collections()));
        // Every game is a PC game: PC would repeat All Games, so only the stores are listed.
        assert_eq!(keys(&l.collections()), ["all", "steam", "gog"]);
        let offered: Vec<(String, bool)> = l.categories().iter().map(|c| (c.collection.key(), c.default)).collect();
        assert_eq!(
            offered,
            [
                ("all".to_string(), true),
                ("pc".to_string(), false),
                ("platform:pc_linux".to_string(), false),
                ("platform:pc_windows".to_string(), false),
                ("steam".to_string(), true),
                ("desktop".to_string(), false),
                ("gog".to_string(), true),
            ]
        );
    }

    /// A library with emulated games, DOS among them, and a GOG game set to DOS: the same list
    /// as before players could choose which categories are listed.
    #[test]
    fn emulated_dos_stays_a_console() {
        let mut games =
            vec![emulated(1, "pc_dos"), emulated(2, "sony_playstation2"), game(3, Source::Gog, Some("DOS"))];
        games.extend((4..=6).map(|i| game(i, Source::Steam, Some("linux"))));
        let l = Library::new(games);
        assert_eq!(
            keys(&l.collections()),
            ["all", "pc", "steam", "emulator", "platform:pc_dos", "platform:sony_playstation2"]
        );
        // PC (DOS) is offered once, as the console it already is.
        let dos = l.categories().into_iter().filter(|c| c.collection.key() == "platform:pc_dos").collect::<Vec<_>>();
        assert_eq!(dos.len(), 1);
        assert!(dos[0].default);
        assert_eq!(l.collection_games(&dos[0].collection).len(), 2);
    }

    #[test]
    fn hiding_the_stores_and_adding_pc() {
        let l = pc_library();
        let mut c = Categories::default();
        for k in ["steam", "gog"] {
            c.set(k, true, false);
        }
        c.set("pc", false, true);
        c.set("platform:pc_linux", false, true);
        let chosen = l.chosen_collections(&c);
        assert_eq!(keys(&chosen), ["all", "pc", "platform:pc_linux"]);
        // PC counts every PC game, like the sidebar's PLATFORMS → PC; Linux only the Linux ones.
        assert_eq!(l.collection_games(&chosen[1]).len(), 9);
        assert_eq!(l.collection_games(&chosen[2]).len(), 3);
        // No game was hidden.
        assert_eq!(l.collection_games(&chosen[0]).len(), 9);
    }

    #[test]
    fn all_games_always_stays() {
        let l = pc_library();
        let mut c = Categories::default();
        for cat in l.categories() {
            c.set(&cat.collection.key(), cat.default, false);
        }
        assert_eq!(keys(&l.chosen_collections(&c)), ["all"]);
        assert!(l.categories().iter().all(|c| c.hideable() == (c.collection.kind != CollectionKind::All)));
    }

    #[test]
    fn keys_the_library_lacks_are_ignored() {
        let l = pc_library();
        let c = Categories { hidden: vec!["epic".into()], added: vec!["coll:99".into(), "platform:nope".into()] };
        assert_eq!(keys(&l.chosen_collections(&c)), keys(&l.collections()));
    }

    #[test]
    fn set_keeps_only_differences_from_the_default() {
        let mut c = Categories::default();
        c.set("steam", true, false);
        c.set("pc", false, true);
        assert_eq!((c.hidden.clone(), c.added.clone()), (vec!["steam".to_string()], vec!["pc".to_string()]));
        assert!(!c.shows("steam", true) && c.shows("pc", false) && c.shows("gog", true) && !c.shows("ea", false));
        c.set("steam", true, true);
        c.set("pc", false, false);
        assert!(c.is_empty());
    }

    /// A bare "PC" platform is in PC but in none of the PC platforms (it used to count as
    /// Windows); a console name set on a store game takes it out of PC, as in the sidebar.
    #[test]
    fn pc_platforms_follow_the_sidebar() {
        let mut games = vec![game(1, Source::Steam, Some("PC")), game(2, Source::Steam, Some("windows"))];
        games.push(game(3, Source::Steam, Some("Sony PlayStation 2")));
        let l = Library::new(games);
        let pc = Collection { kind: CollectionKind::Pc, name: String::new() };
        let win = Collection { kind: CollectionKind::Platform("pc_windows".into()), name: String::new() };
        let ids = |c: &Collection| l.collection_games(c).into_iter().map(|i| l.games[i].id).collect::<Vec<_>>();
        assert_eq!(ids(&pc), [1, 2]);
        assert_eq!(ids(&win), [2]);
        assert!(l.collections().iter().any(|c| c.kind == CollectionKind::Pc), "not everything is PC");
    }
}
