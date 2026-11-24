//! RetroAchievements for emulated games, from its public Web API with the user's own username
//! and Web API key (`api`). A game is found by the id the user set, its ROM hash, its ROM
//! database name or its title (`matching`, `hash`); its achievements and the player's progress
//! come from `API_GetGameInfoAndUserProgress` (`progress`). Everything is cached in
//! `<cache>/achievements/ra/`, per user, so pages show offline and in screenshot runs too.

pub mod api;
pub mod bulk;
pub mod emulators;
pub mod hash;
pub mod matching;
pub mod progress;

use super::{Env, RaAccount, Request, State};
use crate::model::Game;
use api::{ApiErr, Client};
use std::sync::Arc;

pub const HINT_SET_UP: &str =
    "Add your RetroAchievements username and Web API key in Settings → Achievements to see your progress.";
const HINT_BAD_KEY: &str = "RetroAchievements didn't accept your Web API key. Check it in Settings → Achievements.";
const HINT_OFFLINE: &str = "Couldn't reach RetroAchievements. Your progress shows here once you're online.";
const HINT_DOWN: &str = "RetroAchievements isn't answering right now. Your progress shows here once it is.";
const NOTE_TITLE: &str = "Matched by title — your version may not be supported";

/// An emulated game.
pub fn applies(g: &Game) -> bool {
    g.emu.is_some()
}

/// The plain line for a failed lookup (None: show nothing).
fn hint(e: &ApiErr) -> State {
    match e {
        ApiErr::LocalOnly | ApiErr::NotFound | ApiErr::Garbled | ApiErr::Status(_) => State::None,
        ApiErr::Offline => State::Hint(HINT_OFFLINE.into()),
        ApiErr::BadKey => State::Hint(HINT_BAD_KEY.into()),
        ApiErr::Down(_) => State::Hint(HINT_DOWN.into()),
    }
}

/// Look a game up (worker thread).
pub fn load(req: &Request, env: &Env) -> State {
    load_with(req, env, &|key, secrets| Client::new(env.net.as_ref(), key, secrets))
}

pub(crate) fn load_with<'a>(req: &Request, env: &'a Env, client: &dyn Fn(&str, Vec<String>) -> Client<'a>) -> State {
    let cfg = &req.config;
    let user = cfg.ra_username.trim();
    let key = cfg.ra_api_key.as_deref().map(str::trim).unwrap_or("");
    if user.is_empty() || key.is_empty() {
        return State::Hint(HINT_SET_UP.into());
    }
    let Some(emu) = &req.game.emu else { return State::None };
    let manual = req.game.meta.ra_game_id.filter(|i| *i > 0);
    let Some(console) = hash::console(&emu.platform) else {
        // A console RetroAchievements has nothing for (unless the user set an id anyway).
        return match manual {
            Some(id) => progress_of(req, env, &client(key, cfg.secrets()), id, matching::How::Manual),
            None => State::None,
        };
    };
    let c = client(key, cfg.secrets());
    let dir = env.cache_dir.join("ra");
    let found = match matching::find(&c, &dir, &req.game, console) {
        Ok(Some(f)) => f,
        Ok(None) => return State::None,
        Err(e) => return hint(&e),
    };
    progress_of(req, env, &c, found.0, found.1)
}

fn progress_of(req: &Request, env: &Env, c: &Client, game: u32, how: matching::How) -> State {
    let cfg = &req.config;
    let dir = env.cache_dir.join("ra");
    let user = cfg.ra_user();
    let body = match progress::fetch(c, &dir, user, game, req.force) {
        Ok(b) => b,
        Err(ApiErr::NotFound) if how == matching::How::Manual => return manual_unknown(game),
        Err(e) => return hint(&e),
    };
    let Some(mut p) = progress::parse(&body) else {
        // `{}` / `[]`: RetroAchievements knows no such game, or no such user.
        if how == matching::How::Manual {
            return manual_unknown(game);
        }
        return State::Hint(format!(
            "RetroAchievements doesn't know the user “{}”. Check it in Settings → Achievements.",
            cfg.ra_username.trim()
        ));
    };
    if p.list.is_empty() {
        return State::None;
    }
    if how == matching::How::Title {
        p.note = Some(NOTE_TITLE.into());
        p.probable = true;
    }
    progress::badges(c, &dir, &mut p);
    State::Ready(Arc::new(p))
}

fn manual_unknown(game: u32) -> State {
    State::Hint(format!(
        "RetroAchievements has no game with the id {game}. Check it in Edit → Installation → RetroAchievements game id."
    ))
}

/// Check a username + Web API key with one profile call (blocking: call it off the UI thread).
/// Err = a plain sentence for the settings page ("RetroAchievements doesn't know this key").
pub fn check_account(username: &str, api_key: &str, env: &Env) -> Result<RaAccount, String> {
    let secrets =
        crate::config::AchievementsConfig { ra_api_key: Some(api_key.to_string()), ..Default::default() }.secrets();
    check_with(username, &Client::new(env.net.as_ref(), api_key, secrets))
}

pub(crate) fn check_with(username: &str, c: &Client) -> Result<RaAccount, String> {
    let u = username.trim();
    if u.is_empty() {
        return Err("Type your RetroAchievements username first.".into());
    }
    if c.is_keyless() {
        return Err("Paste your Web API key first. It is on retroachievements.org under Settings → Keys.".into());
    }
    let unknown = || format!("RetroAchievements doesn't know the user “{u}”. Check the spelling.");
    match c.call("GetUserProfile", &[("u", u.to_string())]) {
        Ok(v) => {
            let ulid = api::text(&v, "ULID");
            let name = api::text(&v, "User");
            match (name, ulid) {
                (Some(display_name), Some(ulid)) => Ok(RaAccount { display_name, ulid }),
                _ => Err(unknown()),
            }
        }
        Err(ApiErr::NotFound) => Err(unknown()),
        Err(ApiErr::BadKey) => Err(
            "RetroAchievements didn't accept this Web API key. Copy it again from retroachievements.org → Settings → Keys."
                .into(),
        ),
        Err(ApiErr::Offline) => {
            Err("Couldn't reach RetroAchievements. Check your internet connection and try again.".into())
        }
        Err(ApiErr::LocalOnly) => Err("Lodestar doesn't go online in this run.".into()),
        Err(ApiErr::Down(_)) => Err("RetroAchievements isn't answering right now. Try again in a few minutes.".into()),
        Err(ApiErr::Status(s)) => Err(format!("RetroAchievements gave an unexpected answer (error {s}). Try again later.")),
        Err(ApiErr::Garbled) => Err("RetroAchievements gave an unexpected answer. Try again later.".into()),
    }
}

#[cfg(test)]
mod tests;
