//! Steam achievements, read from the Steam client's own files (`local`), with the keyless Web API
//! and the user's optional Steam Web API key for games the client has no files for (`web`).
//!
//! Order of lookups: the achievement list + the player's unlocks from
//! `appcache/stats` (the normal case; no login, no network except pictures and rarity); list
//! without unlocks → everything locked with "Play the game once through Steam …"; no list → with
//! a key `GetPlayerAchievements` + the keyless list, without one the keyless list only tells
//! whether the game has any (none → no section, some → a hint). Parsed results are kept in memory
//! until the files change; pictures are downloaded once into `<cache>/steam/<appid>/`.

pub mod bulk;
pub mod local;
pub mod web;

#[cfg(test)]
mod tests;

use super::{Achievement, Env, Progress, Request, Service, State};
use crate::config::{redact, secret};
use crate::model::{Game, Source};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant, SystemTime};
use web::{Fail, Web};

/// Shown under the count when only the list is known.
pub const PLAY_ONCE: &str = "Play the game once through Steam to sync your progress.";
/// The key lookup was refused (private profile or a wrong key).
pub const NOT_SHARED: &str = "Steam didn't share your progress for this game. Check the Steam Web API key in \
                              Settings → Achievements, and that your Steam profile's Game details are public.";
/// The key lookup couldn't reach Steam.
pub const UNREACHABLE: &str = "Steam couldn't be reached for your progress. It shows here next time.";
/// Name of a hidden achievement that is still locked.
pub const HIDDEN_NAME: &str = "Hidden achievement";

/// A result missing pictures or rarity because the network failed is built again after this long.
const RETRY_AFTER: Duration = Duration::from_secs(5 * 60);

/// A Steam game (not a non-Steam shortcut).
pub fn applies(g: &Game) -> bool {
    g.source == Source::Steam && !g.source_id.starts_with("shortcut:") && g.source_id.parse::<u32>().is_ok()
}

pub fn load(req: &Request, env: &Env) -> State {
    let roots = local::candidate_roots(&req.game);
    load_from(req, env, &roots, local::language(), crate::art::net::local_only())
}

/// The hint for a game Steam has achievements for but nothing local is known.
pub fn has_achievements_hint(total: usize) -> String {
    let s = if total == 1 { "" } else { "s" };
    format!(
        "This game has {total} achievement{s}. Play it once through Steam to see your progress here, or add a Steam \
         Web API key in Settings → Achievements."
    )
}

/// What a remembered result was built from.
#[derive(Clone, Debug, PartialEq)]
struct Fingerprint {
    files: Vec<Option<(u64, SystemTime)>>,
    account: Option<u32>,
    lang: &'static str,
    key: bool,
}

struct Memo {
    fingerprint: Fingerprint,
    progress: Arc<Progress>,
    /// Pictures and rarity are all there (or Steam has none): no reason to build it again.
    complete: bool,
    built: Instant,
}

/// Parsed results per (cache folder, appid): page opens after the first are instant.
static MEMO: LazyLock<Mutex<HashMap<(PathBuf, u32), Memo>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// [`load`] with the Steam installs to look in, the language and the online switch given (tests).
pub(crate) fn load_from(req: &Request, env: &Env, roots: &[PathBuf], lang: &'static str, local_only: bool) -> State {
    let Ok(appid) = req.game.source_id.parse::<u32>() else {
        return State::None;
    };
    let key = secret(&req.config.steam_web_api_key);
    let w = Web {
        net: &*env.net,
        dir: env.cache_dir.join("steam").join(appid.to_string()),
        appid,
        local_only,
        secrets: req.config.secrets(),
    };
    let files = local::find(roots, appid);
    let Some(schema) = &files.schema else {
        return from_web(&w, &files, key, lang, req.force);
    };
    let fingerprint = Fingerprint { files: files.fingerprint(), account: files.account, lang, key: key.is_some() };
    let memo_key = (env.cache_dir.clone(), appid);
    if !req.force
        && let Some(m) = MEMO.lock().get(&memo_key)
        && m.fingerprint == fingerprint
        && (m.complete || m.built.elapsed() < RETRY_AFTER)
    {
        return State::Ready(m.progress.clone());
    }
    let Some(defs) = local::read_schema(schema, lang) else {
        // Unreadable list (being rewritten by Steam?): what the web knows.
        return from_web(&w, &files, key, lang, req.force);
    };
    if defs.is_empty() {
        // Stats only (playtime counters etc.), no achievements.
        return State::None;
    }
    let (progress, complete) = from_files(&w, &files, &defs, key, lang, req.force);
    let progress = Arc::new(progress);
    MEMO.lock().insert(memo_key, Memo { fingerprint, progress: progress.clone(), complete, built: Instant::now() });
    State::Ready(progress)
}

