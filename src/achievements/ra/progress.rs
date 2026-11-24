//! A game's achievements and the player's progress (`API_GetGameInfoAndUserProgress`), cached
//! per user for 10 minutes, and the badge pictures (kept for good: a badge name never changes).

use super::api::{ApiErr, Client, date, num, text};
use super::matching::now;
use crate::achievements::{Achievement, Progress, Service};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A shown game's progress is asked again after this long (sooner after the game was played).
pub const PROGRESS_MAX_AGE: i64 = 10 * 60;
/// Badge downloads stop after this long per page (the rest come the next time).
const BADGE_BUDGET: Duration = Duration::from_secs(15);

/// What `API_GetGameInfoAndUserProgress` said, cached.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cached {
    pub fetched: i64,
    pub body: serde_json::Value,
}

/// A safe file-name part for a user (the ULID or the lower-case username).
pub(crate) fn user_key(user: &str) -> String {
    user.trim().to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '-' }).collect()
}

fn cache_file(dir: &Path, user: &str, game: u32) -> PathBuf {
    dir.join("progress").join(user_key(user)).join(format!("{game}.json"))
}

/// The answer for (user, game): a fresh cached one unless `force`, else asked; when asking fails,
/// an older cached answer.
pub fn fetch(c: &Client, dir: &Path, user: &str, game: u32, force: bool) -> Result<serde_json::Value, ApiErr> {
    let file = cache_file(dir, user, game);
    let cached: Option<Cached> = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok());
    if !force
        && let Some(cc) = &cached
        && now() - cc.fetched < PROGRESS_MAX_AGE
    {
        return Ok(cc.body.clone());
    }
    let asked =
        c.call("GetGameInfoAndUserProgress", &[("g", game.to_string()), ("u", user.to_string()), ("a", "1".into())]);
    match asked {
        Ok(body) => {
            if let Some(d) = file.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if let Ok(b) = serde_json::to_vec(&Cached { fetched: now(), body: body.clone() }) {
                let _ = crate::fsutil::write_atomic(&file, &b);
            }
            Ok(body)
        }
        Err(e) => match cached {
            Some(cc) => {
                log::info!("RetroAchievements: showing the saved progress of game {game} ({e:?})");
                Ok(cc.body)
            }
            None => Err(e),
        },
    }
}

/// A parsed answer: None when RetroAchievements doesn't know the user or the game (`{}` / `[]`).
pub fn parse(v: &serde_json::Value) -> Option<Progress> {
    let id = num(v, "ID")?;
    let players = num(v, "NumDistinctPlayers").unwrap_or(0);
    let mut list: Vec<(u64, u64, Achievement)> = vec![];
    if let Some(map) = v.get("Achievements").and_then(|a| a.as_object()) {
        for a in map.values() {
            let Some(aid) = num(a, "ID") else { continue };
            let soft = date(a, "DateEarned");
            let hard = date(a, "DateEarnedHardcore");
            let rarity = (players > 0)
                .then(|| num(a, "NumAwarded").map(|n| (n as f32 / players as f32 * 100.0).clamp(0.0, 100.0)))
                .flatten();
            list.push((
                num(a, "DisplayOrder").unwrap_or(0),
                aid,
                Achievement {
                    id: aid.to_string(),
                    name: text(a, "Title").unwrap_or_default(),
                    description: text(a, "Description").unwrap_or_default(),
                    // Badge names for now; `badges` turns them into local pictures.
                    icon: text(a, "BadgeName"),
                    icon_locked: None,
                    unlocked_at: soft.or(hard),
                    hardcore: hard.is_some(),
                    hidden: false,
                    rarity,
                },
            ));
        }
    }
    list.sort_by_key(|(order, aid, _)| (*order, *aid));
    let list: Vec<Achievement> = list.into_iter().map(|(_, _, a)| a).collect();
    let unlocked = list.iter().filter(|a| a.unlocked_at.is_some()).count() as u32;
    Some(Progress {
        service: Service::RetroAchievements,
        unlocked,
        total: list.len() as u32,
        list,
        page_url: Some(format!("{}/game/{id}", super::api::SITE)),
        note: None,
        synced: true,
        probable: false,
    })
}

/// A badge name that is safe as a file name (RetroAchievements' are digits).
fn safe_badge(b: &str) -> bool {
    !b.is_empty() && b.len() <= 64 && b.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Turn the badge names in `icon` into local pictures in `<dir>/badges/`: the unlocked picture
/// for unlocked achievements, the grey `_lock` one for locked ones (downloaded when missing,
/// recent unlocks first, within a time budget); `icon` / `icon_locked` are set for every
/// picture that is on disk.
pub fn badges(c: &Client, dir: &Path, p: &mut Progress) {
    let bdir = dir.join("badges");
    let path = |badge: &str, locked: bool| bdir.join(format!("{badge}{}.png", if locked { "_lock" } else { "" }));
    // Download order: the newest unlocks (the page shows five), then display order.
    let mut order: Vec<usize> = (0..p.list.len()).collect();
    order.sort_by_key(|&i| (p.list[i].unlocked_at.is_none(), std::cmp::Reverse(p.list[i].unlocked_at)));
    let start = Instant::now();
    let mut online = !c.local_only;
    for i in order {
        let a = &mut p.list[i];
        let Some(badge) = a.icon.take().filter(|b| safe_badge(b)) else { continue };
        let locked = a.unlocked_at.is_none();
        let want = path(&badge, locked);
        if online && !want.is_file() && start.elapsed() < BADGE_BUDGET {
            match c.media(&format!("Badge/{}", want.file_name().unwrap_or_default().to_string_lossy())) {
                Ok(png) if png.starts_with(b"\x89PNG") => {
                    let _ = std::fs::create_dir_all(&bdir);
                    if let Err(e) = crate::fsutil::write_atomic(&want, &png) {
                        log::warn!("RetroAchievements: couldn't save a badge: {e}");
                    }
                }
                Ok(_) | Err(ApiErr::NotFound) | Err(ApiErr::Status(_)) => {}
                // Offline / refused: no more tries this time.
                Err(_) => online = false,
            }
        }
        let (on, off) = (path(&badge, false), path(&badge, true));
        a.icon = on.is_file().then(|| on.to_string_lossy().into_owned());
        a.icon_locked = off.is_file().then(|| off.to_string_lossy().into_owned());
    }
}
