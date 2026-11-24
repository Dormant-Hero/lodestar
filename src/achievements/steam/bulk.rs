//! Achievement counts for every Steam game at once, for the library (covers, the List column,
//! controller themes): the Steam client's achievement lists and the player's unlock files, read
//! in one pass — counts only, no pictures, no network. Each Steam install's stats folder is
//! listed once; a game's files are parsed again only when their size or time changed, so the
//! pass repeated every half minute costs one folder listing.

use super::local;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Size and modification time of a file.
type Stamp = (u64, SystemTime);

/// One Steam install's stats folder, listed.
#[derive(Debug, Default)]
struct StatsDir {
    /// appid → the achievement list (`UserGameStatsSchema_<appid>.bin`).
    schemas: HashMap<u32, (PathBuf, Stamp)>,
    /// appid → the player's unlocks: the active account's file, else the newest of any account.
    user: HashMap<u32, (PathBuf, Stamp)>,
}

fn stamp(m: &std::fs::Metadata) -> Option<Stamp> {
    (m.is_file() && m.len() > 0 && m.len() <= local::MAX_FILE).then_some(())?;
    Some((m.len(), m.modified().ok()?))
}

fn list(root: &Path) -> StatsDir {
    let mut out = StatsDir::default();
    let Ok(rd) = std::fs::read_dir(root.join("appcache/stats")) else { return out };
    let active: Option<u32> =
        crate::scan::steam::active_user(root).and_then(|d| d.file_name()?.to_str()?.parse().ok()).filter(|a| *a != 0);
    // appid → (is the active account's, stamp, path)
    let mut users: HashMap<u32, (bool, Stamp, PathBuf)> = HashMap::new();
    for e in rd.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(rest) = name.strip_suffix(".bin") else { continue };
        if let Some(appid) = rest.strip_prefix("UserGameStatsSchema_").and_then(|a| a.parse::<u32>().ok()) {
            if let Some(s) = e.metadata().ok().as_ref().and_then(stamp) {
                out.schemas.insert(appid, (e.path(), s));
            }
        } else if let Some((account, appid)) = rest
            .strip_prefix("UserGameStats_")
            .and_then(|r| r.split_once('_'))
            .and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?)))
        {
            let Some(s) = e.metadata().ok().as_ref().and_then(stamp) else { continue };
            let mine = Some(account) == active;
            let better = users.get(&appid).is_none_or(|(m, old, _)| (mine, s.1) > (*m, old.1));
            if better {
                users.insert(appid, (mine, s, e.path()));
            }
        }
    }
    out.user = users.into_iter().map(|(appid, (_, s, p))| (appid, (p, s))).collect();
    out
}

/// What a game's files say: (unlocked, total) once the player's progress is known; None when
/// the game has no achievements or the player's file isn't there yet (not played on this
/// machine).
pub type Count = Option<(u32, u32)>;

/// Counts already parsed, by appid, with the files' stamps they came from.
#[derive(Default)]
pub struct Memo {
    known: HashMap<u32, ((Stamp, Option<Stamp>), Count)>,
}

/// The Steam installs to look in: every install the library scan finds, plus the ones the
/// games are installed in (a custom Steam folder).
pub fn roots(install_dirs: &[&Path]) -> Vec<PathBuf> {
    let mut out = local::candidate_roots(&crate::model::Game::blank(""));
    for dir in install_dirs {
        if let Some(r) = local::install_root(dir)
            && !out.contains(&r)
        {
            out.push(r);
        }
    }
    out
}

/// Counts for `appids` from the first install that has each game's achievement list. Games
/// without a list anywhere are left out (the game page may still know them from the web).
pub fn counts(roots: &[PathBuf], appids: &[u32], memo: &mut Memo) -> HashMap<u32, Count> {
    let dirs: Vec<StatsDir> = roots.iter().map(|r| list(r)).collect();
    let mut out = HashMap::new();
    for &appid in appids {
        let Some(dir) = dirs.iter().find(|d| d.schemas.contains_key(&appid)) else { continue };
        let (schema, s_stamp) = &dir.schemas[&appid];
        let user = dir.user.get(&appid);
        let key = (*s_stamp, user.map(|u| u.1));
        if let Some((k, c)) = memo.known.get(&appid)
            && *k == key
        {
            out.insert(appid, *c);
            continue;
        }
        let c = count(schema, user.map(|u| u.0.as_path()));
        // An unreadable list (Steam rewriting it?) is tried again next time.
        if let Some(c) = c {
            memo.known.insert(appid, (key, c));
            out.insert(appid, c);
        }
    }
    out
}

/// None: the list couldn't be read; Some(None): nothing to show.
fn count(schema: &Path, user: Option<&Path>) -> Option<Count> {
    let defs = local::read_schema(schema, "english")?;
    if defs.is_empty() {
        return Some(None);
    }
    let Some(unlocks) = user.and_then(local::read_user_stats) else { return Some(None) };
    let unlocked = defs.iter().filter(|d| unlocks.contains_key(&(d.stat.clone(), d.bit))).count() as u32;
    Some(Some((unlocked, defs.len() as u32)))
}
