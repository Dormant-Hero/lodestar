//! itch (itch.io app) scanner.
//!
//! Data: butler's SQLite DB `~/.config/itch/db/butler.db` (native) or
//! `~/.var/app/io.itch.itch/config/itch/db/butler.db` (Flatpak), opened read-only + immutable.
//! Tables used: `caves` (one per install: id (uuid), game_id, installed_at, last_touched_at,
//! seconds_run, verdict (JSON: basePath + candidates with flavor), install_location_id,
//! install_folder_name), `games` (id, title, short_text, cover_url …), `install_locations` (id, path).
//! Columns are probed first, so schema drift degrades instead of failing.
//! Art: itch keeps no predictable local copy of covers, so art is left to the metadata fetcher.
//! Launch: `itch://caves/<cave id>/launch`.
//!
//! CONTRACT: `pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>>` (see scan/mod.rs).

use super::ScanContext;
use super::desktop::common;
use crate::model::*;
use anyhow::Context as _;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const FLATPAK_ID: &str = "io.itch.itch";

pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>> {
    let dbs =
        [ctx.config_home().join("itch/db/butler.db"), ctx.flatpak_app(FLATPAK_ID).join("config/itch/db/butler.db")];
    let mut out = vec![];
    let mut seen = HashSet::new();
    for db in dbs {
        if !db.is_file() {
            continue;
        }
        for g in read_db(&db).with_context(|| format!("reading {}", db.display()))? {
            if seen.insert(g.source_id.clone()) {
                out.push(g);
            }
        }
    }
    Ok(out)
}

