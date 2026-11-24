//! Emulated games: ROMs in ROM folders (configured + found by themselves), one game per ROM /
//! disc set / game folder, launched through an emulator (`crate::emulation`).
//!
//! Pipeline: `roots` (which folders; remembered in `<data_dir>/emulation/roots.json`) → `walk`
//! (files grouped into games) → `identify` (header serials, CRCs — cached in
//! `<data_dir>/emulation/hash-cache.json` — and the embedded hash database) → `names` (tidy
//! display names) → one `ScannedGame` each, with the emulator chosen at scan time.

use super::ScanContext;
use crate::config::RomFolder;
use crate::emulation::platforms::{self, Platform};
use crate::emulation::registry::{self, Registry};
use crate::model::{Art, EmuRef, ScannedGame, Source};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub(crate) mod identify;
pub(crate) mod names;
pub(crate) mod roots;
pub(crate) mod walk;

#[cfg(test)]
mod tests;

use walk::Candidate;

/// An add-on that is not played on its own (hidden, attached to its game).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Update,
    Dlc,
}

pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>> {
    scan_env(ctx, &roots::Env::real(ctx))
}

/// What a scan of one folder would import (Add game → Emulated games… preview). Pure apart from
/// reading the folder (and Lodestar's CRC cache); call it off the UI thread. Same rules as
/// [`scan`] for that folder alone.
// Called by the ROM import dialog (Add game → Emulated games…).
pub fn preview_folder(ctx: &ScanContext, folder: &RomFolder) -> anyhow::Result<Vec<ScannedGame>> {
    let id = ctx
        .emulation
        .rom_folders
        .iter()
        .find(|(_, f)| f.path == folder.path)
        .map(|(k, _)| k.clone())
        .unwrap_or_else(|| roots::auto_id(&folder.path));
    if !folder.path.is_dir() {
        anyhow::bail!(roots::missing_message(&folder.path));
    }
    let root = roots::configured(&id, folder);
    let reg = scan_registry(ctx);
    let mut cache = identify::HashCache::load(state_file(ctx, "hash-cache.json"));
    Ok(scan_roots(ctx, &reg, &[root], &mut cache).0)
}

/// The ROM folders the last scans remembered (`<data_dir>/emulation/roots.json`): (id, path),
/// including folders found automatically that have no games yet.
pub fn remembered_roots(data_dir: &Path) -> Vec<(String, PathBuf)> {
    let mem = roots::RootsFile::load(Some(&data_dir.join("emulation/roots.json")));
    mem.roots.into_iter().map(|r| (r.id, r.path)).collect()
}

/// `<data_dir>/emulation/<name>` (None when the data folder isn't a real path, e.g. tests).
fn state_file(ctx: &ScanContext, name: &str) -> Option<PathBuf> {
    ctx.data_dir.is_absolute().then(|| ctx.data_dir.join("emulation").join(name))
}

fn scan_registry(ctx: &ScanContext) -> Registry {
    Registry { emulators: crate::emulation::detect::detect(ctx), config: ctx.emulation.clone(), home: ctx.home.clone() }
}

/// [`scan`] with the machine's drive roots given (tests inject their own).
pub(crate) fn scan_env(ctx: &ScanContext, env: &roots::Env) -> anyhow::Result<Vec<ScannedGame>> {
    let found = roots::collect(ctx, env);
    let roots_file = state_file(ctx, "roots.json");
    let mut mem = roots::RootsFile::load(roots_file.as_deref());
    let before = mem.clone();
    if let Err(msg) = roots::check(&found, &mut mem, &ctx.emulation, env) {
        mem.save(roots_file.as_deref());
        anyhow::bail!(msg);
    }
    if found.is_empty() {
        if mem != before {
            mem.save(roots_file.as_deref());
        }
        return Ok(vec![]);
    }
    let reg = scan_registry(ctx);
    let mut cache = identify::HashCache::load(state_file(ctx, "hash-cache.json"));
    let t0 = std::time::Instant::now();
    let (games, per_root) = scan_roots(ctx, &reg, &found, &mut cache);
    log::info!(
        "emulation: {} games in {} folders ({} files from cache, {} read) in {:?}",
        games.len(),
        found.len(),
        cache.hits,
        cache.probed,
        t0.elapsed()
    );
    cache.save(&found.iter().map(|r| r.path.clone()).collect::<Vec<_>>());
    let counted: Vec<(roots::Root, usize)> = found.iter().cloned().zip(per_root).collect();
    roots::remember(&mut mem, &counted);
    if mem != before {
        mem.save(roots_file.as_deref());
    }
    Ok(games)
}

