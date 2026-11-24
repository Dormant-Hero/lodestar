//! Lutris scanner.
//!
//! Data: `~/.local/share/lutris/pga.db` (native) and
//! `~/.var/app/net.lutris.Lutris/data/lutris/pga.db` (Flatpak), opened read-only + immutable so a
//! running Lutris never sees a lock. `games` table: id, name, slug, runner, platform, installed,
//! playtime (hours, float), lastplayed (unix), directory, hidden (older versions), year,
//! installer_slug …  Store clients / launchers (`scan::clients`) are skipped.
//! Columns are probed with `PRAGMA table_info`, so old and new schemas both work.
//! Art: `coverart/<slug>.jpg` (data dir, or `~/.cache/lutris/coverart` in newer versions),
//! `banners/<slug>.jpg` (data dir or cache dir), icons
//! `~/.local/share/icons/hicolor/128x128/apps/lutris_<slug>.png`.
//!
//! CONTRACT: `pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>>` (see scan/mod.rs).

use super::ScanContext;
use super::desktop::common;
use super::util::image_with_stem;
use crate::model::*;
use anyhow::Context as _;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const FLATPAK_ID: &str = "net.lutris.Lutris";

struct Install {
    db: PathBuf,
    data: PathBuf,
    cache: PathBuf,
    icons: PathBuf,
    flatpak: bool,
}

fn installs(ctx: &ScanContext) -> Vec<Install> {
    let fp = ctx.flatpak_app(FLATPAK_ID);
    vec![
        Install {
            db: ctx.data_home().join("lutris/pga.db"),
            data: ctx.data_home().join("lutris"),
            cache: ctx.home.join(".cache/lutris"),
            icons: ctx.data_home().join("icons/hicolor"),
            flatpak: false,
        },
        Install {
            db: fp.join("data/lutris/pga.db"),
            data: fp.join("data/lutris"),
            cache: fp.join("cache/lutris"),
            icons: fp.join("data/icons/hicolor"),
            flatpak: true,
        },
    ]
}

pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>> {
    let mut out = vec![];
    let mut seen = HashSet::new();
    for inst in installs(ctx) {
        if !inst.db.is_file() {
            continue;
        }
        let games = read_db(ctx, &inst).with_context(|| format!("reading {}", inst.db.display()))?;
        for g in games {
            if seen.insert(g.source_id.clone()) {
                out.push(g);
            }
        }
    }
    Ok(out)
}

