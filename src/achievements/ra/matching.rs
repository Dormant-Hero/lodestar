//! Finding a game's RetroAchievements id, best evidence first:
//!
//! 1. the id the user set by hand (`Metadata::ra_game_id`);
//! 2. the ROM's hash in the console's game list (`API_GetGameList?i=&f=1&h=1`, cached a week);
//! 3. the ROM database name (No-Intro / Redump `dat_name`, or the file name) equal to the name of
//!    one of the game's supported files (`API_GetGameHashes`, cached a week) — discs and dumps
//!    whose hash didn't match;
//! 4. the title alone → shown with "Matched by title — your version may not be supported".

use super::api::{ApiErr, Client, num, text};
use crate::model::Game;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Game lists and hash lists are asked again after this long.
pub const LIST_MAX_AGE: i64 = 7 * 24 * 3600;
/// Candidates whose file names are asked for (one call each).
const NAME_CANDIDATES: usize = 3;

/// How the id was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    Manual,
    Hash,
    Name,
    Title,
}

/// One console's games with achievements, as cached on disk.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GameList {
    pub fetched: i64,
    /// (id, title, number of achievements)
    pub games: Vec<(u32, String, u32)>,
    /// ROM hash (lower-case MD5) → game id.
    pub hashes: HashMap<String, u32>,
}

impl GameList {
    /// From `API_GetGameList` JSON.
    pub fn parse(v: &serde_json::Value, now: i64) -> Option<GameList> {
        let arr = v.as_array()?;
        let mut out = GameList { fetched: now, ..Default::default() };
        for g in arr {
            let Some(id) = num(g, "ID").and_then(|i| u32::try_from(i).ok()).filter(|i| *i > 0) else { continue };
            let title = text(g, "Title").unwrap_or_default();
            out.games.push((id, title, num(g, "NumAchievements").unwrap_or(0) as u32));
            for h in g.get("Hashes").and_then(|h| h.as_array()).into_iter().flatten() {
                if let Some(h) = h.as_str().filter(|h| h.len() == 32) {
                    out.hashes.insert(h.to_ascii_lowercase(), id);
                }
            }
        }
        Some(out)
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn read_json<T: for<'de> Deserialize<'de>>(p: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(p).ok()?).ok()
}

fn write_json<T: Serialize>(p: &Path, v: &T) {
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(b) = serde_json::to_vec(v)
        && let Err(e) = crate::fsutil::write_atomic(p, &b)
    {
        log::warn!("RetroAchievements: couldn't save {}: {e}", p.display());
    }
}

type ListMemo = HashMap<PathBuf, (Option<std::time::SystemTime>, Arc<GameList>)>;
/// Parsed lists kept in memory (they are big; parsing one every page view would be slow).
static LISTS: Mutex<Option<ListMemo>> = Mutex::new(None);

fn load_list_file(p: &Path) -> Option<Arc<GameList>> {
    let mtime = std::fs::metadata(p).ok()?.modified().ok();
    if let Some((t, l)) = LISTS.lock().as_ref().and_then(|m| m.get(p))
        && *t == mtime
    {
        return Some(l.clone());
    }
    let l = Arc::new(read_json::<GameList>(p)?);
    LISTS.lock().get_or_insert_with(HashMap::new).insert(p.to_path_buf(), (mtime, l.clone()));
    Some(l)
}

/// A console's game list: the cached one while it is less than a week old, else asked again
/// (the old one is kept when that fails). Err only when there is none at all.
pub fn game_list(c: &Client, dir: &Path, console: u32) -> Result<Arc<GameList>, ApiErr> {
    let file = dir.join("games").join(format!("{console}.json"));
    let cached = load_list_file(&file);
    if let Some(l) = &cached
        && now() - l.fetched < LIST_MAX_AGE
    {
        return Ok(l.clone());
    }
    let fetched = c
        .call("GetGameList", &[("i", console.to_string()), ("f", "1".into()), ("h", "1".into())])
        .and_then(|v| GameList::parse(&v, now()).ok_or(ApiErr::Garbled));
    match fetched {
        Ok(l) => {
            write_json(&file, &l);
            let l = Arc::new(l);
            let mtime = std::fs::metadata(&file).ok().and_then(|m| m.modified().ok());
            LISTS.lock().get_or_insert_with(HashMap::new).insert(file, (mtime, l.clone()));
            Ok(l)
        }
        Err(e) => cached.ok_or(e),
    }
}

#[derive(Serialize, Deserialize)]
struct CachedNames {
    fetched: i64,
    names: Vec<String>,
}

fn names_file(dir: &Path, game: u32) -> PathBuf {
    dir.join("game-files").join(format!("{game}.json"))
}

/// The names of a game's supported files as saved by an earlier lookup (whatever their age);
/// never asks RetroAchievements.
pub fn cached_file_names(dir: &Path, game: u32) -> Option<Vec<String>> {
    read_json::<CachedNames>(&names_file(dir, game)).map(|c| c.names)
}

/// The names of a game's supported files (`API_GetGameHashes`), cached a week.
pub fn file_names(c: &Client, dir: &Path, game: u32) -> Result<Vec<String>, ApiErr> {
    type Cached = CachedNames;
    let file = names_file(dir, game);
    let cached: Option<Cached> = read_json(&file);
    if let Some(cc) = &cached
        && now() - cc.fetched < LIST_MAX_AGE
    {
        return Ok(cc.names.clone());
    }
    match c.call("GetGameHashes", &[("i", game.to_string())]) {
        Ok(v) => {
            let names: Vec<String> = v
                .get("Results")
                .and_then(|r| r.as_array())
                .into_iter()
                .flatten()
                .filter_map(|h| text(h, "Name"))
                .collect();
            write_json(&file, &Cached { fetched: now(), names: names.clone() });
            Ok(names)
        }
        Err(e) => cached.map(|cc| cc.names).ok_or(e),
    }
}

/// A title for comparing: lower-case words, `(…)` / `[…]` tags and "the" dropped, `&` = "and".
pub fn norm_title(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0i32;
    for ch in s.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth > 0 => {}
            '&' => out.push_str(" and "),
            c if c.is_alphanumeric() => out.extend(c.to_lowercase()),
            _ => out.push(' '),
        }
    }
    out.split_whitespace().filter(|w| *w != "the").collect::<Vec<_>>().join(" ")
}

