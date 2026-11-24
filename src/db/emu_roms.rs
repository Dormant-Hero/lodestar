//! ROMs the user added to another game (Edit → Installation → ROMs, or right-click → "Add to
//! another game as a ROM…") are not listed as games of their own: a scan doesn't import such a
//! file again, and the row it already had is folded into the game that took it — favourite,
//! play time, play count, last played, tags and collections move along, then the row goes.
//! Removing the ROM from that game's list makes the next scan list it on its own again.

use crate::model::{EmuRef, ScannedGame, Source, UserEdits};
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Every ROM the user added to a game, as a real path → that game's row.
fn claimed(conn: &Connection) -> Result<HashMap<PathBuf, i64>> {
    let mut out = HashMap::new();
    let mut st = conn.prepare(
        "SELECT id, install_dir, edits_json FROM games WHERE source = 'emulator' AND edits_json LIKE '%\"roms\"%'",
    )?;
    let rows =
        st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?)))?;
    for row in rows {
        let (id, dir, edits) = row?;
        let Ok(e) = serde_json::from_str::<UserEdits>(&edits) else { continue };
        let dir = e.install_dir.clone().or_else(|| dir.map(PathBuf::from));
        for r in e.roms.iter().filter(|r| !r.path.trim().is_empty()) {
            out.entry(crate::emulation::roms::expand(&r.path, dir.as_deref())).or_insert(id);
        }
    }
    Ok(out)
}

/// Fold row `from` into row `to` (the game that took its ROM).
fn fold_into(conn: &Connection, from: i64, to: i64) -> Result<()> {
    type Row = (i64, i64, i64, Option<i64>, String);
    let read = |id: i64| -> Result<Row> {
        Ok(conn.query_row(
            "SELECT favorite, playtime_secs, play_count, last_played, tags_json FROM games WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?)
    };
    let (fa, pa, ca, la, ta) = read(from)?;
    let (fb, pb, cb, lb, tb) = read(to)?;
    let mut tags: Vec<String> = serde_json::from_str(&tb).unwrap_or_default();
    for t in serde_json::from_str::<Vec<String>>(&ta).unwrap_or_default() {
        if !tags.contains(&t) {
            tags.push(t);
        }
    }
    let last = match (la, lb) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, y) => x.or(y),
    };
    conn.execute(
        "UPDATE games SET favorite = ?2, playtime_secs = ?3, play_count = ?4, last_played = ?5, tags_json = ?6 WHERE id = ?1",
        params![
            to,
            (fa != 0 || fb != 0) as i64,
            pa.max(0).saturating_add(pb.max(0)),
            ca.max(0).saturating_add(cb.max(0)),
            last,
            serde_json::to_string(&tags)?
        ],
    )?;
    super::emu_dupes::replace_in_collections(conn, from, Some(to))?;
    conn.execute("DELETE FROM games WHERE id = ?1", [from])?;
    Ok(())
}

/// A scan: leave out the emulated games whose file another game lists as one of its ROMs (their
/// old rows are folded into that game). Returns the games to merge.
pub(super) fn drop_claimed<'a>(conn: &Connection, games: Vec<&'a ScannedGame>) -> Result<Vec<&'a ScannedGame>> {
    let claims = claimed(conn)?;
    if claims.is_empty() {
        return Ok(games);
    }
    let mut out = Vec::with_capacity(games.len());
    let mut find = conn.prepare_cached("SELECT id FROM games WHERE source = 'emulator' AND source_id = ?1")?;
    for g in games {
        let owner = (g.source == Source::Emulator).then(|| g.emu.as_ref().and_then(|e| claims.get(&e.rom))).flatten();
        let Some(&to) = owner else {
            out.push(g);
            continue;
        };
        let own: Option<i64> = find.query_row([&g.source_id], |r| r.get(0)).optional()?;
        if own == Some(to) {
            // Its own scanned file listed again by hand: still itself.
            out.push(g);
            continue;
        }
        if let Some(from) = own {
            fold_into(conn, from, to)?;
            log::info!("library: {} is a ROM of game {to} now; its entry was folded into it", g.source_id);
        }
    }
    Ok(out)
}

impl super::Db {
    /// After game `id`'s ROM list changed: fold the library entries of the files it took
    /// (right away, so the duplicate doesn't wait for the next scan). Returns how many went.
    pub fn fold_claimed_roms(&mut self, id: crate::model::GameId) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let claims: HashSet<PathBuf> =
            claimed(&tx)?.into_iter().filter(|(_, owner)| *owner == id).map(|(p, _)| p).collect();
        let mut n = 0;
        if !claims.is_empty() {
            let rows: Vec<(i64, String)> = {
                let mut st = tx.prepare(
                    "SELECT id, emu_json FROM games WHERE source = 'emulator' AND emu_json IS NOT NULL AND id != ?1",
                )?;
                st.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?
            };
            for (from, json) in rows {
                let Ok(e) = serde_json::from_str::<EmuRef>(&json) else { continue };
                if claims.contains(&e.rom) {
                    fold_into(&tx, from, id)?;
                    n += 1;
                }
            }
        }
        tx.commit()?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use crate::db::Db;
    use crate::model::{EmuRef, GameRom, ScannedGame, Source};

    fn rom(name: &str) -> ScannedGame {
        let mut g = ScannedGame::new(Source::Emulator, format!("f1/{name}.gb"), name);
        g.installed = true;
        g.install_dir = Some("/roms/gb".into());
        g.emu = Some(EmuRef {
            platform: "nintendo_gameboy".into(),
            rom: format!("/roms/gb/{name}.gb").into(),
            ..Default::default()
        });
        g
    }

    #[test]
    fn a_rom_added_to_another_game_is_folded_and_not_imported_again() {
        let mut db = Db::open_memory().unwrap();
        db.merge_scan(&[rom("Pokemon Blue"), rom("Pokemon Azul")], &[Source::Emulator]).unwrap();
        let all = db.load_all().unwrap();
        let blue = all.iter().find(|g| g.name == "Pokemon Blue").unwrap().clone();
        let azul = all.iter().find(|g| g.name == "Pokemon Azul").unwrap().clone();
        db.set_favorite(azul.id, true).unwrap();
        let mut e = blue.edits.clone();
        e.roms = vec![
            GameRom::default(),
            GameRom { name: "Pokémon Azul".into(), path: "{InstallDir}/Pokemon Azul.gb".into() },
        ];
        db.set_edits(blue.id, &e).unwrap();
        assert_eq!(db.fold_claimed_roms(blue.id).unwrap(), 1);
        let all = db.load_all().unwrap();
        assert_eq!(all.len(), 1, "the duplicate is gone");
        assert!(all[0].favorite, "its favourite moved along");
        // A rescan lists both files again: Azul stays inside Blue, Blue keeps its list.
        db.merge_scan(&[rom("Pokemon Blue"), rom("Pokemon Azul")], &[Source::Emulator]).unwrap();
        let all = db.load_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].edits.roms.len(), 2, "rescans never drop ROMs the user added");
        // Taken out of the list again: the next scan lists it on its own.
        let mut e = all[0].edits.clone();
        e.roms.truncate(1);
        db.set_edits(all[0].id, &e).unwrap();
        db.merge_scan(&[rom("Pokemon Blue"), rom("Pokemon Azul")], &[Source::Emulator]).unwrap();
        assert_eq!(db.load_all().unwrap().len(), 2);
    }
}