fn read_db(ctx: &ScanContext, inst: &Install) -> anyhow::Result<Vec<ScannedGame>> {
    let conn = common::open_sqlite_ro(&inst.db)?;
    let cols = common::table_columns(&conn, "games");
    if !cols.contains("id") || !cols.contains("name") {
        anyhow::bail!("no games table");
    }
    let col = |c: &str| {
        if cols.contains(c) { c.to_string() } else { "NULL".to_string() }
    };
    let sql = format!(
        "SELECT id, name, {}, {}, {}, {}, {}, {}, {}, {}, {} FROM games",
        col("slug"),
        col("runner"),
        col("platform"),
        col("installed"),
        col("playtime"),
        col("lastplayed"),
        col("directory"),
        col("year"),
        col("installer_slug"),
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map([], |r| {
        Ok(Row {
            id: r.get::<_, i64>(0)?,
            name: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            slug: text(r, 2),
            runner: text(r, 3),
            platform: text(r, 4),
            installed: r.get::<_, Option<i64>>(5).ok().flatten().unwrap_or(0) != 0,
            playtime_h: r.get::<_, Option<f64>>(6).ok().flatten(),
            lastplayed: r.get::<_, Option<i64>>(7).ok().flatten(),
            directory: text(r, 8),
            year: r.get::<_, Option<i64>>(9).ok().flatten(),
            installer_slug: text(r, 10),
        })
    })?;
    let mut out = vec![];
    for row in rows {
        let row = match row {
            Ok(r) => r,
            Err(e) => {
                log::warn!("lutris: skipping row: {e}");
                continue;
            }
        };
        if !row.installed && !ctx.config.include_uninstalled {
            continue;
        }
        // Store clients Lutris installed (Battle.net, EA app, Ubisoft Connect, Prism Launcher …):
        // their games are found by the Wine-prefix / Minecraft scanners.
        if super::clients::is_lutris_client(&row.slug, &row.installer_slug, &row.name) {
            continue;
        }
        out.push(row.into_game(inst));
    }
    Ok(out)
}

/// Lenient text column (Lutris has stored ints/blobs in odd places over the years).
fn text(r: &rusqlite::Row, i: usize) -> String {
    use rusqlite::types::ValueRef;
    match r.get_ref(i) {
        Ok(ValueRef::Text(t)) => String::from_utf8_lossy(t).trim().to_string(),
        Ok(ValueRef::Integer(n)) => n.to_string(),
        _ => String::new(),
    }
}

struct Row {
    id: i64,
    name: String,
    slug: String,
    runner: String,
    platform: String,
    installed: bool,
    playtime_h: Option<f64>,
    lastplayed: Option<i64>,
    directory: String,
    year: Option<i64>,
    installer_slug: String,
}

impl Row {
    fn into_game(self, inst: &Install) -> ScannedGame {
        let id = self.id.to_string();
        let mut g = ScannedGame::new(Source::Lutris, &id, self.name.trim());
        g.installed = self.installed;
        if self.directory.starts_with('/') {
            g.install_dir = Some(PathBuf::from(&self.directory));
        }
        let uri = format!("lutris:rungameid/{id}");
        g.launch = Some(if inst.flatpak {
            LaunchSpec::Command {
                program: "flatpak".into(),
                args: vec!["run".into(), FLATPAK_ID.into(), uri],
                cwd: None,
                env: vec![],
            }
        } else {
            LaunchSpec::Command { program: "lutris".into(), args: vec![uri], cwd: None, env: vec![] }
        });
        g.playtime_secs = self.playtime_h.filter(|h| h.is_finite() && *h > 0.0).map(|h| (h * 3600.0).round() as u64);
        g.last_played = self.lastplayed.filter(|&t| t > 0);
        g.platform = platform(&self.platform, &self.runner);
        g.meta.release_year = self.year.filter(|y| (1950..3000).contains(y)).map(|y| y as i32);
        if !self.slug.is_empty() && !self.slug.contains('/') {
            g.art = art(inst, &self.slug);
        }
        g
    }
}

fn platform(platform: &str, runner: &str) -> Option<String> {
    let p = platform.to_ascii_lowercase();
    if p.contains("windows") || runner == "wine" {
        Some("windows".into())
    } else if p.contains("linux") || runner == "linux" {
        Some("linux".into())
    } else if p.is_empty() {
        None
    } else {
        Some(p)
    }
}

fn art(inst: &Install, slug: &str) -> Art {
    let first = |dirs: &[&Path]| dirs.iter().find_map(|d| image_with_stem(d, slug));
    let cover = first(&[&inst.data.join("coverart"), &inst.cache.join("coverart")]);
    let banner = first(&[&inst.data.join("banners"), &inst.cache.join("banners")]);
    let icon_name = format!("lutris_{slug}");
    let icon = ["128x128", "256x256", "64x64", "48x48"]
        .iter()
        .find_map(|s| image_with_stem(&inst.icons.join(s).join("apps"), &icon_name))
        .or_else(|| image_with_stem(&inst.data.join("icons"), slug));
    Art { cover, hero: None, logo: None, icon, banner }
}

#[cfg(test)]
mod tests {
    use super::super::desktop::test_support::*;
    use super::*;

    /// Create a pga.db with the real Lutris schema (columns as of Lutris 0.5.x).
    fn make_db(path: &Path, with_hidden: bool) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(path).unwrap();
        let schema = std::fs::read_to_string(fixture("lutris/schema.sql")).unwrap();
        conn.execute_batch(&schema).unwrap();
        if with_hidden {
            conn.execute_batch("ALTER TABLE games ADD COLUMN hidden INTEGER").unwrap();
        }
        let rows = std::fs::read_to_string(fixture("lutris/games.sql")).unwrap();
        conn.execute_batch(&rows).unwrap();
    }

    #[test]
    fn parses_fixture_db() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let data = home.join(".local/share/lutris");
        make_db(&data.join("pga.db"), true);
        fake_png(&data.join("coverart/the-witcher-3.jpg"), 264, 352);
        fake_png(&home.join(".cache/lutris/banners/the-witcher-3.jpg"), 184, 69);
        fake_png(&home.join(".local/share/icons/hicolor/128x128/apps/lutris_supertuxkart.png"), 128, 128);

        let games = scan(&ctx(home)).unwrap();
        assert_eq!(games.len(), 2, "uninstalled skipped: {games:#?}");
        let w3 = &games[0];
        assert_eq!(w3.source_id, "1");
        assert_eq!(w3.name, "The Witcher 3: Wild Hunt");
        assert_eq!(w3.platform.as_deref(), Some("windows"));
        assert_eq!(w3.playtime_secs, Some(45_000)); // 12.5 h
        assert_eq!(w3.last_played, Some(1_700_000_000));
        assert_eq!(w3.install_dir.as_deref(), Some(Path::new("/home/u/Games/the-witcher-3")));
        assert_eq!(w3.art.cover, Some(data.join("coverart/the-witcher-3.jpg")));
        assert_eq!(w3.art.banner, Some(home.join(".cache/lutris/banners/the-witcher-3.jpg")));
        assert_eq!(w3.meta.release_year, Some(2015));
        assert_eq!(
            w3.launch,
            Some(LaunchSpec::Command {
                program: "lutris".into(),
                args: vec!["lutris:rungameid/1".into()],
                cwd: None,
                env: vec![]
            })
        );
        let stk = &games[1];
        assert_eq!(stk.platform.as_deref(), Some("linux"));
        assert_eq!(stk.playtime_secs, None);
        assert!(stk.art.icon.is_some());

        let mut c = ctx(home);
        c.config.include_uninstalled = true;
        let all = scan(&c).unwrap();
        assert_eq!(all.len(), 3);
        assert!(!all[2].installed);
    }

    #[test]
    fn flatpak_and_old_schema() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let db = home.join(".var/app").join(FLATPAK_ID).join("data/lutris/pga.db");
        make_db(&db, false);
        let games = scan(&ctx(home)).unwrap();
        assert_eq!(games.len(), 2);
        match &games[0].launch {
            Some(LaunchSpec::Command { program, args, .. }) => {
                assert_eq!(program, "flatpak");
                assert_eq!(args, &["run", FLATPAK_ID, "lutris:rungameid/1"]);
            }
            other => panic!("{other:?}"),
        }
        // The immutable read-only open must not create journal files next to Lutris' DB.
        let names: Vec<String> = std::fs::read_dir(db.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["pga.db".to_string()]);
    }

    #[test]
    fn corrupt_db_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        assert!(scan(&ctx(home)).unwrap().is_empty());
        let p = home.join(".local/share/lutris/pga.db");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "this is not sqlite").unwrap();
        assert!(scan(&ctx(home)).is_err());
    }
}