/// The list from the schema file and the unlocks from the player's file (or, without that file,
/// from the key lookup). The bool: nothing is missing for a network reason.
fn from_files(
    w: &Web,
    files: &local::Files,
    defs: &[local::Def],
    key: Option<&str>,
    lang: &str,
    force: bool,
) -> (Progress, bool) {
    let mut complete = true;
    let unlocks = files.user_stats.as_deref().and_then(local::read_user_stats);
    let mut by_name: Option<HashMap<String, i64>> = None;
    let mut note = None;
    if unlocks.is_none() {
        note = Some(PLAY_ONCE.to_string());
        if let (Some(key), Some(account)) = (key, files.account) {
            match web::player_achievements(w, key, account, lang, force) {
                Ok(m) => {
                    by_name = Some(m);
                    note = None;
                    // Web progress goes stale without a file changing: ask again later.
                    complete = false;
                }
                Err(e) => {
                    complete &= log_fail(w, "your progress", &e);
                    if matches!(e, Fail::Refused(_)) {
                        note = Some(NOT_SHARED.to_string());
                    }
                }
            }
        }
    }
    let synced = unlocks.is_some() || by_name.is_some();
    let ids: Vec<&str> = defs.iter().map(|d| d.id.as_str()).collect();
    let (rarity, rarity_complete) = rarity(w, files.library_json.as_deref(), &ids);
    complete &= rarity_complete;
    let mut list: Vec<Achievement> = defs
        .iter()
        .map(|d| {
            let unlocked_at = match (&unlocks, &by_name) {
                (Some(u), _) => u.get(&(d.stat.clone(), d.bit)).copied(),
                (None, Some(m)) => m.get(&d.id).copied(),
                (None, None) => None,
            };
            achievement(
                &d.id,
                &d.name,
                &d.desc,
                d.hidden,
                (d.icon.clone(), d.icon_gray.clone()),
                unlocked_at,
                rarity.get(&d.id).copied(),
            )
        })
        .collect();
    complete &= attach_icons(w, &mut list);
    (progress(list, files.account, w.appid, note, synced), complete)
}

/// No schema file: the keyless list (cached a week) and, with a key, the player's unlocks.
fn from_web(w: &Web, files: &local::Files, key: Option<&str>, lang: &str, force: bool) -> State {
    let list = match web::game_achievements(w, lang) {
        Ok(l) => l,
        Err(e) => {
            // Can't tell whether the game has achievements: no section rather than a wrong hint.
            log_fail(w, "the achievement list", &e);
            return State::None;
        }
    };
    if list.is_empty() {
        return State::None;
    }
    let (Some(key), Some(account)) = (key, files.account) else {
        return State::Hint(has_achievements_hint(list.len()));
    };
    let (unlocks, note) = match web::player_achievements(w, key, account, lang, force) {
        Ok(m) => (Some(m), None),
        Err(e) => {
            log_fail(w, "your progress", &e);
            let note = if matches!(e, Fail::Refused(_)) { NOT_SHARED } else { UNREACHABLE };
            (None, Some(note.to_string()))
        }
    };
    let synced = unlocks.is_some();
    let mut out: Vec<Achievement> = list
        .iter()
        .map(|a| {
            let unlocked_at = unlocks.as_ref().and_then(|m| m.get(&a.id).copied());
            achievement(
                &a.id,
                &a.name,
                &a.desc,
                a.hidden,
                (a.icon.clone(), a.icon_gray.clone()),
                unlocked_at,
                a.percent,
            )
        })
        .collect();
    attach_icons(w, &mut out);
    State::Ready(Arc::new(progress(out, files.account, w.appid, note, synced)))
}