/// A file name for comparing: no folder, no extension, lower case.
fn norm_file(s: &str) -> String {
    let base = s.rsplit(['/', '\\']).next().unwrap_or(s);
    let stem = match base.rfind('.') {
        Some(i) if i > 0 && base.len() - i <= 5 => &base[..i],
        _ => base,
    };
    stem.trim().to_lowercase()
}

/// Hacks, homebrew, prototypes, subsets: never a title match unless named exactly.
fn special(title: &str) -> bool {
    title.starts_with('~') || title.contains("[Subset")
}

/// RetroAchievements games of a console whose title matches the game's names, best first.
fn candidates<'a>(list: &'a GameList, names: &[String]) -> Vec<&'a (u32, String, u32)> {
    let wanted: Vec<String> = names.iter().map(|n| norm_title(n)).filter(|n| !n.is_empty()).collect();
    let mut v: Vec<(u8, &(u32, String, u32))> = list
        .games
        .iter()
        .filter_map(|g| {
            let exact = names.iter().any(|n| n.trim().eq_ignore_ascii_case(g.1.trim()));
            if exact {
                return Some((0, g));
            }
            (!special(&g.1) && wanted.contains(&norm_title(&g.1))).then_some((1, g))
        })
        .collect();
    v.sort_by_key(|(rank, g)| (*rank, std::cmp::Reverse(g.2)));
    v.into_iter().map(|(_, g)| g).collect()
}

