//! Emulated games listed twice by an older scan (0.7.0 scanned a folder and an emulator's
//! document-portal view of its `ps2` sub-folder as two ROM folders): when a scan reports a game
//! once, the library row of the other copy (same console, same ROM file name and size, a
//! source id the scan no longer reports) is folded into the row of the copy that stays.
//!
//! What the user gave the duplicate moves to the surviving row ([`super::fold`]) and the
//! duplicate row is deleted, so no "not installed" ghost is left. When the surviving copy has no
//! row yet, the duplicate's row simply takes the new id.

use crate::model::{EmuRef, ScannedGame, Source};
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{HashMap, HashSet};

/// A ROM's identity across folders: console, ROM file (or game folder) name, size.
type Key = (String, String, Option<i64>);

fn key_of(e: &EmuRef, size: Option<i64>) -> Option<Key> {
    let name = e.rom.file_name()?.to_string_lossy().to_lowercase();
    (!e.platform.is_empty()).then(|| (e.platform.clone(), name, size))
}

/// Fold the duplicates (see the module docs). Returns how many rows were folded.
pub(super) fn fold(conn: &Connection, games: &[&ScannedGame], ok: &HashSet<Source>) -> Result<usize> {
    if !ok.contains(&Source::Emulator) {
        return Ok(0);
    }
    let reported: HashSet<&str> =
        games.iter().filter(|g| g.source == Source::Emulator).map(|g| g.source_id.as_str()).collect();
    // Scanned games by identity (None when two of them share it: ambiguous, left alone).
    let mut by_key: HashMap<Key, Option<&str>> = HashMap::new();
    for g in games.iter().filter(|g| g.source == Source::Emulator) {
        let Some(k) = g.emu.as_ref().and_then(|e| key_of(e, g.size_bytes.map(|v| v.min(i64::MAX as u64) as i64)))
        else {
            continue;
        };
        by_key.entry(k).and_modify(|v| *v = None).or_insert(Some(g.source_id.as_str()));
    }
    let rows: Vec<(i64, String, Option<String>, Option<i64>)> = {
        let mut st = conn.prepare("SELECT id, source_id, emu_json, size_bytes FROM games WHERE source = 'emulator'")?;
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?
    };
    let mut folded = 0;
    for (id, sid, emu, size) in rows {
        if reported.contains(sid.as_str()) {
            continue;
        }
        let Some(e) = emu.and_then(|j| serde_json::from_str::<EmuRef>(&j).ok()) else { continue };
        let Some(Some(keep)) = key_of(&e, size).and_then(|k| by_key.get(&k).copied()) else { continue };
        if keep == sid {
            continue;
        }
        let survivor: Option<i64> = conn
            .query_row("SELECT id FROM games WHERE source = 'emulator' AND source_id = ?1", [keep], |r| r.get(0))
            .optional()?;
        match survivor {
            None => {
                conn.execute("UPDATE games SET source_id = ?2 WHERE id = ?1", params![id, keep])?;
                conn.execute(
                    "UPDATE OR IGNORE exclusions SET source_id = ?2 WHERE source = 'emulator' AND source_id = ?1",
                    params![sid, keep],
                )?;
            }
            Some(to) => super::fold::fold_into(conn, id, to)?,
        }
        log::info!("library: {sid} was a second copy of {keep}; folded into it");
        folded += 1;
    }
    Ok(folded)
}