/// Log a web failure (key scrubbed); true when it's final (no reason to try again soon).
fn log_fail(w: &Web, what: &str, e: &Fail) -> bool {
    let (Fail::Transient(why) | Fail::Refused(why)) = e;
    log::debug!("achievements: Steam app {}: no {what}: {}", w.appid, redact(why, &w.secrets));
    matches!(e, Fail::Refused(_))
}

/// One achievement; a locked hidden one keeps its name and description to itself. `icons` are
/// Steam's picture names here (local paths after [`attach_icons`]).
fn achievement(
    id: &str,
    name: &str,
    desc: &str,
    hidden: bool,
    icons: (Option<String>, Option<String>),
    unlocked_at: Option<i64>,
    rarity: Option<f32>,
) -> Achievement {
    let secret = hidden && unlocked_at.is_none();
    Achievement {
        id: id.to_string(),
        name: if secret { HIDDEN_NAME.to_string() } else { name.to_string() },
        description: if secret { String::new() } else { desc.to_string() },
        icon: icons.0,
        icon_locked: icons.1,
        unlocked_at,
        hardcore: false,
        hidden,
        rarity,
    }
}

fn progress(list: Vec<Achievement>, account: Option<u32>, appid: u32, note: Option<String>, synced: bool) -> Progress {
    let page_url = Some(match account {
        Some(a) => format!(
            "https://steamcommunity.com/profiles/{}/stats/{appid}/?tab=achievements",
            local::STEAMID64_BASE + a as u64
        ),
        None => format!("https://steamcommunity.com/stats/{appid}/achievements"),
    });
    Progress {
        service: Service::Steam,
        unlocked: list.iter().filter(|a| a.unlocked_at.is_some()).count() as u32,
        total: list.len() as u32,
        list,
        page_url,
        note,
        synced,
        probable: false,
    }
}

/// Global percentages: the library page cache when it has every achievement, else Steam's keyless
/// list (cached a week), gaps filled from the library cache. The bool: nothing missing for a
/// network reason.
fn rarity(w: &Web, library_json: Option<&Path>, ids: &[&str]) -> (HashMap<String, f32>, bool) {
    let lib = library_json.map(local::read_library_rarity).unwrap_or_default();
    if ids.iter().all(|id| lib.contains_key(*id)) {
        return (lib, true);
    }
    match web::global_percentages(w) {
        Ok(mut g) => {
            for (k, v) in lib {
                g.entry(k).or_insert(v);
            }
            (g, true)
        }
        Err(e) => (lib, log_fail(w, "rarity", &e)),
    }
}

/// The pictures to fetch, most wanted first: the one each achievement shows (the colour picture
/// once unlocked, the grey one while locked), newest unlocks first (the page's "recent" row),
/// then the rest in display order; after those, each achievement's other picture.
fn icon_order(list: &[Achievement]) -> Vec<String> {
    let mut order: Vec<&Achievement> = list.iter().collect();
    order.sort_by_key(|a| (a.unlocked_at.is_none(), std::cmp::Reverse(a.unlocked_at)));
    let shown = order.iter().map(|a| if a.unlocked_at.is_some() { &a.icon } else { &a.icon_locked });
    let other = order.iter().map(|a| if a.unlocked_at.is_some() { &a.icon_locked } else { &a.icon });
    shown.chain(other).flatten().cloned().collect()
}

/// Replace Steam's picture names with local copies (None when there's no copy). The bool: no
/// download failed for a network reason.
fn attach_icons(w: &Web, list: &mut [Achievement]) -> bool {
    let names = icon_order(list);
    let (paths, complete) = web::icons(w, &names);
    let local = |n: &Option<String>| n.as_ref().and_then(|n| paths.get(n)).map(|p| p.to_string_lossy().into_owned());
    for a in list {
        a.icon = local(&a.icon);
        a.icon_locked = local(&a.icon_locked);
    }
    complete
}
