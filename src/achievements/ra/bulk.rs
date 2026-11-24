//! RetroAchievements counts for the whole library: one `API_GetUserCompletionProgress` answer
//! (500 games a page; one request for most players, cached an hour) gives the player's count
//! for every game they have played; the library's emulated games are matched to RetroAchievements
//! ids the cheap ways only (`matching::find_for_library`: the user's id, the ROM hash, a saved
//! file name), so a library pass never sends a request per game. Games matched but never played
//! count "0 of N" from the console's game list.

use super::api::{ApiErr, Client, num};
use super::hash::{self, HashCache};
use super::matching::{self, GameList, now};
use crate::model::{Game, GameId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// The player's completion list is asked again after this long (sooner after playing).
pub const COMPLETION_MAX_AGE: i64 = 3600;
/// Games per page (the API's maximum).
const PAGE: u64 = 500;
/// At most this many pages (100,000 games played): a broken answer can't loop for ever.
const MAX_PAGES: u64 = 200;

/// The player's progress per RetroAchievements game: id → (unlocked, total).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Completion {
    pub fetched: i64,
    pub games: HashMap<u32, (u32, u32)>,
}

/// A game's line on a completion page: (RetroAchievements id, unlocked, total).
pub type PageGame = (u32, u32, u32);

/// One page of `API_GetUserCompletionProgress`: (the player's number of games, the games on
/// this page). None: not that answer.
pub fn parse_page(v: &serde_json::Value) -> Option<(u64, Vec<PageGame>)> {
    let results = v.get("Results")?.as_array()?;
    let total = num(v, "Total").unwrap_or(results.len() as u64);
    let games = results
        .iter()
        .filter_map(|r| {
            let id = u32::try_from(num(r, "GameID")?).ok().filter(|i| *i > 0)?;
            let max = num(r, "MaxPossible").unwrap_or(0) as u32;
            // Softcore unlocks include the hardcore ones (the game page counts them the same way).
            let got = num(r, "NumAwarded").unwrap_or(0).max(num(r, "NumAwardedHardcore").unwrap_or(0)) as u32;
            Some((id, got.min(max), max))
        })
        .collect();
    Some((total, games))
}

fn completion_file(dir: &Path, user: &str) -> std::path::PathBuf {
    dir.join("completion").join(format!("{}.json", super::progress::user_key(user)))
}

/// The player's completion list: the saved one while it is under an hour old (unless `force`),
/// else asked page by page; when asking fails, the saved one whatever its age.
pub fn completion(c: &Client, dir: &Path, user: &str, force: bool) -> Result<Completion, ApiErr> {
    let file = completion_file(dir, user);
    let cached: Option<Completion> = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok());
    if !force
        && let Some(cc) = &cached
        && now() - cc.fetched < COMPLETION_MAX_AGE
    {
        return Ok(cc.clone());
    }
    match fetch_all(c, user) {
        Ok(games) => {
            let out = Completion { fetched: now(), games };
            if let Some(d) = file.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if let Ok(b) = serde_json::to_vec(&out) {
                let _ = crate::fsutil::write_atomic(&file, &b);
            }
            Ok(out)
        }
        Err(e) => cached.ok_or(e),
    }
}

fn fetch_all(c: &Client, user: &str) -> Result<HashMap<u32, (u32, u32)>, ApiErr> {
    let mut out = HashMap::new();
    let mut offset = 0u64;
    for _ in 0..MAX_PAGES {
        let v = c.call(
            "GetUserCompletionProgress",
            &[("u", user.to_string()), ("c", PAGE.to_string()), ("o", offset.to_string())],
        )?;
        let (total, page) = parse_page(&v).ok_or(ApiErr::Garbled)?;
        let n = page.len() as u64;
        for (id, got, max) in page {
            out.insert(id, (got, max));
        }
        offset += n;
        if n == 0 || offset >= total {
            break;
        }
    }
    Ok(out)
}

/// The count a matched game shows: the player's progress, else (never played) none of the
/// console list's achievements. None: nothing known (or a game without achievements).
pub fn count_of(id: u32, done: &Completion, lists: &[Arc<GameList>]) -> Option<(u32, u32)> {
    let (got, total) = match done.games.get(&id) {
        Some(&c) => c,
        None => (0, lists.iter().flat_map(|l| l.games.iter()).find(|g| g.0 == id).map(|g| g.2)?),
    };
    (total > 0).then_some((got.min(total), total))
}

/// The console lists a game is matched against (asked once per console and pass).
pub struct Lists<'a> {
    c: &'a Client<'a>,
    dir: &'a Path,
    by_console: HashMap<u32, Option<Arc<GameList>>>,
}

impl<'a> Lists<'a> {
    pub fn new(c: &'a Client<'a>, dir: &'a Path) -> Lists<'a> {
        Lists { c, dir, by_console: HashMap::new() }
    }

    pub fn of(&mut self, console: &hash::Console) -> Vec<Arc<GameList>> {
        console
            .ids
            .iter()
            .filter_map(|id| {
                self.by_console.entry(*id).or_insert_with(|| matching::game_list(self.c, self.dir, *id).ok()).clone()
            })
            .collect()
    }
}

/// Counts for the emulated games that can be matched. `hash_new`: also hash ROMs not hashed
/// before (slow for a big library; the caller runs it at low priority, after a first pass
/// with the hashes already known).
pub fn library_counts(
    games: &[Game],
    dir: &Path,
    lists: &mut Lists,
    hashes: &mut HashCache,
    done: &Completion,
    hash_new: bool,
    keep_going: &dyn Fn() -> bool,
) -> HashMap<GameId, (u32, u32)> {
    let mut out = HashMap::new();
    for g in games {
        if !keep_going() {
            break;
        }
        let Some(emu) = &g.emu else { continue };
        let console = hash::console(&emu.platform);
        let ls = console.as_ref().map(|c| lists.of(c)).unwrap_or_default();
        let h = match (console, hash_new) {
            (None, _) => None,
            (Some(_), false) => hashes.cached(&emu.platform, &emu.rom).flatten(),
            (Some(_), true) => hashes.get(&emu.platform, &emu.rom),
        };
        if hash_new && hashes.unsaved() >= 100 {
            hashes.save();
        }
        // A console RetroAchievements doesn't cover: only the id the user set counts.
        if console.is_none() && g.meta.ra_game_id.is_none_or(|i| i == 0) {
            continue;
        }
        let Some(id) = matching::find_for_library(dir, g, &ls, h.as_deref()) else { continue };
        if let Some(c) = count_of(id, done, &ls) {
            out.insert(g.id, c);
        }
    }
    hashes.save();
    out
}
