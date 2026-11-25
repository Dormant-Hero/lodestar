//! Keeping the art folder in step with the library. Downloaded and chosen pictures live in
//! folders named after the game's id (`sgdb/<id>`, `auto/<id>`, `user/<id>`). Before 0.17.2 an id
//! could be handed to a new game once its old game was gone, and the art fetcher, which reuses
//! files left by an interrupted run, then showed the old game's pictures for the new one. Ids are
//! no longer reused (`Db::NEXT_GAME_ID`); this removes the folders of games that are gone and,
//! once, drops pictures a game inherited that way so they are fetched again.

use crate::db::Db;
use crate::model::{Art, GameId};
use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

/// The per-game folders under the art folder (Steam's art is keyed by app id instead).
const PER_GAME: [&str; 3] = ["sgdb", "auto", "user"];
/// Folders the art fetcher fills on its own; a file there older than the game is inherited.
const FETCHED: [&str; 2] = ["sgdb", "auto"];
/// Allowance for clocks and for a scan that added the game a little after the file was written.
const SLACK: Duration = Duration::from_secs(300);
const REPAIRED_KEY: &str = "art_reused_ids_repaired";

/// What the clean-up needs to know about one game row.
pub struct ArtRow {
    pub id: GameId,
    /// When the row was added (unix seconds).
    pub added: i64,
    pub fetched: Art,
    /// Every art path the row points at (scanner, user and fetched art).
    pub referenced: Vec<PathBuf>,
}

/// What a clean-up did.
#[derive(Debug, Default, PartialEq)]
pub struct Tidied {
    pub folders_removed: usize,
    pub games_refetched: usize,
}

/// Remove folders of games that are gone, and (once per library) forget inherited pictures.
pub fn run(db: &Db, art_dir: &Path) -> Result<Tidied> {
    let rows = db.art_rows()?;
    let mut done = Tidied { folders_removed: remove_orphans(art_dir, &rows), ..Default::default() };
    if db.kv_get(REPAIRED_KEY).is_none() {
        for row in &rows {
            let (kept, stale) = split_inherited(art_dir, row);
            if stale.is_empty() {
                continue;
            }
            db.reset_fetched_art(row.id, &kept)?;
            for f in &stale {
                let _ = std::fs::remove_file(f);
            }
            done.games_refetched += 1;
        }
        db.kv_set(REPAIRED_KEY, "1")?;
    }
    Ok(done)
}

/// Delete `<art_dir>/<kind>/<n>` folders whose game is gone and that no game's art points into
/// (a merged multi-ROM game keeps pictures from the row it absorbed). Only numeric folder names
/// are touched.
pub fn remove_orphans(art_dir: &Path, rows: &[ArtRow]) -> usize {
    let mut live: HashSet<PathBuf> = HashSet::new();
    for row in rows {
        for kind in PER_GAME {
            live.insert(art_dir.join(kind).join(row.id.to_string()));
        }
        live.extend(row.referenced.iter().filter_map(|p| p.parent().map(Path::to_path_buf)));
    }
    let mut removed = 0;
    for kind in PER_GAME {
        let Ok(entries) = std::fs::read_dir(art_dir.join(kind)) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            let numeric =
                e.file_name().to_str().is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
            if numeric && e.file_type().is_ok_and(|t| t.is_dir()) && !live.contains(&path) {
                match std::fs::remove_dir_all(&path) {
                    Ok(()) => removed += 1,
                    Err(err) => log::warn!("couldn't remove {}: {err}", path.display()),
                }
            }
        }
    }
    removed
}

/// The row's fetched art without the pictures it inherited from an earlier game with its id, and
/// every inherited file: anything in its own fetched-art folders written before the row was
/// added, used or not (the fetcher would pick an unused one up again).
pub fn split_inherited(art_dir: &Path, row: &ArtRow) -> (Art, Vec<PathBuf>) {
    let added = UNIX_EPOCH + Duration::from_secs(row.added.max(0) as u64);
    let older = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).is_ok_and(|t| t + SLACK < added);
    let stale: Vec<PathBuf> = FETCHED
        .iter()
        .filter_map(|k| std::fs::read_dir(art_dir.join(k).join(row.id.to_string())).ok())
        .flat_map(|rd| rd.flatten().map(|e| e.path()))
        .filter(|p| p.is_file() && older(p))
        .collect();
    let mut kept = row.fetched.clone();
    for slot in [&mut kept.cover, &mut kept.hero, &mut kept.logo, &mut kept.icon, &mut kept.banner] {
        slot.take_if(|p| stale.contains(p));
    }
    (kept, stale)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path, age: Duration) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
        let f = std::fs::File::options().write(true).open(p).unwrap();
        f.set_modified(std::time::SystemTime::now() - age).unwrap();
    }

    fn now() -> i64 {
        std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
    }

    #[test]
    fn pictures_older_than_the_game_are_dropped_newer_ones_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let art = tmp.path();
        let day = Duration::from_secs(86_400);
        touch(&art.join("sgdb/7/cover.png"), 3 * day); // left by the id's earlier game
        touch(&art.join("sgdb/7/hero.png"), Duration::ZERO); // fetched for this game
        touch(&art.join("sgdb/3/logo.png"), 3 * day); // another row's folder: not judged here
        let row = ArtRow {
            id: 7,
            added: now() - day.as_secs() as i64,
            fetched: Art {
                cover: Some(art.join("sgdb/7/cover.png")),
                hero: Some(art.join("sgdb/7/hero.png")),
                logo: Some(art.join("sgdb/3/logo.png")),
                ..Default::default()
            },
            referenced: vec![],
        };
        touch(&art.join("auto/7/banner.jpg"), 3 * day); // unused, but the fetcher would reuse it
        let (kept, mut stale) = split_inherited(art, &row);
        stale.sort();
        assert_eq!(stale, vec![art.join("auto/7/banner.jpg"), art.join("sgdb/7/cover.png")]);
        assert_eq!(kept.cover, None);
        assert_eq!(kept.hero, row.fetched.hero);
        assert_eq!(kept.logo, row.fetched.logo);
    }

    #[test]
    fn only_folders_of_games_that_are_gone_are_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let art = tmp.path();
        for p in
            ["sgdb/1/cover.png", "sgdb/2/cover.png", "auto/9/hero.png", "user/4/logo.png", "user/collections/x.png"]
        {
            touch(&art.join(p), Duration::ZERO);
        }
        let rows = [
            ArtRow { id: 1, added: 0, fetched: Art::default(), referenced: vec![] },
            // Game 5 absorbed game 4 and still shows its logo.
            ArtRow { id: 5, added: 0, fetched: Art::default(), referenced: vec![art.join("user/4/logo.png")] },
        ];
        assert_eq!(remove_orphans(art, &rows), 2);
        assert!(art.join("sgdb/1").is_dir());
        assert!(!art.join("sgdb/2").exists());
        assert!(!art.join("auto/9").exists());
        assert!(art.join("user/4").is_dir(), "still pointed at");
        assert!(art.join("user/collections").is_dir(), "not a game folder");
    }
}
