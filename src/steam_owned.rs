//! Steam games that are no longer in the account's Steam library.
//!
//! Not-installed Steam games come from the account's `localconfig.vdf`: every game it played on
//! this PC. Some of those came with a subscription (EA Play, Ubisoft+) that has since ended, and
//! Steam answers Install with "No licenses". With the user's own Steam Web API key Lodestar has
//! the account's owned-games list (steam_names.rs), and a not-installed game that played here
//! but isn't on that list is shown as "Not in your Steam library": its page says why, and
//! Install opens the game in Steam instead (`store_app`).
//!
//! These games are marked, never hidden: a game from a Steam Family member's library is not
//! on the owned list either, yet it does install. And the mark is worked out on every look
//! (`Library::not_in_steam_library`), so it goes away once the game is on the list again.
//!
//! Free games: `GetOwnedGames` lists them only once they have been played
//! (`include_played_free_games`). A game with no play time can't be told apart from a free one
//! never started, so it is left unmarked.

use crate::model::{Game, Source};
use crate::scan::steam::names;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

/// Appids on the owned-games list of every Steam account Lodestar has one for.
pub type OwnedSet = Arc<HashSet<u32>>;

/// The saved owned-games lists, when they can be trusted: a Steam Web API key is set and a
/// fetch saved a list with games in it. An old list is fine (it is refreshed once a day); a
/// missing or empty one gives None, and nothing is marked.
pub fn load(cache_dir: &Path, key_set: bool) -> Option<OwnedSet> {
    if !key_set {
        return None;
    }
    let cache = names::load_owned(cache_dir);
    let ids: HashSet<u32> = cache.accounts.values().flat_map(|l| l.games.iter().map(|g| g.appid)).collect();
    (!ids.is_empty()).then(|| Arc::new(ids))
}

/// `g` is a not-installed Steam game, played here, that the owned-games list doesn't have.
pub fn not_in_library(g: &Game, owned: Option<&HashSet<u32>>) -> bool {
    let (Some(owned), Some(appid)) = (owned, g.steam_appid()) else {
        return false;
    };
    g.source == Source::Steam && !g.installed && g.missing.is_none() && g.playtime_secs > 0 && !owned.contains(&appid)
}