/// Extensions the detected emulators accept per console (beyond the platform table's).
fn extra_exts(reg: &Registry) -> walk::ExtraExts {
    let mut out: walk::ExtraExts = HashMap::new();
    for e in &reg.emulators {
        for p in crate::emulation::detect::profiles(e, &reg.config) {
            for plat in &p.platforms {
                if let Some(pl) = platforms::by_id(plat) {
                    out.entry(pl.id)
                        .or_default()
                        .extend(p.extensions.iter().map(|x| x.trim_start_matches('.').to_ascii_lowercase()));
                }
            }
        }
    }
    out
}

/// Walk, group, identify and build the games of `roots`; also returns games per root.
fn scan_roots(
    ctx: &ScanContext,
    reg: &Registry,
    roots: &[roots::Root],
    cache: &mut identify::HashCache,
) -> (Vec<ScannedGame>, Vec<usize>) {
    let extra = extra_exts(reg);
    let mut skip: HashSet<PathBuf> = roots.iter().flat_map(|r| [r.path.clone(), r.real.clone()]).collect();
    skip.extend(ctx.emulation.ignored_folders.iter().cloned());
    let mut cands: Vec<Candidate> = vec![];
    for (i, r) in roots.iter().enumerate() {
        log::debug!("emulation: scanning {} ({})", r.path.display(), r.how);
        let mut own = skip.clone();
        own.remove(&r.path);
        own.remove(&r.real);
        let mut w = walk::Walker::new(&own, &extra);
        w.walk_root(i, r);
        cands.extend(w.out);
    }
    // Content: header serials / CRCs (cached), and the console the content proves.
    for c in cands.iter_mut() {
        if let Some(f) = &c.probe_file {
            c.probe = cache.probe(&f.path, f.size, f.mtime);
            if c.role.is_none() {
                c.role = c.probe.role;
            }
        }
        if let Some(inner) = c.probe.inner.clone() {
            c.rom_names.push(inner);
        }
        if !roots[c.root].explicit
            && let Some(p) = c.probe.platform.as_deref().and_then(platforms::by_id)
            && p.id != c.platform.id
        {
            log::info!("emulation: {} is a {} game (found in a {} folder)", c.rom.display(), p.name, c.platform.name);
            c.platform = p;
        }
    }
    let cands = same_file_once(cands, roots);
    let cands = attach_addons(cands);
    let cands = dedupe(cands);
    let media = roots::esde_media_dir(&ctx.home);
    let mut games: Vec<(ScannedGame, Option<String>)> = vec![];
    let mut per_root = vec![0usize; roots.len()];
    for c in cands {
        per_root[c.root] += 1;
        let root = &roots[c.root];
        games.push(build(reg, root, c, media.as_deref()));
    }
    // Two games with the same name on one console keep their region in the name.
    let mut count: HashMap<(String, String), usize> = HashMap::new();
    for (g, _) in &games {
        *count.entry((g.platform.clone().unwrap_or_default(), g.name.to_lowercase())).or_default() += 1;
    }
    let mut out = Vec::with_capacity(games.len());
    for (mut g, region) in games {
        if count[&(g.platform.clone().unwrap_or_default(), g.name.to_lowercase())] > 1
            && let Some(r) = region
        {
            g.name = format!("{} ({r})", g.name);
        }
        out.push(g);
    }
    (out, per_root)
}

/// Updates and DLC (Switch title ids, Wii U TMDs, PS3 patches, "(Update)" / "(DLC)" names) are
/// hidden and listed with their game.
fn attach_addons(cands: Vec<Candidate>) -> Vec<Candidate> {
    let (addons, mut games): (Vec<Candidate>, Vec<Candidate>) = cands.into_iter().partition(|c| c.role.is_some());
    for a in addons {
        let id = a.probe.title_id.as_deref().and_then(|t| u64::from_str_radix(t, 16).ok());
        let base = games.iter_mut().find(|g| {
            if g.platform.id != a.platform.id {
                return false;
            }
            let gid = g.probe.title_id.as_deref().and_then(|t| u64::from_str_radix(t, 16).ok());
            match (a.platform.id, id, gid) {
                ("nintendo_switch", Some(a), Some(g)) => {
                    let base = if a & 0xFFF == 0x800 { a & !0xFFF } else { a.wrapping_sub(0x1000) & !0xFFF };
                    base == g
                }
                ("nintendo_wiiu", Some(a), Some(g)) => a as u32 == g as u32,
                _ => match (&a.probe.serial, &g.probe.serial) {
                    (Some(x), Some(y)) if matches!(a.platform.id, "sony_playstation3" | "sony_playstation4") => x == y,
                    _ => names::key(&a.name_src) == names::key(&g.name_src),
                },
            }
        });
        match base {
            Some(g) => {
                g.files.push(a.rom.clone());
                g.files.extend(a.files);
            }
            // A PS4 game with its update merged in (the usual way PS4 dumps are kept) carries the
            // update's param.sfo, CATEGORY "gp": with no separate base game it IS the game.
            None if merged_ps4_game(&a) => {
                log::info!("emulation: {} is a PS4 game with its update merged in", a.rom.display());
                let mut g = a;
                g.role = None;
                g.probe.role = None;
                games.push(g);
            }
            None => log::info!("emulation: {} is an update / DLC without its game", a.rom.display()),
        }
    }
    games
}

