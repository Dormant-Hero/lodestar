//! Folding one library row into another when both are the same game (a second copy of a ROM,
//! a RomM game now on disk, a shortcut into another library's game).
//!
//! What the user gave the row that goes moves to the row that stays: favourite, playtime and
//! play count (added up), last played (the later one), tags (both sets), custom name, rating,
//! own artwork, launch / controller overrides and edits (each only where the row that stays has
//! none of its own), and its places in the user's collections. Hidden is not carried over: hiding
//! the extra copy was the way to tidy the list, and it must not hide the game that stays.
//! Fetched artwork / metadata move only when the row that stays has none.

use crate::model::sort_key;
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

/// Move what the user gave row `from` to row `to`, then delete `from`.
pub(super) fn fold_into(conn: &Connection, from: i64, to: i64) -> Result<()> {
    move_user_data(conn, from, to)?;
    replace_in_collections(conn, from, Some(to))?;
    conn.execute("DELETE FROM games WHERE id = ?1", [from])?;
    Ok(())
}

#[derive(Default)]
struct UserData {
    favorite: bool,
    playtime: i64,
    play_count: i64,
    last_played: Option<i64>,
    user_name: Option<String>,
    user_rating: Option<i64>,
    tags: Vec<String>,
    user_art: String,
    launch_override: Option<String>,
    controller_override: Option<String>,
    edits: String,
    fetched_at: Option<i64>,
    fetched_art: String,
    fetched_meta: String,
    added: Option<i64>,
}

fn read(conn: &Connection, id: i64) -> Result<UserData> {
    Ok(conn.query_row(
        "SELECT favorite, playtime_secs, play_count, last_played, user_name, user_rating, tags_json, user_art_json, \
         launch_override_json, controller_override, edits_json, fetched_at, fetched_art_json, fetched_meta_json, added \
         FROM games WHERE id = ?1",
        [id],
        |r| {
            Ok(UserData {
                favorite: r.get::<_, i64>(0)? != 0,
                playtime: r.get(1)?,
                play_count: r.get(2)?,
                last_played: r.get(3)?,
                user_name: r.get(4)?,
                user_rating: r.get(5)?,
                tags: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
                user_art: r.get(7)?,
                launch_override: r.get(8)?,
                controller_override: r.get(9)?,
                edits: r.get(10)?,
                fetched_at: r.get(11)?,
                fetched_art: r.get(12)?,
                fetched_meta: r.get(13)?,
                added: r.get(14)?,
            })
        },
    )?)
}

fn empty_json(s: &str) -> bool {
    matches!(s.trim(), "" | "{}" | "null")
}

/// Row `old` goes (folded into `new`, or deleted): its place in the manual collections goes to
/// `new` (or it leaves them). SQLite may give a deleted row's id to a new game, which would
/// otherwise turn up in the collection.
pub(super) fn replace_in_collections(conn: &Connection, old: i64, new: Option<i64>) -> Result<()> {
    use crate::library::user;
    let Some(s) =
        conn.query_row("SELECT value FROM kv WHERE key = ?1", [user::KV], |r| r.get::<_, String>(0)).optional()?
    else {
        return Ok(());
    };
    let mut colls = user::parse(&s);
    if user::replace_in(&mut colls, old, new) {
        conn.execute("UPDATE kv SET value = ?2 WHERE key = ?1", params![user::KV, serde_json::to_string(&colls)?])?;
    }
    Ok(())
}

/// Move what the user gave row `from` onto row `to` (see the module docs).
fn move_user_data(conn: &Connection, from: i64, to: i64) -> Result<()> {
    let a = read(conn, from)?;
    let mut b = read(conn, to)?;
    b.favorite |= a.favorite;
    b.playtime = b.playtime.max(0).saturating_add(a.playtime.max(0));
    b.play_count = b.play_count.max(0).saturating_add(a.play_count.max(0));
    b.last_played = match (b.last_played, a.last_played) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, y) => x.or(y),
    };
    let renamed = b.user_name.is_none() && a.user_name.is_some();
    b.user_name = b.user_name.or(a.user_name);
    b.user_rating = b.user_rating.or(a.user_rating);
    for t in a.tags {
        if !b.tags.contains(&t) {
            b.tags.push(t);
        }
    }
    if empty_json(&b.user_art) && !empty_json(&a.user_art) {
        b.user_art = a.user_art;
    }
    b.launch_override = b.launch_override.or(a.launch_override);
    b.controller_override = b.controller_override.or(a.controller_override);
    if empty_json(&b.edits) && !empty_json(&a.edits) {
        b.edits = a.edits;
    }
    if b.fetched_at.is_none() && a.fetched_at.is_some() {
        b.fetched_at = a.fetched_at;
        b.fetched_art = a.fetched_art;
        b.fetched_meta = a.fetched_meta;
    }
    b.added = match (b.added, a.added) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, y) => x.or(y),
    };
    conn.execute(
        "UPDATE games SET favorite=?2, playtime_secs=?3, play_count=?4, last_played=?5, user_name=?6, user_rating=?7, \
         tags_json=?8, user_art_json=?9, launch_override_json=?10, controller_override=?11, edits_json=?12, \
         fetched_at=?13, fetched_art_json=?14, fetched_meta_json=?15, added=COALESCE(?16, added) WHERE id=?1",
        params![
            to,
            b.favorite as i64,
            b.playtime,
            b.play_count,
            b.last_played,
            b.user_name,
            b.user_rating,
            serde_json::to_string(&b.tags)?,
            b.user_art,
            b.launch_override,
            b.controller_override,
            b.edits,
            b.fetched_at,
            b.fetched_art,
            b.fetched_meta,
            b.added,
        ],
    )?;
    if renamed && let Some(n) = &b.user_name {
        conn.execute("UPDATE games SET sort_name = ?2 WHERE id = ?1", params![to, sort_key(n)])?;
    }
    Ok(())
}