fn read_db(path: &Path) -> anyhow::Result<Vec<ScannedGame>> {
    let conn = common::open_sqlite_ro(path)?;
    let caves = common::table_columns(&conn, "caves");
    let games = common::table_columns(&conn, "games");
    if !caves.contains("id") || !caves.contains("game_id") || !games.contains("title") {
        // A fresh itch install creates the DB lazily; no caves table = nothing installed.
        if caves.is_empty() && games.is_empty() {
            return Ok(vec![]);
        }
        anyhow::bail!("unexpected butler.db schema");
    }
    let locs = common::table_columns(&conn, "install_locations");
    let c = |col: &str| {
        if caves.contains(col) { format!("c.{col}") } else { "NULL".into() }
    };
    let g = |col: &str| {
        if games.contains(col) { format!("g.{col}") } else { "NULL".into() }
    };
    let (loc_join, loc_path) = if locs.contains("id") && locs.contains("path") && caves.contains("install_location_id")
    {
        ("LEFT JOIN install_locations l ON l.id = c.install_location_id", "l.path")
    } else {
        ("", "NULL")
    };
    let sql = format!(
        "SELECT c.id, g.title, {}, {}, {}, {}, {}, {}, {} FROM caves c JOIN games g ON g.id = c.game_id {loc_join}",
        c("seconds_run"),
        c("last_touched_at"),
        c("verdict"),
        c("install_folder_name"),
        loc_path,
        g("short_text"),
        c("installed_size"),
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map([], |r| {
        Ok(Row {
            cave_id: text(r, 0),
            title: text(r, 1),
            seconds_run: r.get::<_, Option<i64>>(2).ok().flatten(),
            last_touched: text(r, 3),
            verdict: text(r, 4),
            folder: text(r, 5),
            location: text(r, 6),
            short_text: text(r, 7),
            size: r.get::<_, Option<i64>>(8).ok().flatten(),
        })
    })?;
    let mut out = vec![];
    for row in rows {
        match row {
            Ok(r) => {
                if let Some(g) = r.into_game() {
                    out.push(g);
                }
            }
            Err(e) => log::warn!("itch: skipping cave: {e}"),
        }
    }
    Ok(out)
}

fn text(r: &rusqlite::Row, i: usize) -> String {
    use rusqlite::types::ValueRef;
    match r.get_ref(i) {
        Ok(ValueRef::Text(t)) => String::from_utf8_lossy(t).trim().to_string(),
        Ok(ValueRef::Integer(n)) => n.to_string(),
        _ => String::new(),
    }
}

struct Row {
    cave_id: String,
    title: String,
    seconds_run: Option<i64>,
    last_touched: String,
    verdict: String,
    folder: String,
    location: String,
    short_text: String,
    size: Option<i64>,
}

impl Row {
    fn into_game(self) -> Option<ScannedGame> {
        if self.cave_id.is_empty() || self.title.is_empty() {
            return None;
        }
        let mut g = ScannedGame::new(Source::Itch, &self.cave_id, &self.title);
        g.launch = Some(LaunchSpec::Url(format!("itch://caves/{}/launch", self.cave_id)));
        g.playtime_secs = self.seconds_run.filter(|&s| s > 0).map(|s| s as u64);
        g.last_played = common::parse_datetime(&self.last_touched, false);
        g.size_bytes = self.size.filter(|&s| s > 0).map(|s| s as u64);
        if !self.short_text.is_empty() {
            g.meta.description = Some(self.short_text.clone());
        }
        let verdict: serde_json::Value = serde_json::from_str(&self.verdict).unwrap_or(serde_json::Value::Null);
        let base = verdict.get("basePath").and_then(|v| v.as_str()).filter(|p| p.starts_with('/'));
        g.install_dir = base.map(PathBuf::from).or_else(|| {
            (self.location.starts_with('/') && !self.folder.is_empty() && !self.folder.contains(".."))
                .then(|| Path::new(&self.location).join(&self.folder))
        });
        if let Some(dir) = &g.install_dir {
            g.installed = dir.exists();
        }
        g.platform = verdict
            .get("candidates")
            .and_then(|c| c.as_array())
            .and_then(|c| c.first())
            .and_then(|c| c.get("flavor"))
            .and_then(|f| f.as_str())
            .and_then(flavor_platform);
        Some(g)
    }
}

fn flavor_platform(f: &str) -> Option<String> {
    Some(
        match f {
            "windows" | "script-windows" | "msi" => "windows",
            "linux" | "script" | "jar" | "love" => "linux",
            "macos" | "app-macos" => "mac",
            "html" => "browser",
            _ => return None,
        }
        .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::super::desktop::test_support::*;
    use super::*;

    fn make_db(path: &Path, home: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(&std::fs::read_to_string(fixture("itch/schema.sql")).unwrap()).unwrap();
        let rows = std::fs::read_to_string(fixture("itch/data.sql")).unwrap();
        conn.execute_batch(&rows.replace("/HOME", &home.to_string_lossy())).unwrap();
    }

    #[test]
    fn parses_fixture_db() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        make_db(&home.join(".config/itch/db/butler.db"), home);
        std::fs::create_dir_all(home.join(".config/itch/apps/celeste")).unwrap();
        let mut games = scan(&ctx(home)).unwrap();
        games.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(games.len(), 2, "{games:#?}");
        let c = &games[0];
        assert_eq!(c.name, "Celeste");
        assert_eq!(c.source_id, "0b5ac1e5-6f2d-4a3e-9d4c-1a2b3c4d5e6f");
        assert_eq!(c.launch, Some(LaunchSpec::Url("itch://caves/0b5ac1e5-6f2d-4a3e-9d4c-1a2b3c4d5e6f/launch".into())));
        assert_eq!(c.playtime_secs, Some(7200));
        assert_eq!(c.last_played, Some(1_682_971_200));
        assert_eq!(c.platform.as_deref(), Some("linux"));
        assert!(c.installed);
        assert_eq!(c.install_dir, Some(home.join(".config/itch/apps/celeste")));
        assert_eq!(c.meta.description.as_deref(), Some("Help Madeline survive her inner demons"));
        let w = &games[1];
        assert_eq!(w.name, "Windows Thing");
        assert_eq!(w.platform.as_deref(), Some("windows"));
        assert!(!w.installed, "folder missing");
        assert_eq!(w.install_dir, Some(home.join("Games/itch/windows-thing")));
    }

    #[test]
    fn empty_and_corrupt() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        assert!(scan(&ctx(home)).unwrap().is_empty());
        let p = home.join(".config/itch/db/butler.db");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        rusqlite::Connection::open(&p).unwrap().execute_batch("CREATE TABLE x (a)").unwrap();
        assert!(scan(&ctx(home)).unwrap().is_empty(), "fresh DB without caves");
        std::fs::write(&p, "garbage garbage garbage").unwrap();
        assert!(scan(&ctx(home)).is_err());
    }
}