/// A PS4 folder marked as an update ("gp") that is a whole game: its eboot.bin and game data
/// (not a lone patch folder whose name says so: "CUSA00900-UPDATE", "CUSA03173 Update",
/// "CUSA03173-UPDATE-1.09", "Game [Patch]"). The word anywhere in the name, not only as an
/// "-update" ending; a name that also says "merged" is the merged game.
fn merged_ps4_game(a: &Candidate) -> bool {
    if a.platform.id != "sony_playstation4" || a.role != Some(Role::Update) || !a.folder_game {
        return false;
    }
    let name = a.rom.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |list: &[&str]| words.iter().any(|w| list.contains(w));
    has(&["merged"]) || !has(&["update", "upd", "patch"])
}

/// One ROM reached through two roots (a folder and a link to it, or a Flatpak's document-portal
/// path to a folder of another root) is one game: the first root's copy is kept (configured
/// folders come first, then the folders found automatically, outer ones before inner ones).
/// Compared by real path, and — for files in different roots — by console, file name and size
/// (a document-portal copy of a file has its own inode, but the same name and size).
fn same_file_once(cands: Vec<Candidate>, roots: &[roots::Root]) -> Vec<Candidate> {
    let mut by_real: HashSet<PathBuf> = HashSet::new();
    let mut by_file: HashMap<(&'static str, String, u64), usize> = HashMap::new();
    let mut out: Vec<Candidate> = Vec::with_capacity(cands.len());
    for c in cands {
        let r = &roots[c.root];
        let real = match c.rom.strip_prefix(&r.path) {
            Ok(rest) => r.real.join(rest),
            Err(_) => c.rom.clone(),
        };
        let real = std::fs::canonicalize(&real).unwrap_or(real);
        if !by_real.insert(real) {
            log::info!("emulation: {} is already listed (same file through another folder)", c.rom.display());
            continue;
        }
        if !c.folder_game
            && let Some(size) = c.size.filter(|s| *s > 0)
        {
            let key = (c.platform.id, walk_name(&c.rom).to_lowercase(), size);
            match by_file.get(&key) {
                Some(&i) if out[i].root != c.root && crc_agrees(&out[i], &c) => {
                    log::info!(
                        "emulation: {} is already listed (same file in {})",
                        c.rom.display(),
                        roots[out[i].root].path.display()
                    );
                    continue;
                }
                Some(_) => {}
                None => {
                    by_file.insert(key, out.len());
                }
            }
        }
        out.push(c);
    }
    out
}

/// Two candidates' contents don't contradict each other (no CRCs, or the same one).
fn crc_agrees(a: &Candidate, b: &Candidate) -> bool {
    match (a.probe.crc, b.probe.crc) {
        (Some(x), Some(y)) => x == y,
        _ => true,
    }
}

/// The same ROM twice on one console (same CRC): one game, the other copy kept as an alternate.
fn dedupe(cands: Vec<Candidate>) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::with_capacity(cands.len());
    let mut by_crc: HashMap<(&'static str, u32), usize> = HashMap::new();
    for c in cands {
        let Some(crc) = c.probe.crc.filter(|_| !c.folder_game) else {
            out.push(c);
            continue;
        };
        match by_crc.get(&(c.platform.id, crc)) {
            Some(&i) => {
                let better = |x: &Candidate| (x.nested, !names::has_region_tag(&x.name_src));
                if better(&c) < better(&out[i]) {
                    let old = std::mem::replace(&mut out[i], c);
                    out[i].files.push(old.rom);
                } else {
                    out[i].files.push(c.rom);
                }
            }
            None => {
                by_crc.insert((c.platform.id, crc), out.len());
                out.push(c);
            }
        }
    }
    out
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).to_string_lossy().into_owned()
}