/// The names a game goes by: the ROM database name, the ROM's file name, the shown title.
fn names_of(game: &Game) -> (Vec<String>, Vec<String>) {
    let mut files = vec![];
    let mut titles = vec![];
    if let Some(e) = &game.emu {
        if let Some(d) = e.dat_name.as_deref().filter(|d| !d.trim().is_empty()) {
            files.push(d.to_string());
            titles.push(d.to_string());
        }
        if let Some(stem) = e.rom.file_stem().map(|s| s.to_string_lossy().into_owned()) {
            files.push(stem.clone());
            titles.push(stem);
        }
    }
    titles.insert(0, game.user_name.clone().unwrap_or_else(|| game.name.clone()));
    titles.push(game.name.clone());
    titles.dedup();
    (files, titles)
}

/// Find the game's RetroAchievements id. Ok(None): RetroAchievements has no match. Err: no
/// game list could be had (offline, …) and nothing else was known.
pub fn find(c: &Client, dir: &Path, game: &Game, console: super::hash::Console) -> Result<Option<(u32, How)>, ApiErr> {
    if let Some(id) = game.meta.ra_game_id.filter(|i| *i > 0) {
        return Ok(Some((id, How::Manual)));
    }
    let Some(emu) = &game.emu else { return Ok(None) };
    let hash = super::hash::hash_cached(dir, &emu.platform, &emu.rom);
    let mut lists = vec![];
    let mut first_err = None;
    for id in console.ids {
        match game_list(c, dir, *id) {
            Ok(l) => lists.push(l),
            Err(e) => {
                first_err.get_or_insert(e);
            }
        }
    }
    if lists.is_empty() {
        return Err(first_err.unwrap_or(ApiErr::Garbled));
    }
    if let Some(h) = &hash
        && let Some(id) = lists.iter().find_map(|l| l.hashes.get(h))
    {
        return Ok(Some((*id, How::Hash)));
    }
    let (files, titles) = names_of(game);
    let wanted_files: Vec<String> = files.iter().map(|f| norm_file(f)).collect();
    let mut title_match = None;
    for l in &lists {
        let cands = candidates(l, &titles);
        for g in cands.iter().take(NAME_CANDIDATES) {
            if wanted_files.is_empty() {
                break;
            }
            match file_names(c, dir, g.0) {
                Ok(names) if names.iter().any(|n| wanted_files.contains(&norm_file(n))) => {
                    return Ok(Some((g.0, How::Name)));
                }
                Ok(_) => {}
                // Offline after the lists: a title match is still worth showing.
                Err(_) => break,
            }
        }
        if title_match.is_none() {
            title_match = cands.first().map(|g| g.0);
        }
    }
    Ok(title_match.map(|id| (id, How::Title)))
}

/// The RetroAchievements id of a game for the library's counts: the id the user set, the ROM's
/// hash, or its ROM database / file name among the names already saved by a game-page lookup —
/// never a title alone (it may be another version) and never a request per game. `lists` are
/// the console's game lists (asked once per console by the caller); `hash` is the ROM's hash
/// when known.
pub fn find_for_library(dir: &Path, game: &Game, lists: &[Arc<GameList>], hash: Option<&str>) -> Option<u32> {
    if let Some(id) = game.meta.ra_game_id.filter(|i| *i > 0) {
        return Some(id);
    }
    game.emu.as_ref()?;
    if let Some(h) = hash
        && let Some(id) = lists.iter().find_map(|l| l.hashes.get(h))
    {
        return Some(*id);
    }
    let (files, titles) = names_of(game);
    let wanted: Vec<String> = files.iter().map(|f| norm_file(f)).collect();
    if wanted.is_empty() {
        return None;
    }
    for l in lists {
        for g in candidates(l, &titles).into_iter().take(NAME_CANDIDATES) {
            if let Some(names) = cached_file_names(dir, g.0)
                && names.iter().any(|n| wanted.contains(&norm_file(n)))
            {
                return Some(g.0);
            }
        }
    }
    None
}
