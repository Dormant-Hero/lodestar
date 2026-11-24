//! RomM server games (`Source::Romm`, `source_id` = RomM's ROM id) from the library Lodestar
//! cached at the last sync (`<data_dir>/romm/library.json`, see romm.rs): a scan never waits on
//! the network. Every game is "not installed"; Download (romm_app.rs) puts it in a ROM folder,
//! where the emulation scanner finds it, and [`dedupe_with_emulation`] then drops the RomM entry
//! (lending it the server's cover and description).

use super::ScanContext;
use crate::model::*;
use crate::romm::{self, RommGame};
use std::collections::HashMap;
use std::path::Path;

pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>> {
    // Not set up / never synced: nothing (a sync that finds games writes the cache).
    let Some(lib) = romm::load_cache(&ctx.data_dir) else {
        return Ok(vec![]);
    };
    let covers = romm::covers_dir(&ctx.data_dir);
    Ok(lib.games.iter().map(|g| scanned(g, &covers)).collect())
}

pub fn scanned(g: &RommGame, covers: &Path) -> ScannedGame {
    let mut sg = ScannedGame::new(Source::Romm, g.id.to_string(), g.name.clone());
    sg.installed = false;
    // Lodestar's console name when it knows the console (it is what "Download" needs), else
    // RomM's.
    sg.platform = Some(g.platform_def().map(|p| p.name.to_string()).unwrap_or_else(|| g.platform_name.clone()))
        .filter(|p| !p.is_empty());
    sg.size_bytes = (g.size > 0).then_some(g.size);
    sg.meta.description = g.summary.clone();
    sg.meta.genres = g.genres.clone();
    sg.meta.release_year = g.year;
    sg.meta.developer = g.developer.clone();
    sg.meta.publisher = g.publisher.clone();
    sg.art.cover = romm::cover_file(covers, g);
    sg
}

/// Names a ROM is known by, lower-case: its file, its folder (a game folder / m3u set), its
/// other files.
fn emu_keys(e: &EmuRef) -> Vec<String> {
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().to_lowercase());
    let mut v: Vec<String> = vec![];
    v.extend(name(&e.rom));
    if let Some(parent) = e.rom.parent() {
        v.extend(name(parent));
    }
    v.extend(e.files.iter().filter_map(|f| name(f)));
    v
}

/// Drop RomM games whose ROM the emulation scan found (downloaded, or the user had it already):
/// same console and same file / folder name (or the server's zip of it). The emulated game
/// borrows the RomM entry's cover and metadata where it has none. Returns how many were dropped.
pub fn dedupe_with_emulation(ctx: &ScanContext, games: &mut Vec<ScannedGame>) -> usize {
    if !games.iter().any(|g| g.source == Source::Romm) || !games.iter().any(|g| g.source == Source::Emulator) {
        return 0;
    }
    let Some(lib) = romm::load_cache(&ctx.data_dir) else { return 0 };
    let by_id: HashMap<String, &RommGame> = lib.games.iter().map(|g| (g.id.to_string(), g)).collect();
    let mut index: HashMap<(String, String), usize> = HashMap::new();
    for (i, g) in games.iter().enumerate() {
        if g.source != Source::Emulator {
            continue;
        }
        if let Some(e) = &g.emu {
            for k in emu_keys(e) {
                index.entry((e.platform.clone(), k)).or_insert(i);
            }
        }
    }
    let mut lend: Vec<(usize, usize)> = vec![];
    for (i, g) in games.iter().enumerate() {
        if g.source != Source::Romm {
            continue;
        }
        let Some(r) = by_id.get(&g.source_id) else { continue };
        let Some(plat) = &r.platform else { continue };
        let fs = r.fs_name.to_lowercase();
        let hit = [fs.clone(), format!("{fs}.zip")].into_iter().find_map(|k| index.get(&(plat.clone(), k)).copied());
        if let Some(t) = hit {
            lend.push((i, t));
        }
    }
    let mut drop = vec![false; games.len()];
    for (from, to) in &lend {
        let (art, meta) = (games[*from].art.clone(), games[*from].meta.clone());
        let absorbed = (games[*from].source, games[*from].source_id.clone());
        games[*to].art.fill_from(&art);
        games[*to].meta.fill_from(&meta);
        // The RomM row's favourite, tags, collections and playtime move to the ROM's row.
        games[*to].absorbs.push(absorbed);
        drop[*from] = true;
    }
    let mut i = 0;
    games.retain(|_| {
        let keep = !drop[i];
        i += 1;
        keep
    });
    lend.len()
}