/// Box art next to the ROM (`<stem>.png`) or in ES-DE's downloaded media.
fn local_art(c: &Candidate, platform: &Platform, media: Option<&Path>) -> Art {
    let mut art = Art::default();
    // PS4 folders carry their own art: sce_sys/icon0.png, pic1.png (or pic0.png) as the background.
    if c.folder_game && platform.id == "sony_playstation4" {
        let sys = c.rom.join("sce_sys");
        let img = |n: &str| Some(sys.join(n)).filter(|p| p.is_file());
        art.icon = img("icon0.png");
        art.hero = img("pic1.png").or_else(|| img("pic0.png"));
    }
    let parent = c.rom.parent().unwrap_or(Path::new("/"));
    let stem = if c.folder_game { walk_name(&c.rom) } else { names::stem(&walk_name(&c.rom)).to_string() };
    art.cover = super::util::image_with_stem(parent, &stem);
    if let Some(media) = media {
        let console = walk_name(&c.console_dir);
        let sub =
            c.rom.strip_prefix(&c.console_dir).ok().and_then(|r| r.parent()).map(Path::to_path_buf).unwrap_or_default();
        for sys in std::iter::once(console.as_str()).chain(platform.es.iter().copied()) {
            let base = media.join(sys);
            if !base.is_dir() {
                continue;
            }
            let find = |kind: &str| super::util::image_with_stem(&base.join(kind).join(&sub), &stem);
            if art.cover.is_none() {
                art.cover = find("covers");
            }
            art.logo = art.logo.take().or_else(|| find("marquees"));
            art.hero = art.hero.take().or_else(|| find("fanart"));
            break;
        }
    }
    art
}

fn walk_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// One candidate → the scanned game (and its region, for same-name disambiguation).
fn build(reg: &Registry, root: &roots::Root, c: Candidate, media: Option<&Path>) -> (ScannedGame, Option<String>) {
    let platform = c.platform;
    let disc = walk::is_disc(&c);
    // Folder games: the content's own title is a better fallback than an odd folder name.
    let name_src = c.name_src.clone();
    let ident = identify::identify(platform, &name_src, &c.rom_names, &c.probe, disc);
    let (name, region) = match &ident.entry {
        Some(e) => (names::tidy(&e.name, true), ident.region.clone()),
        None => {
            // Dump-set names (region tags, a Switch title id) are official titles.
            let official = names::has_region_tag(&name_src) || names::switch_title_id(&name_src).is_some();
            let from_file = names::tidy(&name_src, official);
            let name = match (&c.probe.title, c.folder_game) {
                // PS4 folders are named by serial or by hand: the game's own title is better.
                (Some(t), true) if names::uninformative(&name_src) || platform.id == "sony_playstation4" => t.clone(),
                _ => from_file,
            };
            let region = names::region(&name_src).or_else(|| ident.region.clone());
            (name, region)
        }
    };
    let serial = c.probe.serial.clone().or_else(|| c.probe.title_id.clone());
    let mut emu = EmuRef {
        platform: platform.id.to_string(),
        folder: root.id.clone(),
        rom: c.rom.clone(),
        files: c.files.clone(),
        emulator: String::new(),
        profile: String::new(),
        serial,
        crc: c.probe.crc.map(|v| format!("{v:08x}")),
        dat_name: ident.dat_name.clone(),
        region: region.clone(),
    };
    let chosen = registry::choose(reg, platform.id, &c.rom, None, None, root.folder.as_ref());
    let launch = chosen.as_ref().and_then(|(e, p)| {
        emu.emulator = e.id.clone();
        emu.profile = p.id.clone();
        registry::build_spec(reg, e, p, &emu)
    });
    // A Wii U folder Cemu can't open: listed, but Play explains instead of starting it.
    let launch = if registry::unplayable_ref(&emu).is_some() { None } else { launch };
    let mut g = ScannedGame::new(Source::Emulator, format!("{}/{}", root.id, rel(&root.path, &c.rom)), name);
    g.installed = true;
    g.install_dir = if c.folder_game { Some(c.rom.clone()) } else { c.rom.parent().map(Path::to_path_buf) };
    g.size_bytes = if c.folder_game { None } else { c.size };
    g.platform = Some(platform.name.to_string());
    g.meta.release_year = ident.year;
    g.art = local_art(&c, platform, media);
    g.launch = launch;
    g.emu = Some(emu);
    (g, region)
}
