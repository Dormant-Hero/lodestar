//! SQLite library store (`library.db`, WAL mode).
//!
//! Games are keyed by `(source, source_id)`. Three layers of art/metadata are stored so a
//! rescan never clobbers what the user or the online fetcher added:
//!   effective = user override  >  scanner value  >  fetched (online) value
//! User fields (favourite, hidden, tags, custom name, rating, our own playtime) survive rescans.

use crate::model::*;
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: i32 = 5;

pub struct Db {
    conn: Connection,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MergeStats {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub uninstalled: usize,
    /// Games that went missing with this scan (see [`Missing`]).
    pub missing: usize,
}

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn open_memory() -> Result<Db> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Db> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=3000;",
        )?;
        let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS games (
                    id INTEGER PRIMARY KEY,
                    source TEXT NOT NULL,
                    source_id TEXT NOT NULL,
                    name TEXT NOT NULL,
                    user_name TEXT,
                    sort_name TEXT NOT NULL,
                    installed INTEGER NOT NULL DEFAULT 1,
                    install_dir TEXT,
                    launch_json TEXT,
                    art_json TEXT NOT NULL DEFAULT '{}',
                    user_art_json TEXT NOT NULL DEFAULT '{}',
                    fetched_art_json TEXT NOT NULL DEFAULT '{}',
                    meta_json TEXT NOT NULL DEFAULT '{}',
                    fetched_meta_json TEXT NOT NULL DEFAULT '{}',
                    playtime_secs INTEGER NOT NULL DEFAULT 0,
                    last_played INTEGER,
                    play_count INTEGER NOT NULL DEFAULT 0,
                    added INTEGER NOT NULL,
                    size_bytes INTEGER,
                    platform TEXT,
                    favorite INTEGER NOT NULL DEFAULT 0,
                    hidden INTEGER NOT NULL DEFAULT 0,
                    tags_json TEXT NOT NULL DEFAULT '[]',
                    user_rating INTEGER,
                    fetched_at INTEGER,
                    launch_override_json TEXT,
                    source_launch_options TEXT,
                    controller_override TEXT,
                    edits_json TEXT NOT NULL DEFAULT '{}',
                    emu_json TEXT,
                    UNIQUE(source, source_id)
                );
                CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
            )?;
            conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
        } else {
            // Upgrades, one step at a time; each only adds what is missing.
            let has = |col: &str| -> rusqlite::Result<bool> {
                conn.prepare("SELECT 1 FROM pragma_table_info('games') WHERE name = ?1")?.exists([col])
            };
            if version < 2 && !has("controller_override")? {
                // v2: the user's controller-support setting.
                conn.execute_batch("ALTER TABLE games ADD COLUMN controller_override TEXT;")?;
            }
            if version < 3 && !has("edits_json")? {
                // v3: Edit Game Details (hand edits, links, actions, scripts …).
                conn.execute_batch("ALTER TABLE games ADD COLUMN edits_json TEXT NOT NULL DEFAULT '{}';")?;
            }
            if version < 5 && !has("emu_json")? {
                // v5: emulated games (console, ROM files and identity, `EmuRef`).
                conn.execute_batch("ALTER TABLE games ADD COLUMN emu_json TEXT;")?;
            }
            if version < SCHEMA_VERSION {
                conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
            }
        }
        // 0.9: when the game's data last changed (Sort → Recently modified), kept by a trigger
        // so every writer counts without knowing about it. Outside the version steps: other
        // upgrades don't depend on it and it is cheap to check.
        let has_modified: bool =
            conn.prepare("SELECT 1 FROM pragma_table_info('games') WHERE name = 'modified'")?.exists([])?;
        if !has_modified {
            conn.execute_batch("ALTER TABLE games ADD COLUMN modified INTEGER;")?;
        }
        // 0.10: games that vanished from a library that scanned fine (`Missing`).
        let has_missing: bool =
            conn.prepare("SELECT 1 FROM pragma_table_info('games') WHERE name = 'missing_since'")?.exists([])?;
        if !has_missing {
            conn.execute_batch(
                "ALTER TABLE games ADD COLUMN missing_since INTEGER;
                 ALTER TABLE games ADD COLUMN missing_kept INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
        conn.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS games_modified AFTER UPDATE OF
                user_name, favorite, hidden, tags_json, user_rating, user_art_json, fetched_art_json,
                fetched_meta_json, launch_override_json, controller_override, edits_json
             ON games FOR EACH ROW
             WHEN OLD.user_name IS NOT NEW.user_name OR OLD.favorite IS NOT NEW.favorite OR OLD.hidden IS NOT NEW.hidden OR
                  OLD.tags_json IS NOT NEW.tags_json OR OLD.user_rating IS NOT NEW.user_rating OR OLD.user_art_json IS NOT NEW.user_art_json OR
                  OLD.fetched_art_json IS NOT NEW.fetched_art_json OR OLD.fetched_meta_json IS NOT NEW.fetched_meta_json OR OLD.launch_override_json IS NOT NEW.launch_override_json OR
                  OLD.controller_override IS NOT NEW.controller_override OR OLD.edits_json IS NOT NEW.edits_json
             BEGIN
                UPDATE games SET modified = CAST(strftime('%s', 'now') AS INTEGER) WHERE id = NEW.id;
             END;",
        )?;
        // 0.17.2: a game's id is never handed out again once its row is gone. Art, logs and caches
        // live in folders named after the id, so a new game given a removed game's id picked up
        // that game's pictures. SQLite reuses the highest id after a delete; the trigger keeps the
        // highest id ever used in kv and `NEXT_GAME_ID` starts above it.
        conn.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS games_high_id AFTER INSERT ON games FOR EACH ROW
             BEGIN
                INSERT INTO kv (key, value) VALUES ('games_high_id', NEW.id)
                ON CONFLICT(key) DO UPDATE SET value = MAX(CAST(value AS INTEGER), NEW.id);
             END;",
        )?;
        // v4: "Not a game" marks and the games brought back from a rule, by
        // source + source id so they outlive the game's row and its renames.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS exclusions (
                source TEXT NOT NULL,
                source_id TEXT NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                at INTEGER NOT NULL,
                PRIMARY KEY (source, source_id)
            );",
        )?;
        Ok(Db { conn })
    }

    /// The id for a new game: above every id ever used (see the `games_high_id` trigger).
    const NEXT_GAME_ID: &'static str = "(SELECT MAX(COALESCE((SELECT MAX(id) FROM games), 0), \
        COALESCE((SELECT CAST(value AS INTEGER) FROM kv WHERE key = 'games_high_id'), 0)) + 1)";

    const COLS: &'static str = "id, source, source_id, name, user_name, sort_name, installed, install_dir, \
        launch_json, art_json, user_art_json, fetched_art_json, meta_json, fetched_meta_json, playtime_secs, \
        last_played, play_count, added, size_bytes, platform, favorite, hidden, tags_json, user_rating, \
        launch_override_json, source_launch_options, controller_override, edits_json, emu_json, modified, missing_since, missing_kept";

    fn row_to_game(r: &Row) -> rusqlite::Result<Game> {
        let source: String = r.get(1)?;
        let launch: Option<String> = r.get(8)?;
        let art: Art = json_or_default(&r.get::<_, String>(9)?);
        let user_art: Art = json_or_default(&r.get::<_, String>(10)?);
        let fetched_art: Art = json_or_default(&r.get::<_, String>(11)?);
        let mut meta: Metadata = json_or_default(&r.get::<_, String>(12)?);
        let fetched_meta: Metadata = json_or_default(&r.get::<_, String>(13)?);
        let mut eff_art = user_art;
        eff_art.fill_from(&art);
        eff_art.fill_from(&fetched_art);
        if fetched_meta.looked_up {
            // A lookup the user asked for: its answer wins, the scanner's values fill its gaps.
            let mut m = fetched_meta;
            m.looked_up = false;
            m.fill_from(&meta);
            meta = m;
        } else {
            if fetched_meta.wikipedia.is_some() && fetched_meta.description.is_some() {
                // Wikipedia's text is only fetched over a launcher's when the user prefers it.
                meta.description = None;
            }
            meta.fill_from(&fetched_meta);
        }
        let source = Source::from_key(&source).unwrap_or(Source::Manual);
        let source_id: String = r.get(2)?;
        let playtime_secs = r.get::<_, i64>(14)?.max(0) as u64;
        let play_count = r.get::<_, i64>(16)?.max(0) as u32;
        let last_played = trusted_last_played(source, &source_id, playtime_secs, play_count, r.get(15)?);
        Ok(Game {
            id: r.get(0)?,
            source,
            source_id,
            name: r.get(3)?,
            user_name: r.get(4)?,
            sort_name: r.get(5)?,
            installed: r.get::<_, i64>(6)? != 0,
            install_dir: r.get::<_, Option<String>>(7)?.map(PathBuf::from),
            launch: launch.and_then(|s| serde_json::from_str(&s).ok()),
            art: eff_art,
            meta,
            playtime_secs,
            last_played,
            play_count,
            added: r.get(17)?,
            modified: r.get(29)?,
            size_bytes: r.get::<_, Option<i64>>(18)?.map(|v| v.max(0) as u64),
            platform: r.get(19)?,
            favorite: r.get::<_, i64>(20)? != 0,
            hidden: r.get::<_, i64>(21)? != 0,
            tags: json_or_default(&r.get::<_, String>(22)?),
            user_rating: r.get::<_, Option<i64>>(23)?.map(|v| v.clamp(0, 5) as u8),
            launch_override: r
                .get::<_, Option<String>>(24)?
                .and_then(|s| serde_json::from_str::<LaunchOverride>(&s).ok())
                .filter(|o| !o.is_empty()),
            source_launch_options: r.get(25)?,
            controller_override: r
                .get::<_, Option<String>>(26)?
                .and_then(|s| serde_json::from_value(serde_json::Value::String(s)).ok()),
            edits: r
                .get::<_, Option<String>>(27)?
                .map(|s| {
                    let mut e = edits_from_json(&s, r.get(0).unwrap_or_default());
                    e.migrate();
                    e
                })
                .unwrap_or_default(),
            source_meta: Metadata::default(),
            emu: r.get::<_, Option<String>>(28)?.and_then(|s| serde_json::from_str(&s).ok()),
            missing: r
                .get::<_, Option<i64>>(30)?
                .map(|since| Missing { since, kept: r.get::<_, i64>(31).unwrap_or(0) != 0 }),
        }
        .with_edits_applied())
    }

    pub fn load_all(&self) -> Result<Vec<Game>> {
        let mut st = self.conn.prepare_cached(&format!("SELECT {} FROM games ORDER BY sort_name", Self::COLS))?;
        let rows = st.query_map([], Self::row_to_game)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get(&self, id: GameId) -> Result<Option<Game>> {
        let mut st = self.conn.prepare_cached(&format!("SELECT {} FROM games WHERE id = ?1", Self::COLS))?;
        Ok(st.query_row([id], Self::row_to_game).optional()?)
    }

    /// Merge one scan. `scanned_sources` lists the sources whose scanner *succeeded*; games of those
    /// sources that were not seen:
    ///   * a store game the user owns ([`Source::owned_entry`]: Steam app, Epic, GOG …) was
    ///     uninstalled: marked not installed (removed only when a non-store source such as a
    ///     Wine launcher reported it and it has no user data); never missing;
    ///   * anything else (non-Steam shortcut, Faugus / Lutris / Heroic entry, ROM, menu entry …)
    ///     is gone: removed when it has no user data, else marked not installed and *missing*
    ///     (`Missing`), kept until the user removes it (right-click, Settings → Libraries).
    ///     User data = favourite, hidden, custom name, tags, playtime / play count / last played,
    ///     score, own artwork, launch / controller settings, hand edits (notes, completion …),
    ///     a collection, or a "Keep" — not fetched metadata / artwork. A missing game that loses
    ///     its user data is removed by the next scan. A game seen again stops being missing.
    ///
    /// Sources not listed are left untouched.
    ///
    /// Data safety rules:
    ///   * a source that succeeded but returned *no* games (e.g. `games.json` caught mid-rewrite),
    ///     or Steam with no non-Steam shortcuts (`shortcuts.vdf` unreadable for a moment), never
    ///     deletes anything: those games are marked missing (owned store games: uninstalled) and
    ///     stop being missing when the next scan lists them again;
    ///   * a game whose id changed but whose name did not (renamed `.desktop` file, Lutris/itch
    ///     reinstall, re-created Faugus entry or Steam shortcut) takes over its old row, so
    ///     favourite/tags/playtime/overrides follow it — only when the match is unambiguous;
    ///   * a custom name keeps deciding the sort order.
    pub fn merge_scan(&mut self, scanned: &[ScannedGame], scanned_sources: &[Source]) -> Result<MergeStats> {
        let now = now_unix();
        // IMMEDIATE: take the write lock before reading. With a deferred transaction, a UI write
        // on the app's other connection between our first read and first write makes the upgrade
        // fail with SQLITE_BUSY_SNAPSHOT (WAL) and the whole scan would be lost.
        let tx = self.conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut stats = MergeStats::default();
        let ok: HashSet<Source> = scanned_sources.iter().copied().collect();
        // Duplicates within one scan: first wins.
        let mut seen: HashSet<(Source, String)> = HashSet::new();
        let games: Vec<&ScannedGame> =
            scanned.iter().filter(|g| seen.insert((g.source, g.source_id.clone()))).collect();
        // Emulated games an older scan listed twice: fold the extra rows into the copy that stays.
        emu_dupes::fold(&tx, &games, &ok)?;
        // Files the user added to another game as one of its ROMs aren't games of their own.
        let games = if ok.contains(&Source::Emulator) { emu_roms::drop_claimed(&tx, games)? } else { games };
        let mut renames = Renames { vanished: vanished_rows(&tx, &ok, &seen)?, new_names: new_names(&tx, &games)? };
        for g in &games {
            upsert_scanned(&tx, g, &ok, &mut renames, now, &mut stats)?;
        }
        absorb_replaced(&tx, &games, &seen)?;
        retire_unseen(&tx, &games, &ok, &seen, now, &mut stats)?;
        tx.commit()?;
        Ok(stats)
    }

    /// Take games out of the library for good (Remove from library… on missing games), every
    /// column kept in the result so [`Db::restore_games`] can put them back (Undo).
    pub fn take_games(&self, ids: &[GameId]) -> Result<TakenRows> {
        let tx = self.conn.unchecked_transaction()?;
        let mut out = TakenRows::default();
        {
            let mut get = tx.prepare("SELECT * FROM games WHERE id = ?1")?;
            out.cols = get.column_names().map(|c| c.to_string()).collect();
            let n = out.cols.len();
            let mut del = tx.prepare("DELETE FROM games WHERE id = ?1")?;
            for id in ids {
                let row = get
                    .query_row([id], |r| {
                        (0..n).map(|i| r.get::<_, rusqlite::types::Value>(i)).collect::<rusqlite::Result<Vec<_>>>()
                    })
                    .optional()?;
                if let Some(row) = row {
                    del.execute([id])?;
                    out.rows.push(row);
                }
            }
        }
        tx.commit()?;
        Ok(out)
    }

    /// Put back what [`Db::take_games`] took (same ids, same data). A game whose id or
    /// (source, source id) is taken again by now (it came back with a scan) is skipped.
    /// Returns how many came back.
    pub fn restore_games(&self, t: &TakenRows) -> Result<usize> {
        if t.is_empty() {
            return Ok(0);
        }
        let cols: Vec<String> = t.cols.iter().map(|c| format!("\"{}\"", c.replace('"', "\"\""))).collect();
        let marks: Vec<String> = (1..=cols.len()).map(|i| format!("?{i}")).collect();
        let sql = format!("INSERT OR IGNORE INTO games ({}) VALUES ({})", cols.join(", "), marks.join(", "));
        let tx = self.conn.unchecked_transaction()?;
        let mut n = 0;
        {
            let mut ins = tx.prepare(&sql)?;
            for row in &t.rows {
                n += ins.execute(rusqlite::params_from_iter(row.iter()))?;
            }
        }
        tx.commit()?;
        Ok(n)
    }

    /// Keep (or with false: list again) missing games: Settings → Libraries → Missing games.
    /// A kept game counts as the user's (a scan never removes it).
    pub fn set_missing_kept(&self, ids: &[GameId], kept: bool) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut st =
                tx.prepare_cached("UPDATE games SET missing_kept = ?2 WHERE id = ?1 AND missing_since IS NOT NULL")?;
            for id in ids {
                st.execute(params![id, kept as i64])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Add a game by hand (Source::Manual). Returns its id.
    pub fn add_manual(&mut self, name: &str, launch: LaunchSpec, art: Art) -> Result<GameId> {
        let sid = format!("manual-{}-{}", now_unix(), fnv1a(name.as_bytes()));
        self.add_manual_as(&sid, name, launch, art)
    }

    /// `add_manual` with a given `source_id` (cloud games keep their service in it).
    pub fn add_manual_as(&mut self, sid: &str, name: &str, launch: LaunchSpec, art: Art) -> Result<GameId> {
        self.conn.execute(
            &format!(
                "INSERT INTO games (id, source, source_id, name, sort_name, installed, launch_json, user_art_json, \
                 added) VALUES ({}, 'manual', ?1, ?2, ?3, 1, ?4, ?5, ?6)",
                Self::NEXT_GAME_ID
            ),
            params![
                sid,
                name,
                sort_key(name),
                serde_json::to_string(&launch)?,
                serde_json::to_string(&art)?,
                now_unix()
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// A hand-added game's new `source_id` (a cloud game's "Opens with" changed in Edit).
    pub fn set_manual_source_id(&self, id: GameId, sid: &str) -> Result<()> {
        self.conn.execute("UPDATE games SET source_id = ?2 WHERE id = ?1 AND source = 'manual'", params![id, sid])?;
        Ok(())
    }

    pub fn delete(&self, id: GameId) -> Result<()> {
        self.conn.execute("DELETE FROM games WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_favorite(&self, id: GameId, v: bool) -> Result<()> {
        self.conn.execute("UPDATE games SET favorite = ?2 WHERE id = ?1", params![id, v as i64])?;
        Ok(())
    }

    pub fn set_hidden(&self, id: GameId, v: bool) -> Result<()> {
        self.conn.execute("UPDATE games SET hidden = ?2 WHERE id = ?1", params![id, v as i64])?;
        Ok(())
    }

    /// `None` or empty string clears the override.
    pub fn set_user_name(&self, id: GameId, name: Option<&str>) -> Result<()> {
        let name = name.map(str::trim).filter(|s| !s.is_empty());
        let current: String = self.conn.query_row("SELECT name FROM games WHERE id=?1", [id], |r| r.get(0))?;
        let sort = sort_key(name.unwrap_or(&current));
        self.conn.execute("UPDATE games SET user_name = ?2, sort_name = ?3 WHERE id = ?1", params![id, name, sort])?;
        Ok(())
    }

    pub fn set_tags(&self, id: GameId, tags: &[String]) -> Result<()> {
        self.conn
            .execute("UPDATE games SET tags_json = ?2 WHERE id = ?1", params![id, serde_json::to_string(tags)?])?;
        Ok(())
    }

    pub fn set_rating(&self, id: GameId, rating: Option<u8>) -> Result<()> {
        self.conn
            .execute("UPDATE games SET user_rating = ?2 WHERE id = ?1", params![id, rating.map(|r| r.min(5) as i64)])?;
        Ok(())
    }

    /// Set or clear (None / empty) the user's launch override.
    pub fn set_launch_override(&self, id: GameId, o: Option<&LaunchOverride>) -> Result<()> {
        let json = match o.filter(|o| !o.is_empty()) {
            Some(o) => Some(serde_json::to_string(o)?),
            None => None,
        };
        self.conn.execute("UPDATE games SET launch_override_json = ?2 WHERE id = ?1", params![id, json])?;
        Ok(())
    }

    /// Set or clear (None = follow the source) the user's controller-support setting.
    pub fn set_controller_override(&self, id: GameId, v: Option<ControllerSupport>) -> Result<()> {
        let s = v.map(|c| match c {
            ControllerSupport::Full => "full",
            ControllerSupport::Partial => "partial",
            ControllerSupport::None => "none",
        });
        self.conn.execute("UPDATE games SET controller_override = ?2 WHERE id = ?1", params![id, s])?;
        Ok(())
    }

    /// Hide / unhide several games in one transaction.
    pub fn set_hidden_many(&self, ids: &[GameId], v: bool) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut st = tx.prepare_cached("UPDATE games SET hidden = ?2 WHERE id = ?1")?;
            for id in ids {
                st.execute(params![id, v as i64])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Every "Not a game" mark and keep (exclude.rs), oldest first.
    pub fn exclusions(&self) -> Result<Vec<crate::exclude::Exclusion>> {
        let mut st =
            self.conn.prepare_cached("SELECT source, source_id, name, kind, at FROM exclusions ORDER BY at, rowid")?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?;
        let mut out = vec![];
        for row in rows {
            let (src, source_id, name, kind, at) = row?;
            // Rows of a newer build's source / kind are kept in the table, just not used.
            if let (Some(source), Some(mark)) = (Source::from_key(&src), crate::exclude::Mark::from_key(&kind)) {
                out.push(crate::exclude::Exclusion { source, source_id, name, mark, at });
            }
        }
        Ok(out)
    }

    /// Mark games (source, source id, name) as `mark`, replacing any earlier mark of theirs.
    pub fn set_exclusions(&self, items: &[(Source, String, String)], mark: crate::exclude::Mark) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut st = tx.prepare_cached(
                "INSERT INTO exclusions (source, source_id, name, kind, at) VALUES (?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT(source, source_id) DO UPDATE SET name = excluded.name, kind = excluded.kind, at = excluded.at",
            )?;
            let now = now_unix();
            for (src, sid, name) in items {
                st.execute(params![src.key(), sid, name, mark.key(), now])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Forget the marks of these games (they follow the rules again).
    pub fn clear_exclusions(&self, keys: &[(Source, String)]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut st = tx.prepare_cached("DELETE FROM exclusions WHERE source = ?1 AND source_id = ?2")?;
            for (src, sid) in keys {
                st.execute(params![src.key(), sid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Save several games' hand edits in one transaction (bulk game options).
    pub fn set_edits_many(&self, edits: &[(GameId, UserEdits)]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut st = tx.prepare_cached("UPDATE games SET edits_json = ?2 WHERE id = ?1")?;
            for (id, e) in edits {
                let json = if e.is_empty() { "{}".to_string() } else { serde_json::to_string(e)? };
                st.execute(params![id, json])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Save the user's hand edits (Edit Game Details). An empty set clears them.
    pub fn set_edits(&self, id: GameId, e: &UserEdits) -> Result<()> {
        let json = if e.is_empty() { "{}".to_string() } else { serde_json::to_string(e)? };
        self.conn.execute("UPDATE games SET edits_json = ?2 WHERE id = ?1", params![id, json])?;
        Ok(())
    }

    /// Hand-edit the play statistics (Edit Game Details → Advanced).
    pub fn set_play_stats(
        &self,
        id: GameId,
        playtime_secs: u64,
        play_count: u32,
        last_played: Option<i64>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE games SET playtime_secs = ?2, play_count = ?3, last_played = ?4 WHERE id = ?1",
            params![id, playtime_secs as i64, play_count as i64, last_played],
        )?;
        Ok(())
    }

    /// Hand-edit the date added.
    pub fn set_added(&self, id: GameId, added: i64) -> Result<()> {
        self.conn.execute("UPDATE games SET added = ?2 WHERE id = ?1", params![id, added])?;
        Ok(())
    }

    /// User-chosen art; replaces the whole override set.
    pub fn set_user_art(&self, id: GameId, art: &Art) -> Result<()> {
        self.conn
            .execute("UPDATE games SET user_art_json = ?2 WHERE id = ?1", params![id, serde_json::to_string(art)?])?;
        Ok(())
    }

    /// The user-chosen art layer alone (`Game::art` is the merged, effective art).
    pub fn user_art(&self, id: GameId) -> Result<Art> {
        let s: String = self.conn.query_row("SELECT user_art_json FROM games WHERE id = ?1", [id], |r| r.get(0))?;
        Ok(json_or_default(&s))
    }

    /// Set (or with None clear) one slot of the user art layer, keeping the others.
    pub fn set_user_art_slot(&self, id: GameId, f: impl FnOnce(&mut Art)) -> Result<Art> {
        let mut art = self.user_art(id)?;
        f(&mut art);
        self.set_user_art(id, &art)?;
        Ok(art)
    }

    /// Store art/metadata fetched online (merged into what was fetched before).
    pub fn set_fetched(&self, id: GameId, art: &Art, meta: &Metadata) -> Result<()> {
        let (a, m): (String, String) =
            self.conn.query_row("SELECT fetched_art_json, fetched_meta_json FROM games WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        let mut new_art = art.clone();
        new_art.fill_from(&json_or_default::<Art>(&a));
        let mut new_meta = meta.clone();
        let old = json_or_default::<Metadata>(&m);
        new_meta.fill_from(&old);
        // What the user picked with Download metadata… keeps winning over the scanner's values.
        new_meta.looked_up |= old.looked_up;
        self.conn.execute(
            "UPDATE games SET fetched_art_json = ?2, fetched_meta_json = ?3, fetched_at = ?4 WHERE id = ?1",
            params![id, serde_json::to_string(&new_art)?, serde_json::to_string(&new_meta)?, now_unix()],
        )?;
        Ok(())
    }

    /// Store what a lookup the user asked for found (Edit → Download metadata…): it *replaces*
    /// the metadata fetched before (`Metadata::replaced_by`: nothing left from an earlier match)
    /// and wins over the scanner's values; art only fills empty slots.
    pub fn replace_fetched(&self, id: GameId, art: &Art, meta: &Metadata) -> Result<()> {
        let Some((a, m)) = self.replaced_fetched(id, art, meta)? else { return Ok(()) };
        self.conn.execute(
            "UPDATE games SET fetched_art_json = ?2, fetched_meta_json = ?3, fetched_at = ?4 WHERE id = ?1",
            params![id, a, m, now_unix()],
        )?;
        Ok(())
    }

    /// The game as it would load after `replace_fetched(id, art, meta)`; nothing is written
    /// (Edit Game Details shows a lookup's answer before Save).
    pub fn preview_fetched(&self, id: GameId, art: &Art, meta: &Metadata) -> Result<Option<Game>> {
        let Some((a, m)) = self.replaced_fetched(id, art, meta)? else { return Ok(None) };
        let cols = Self::COLS.replacen("fetched_art_json", "?2", 1).replacen("fetched_meta_json", "?3", 1);
        let mut st = self.conn.prepare(&format!("SELECT {cols} FROM games WHERE id = ?1"))?;
        Ok(st.query_row(params![id, a, m], Self::row_to_game).optional()?)
    }

    /// The fetched art / metadata JSON `replace_fetched` stores (None: no such game).
    fn replaced_fetched(&self, id: GameId, art: &Art, meta: &Metadata) -> Result<Option<(String, String)>> {
        let old: Option<(String, String)> = self
            .conn
            .query_row("SELECT fetched_art_json, fetched_meta_json FROM games WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let Some((a, m)) = old else { return Ok(None) };
        let mut new_art = art.clone();
        new_art.fill_from(&json_or_default::<Art>(&a));
        let new_meta = json_or_default::<Metadata>(&m).replaced_by(meta);
        Ok(Some((serde_json::to_string(&new_art)?, serde_json::to_string(&new_meta)?)))
    }

    /// The scanner's own metadata of a game (without anything fetched online or edited).
    pub fn scan_meta(&self, id: GameId) -> Result<Metadata> {
        let s: Option<String> =
            self.conn.query_row("SELECT meta_json FROM games WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        Ok(s.map(|s| json_or_default(&s)).unwrap_or_default())
    }

    /// Every game's id, when it was added, its fetched art and every art path it points at
    /// (`art::tidy`).
    pub fn art_rows(&self) -> Result<Vec<crate::art::tidy::ArtRow>> {
        let mut st = self.conn.prepare("SELECT id, added, art_json, user_art_json, fetched_art_json FROM games")?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, GameId>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = vec![];
        for row in rows {
            let (id, added, scanned, user, fetched) = row?;
            let fetched: Art = json_or_default(&fetched);
            let referenced = [json_or_default::<Art>(&scanned), json_or_default::<Art>(&user), fetched.clone()]
                .into_iter()
                .flat_map(|a| [a.cover, a.hero, a.logo, a.icon, a.banner])
                .flatten()
                .collect();
            out.push(crate::art::tidy::ArtRow { id, added, fetched, referenced });
        }
        Ok(out)
    }

    /// Replace a game's fetched art and mark it as never fetched, so the fetcher fills the
    /// emptied slots again.
    pub fn reset_fetched_art(&self, id: GameId, art: &Art) -> Result<()> {
        self.conn.execute(
            "UPDATE games SET fetched_art_json = ?2, fetched_at = NULL WHERE id = ?1",
            params![id, serde_json::to_string(art)?],
        )?;
        Ok(())
    }

    /// When metadata was last fetched for this game (None = never).
    pub fn fetched_at(&self, id: GameId) -> Result<Option<i64>> {
        Ok(self.conn.query_row("SELECT fetched_at FROM games WHERE id = ?1", [id], |r| r.get(0)).optional()?.flatten())
    }

    /// A launch happened: bump play count and last played. Returns the stored last-played value
    /// from before, for `undo_launch`.
    pub fn record_launch(&self, id: GameId, at: i64) -> Result<Option<i64>> {
        let prev: Option<i64> = self
            .conn
            .query_row("SELECT last_played FROM games WHERE id = ?1", [id], |r| r.get(0))
            .optional()?
            .flatten();
        self.conn.execute(
            "UPDATE games SET play_count = play_count + 1, last_played = MAX(COALESCE(last_played, 0), ?2) WHERE id = ?1",
            params![id, at],
        )?;
        Ok(prev)
    }

    /// The launch recorded by `record_launch` failed right away (crash at start, bad command):
    /// it doesn't count as a play. `prev` is what `record_launch` returned.
    pub fn undo_launch(&self, id: GameId, prev: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE games SET play_count = MAX(play_count - 1, 0), last_played = ?2 WHERE id = ?1",
            params![id, prev],
        )?;
        Ok(())
    }

    /// A tracked session ended: add its length to playtime.
    pub fn add_playtime(&self, id: GameId, secs: u64) -> Result<()> {
        self.conn
            .execute("UPDATE games SET playtime_secs = playtime_secs + ?2 WHERE id = ?1", params![id, secs as i64])?;
        Ok(())
    }

    pub fn kv_get(&self, key: &str) -> Option<String> {
        self.conn.query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0)).optional().ok().flatten()
    }

    pub fn kv_set(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

/// SQL: the row carries something of the user's, so a scan that no longer finds the game keeps
/// it (missing) instead of removing it. Fetched metadata / artwork don't count: they come back
/// by themselves. See [`Db::merge_scan`].
const USER_DATA: &str = "favorite = 1 OR hidden = 1 OR user_name IS NOT NULL OR tags_json != '[]' \
    OR playtime_secs > 0 OR play_count > 0 OR last_played IS NOT NULL OR user_rating IS NOT NULL \
    OR user_art_json != '{}' OR launch_override_json IS NOT NULL OR controller_override IS NOT NULL \
    OR edits_json != '{}' OR missing_kept = 1";

/// Library rows taken out by [`Db::take_games`], every column as stored (for Undo).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TakenRows {
    cols: Vec<String>,
    rows: Vec<Vec<rusqlite::types::Value>>,
}

impl TakenRows {
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// The last-played time to show, or None when the game doesn't count as played.
///
/// Steam writes `LastPlayed` to `localconfig.vdf` / app manifests without the game being played:
/// installing, first-run setup (EULA, cloud sync) and validation set it, often for a dozen games
/// within seconds, while `Playtime` stays absent. A Steam app therefore counts as played only
/// with playtime (Steam's or ours) or a launch from Lodestar. Other sources (Lutris, Heroic) and
/// Steam's non-Steam shortcuts (which have no playtime at all; their `LastPlayTime` is only set
/// by a launch) are trusted. The stored value is kept, so a later playtime makes it count.
pub fn trusted_last_played(
    source: Source,
    source_id: &str,
    playtime_secs: u64,
    play_count: u32,
    last_played: Option<i64>,
) -> Option<i64> {
    let steam_app = source == Source::Steam && !source_id.starts_with("shortcut:");
    if steam_app && playtime_secs == 0 && play_count == 0 { None } else { last_played }
}

/// Keyed by (source, sort key of the name).
type ByName<T> = HashMap<(Source, String), T>;

/// What [`Db::merge_scan`] needs to let a renamed game take over its old row.
struct Renames {
    /// Rows (id, source id) of successfully scanned sources that this scan did not report.
    vanished: ByName<Vec<(i64, String)>>,
    /// How many games of this scan are new under each name.
    new_names: ByName<usize>,
}

fn vanished_rows(
    conn: &Connection,
    ok: &HashSet<Source>,
    seen: &HashSet<(Source, String)>,
) -> Result<ByName<Vec<(i64, String)>>> {
    let mut vanished: ByName<Vec<(i64, String)>> = HashMap::new();
    let mut st = conn.prepare_cached("SELECT id, source, source_id, name FROM games")?;
    let rows = st.query_map([], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
    })?;
    for row in rows {
        let (id, src, sid, name) = row?;
        let Some(src) = Source::from_key(&src).filter(|s| ok.contains(s)) else { continue };
        if !seen.contains(&(src, sid.clone())) {
            vanished.entry((src, sort_key(&name))).or_default().push((id, sid));
        }
    }
    Ok(vanished)
}

fn new_names(conn: &Connection, games: &[&ScannedGame]) -> Result<ByName<usize>> {
    let mut new_names: ByName<usize> = HashMap::new();
    let mut exists = conn.prepare_cached("SELECT 1 FROM games WHERE source = ?1 AND source_id = ?2")?;
    for g in games {
        if !exists.exists(params![g.source.key(), g.source_id])? {
            *new_names.entry((g.source, sort_key(&g.name))).or_default() += 1;
        }
    }
    Ok(new_names)
}

/// The row of `g` (id, playtime, last played), once it has taken over a row it used to have: a
/// Heroic row of an Epic / GOG / Amazon game, or (unambiguously) the row of a renamed entry.
fn existing_row(
    conn: &Connection,
    g: &ScannedGame,
    ok: &HashSet<Source>,
    renames: &mut Renames,
) -> Result<Option<(i64, i64, Option<i64>)>> {
    let mut find =
        conn.prepare_cached("SELECT id, playtime_secs, last_played FROM games WHERE source = ?1 AND source_id = ?2")?;
    let mut find = || -> Result<Option<(i64, i64, Option<i64>)>> {
        Ok(find
            .query_row(params![g.source.key(), g.source_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?)
    };
    let mut existing = find()?;
    // Heroic games used to be stored as `heroic`; they now come back as Epic / GOG /
    // Amazon with the same id (the store's appName). Move the old row over so
    // favourites, playtime, tags and edits follow, instead of leaving a duplicate.
    if existing.is_none() && matches!(g.source, Source::Epic | Source::Gog | Source::Amazon) {
        let moved = conn.execute(
            "UPDATE games SET source = ?1 WHERE source = 'heroic' AND source_id = ?2",
            params![g.source.key(), g.source_id],
        )?;
        if moved > 0 {
            for cands in renames.vanished.values_mut() {
                cands.retain(|(_, sid)| sid != &g.source_id);
            }
            existing = find()?;
        }
    }
    if existing.is_none() && ok.contains(&g.source) {
        let key = (g.source, sort_key(&g.name));
        if renames.new_names.get(&key) == Some(&1)
            && let Some(cands) = renames.vanished.get_mut(&key)
            && let [(old_id, old_sid)] = cands.as_slice()
            && adoptable(g.source, old_sid, &g.source_id)
        {
            let old_id = *old_id;
            conn.prepare_cached("UPDATE games SET source_id = ?2 WHERE id = ?1")?
                .execute(params![old_id, g.source_id])?;
            // A "Not a game" mark (or a keep) follows the row to its new id.
            conn.execute(
                "UPDATE OR IGNORE exclusions SET source_id = ?3 WHERE source = ?1 AND source_id = ?2",
                params![g.source.key(), old_sid, g.source_id],
            )?;
            cands.clear();
            existing = find()?;
        }
    }
    Ok(existing)
}

/// Update the row of scanned game `g` (keeping the larger playtime and the later last-played)
/// or insert it.
fn upsert_scanned(
    conn: &Connection,
    g: &ScannedGame,
    ok: &HashSet<Source>,
    renames: &mut Renames,
    now: i64,
    stats: &mut MergeStats,
) -> Result<()> {
    let launch = g.launch.as_ref().map(|l| serde_json::to_string(l).unwrap_or_default());
    let art = serde_json::to_string(&g.art)?;
    let meta = serde_json::to_string(&g.meta)?;
    let dir = g.install_dir.as_ref().map(|p| p.to_string_lossy().into_owned());
    let emu = g.emu.as_ref().map(serde_json::to_string).transpose()?;
    let size = g.size_bytes.map(|v| v.min(i64::MAX as u64) as i64);
    match existing_row(conn, g, ok, renames)? {
        Some((id, pt, lp)) => {
            let playtime = (pt.max(0) as u64).max(g.playtime_secs.unwrap_or(0));
            let last = match (lp, g.last_played) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            };
            conn.prepare_cached(
                "UPDATE games SET name=?2, sort_name=CASE WHEN user_name IS NULL THEN ?3 ELSE sort_name END, \
                 installed=?4, install_dir=?5, launch_json=?6, \
                 art_json=?7, meta_json=?8, playtime_secs=?9, last_played=?10, size_bytes=?11, platform=?12, \
                 source_launch_options=?13, emu_json=?14, missing_since=NULL, missing_kept=0 WHERE id=?1",
            )?
            .execute(params![
                id,
                g.name,
                sort_key(&g.name),
                g.installed as i64,
                dir,
                launch,
                art,
                meta,
                playtime.min(i64::MAX as u64) as i64,
                last,
                size,
                g.platform,
                g.source_launch_options,
                emu,
            ])?;
            stats.updated += 1;
        }
        None => {
            conn.prepare_cached(&format!(
                "INSERT INTO games (id, source, source_id, name, sort_name, installed, install_dir, launch_json, \
                 art_json, meta_json, playtime_secs, last_played, added, size_bytes, platform, source_launch_options, \
                 emu_json) VALUES ({}, ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
                Db::NEXT_GAME_ID
            ))?
            .execute(params![
                g.source.key(),
                g.source_id,
                g.name,
                sort_key(&g.name),
                g.installed as i64,
                dir,
                launch,
                art,
                meta,
                g.playtime_secs.unwrap_or(0).min(i64::MAX as u64) as i64,
                g.last_played,
                now,
                size,
                g.platform,
                g.source_launch_options,
                emu,
            ])?;
            stats.added += 1;
        }
    }
    Ok(())
}

/// Entries another game took the place of (a RomM game downloaded: the ROM on disk is the
/// game now): the user's data moves to that game's row and the old row goes, so no
/// "missing" ghost keeps the favourite, tags or collections.
fn absorb_replaced(conn: &Connection, games: &[&ScannedGame], seen: &HashSet<(Source, String)>) -> Result<()> {
    let id_of = |src: Source, sid: &str| -> Result<Option<i64>> {
        Ok(conn
            .query_row("SELECT id FROM games WHERE source = ?1 AND source_id = ?2", params![src.key(), sid], |r| {
                r.get::<_, i64>(0)
            })
            .optional()?)
    };
    for g in games.iter().filter(|g| !g.absorbs.is_empty()) {
        let Some(to) = id_of(g.source, &g.source_id)? else { continue };
        for (src, sid) in &g.absorbs {
            if seen.contains(&(*src, sid.clone())) {
                continue; // still listed by its own source this scan
            }
            let Some(from) = id_of(*src, sid)?.filter(|f| *f != to) else { continue };
            emu_dupes::move_user_data(conn, from, to)?;
            emu_dupes::replace_in_collections(conn, from, Some(to))?;
            conn.execute("DELETE FROM games WHERE id = ?1", [from])?;
            log::info!("library: {} {sid} is now {} (its data moved along)", src.key(), g.name);
        }
    }
    Ok(())
}

/// Games that disappeared from successfully scanned sources: uninstalled, missing or removed
/// (see [`Db::merge_scan`]). A game in one of the user's collections counts as user data
/// (deleting it would drop it from the collection).
fn retire_unseen(
    conn: &Connection,
    games: &[&ScannedGame],
    ok: &HashSet<Source>,
    seen: &HashSet<(Source, String)>,
    now: i64,
    stats: &mut MergeStats,
) -> Result<()> {
    let mut per_source: HashMap<Source, usize> = HashMap::new();
    // Sources that listed at least one entry that isn't an owned store game (for Steam:
    // a non-Steam shortcut). None listed = maybe `shortcuts.vdf` caught mid-rewrite.
    let mut lists_unowned: HashSet<Source> = HashSet::new();
    for g in games {
        *per_source.entry(g.source).or_default() += 1;
        if !g.source.owned_entry(&g.source_id) {
            lists_unowned.insert(g.source);
        }
    }
    let in_collections: HashSet<GameId> = conn
        .query_row("SELECT value FROM kv WHERE key = ?1", [crate::library::user::KV], |r| r.get::<_, String>(0))
        .optional()?
        .map(|s| crate::library::user::member_ids(&s))
        .unwrap_or_default();
    let uninstall = |id: i64| conn.execute("UPDATE games SET installed = 0 WHERE id = ?1 AND installed = 1", [id]);
    for src in ok {
        let rows: Vec<(i64, String, bool)> = {
            let mut st =
                conn.prepare_cached(&format!("SELECT id, source_id, ({USER_DATA}) FROM games WHERE source = ?1"))?;
            st.query_map([src.key()], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0)))?
                .collect::<rusqlite::Result<_>>()?
        };
        let empty = per_source.get(src).copied().unwrap_or(0) == 0;
        for (id, sid, has_user_data) in rows {
            if seen.contains(&(*src, sid.clone())) {
                continue;
            }
            let keep = has_user_data || in_collections.contains(&id);
            if src.owned_entry(&sid) {
                // Uninstalled (or out of the scan while the owned list is off): not missing.
                if keep || empty || src.is_store() {
                    stats.uninstalled += uninstall(id)?;
                } else {
                    conn.execute("DELETE FROM games WHERE id = ?1", [id])?;
                    stats.removed += 1;
                }
            } else if keep || empty || !lists_unowned.contains(src) {
                // Gone with the user's data, or the library (its shortcuts) listed nothing at
                // all: maybe a half-written file, maybe really emptied. Never deleted here:
                // marked missing (seen again = not missing), removable by the user.
                stats.uninstalled += uninstall(id)?;
                stats.missing += conn.execute(
                    "UPDATE games SET missing_since = ?2 WHERE id = ?1 AND missing_since IS NULL",
                    params![id, now],
                )?;
            } else {
                conn.execute("DELETE FROM games WHERE id = ?1", [id])?;
                stats.removed += 1;
            }
        }
    }
    Ok(())
}

/// May a game with id `new` of `src` take over the row of a vanished game with id `old` and the
/// same name? Not for sources whose ids are permanent store ids (a different Steam appid or
/// Heroic app is a different product, e.g. a remaster with the same title).
fn adoptable(src: Source, old: &str, new: &str) -> bool {
    match src {
        Source::Steam => old.starts_with("shortcut:") && new.starts_with("shortcut:"),
        Source::Heroic | Source::Manual | Source::Epic | Source::Gog | Source::Amazon => false,
        Source::BattleNet | Source::Ea | Source::Ubisoft | Source::Rockstar | Source::Riot | Source::HoYoPlay => false,
        Source::Addon | Source::Romm => false,
        Source::Faugus | Source::Lutris | Source::Bottles | Source::Itch | Source::Desktop => true,
        Source::Minecraft | Source::Moonlight => true,
        // A renamed or moved ROM with the same clean name takes over its row.
        Source::Emulator => true,
    }
}

fn json_or_default<T: serde::de::DeserializeOwned + Default>(s: &str) -> T {
    serde_json::from_str(s).unwrap_or_default()
}

/// A game's `edits_json`, keeping every edit that is valid: one unknown or out-of-range value
/// (a newer build's enum variant, a hand edit) drops only that field, never the game's name,
/// notes, actions or scripts (the next save would make that loss permanent). The game options
/// are lenient per option themselves (`game_options::deserialize_lenient`). Dropped values are
/// logged, so they aren't lost silently.
fn edits_from_json(s: &str, id: GameId) -> UserEdits {
    if let Ok(e) = serde_json::from_str(s) {
        return e;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(s) else {
        log::warn!("game {id}: edits aren't valid JSON; ignored: {s}");
        return UserEdits::default();
    };
    let (e, dropped) = crate::game_options::lenient_from_value::<UserEdits>(v, "");
    if !dropped.is_empty() {
        log::warn!("game {id}: ignored invalid edits {}", dropped.join(", "));
    }
    e
}

/// FNV-1a 64-bit; stable across Rust versions (unlike DefaultHasher). Use for cache keys.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

mod emu_dupes;
mod emu_roms;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod audit_tests;

#[cfg(test)]
mod controller_tests;

#[cfg(test)]
mod exclusion_tests;

#[cfg(test)]
mod edits_tests;

#[cfg(test)]
mod emu_tests;

#[cfg(test)]
mod missing_tests;