/// The game's page in Steam's own library. For a game shared by a family member Steam shows
/// Install there; otherwise it offers what it can (buy it, or the subscription).
pub fn steam_page_uri(appid: u32) -> String {
    format!("steam://nav/games/details/{appid}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::steam::names::{OwnedCache, OwnedGame, OwnedList};

    fn steam(appid: u32, installed: bool, playtime_secs: u64) -> Game {
        let mut g = Game::blank("Need for Speed Heat");
        g.source = Source::Steam;
        g.source_id = appid.to_string();
        g.installed = installed;
        g.playtime_secs = playtime_secs;
        g
    }

    fn owned(ids: &[u32]) -> HashSet<u32> {
        ids.iter().copied().collect()
    }

    #[test]
    fn a_game_played_here_that_left_the_library_is_marked() {
        let heat = steam(1_222_680, false, 19 * 3600);
        assert!(not_in_library(&heat, Some(&owned(&[70, 440]))));
    }

    #[test]
    fn installed_games_are_never_marked() {
        // A family member's game, installed from their library: plays fine.
        let g = steam(1_222_680, true, 3600);
        assert!(!not_in_library(&g, Some(&owned(&[70]))));
    }

    #[test]
    fn owned_games_are_not_marked() {
        let g = steam(70, false, 3600);
        assert!(!not_in_library(&g, Some(&owned(&[70]))));
    }

    #[test]
    fn without_a_list_nothing_is_marked() {
        let g = steam(1_222_680, false, 3600);
        assert!(!not_in_library(&g, None));
    }

    #[test]
    fn games_without_play_time_are_left_alone() {
        // A free game installed once and never started isn't on the list (Steam lists free
        // games only once played): it may well be fine.
        let free = steam(1_172_470, false, 0);
        assert!(!not_in_library(&free, Some(&owned(&[70]))));
        // Played, it is on the list like any other game.
        let played = steam(1_172_470, false, 4560);
        assert!(!not_in_library(&played, Some(&owned(&[70, 1_172_470]))));
    }

    #[test]
    fn only_steam_games_are_marked() {
        let mut gog = steam(1_222_680, false, 3600);
        gog.source = Source::Gog;
        assert!(!not_in_library(&gog, Some(&owned(&[70]))));
        // A non-Steam shortcut has no appid.
        let mut shortcut = steam(1, false, 3600);
        shortcut.source_id = "shortcut:123".into();
        assert!(!not_in_library(&shortcut, Some(&owned(&[70]))));
        // A game the scan lost has its own note.
        let mut gone = steam(1_222_680, false, 3600);
        gone.missing = Some(crate::model::Missing { since: 1, kept: false });
        assert!(!not_in_library(&gone, Some(&owned(&[70]))));
    }

    fn save(dir: &Path, lists: &[(u32, &[u32])]) {
        let mut c = OwnedCache::default();
        for (account, ids) in lists {
            let games =
                ids.iter().map(|&appid| OwnedGame { appid, name: format!("App {appid}"), ..Default::default() });
            c.accounts.insert(*account, OwnedList { fetched: 1, games: games.collect() });
        }
        names::save_owned(dir, &c).unwrap();
    }

    #[test]
    fn the_list_counts_only_with_a_key_and_games_on_it() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path(), true), None, "never fetched");
        save(dir.path(), &[(1, &[])]);
        assert_eq!(load(dir.path(), true), None, "an empty list says nothing");
        save(dir.path(), &[(1, &[70]), (2, &[440])]);
        assert_eq!(load(dir.path(), false), None, "the key was removed");
        let set = load(dir.path(), true).unwrap();
        assert_eq!(*set, owned(&[70, 440]), "every account's games");
    }

    #[test]
    fn the_app_marks_the_game_and_install_opens_steam() {
        use crate::model::{LaunchSpec, ScannedGame};
        let dir = tempfile::tempdir().unwrap();
        let mut st = crate::app::AppState::for_test(dir.path());
        st.no_launch = true;
        let mut heat = ScannedGame::new(Source::Steam, "1222680", "Need for Speed Heat");
        heat.installed = false;
        heat.playtime_secs = Some(19 * 3600);
        heat.launch = Some(LaunchSpec::Steam { appid: 1_222_680 });
        let mut db = crate::db::Db::open(&st.paths.db_file()).unwrap();
        db.merge_scan(&[heat], &[Source::Steam]).unwrap();
        st.reload_library();
        let id = st.library.games[0].id;
        std::fs::create_dir_all(&st.paths.cache_dir).unwrap();
        save(&st.paths.cache_dir, &[(1, &[70])]);
        let marked = |st: &crate::app::AppState| st.library.not_in_steam_library(st.library.get(id).unwrap());

        st.refresh_steam_owned();
        assert!(!marked(&st), "no Steam Web API key: the list isn't used");
        st.config.achievements.steam_web_api_key = Some("0123456789abcdef0123456789abcdef".into());
        st.refresh_steam_owned();
        assert!(marked(&st));
        assert!(st.can_install(id));
        assert_eq!(st.install_label(st.library.get(id).unwrap()), "Open in Steam");
        st.install_game(id);
        let toast = &st.toasts.last().unwrap().text;
        assert!(toast.contains("Opening Need for Speed Heat in Steam"), "{toast}");
    }

    #[test]
    fn the_mark_clears_once_the_game_is_owned_again() {
        let dir = tempfile::tempdir().unwrap();
        let heat = steam(1_222_680, false, 19 * 3600);
        save(dir.path(), &[(1, &[70])]);
        assert!(not_in_library(&heat, load(dir.path(), true).as_deref()));
        // Subscribed again (or bought): the next fetch lists it.
        save(dir.path(), &[(1, &[70, 1_222_680])]);
        assert!(!not_in_library(&heat, load(dir.path(), true).as_deref()));
    }
}
