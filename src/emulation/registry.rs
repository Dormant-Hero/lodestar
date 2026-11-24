//! The emulation state published process-wide (emulators found + the `[emulation]` config) and
//! building an emulated game's launch command from it. `launch::resolve` calls [`spec_for`] on
//! every resolve, so nothing here touches the filesystem except [`refresh`] — the few facts a
//! command needs from the disk (is the ROM a folder, which file inside a folder game starts it,
//! the tracks a cue sheet lists, a Flatpak's permissions) are looked up once and cached until
//! the next refresh.
//!
//! Choosing an emulator for a game: the game's own choice (Edit → Installation) > the ROM
//! folder's > Settings → Emulation's per-console choice > the best one found: emulators the user
//! added first, then the built-in preference order (`defs::for_platform`: standalone emulators
//! first, except for the consoles where RetroArch is the usual choice), then how each was
//! installed (Flatpak, program, AppImage, Steam), then profile order. A profile that opens the
//! ROM's file type wins over one that only opens the tracks of its cue sheet (PCSX2 + .cue).
//! An AppImage that isn't allowed to run yet (`Emulator::blocked`) is never chosen.
//!
//! The disk facts are cached per path and never looked up again on the UI thread once known:
//! [`refresh`] re-reads every cached path on its own (background) thread and swaps the results
//! in, and [`warm`] (called after a refresh with the library's emulated games) fills them for
//! games not seen yet.

use super::detect::{self, Emulator, InstallKind, Profile};
use crate::config::{EmulationConfig, RomFolder};
use crate::model::{EmuRef, Game, LaunchSpec};
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Emulators found plus the settings, as of the last [`refresh`].
#[derive(Clone, Debug)]
pub struct Registry {
    pub emulators: Vec<Emulator>,
    pub config: EmulationConfig,
    pub home: PathBuf,
}

impl Registry {
    pub fn emulator(&self, id: &str) -> Option<&Emulator> {
        self.emulators.iter().find(|e| e.id == id)
    }

    /// The ROM folder root a path is in (configured folders, then roots the scanner noted).
    pub fn root_of(&self, folder: &str, path: &Path) -> Option<PathBuf> {
        if let Some(f) = self.config.rom_folders.get(folder)
            && path.starts_with(&f.path)
        {
            return Some(f.path.clone());
        }
        if let Some(p) = ROOTS.read().as_ref().and_then(|m| m.get(folder))
            && path.starts_with(p)
        {
            return Some(p.clone());
        }
        self.config.rom_folders.values().map(|f| &f.path).find(|p| path.starts_with(p)).cloned()
    }
}

static CURRENT: RwLock<Option<Arc<Registry>>> = RwLock::new(None);

/// Replace the published registry.
pub fn publish(reg: Registry) {
    *CURRENT.write() = Some(Arc::new(reg));
}

/// The published registry (None until the first publish: tests, early startup).
pub fn current() -> Option<Arc<Registry>> {
    CURRENT.read().clone()
}

/// Detect emulators and publish the result with `ctx.emulation`. Touches the filesystem: call
/// from a background thread (or headless). When refreshes overlap, the one that started last
/// wins. Detects again (the detection cache and the launch-file cache are dropped).
pub fn refresh(ctx: &crate::scan::ScanContext) {
    static STARTED: AtomicU64 = AtomicU64::new(0);
    static PUBLISHED: parking_lot::Mutex<u64> = parking_lot::Mutex::new(0);
    let ticket = STARTED.fetch_add(1, Ordering::SeqCst) + 1;
    detect::invalidate();
    rewarm_facts();
    note_remembered_roots(ctx);
    let roots = detect::ProbeRoots::system(&ctx.home);
    if !ctx.emulation.emulator_dirs_checked {
        let keep = detect::dirs_to_migrate(&ctx.emulation, &roots);
        log::info!("emulation: emulator folders kept from before: {keep:?} (drives {:?})", roots.drives);
        *MIGRATION.lock() = Some(keep);
    }
    let suggested = detect::suggested_dirs(&ctx.emulation, &roots);
    let emulators = super::detect::detect(ctx);
    let mut last = PUBLISHED.lock();
    if ticket > *last {
        *last = ticket;
        *SUGGESTED.write() = suggested;
        publish(Registry { emulators, config: ctx.emulation.clone(), home: ctx.home.clone() });
    }
}

/// Folders named "Emulators" on the drives to offer as emulator folders (`detect::suggested_dirs`),
/// as of the last [`refresh`].
static SUGGESTED: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());

/// The one-time check's result (`detect::dirs_to_migrate`), waiting for the app to save it.
static MIGRATION: Mutex<Option<Vec<PathBuf>>> = Mutex::new(None);

/// "Found emulators in <folder>" suggestions (Settings → Emulation).
pub fn suggested_dirs() -> Vec<PathBuf> {
    SUGGESTED.read().clone()
}

/// The emulator folders a config from before used (taken once; None = no check ran yet).
pub fn take_migration() -> Option<Vec<PathBuf>> {
    MIGRATION.lock().take()
}

/// ROM folder roots found automatically (`EmuRef.folder` id → path). The scanner notes them so
/// Flatpak access is granted for the whole root and BIOS folders inside it are found.
static ROOTS: RwLock<Option<HashMap<String, PathBuf>>> = RwLock::new(None);

/// Remember automatically found ROM folder roots (id, path); adds to / replaces earlier ones.
pub fn note_roots(roots: &[(String, PathBuf)]) {
    let mut g = ROOTS.write();
    let m = g.get_or_insert_with(HashMap::new);
    for (id, p) in roots {
        m.insert(id.clone(), p.clone());
    }
}

/// The ROM folders the scanner remembered (`<data_dir>/emulation/roots.json`) as noted roots, so
/// Play grants a Flatpak the whole folder and BIOS folders inside them are found before a scan.
fn note_remembered_roots(ctx: &crate::scan::ScanContext) {
    if !ctx.data_dir.is_absolute() {
        return;
    }
    let mem = crate::scan::emulation::remembered_roots(&ctx.data_dir);
    let auto: Vec<(String, PathBuf)> = mem
        .into_iter()
        .filter(|(id, p)| {
            id.starts_with("auto-")
                && ctx.emulation.auto_folders
                && !ctx.emulation.ignored_folders.iter().any(|i| p.starts_with(i))
        })
        .collect();
    note_roots(&auto);
}

/// The noted roots with their ids (`auto-…` → path), sorted by path.
pub fn noted() -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> =
        ROOTS.read().iter().flat_map(|m| m.iter().map(|(k, p)| (k.clone(), p.clone()))).collect();
    v.sort_by(|a, b| a.1.cmp(&b.1));
    v
}

/// The roots noted with [`note_roots`].
pub fn noted_roots() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = ROOTS.read().iter().flat_map(|m| m.values().cloned()).collect();
    v.sort();
    v.dedup();
    v
}

// ---------------------------------------------------------------------------------------------
// Small disk facts, cached until the next refresh
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Default)]
struct FsFacts {
    is_dir: Option<bool>,
    /// Folder games: (pattern list, file found).
    folder_launch: Option<(Vec<String>, Option<PathBuf>)>,
    /// Cue sheet / m3u: the files it lists.
    tracks: Option<Vec<PathBuf>>,
    /// Wii U game folders: what keeps Cemu from opening it (Some(None) = nothing).
    wiiu: Option<Option<String>>,
}

static FS: Mutex<Option<HashMap<PathBuf, FsFacts>>> = Mutex::new(None);

fn with_facts<T>(p: &Path, f: impl FnOnce(&mut FsFacts) -> T) -> T {
    let mut g = FS.lock();
    let m = g.get_or_insert_with(HashMap::new);
    if m.len() > 20_000 {
        m.clear();
    }
    f(m.entry(p.to_path_buf()).or_default())
}

/// Look every cached path up again (off the UI thread: `refresh`) and swap the new facts in,
/// so a changed disk is noticed while the UI keeps finding everything in the cache.
fn rewarm_facts() {
    let old: Vec<(PathBuf, FsFacts)> =
        FS.lock().as_ref().map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
    let mut fresh: HashMap<PathBuf, FsFacts> = HashMap::with_capacity(old.len());
    for (p, f) in old {
        let mut n = FsFacts::default();
        if f.is_dir.is_some() {
            n.is_dir = Some(p.is_dir());
        }
        if let Some((pats, _)) = &f.folder_launch {
            n.folder_launch = Some((pats.clone(), find_folder_launch(&p, pats)));
        }
        if f.tracks.is_some() {
            n.tracks = Some(read_tracks(&p));
        }
        if f.wiiu.is_some() {
            n.wiiu = Some(wiiu_problem_uncached(&p));
        }
        fresh.insert(p, n);
    }
    // Swap in the new facts; anything cached meanwhile (not in the snapshot) is kept.
    let mut g = FS.lock();
    let m = g.get_or_insert_with(HashMap::new);
    m.extend(fresh);
}

/// Fill the disk facts for these games (their launch commands, whether they can start) so the
/// UI never has to look at the disk for them. Call off the UI thread, after a refresh.
pub fn warm(games: &[Game]) {
    let Some(reg) = current() else { return };
    for g in games {
        if let Some(e) = &g.emu {
            let _ = unplayable_ref(e);
            let _ = spec_with(&reg, g, e, None);
            // The other ROMs of its list, and what its emulator actions start.
            for r in super::roms::list(g).iter().skip(1) {
                let _ = is_dir(&r.path);
            }
            for a in g.edits.actions.iter().filter(|a| a.kind == crate::model::ActionKind::Emulator) {
                let _ = action_spec(&reg, g, a, None);
            }
        }
    }
}

fn is_dir(p: &Path) -> bool {
    if let Some(d) = with_facts(p, |f| f.is_dir) {
        return d;
    }
    let d = p.is_dir();
    with_facts(p, |f| f.is_dir = Some(d));
    d
}

fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// The first file matching one of `patterns` inside a folder game (`*` in the last part).
fn folder_launch_file(dir: &Path, patterns: &[String]) -> Option<PathBuf> {
    if let Some((pats, found)) = with_facts(dir, |f| f.folder_launch.clone())
        && pats == patterns
    {
        return found;
    }
    let found = find_folder_launch(dir, patterns);
    with_facts(dir, |f| f.folder_launch = Some((patterns.to_vec(), found.clone())));
    found
}

fn find_folder_launch(dir: &Path, patterns: &[String]) -> Option<PathBuf> {
    let mut found = None;
    for pat in patterns {
        let (sub, last) = match pat.rsplit_once('/') {
            Some((s, l)) => (dir.join(s), l),
            None => (dir.to_path_buf(), pat.as_str()),
        };
        if !last.contains('*') && !last.contains('?') {
            // Exact name, but matched case-insensitively (EBOOT.BIN vs eboot.bin).
            let exact = sub.join(last);
            if exact.is_file() {
                found = Some(exact);
                break;
            }
        }
        let lower = last.to_lowercase();
        let mut names: Vec<PathBuf> =
            std::fs::read_dir(&sub).into_iter().flatten().flatten().map(|e| e.path()).collect();
        names.sort();
        if let Some(p) = names
            .into_iter()
            .find(|p| p.is_file() && p.file_name().is_some_and(|n| detect::glob_match(&lower, &n.to_string_lossy())))
        {
            found = Some(p);
            break;
        }
    }
    found
}

/// The files a cue sheet / m3u / gdi lists (cached).
fn tracks(sheet: &Path) -> Vec<PathBuf> {
    if let Some(t) = with_facts(sheet, |f| f.tracks.clone()) {
        return t;
    }
    let out = read_tracks(sheet);
    with_facts(sheet, |f| f.tracks = Some(out.clone()));
    out
}

fn read_tracks(sheet: &Path) -> Vec<PathBuf> {
    let dir = sheet.parent().unwrap_or(Path::new("/"));
    let text = std::fs::metadata(sheet)
        .ok()
        .filter(|m| m.len() < 256 * 1024)
        .and_then(|_| std::fs::read(sheet).ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    let ext = ext_of(sheet);
    let mut out = vec![];
    for line in text.lines() {
        let l = line.trim();
        let name = match ext.as_str() {
            "cue" => l.strip_prefix("FILE ").or_else(|| l.strip_prefix("file ")).map(|r| {
                match r.trim_start().strip_prefix('"') {
                    Some(q) => q.split('"').next().unwrap_or("").to_string(),
                    None => r.split_whitespace().next().unwrap_or("").to_string(),
                }
            }),
            "gdi" => l.split_whitespace().nth(4).map(|s| s.trim_matches('"').to_string()),
            _ => (!l.is_empty() && !l.starts_with('#')).then(|| l.to_string()),
        };
        if let Some(n) = name.filter(|n| !n.is_empty()) {
            out.push(dir.join(n));
        }
    }
    out
}

/// Why Cemu can't open a Wii U game folder (None: it can). Cemu opens an NUS folder by its
/// `title.tmd` (with `title.tik` and the `.app` content files next to it), a decrypted copy
/// (`code`, `content`, `meta` folders), or a .wua / .wud / .wux file — not a folder of content
/// files without those names (some download tools keep `tmd.0` and bare content files).
fn wiiu_problem_uncached(dir: &Path) -> Option<String> {
    let names: Vec<String> =
        std::fs::read_dir(dir).ok()?.flatten().map(|e| e.file_name().to_string_lossy().to_lowercase()).collect();
    let has = |n: &str| names.iter().any(|x| x == n);
    if has("code") && has("content") && has("meta") {
        return None;
    }
    if names.iter().any(|n| n.ends_with(".wua") || n.ends_with(".wud") || n.ends_with(".wux") || n.ends_with(".rpx")) {
        return None;
    }
    let tmd = has("title.tmd");
    let tik = has("title.tik");
    let apps = names.iter().any(|n| n.ends_with(".app"));
    if tmd && tik && apps {
        return None;
    }
    let mut missing: Vec<&str> = vec![];
    if !tmd {
        missing.push("title.tmd");
    }
    if !tik {
        missing.push("title.tik");
    }
    if !apps {
        missing.push("the .app content files");
    }
    let what = match missing.as_slice() {
        [one] => format!("no {one}"),
        [a, b] => format!("no {a} and no {b}"),
        _ => format!("no {}, no {} and none of {}", missing[0], missing[1], missing[2]),
    };
    let extra = if has("tmd.0") || names.iter().any(|n| n.len() == 8 && n.chars().all(|c| c.is_ascii_hexdigit())) {
        " (this copy has tmd.0 and content files without the .app names)"
    } else {
        ""
    };
    Some(format!(
        "Cemu can't open this Wii U game folder: it has {what}{extra}. Cemu needs title.tmd, title.tik and the \
         .app files together, or the game as a .wua / .wud / .wux file, or a decrypted copy (code, content and meta \
         folders)."
    ))
}

/// [`wiiu_problem_uncached`], cached.
fn wiiu_problem(dir: &Path) -> Option<String> {
    if let Some(v) = with_facts(dir, |f| f.wiiu.clone()) {
        return v;
    }
    let v = if is_dir(dir) { wiiu_problem_uncached(dir) } else { None };
    with_facts(dir, |f| f.wiiu = Some(v.clone()));
    v
}

pub fn unplayable_ref(e: &EmuRef) -> Option<String> {
    (e.platform == "nintendo_wiiu").then(|| wiiu_problem(&e.rom)).flatten()
}

/// Why this emulated game's files can't be started by any emulator, in plain words (None: they
/// can, or it isn't an emulated game). Cached; the disk is read once per game.
pub fn unplayable(game: &Game) -> Option<String> {
    unplayable_ref(game.emu.as_ref()?)
}

const SHEETS: &[&str] = &["cue", "ccd", "toc", "m3u", "gdi", "mds"];
const TRACKS: &[&str] = &["bin", "img", "iso", "chd", "mdf", "cso"];

/// How well a profile opens a ROM: 0 = its file type, 1 = only the tracks of its cue sheet,
/// 2 = a folder game it starts from inside, None = it can't.
fn opens(profile: &Profile, rom: &Path, platform_folder_games: bool) -> Option<u8> {
    let ext = ext_of(rom);
    if !ext.is_empty() && profile.accepts_ext(&ext) {
        return Some(0);
    }
    if SHEETS.contains(&ext.as_str()) && TRACKS.iter().any(|t| profile.accepts_ext(t)) {
        return Some(1);
    }
    if (platform_folder_games || !profile.folder_launch.is_empty()) && is_dir(rom) {
        return Some(2);
    }
    None
}

fn platform_has_folder_games(platform: &str) -> bool {
    super::platforms::by_id(platform).is_some_and(|p| p.folder_games)
}

/// "emu/profile" or "emu" → (emu, profile).
fn split_choice(s: &str) -> Option<(&str, Option<&str>)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    Some(match s.split_once('/') {
        Some((e, p)) if !p.trim().is_empty() => (e.trim(), Some(p.trim())),
        Some((e, _)) => (e.trim(), None),
        None => (s, None),
    })
}

/// Best emulator + profile for a platform: per-game edit > per-folder > `platform_emulator` >
/// the first found that supports it.
pub fn choose(
    reg: &Registry,
    platform: &str,
    rom: &Path,
    game_emu: Option<&str>,
    game_profile: Option<&str>,
    folder: Option<&RomFolder>,
) -> Option<(Emulator, Profile)> {
    let fg = platform_has_folder_games(platform);
    // The explicit choices, strongest first.
    let mut explicit: Vec<(String, Option<String>)> = vec![];
    if let Some(e) = game_emu.map(str::trim).filter(|e| !e.is_empty()) {
        explicit.push((e.to_string(), game_profile.map(str::trim).filter(|p| !p.is_empty()).map(str::to_string)));
    }
    if let Some(f) = folder.filter(|f| !f.emulator.trim().is_empty()) {
        explicit.push((f.emulator.trim().to_string(), Some(f.profile.trim().to_string()).filter(|p| !p.is_empty())));
    }
    if let Some((e, p)) = reg.config.platform_emulator.get(platform).and_then(|s| split_choice(s)) {
        explicit.push((e.to_string(), p.map(str::to_string)));
    }
    for (emu_id, prof_id) in explicit {
        let Some(emu) = reg.emulator(&emu_id) else { continue };
        // Picked by the user but not allowed to run yet: never quietly another emulator
        // (Play asks to allow it, see `blocked_choice`).
        if emu.blocked {
            return None;
        }
        let profs = detect::profiles(emu, &reg.config);
        let pick = match &prof_id {
            Some(pid) => profs.iter().find(|p| p.id == *pid),
            None => None,
        }
        .or_else(|| profs.iter().filter(|p| p.supports(platform)).min_by_key(|p| opens(p, rom, fg).unwrap_or(9)));
        if let Some(p) = pick {
            return Some((emu.clone(), p.clone()));
        }
    }
    // Automatic.
    let order = super::defs::for_platform(platform);
    let rank_def = |e: &Emulator| -> usize { order.iter().position(|d| d.id == e.def).unwrap_or(order.len()) };
    // (can't open the file, not added by the user, preference, how well it opens it, an old copy
    // kept aside, order found, profile order)
    type Key = (bool, bool, usize, u8, bool, usize, usize);
    let mut best: Option<(Key, &Emulator, Profile)> = None;
    for (ei, emu) in reg.emulators.iter().enumerate().filter(|(_, e)| !e.blocked) {
        for (pi, p) in detect::profiles(emu, &reg.config).into_iter().enumerate() {
            if !p.supports(platform) {
                continue;
            }
            let fit = opens(&p, rom, fg);
            let key: Key = (fit.is_none(), !emu.user, rank_def(emu), fit.unwrap_or(9), emu.looks_old(), ei, pi);
            if best.as_ref().is_none_or(|(k, _, _)| key < *k) {
                best = Some((key, emu, p));
            }
        }
    }
    best.map(|(_, e, p)| (e.clone(), p))
}

/// The emulator the user picked for this game (its own choice, else its ROM folder's, else its
/// console's — the strongest one that is set and found) when that one isn't allowed to run yet
/// (a downloaded AppImage without the executable bit). [`choose`] then gives nothing, and Play
/// offers to allow it.
pub fn blocked_choice(reg: &Registry, game: &Game) -> Option<Emulator> {
    let e = game.emu.as_ref()?;
    let folder = reg.config.rom_folders.get(&e.folder);
    let picks = [
        game.edits.emulator.as_deref().map(str::to_string),
        folder.map(|f| f.emulator.clone()),
        reg.config.platform_emulator.get(&e.platform).and_then(|s| split_choice(s)).map(|(e, _)| e.to_string()),
    ];
    let first = picks
        .into_iter()
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .find_map(|id| reg.emulator(&id))?;
    first.blocked.then(|| first.clone())
}

/// [`blocked_choice`] against the published registry.
pub fn blocked_choice_now(game: &Game) -> Option<Emulator> {
    current().and_then(|r| blocked_choice(&r, game))
}

/// "Allow it to run" was clicked and worked: publish the registry with that emulator allowed
/// right away (the re-detection that follows confirms it), so Play can start it now.
pub fn mark_allowed(id: &str) {
    let Some(cur) = current() else { return };
    let mut reg = (*cur).clone();
    let mut changed = false;
    for e in reg.emulators.iter_mut().filter(|e| e.id == id && e.blocked) {
        e.blocked = false;
        e.how = e.how.replace(" — not allowed to run yet", "");
        changed = true;
    }
    if changed {
        publish(reg);
    }
}

/// Play must not fall back to the command an earlier scan stored: the game's files can't be
/// opened, or the emulator the user picked isn't allowed to run yet.
pub fn no_fallback(game: &Game) -> bool {
    unplayable(game).is_some() || blocked_choice_now(game).is_some()
}

/// [`choose`] for an emulated game (its edits and ROM folder).
pub fn choose_for(reg: &Registry, game: Option<&Game>, e: &EmuRef) -> Option<(Emulator, Profile)> {
    let folder = reg.config.rom_folders.get(&e.folder);
    let (ge, gp) =
        game.map(|g| (g.edits.emulator.as_deref(), g.edits.emulator_profile.as_deref())).unwrap_or((None, None));
    choose(reg, &e.platform, &e.rom, ge, gp, folder)
}

// ---------------------------------------------------------------------------------------------
// Building the command
// ---------------------------------------------------------------------------------------------

/// The file an emulator is started with: folder games → the profile's file inside the folder;
/// multi-file games → the first of `rom` + `files` (+ the tracks of a cue sheet) whose type the
/// profile opens, in the profile's extension order.
pub fn launch_file(profile: &Profile, e: &EmuRef) -> PathBuf {
    if (!profile.folder_launch.is_empty() || platform_has_folder_games(&e.platform)) && is_dir(&e.rom) {
        return folder_launch_file(&e.rom, &profile.folder_launch).unwrap_or_else(|| e.rom.clone());
    }
    let mut cands: Vec<&PathBuf> = std::iter::once(&e.rom).chain(e.files.iter()).collect();
    let pick = |c: &[&PathBuf]| -> Option<PathBuf> {
        profile.extensions.iter().find_map(|x| c.iter().find(|p| ext_of(p) == *x).map(|p| (*p).clone()))
    };
    if let Some(p) = pick(&cands) {
        return p;
    }
    let rom_ext = ext_of(&e.rom);
    if SHEETS.contains(&rom_ext.as_str()) {
        let t = tracks(&e.rom);
        cands.extend(t.iter());
        if let Some(p) = pick(&cands) {
            return p;
        }
    }
    e.rom.clone()
}

/// The folder a Flatpak must be allowed to see for this game: the ROM folder root it is in,
/// else the folder holding all of its files.
fn grant_dir(reg: &Registry, e: &EmuRef, launch: &Path) -> PathBuf {
    if let Some(r) = reg.root_of(&e.folder, &e.rom) {
        return r;
    }
    let base = if is_dir(&e.rom) { e.rom.clone() } else { e.rom.parent().map(Path::to_path_buf).unwrap_or_default() };
    let mut dir = base.clone();
    for f in e.files.iter().map(PathBuf::as_path).chain([launch]) {
        let fd = f.parent().unwrap_or(f);
        while !fd.starts_with(&dir) {
            match dir.parent() {
                Some(p) => dir = p.to_path_buf(),
                None => break,
            }
        }
    }
    if dir.parent().is_none() { base } else { dir }
}

/// When a Flatpak emulator can't see the game's files: the folder to grant with
/// `--filesystem=<dir>` (None: not a Flatpak, or its permissions already cover it).
pub fn needs_filesystem(reg: &Registry, emu: &Emulator, profile: &Profile, e: &EmuRef) -> Option<PathBuf> {
    let app = emu.flatpak_id()?;
    let launch = launch_file(profile, e);
    let dir = grant_dir(reg, e, &launch);
    let access = detect::flatpak_access(&reg.home, app);
    let seen = |p: &Path| detect::fs_covers(&access, &reg.home, p);
    (!(seen(&dir) || seen(&launch) && e.files.iter().all(|f| seen(f)) && seen(&e.rom))).then_some(dir)
}

/// Fill one template word; None = the word is left out.
fn fill(word: &str, v: &Vars, fullscreen: bool) -> Result<Option<String>, ()> {
    let w = if let Some(r) = word.strip_prefix("{fs}") {
        if !fullscreen {
            return Ok(None);
        }
        r
    } else if let Some(r) = word.strip_prefix("{win}") {
        if fullscreen {
            return Ok(None);
        }
        r
    } else {
        word
    };
    if w == "{fullscreen}" || w == "{args}" || w.is_empty() {
        return Ok(None);
    }
    if (w.contains("{core}")) && v.core.is_none() {
        return Err(());
    }
    let empty = String::new();
    let tokens: [(&str, &String); 15] = [
        ("{rom}", &v.rom),
        ("{ImagePath}", &v.rom),
        ("{Rom}", &v.rom),
        ("{rom_dir}", &v.rom_dir),
        ("{ImageDir}", &v.rom_dir),
        ("{RomDir}", &v.rom_dir),
        ("{rom_name}", &v.rom_name),
        ("{ImageNameNoExt}", &v.rom_name),
        ("{RomName}", &v.rom_name),
        ("{ImageName}", &v.file_name),
        ("{emu_dir}", &v.emu_dir),
        ("{EmuDir}", &v.emu_dir),
        ("{Platform}", &v.platform),
        ("{Emulator}", &v.emulator),
        ("{core}", v.core.as_ref().unwrap_or(&empty)),
    ];
    // One pass over the template word: a value (a ROM path with "{…}" in its name) is never
    // substituted again.
    let mut out = String::with_capacity(w.len() + v.rom.len());
    let mut rest = w;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        match tokens.iter().find(|(t, _)| tail.starts_with(t)) {
            Some((t, val)) => {
                out.push_str(val);
                rest = &tail[t.len()..];
            }
            None => {
                out.push('{');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    Ok(Some(out))
}

struct Vars {
    rom: String,
    rom_dir: String,
    rom_name: String,
    file_name: String,
    emu_dir: String,
    platform: String,
    emulator: String,
    core: Option<String>,
}

/// What goes into one command besides the emulator and profile ([`build`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct BuildOpts<'a> {
    /// Some = fullscreen or not whatever the switches say (Play in a window = Some(false)).
    pub fullscreen: Option<bool>,
    /// The game: its own switches and extra arguments (Edit → Installation).
    pub game: Option<&'a Game>,
    /// An emulator action: its switches, extra arguments, "replace the profile's arguments".
    pub action: Option<&'a crate::model::GameAction>,
}

type LevelData = (std::collections::BTreeMap<String, bool>, Vec<String>);

/// The switches of this emulator + profile and every level's (switches, extra words).
fn switch_levels(
    reg: &Registry,
    emu: &Emulator,
    profile: &Profile,
    platform: &str,
    o: &BuildOpts,
) -> Result<(Vec<super::options::Switch>, Vec<LevelData>), String> {
    let words = |s: &str, what: &str| crate::launch::split_command(s).map_err(|e| format!("{what}: {e}"));
    let tpl = match o.action.filter(|a| a.replace_args) {
        Some(a) => a.args.clone(),
        None => super::options::template(emu, profile).to_string(),
    };
    let tpl_words = words(&tpl, "arguments")?;
    let defs = emu.definition().map(|d| d.options).unwrap_or(&[]);
    let sws = super::options::switches_for(&tpl_words, defs, &emu.name, reg.config.fullscreen);
    let mut levels: Vec<LevelData> = vec![];
    if let Some(a) = o.action {
        let w = if a.replace_args { vec![] } else { words(&a.args, "extra arguments")? };
        levels.push((a.options.clone(), w));
    }
    if let Some(g) = o.game {
        levels.push((g.edits.emulator_options.clone(), words(&g.edits.emulator_args, "the game's extra arguments")?));
    }
    for set in [reg.config.console_settings.get(platform), reg.config.emulator_settings.get(&emu.id)] {
        let set = set.cloned().unwrap_or_default();
        let w = words(&set.extra_args, "extra arguments (Settings → Emulation)")?;
        levels.push((set.options, w));
    }
    Ok((sws, levels))
}

/// The command for a ROM: the profile's arguments (or the user's), the switches and extra
/// arguments of every level (see [`super::options`]), `flatpak run --filesystem=<dir> …` when
/// the sandbox can't see the ROM. Err says why it can't be built (a typo in typed arguments,
/// a RetroArch core that isn't there).
pub fn build(
    reg: &Registry,
    emu: &Emulator,
    profile: &Profile,
    e: &EmuRef,
    o: &BuildOpts,
) -> Result<LaunchSpec, String> {
    use super::options;
    let (sws, levels) = switch_levels(reg, emu, profile, &e.platform, o)?;
    let replace = o.action.filter(|a| a.replace_args).map(|a| a.args.as_str());
    let template = replace.unwrap_or_else(|| options::template(emu, profile));
    let mut words = crate::launch::split_command(template).map_err(|e| format!("arguments: {e}"))?;
    // The user's own arguments that name no game file: the game goes at the end.
    if replace.is_some() && !words.iter().any(|w| options::is_rom_word(w) || w.starts_with('/') || w.starts_with("~/"))
    {
        words.push("{rom}".into());
    }
    let launch = launch_file(profile, e);
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let vars = Vars {
        rom: s(&launch),
        rom_dir: launch.parent().map(s).unwrap_or_default(),
        rom_name: launch.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        file_name: launch.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        emu_dir: emu.program_path().and_then(Path::parent).map(s).unwrap_or_default(),
        platform: e.platform.clone(),
        emulator: emu.name.clone(),
        core: profile.core.clone(),
    };
    let lv: Vec<options::Level> = levels.iter().map(|(opts, w)| options::Level { options: opts, words: w }).collect();
    let decided: Vec<(options::Switch, bool)> =
        sws.iter().filter_map(|sw| options::decide(sw, &lv).map(|v| (sw.clone(), v))).collect();
    let fs_default = sws.iter().find(|s| s.id == options::FULLSCREEN).map_or(reg.config.fullscreen, |s| s.default);
    let fullscreen = o
        .fullscreen
        .unwrap_or_else(|| decided.iter().find(|(s, _)| s.id == options::FULLSCREEN).map_or(fs_default, |(_, v)| *v));
    // A forced fullscreen choice (Play in a window) decides that switch, typed words included.
    let decided: Vec<(options::Switch, bool)> = decided
        .into_iter()
        .filter(|(s, _)| s.id != options::FULLSCREEN)
        .chain(sws.iter().find(|s| s.id == options::FULLSCREEN).map(|s| (s.clone(), fullscreen)))
        .collect();
    let no_core = || format!("the RetroArch core for “{}” isn't installed", profile.name);
    let mut args: Vec<String> = vec![];
    let mut insert_at: Option<usize> = None;
    for w in &words {
        if w == "{args}" {
            insert_at.get_or_insert(args.len());
            continue;
        }
        if insert_at.is_none() && options::is_rom_word(w) {
            let at = args.len();
            insert_at = Some(if at > 0 && args[at - 1] == "--" { at - 1 } else { at });
        }
        if let Some(a) = fill(w, &vars, fullscreen).map_err(|_| no_core())? {
            args.push(a);
        }
    }
    let mut at = insert_at.unwrap_or(args.len());
    // Extra arguments: the emulator's, the console's, the game's, the action's.
    let mut extra: Vec<String> = vec![];
    for (_, ws) in levels.iter().rev() {
        for w in ws {
            if let Some(a) = fill(w, &vars, fullscreen).map_err(|_| no_core())? {
                extra.push(a);
            }
        }
    }
    options::apply(&mut args, &mut at, &mut extra, &decided);
    let at = at.min(args.len());
    args.splice(at..at, extra);
    let emu_dir = vars.emu_dir.clone();
    let expand_dirs = |s: &str| -> String {
        let mut out = s.trim().to_string();
        for t in ["{EmulatorDir}", "{EmuDir}", "{emu_dir}"] {
            out = out.replace(t, &emu_dir);
        }
        let install = o.game.and_then(|g| g.install_dir.as_ref()).map(|d| s_of(d)).unwrap_or_default();
        out = out.replace("{InstallDir}", &install);
        for t in ["{RomDir}", "{rom_dir}", "{ImageDir}"] {
            out = out.replace(t, &vars.rom_dir);
        }
        match (out.strip_prefix("~/"), dirs::home_dir()) {
            (Some(rest), Some(h)) => s_of(&h.join(rest)),
            _ => out,
        }
    };
    // A profile with its own program (a fork, a test build): that one runs, as it is.
    if let Some(own) = &profile.program {
        let program = expand_dirs(&own.to_string_lossy());
        if program.is_empty() {
            return Err(format!("the profile “{}” has no program", profile.name));
        }
        let cwd = match profile.cwd.trim() {
            "" => Path::new(&program).parent().filter(|p| p.is_absolute()).map(Path::to_path_buf),
            c => Some(PathBuf::from(expand_dirs(c))),
        };
        return Ok(LaunchSpec::Command { program, args, cwd, env: vec![] });
    }
    let cwd = match profile.cwd.trim() {
        // Like Playnite ({EmulatorDir}): the emulator's own folder, so portable set-ups that look
        // for their files in the folder they're started from (mGBA's portable.ini) keep working.
        "" => program_dir(&emu.kind),
        c => Some(PathBuf::from(expand_dirs(c))).filter(|p| !p.as_os_str().is_empty()),
    };
    let (program, args) = match &emu.kind {
        InstallKind::Flatpak { app_id } => {
            let mut a = vec!["run".to_string()];
            if let Some(dir) = needs_filesystem(reg, emu, profile, e) {
                a.push(format!("--filesystem={}", dir.display()));
            }
            if let Some(cmd) = super::defs::flatpak_command(&emu.def) {
                a.push(format!("--command={cmd}"));
            }
            a.push(app_id.clone());
            a.extend(args);
            ("flatpak".to_string(), a)
        }
        InstallKind::AppImage { path } | InstallKind::Binary { path } => (s(path), args),
    };
    Ok(LaunchSpec::Command { program, args, cwd, env: vec![] })
}

fn s_of(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// How Play follows an emulated game when its emulator profile says (Settings → Emulation →
/// profile → Tracking): (mode, folder for "Any process in the folder": the profile's own
/// program's, else the emulator's). None = the action's / the usual way.
pub fn profile_tracking(
    reg: &Registry,
    game: &Game,
    action: Option<&crate::model::GameAction>,
) -> Option<(crate::model::TrackingMode, Option<PathBuf>)> {
    let base = game.emu.as_ref()?;
    let (emu, prof) = match action {
        Some(a) if a.kind == crate::model::ActionKind::Emulator => action_choice(reg, game, a).ok()?,
        Some(_) => return None,
        None => choose_for(reg, Some(game), &super::roms::default_ref(game, base))?,
    };
    if prof.tracking == crate::model::TrackingMode::Default {
        return None;
    }
    let folder = match &prof.program {
        Some(p) => Path::new(p).parent().filter(|d| d.is_absolute()).map(Path::to_path_buf),
        None => emu.program_path().and_then(Path::parent).map(Path::to_path_buf),
    };
    Some((prof.tracking, folder))
}

/// [`build_spec`] with an explicit fullscreen choice ("Play in a window" = false).
#[cfg(test)]
pub fn build_spec_opts(
    reg: &Registry,
    emu: &Emulator,
    profile: &Profile,
    e: &EmuRef,
    fullscreen: bool,
) -> Option<LaunchSpec> {
    build(reg, emu, profile, e, &BuildOpts { fullscreen: Some(fullscreen), ..Default::default() }).ok()
}

/// The command for a ROM with a given emulator / profile (`flatpak run --filesystem=<dir> …`
/// when the sandbox can't see the ROM), with the switches of Settings → Emulation.
pub fn build_spec(reg: &Registry, emu: &Emulator, profile: &Profile, e: &EmuRef) -> Option<LaunchSpec> {
    build(reg, emu, profile, e, &BuildOpts::default()).ok()
}

/// A game's command from a given registry (`fullscreen` None = the switches / setting).
/// `e` is the scanned ROM; the game's default ROM (the first of its ROM list) is started.
pub fn spec_with(reg: &Registry, game: &Game, e: &EmuRef, fullscreen: Option<bool>) -> Option<LaunchSpec> {
    let e = &super::roms::default_ref(game, e);
    if unplayable_ref(e).is_some() {
        return None;
    }
    let (emu, prof) = choose_for(reg, Some(game), e)?;
    build(reg, &emu, &prof, e, &BuildOpts { fullscreen, game: Some(game), action: None }).ok()
}

/// The emulator + profile an emulator action starts: its own choice, else the game's.
pub fn action_choice(reg: &Registry, game: &Game, a: &crate::model::GameAction) -> Result<(Emulator, Profile), String> {
    let base = game.emu.as_ref().ok_or("this isn't an emulated game")?;
    let e = &super::roms::rom_ref(game, base, a.rom.as_deref())?;
    let some = |s: &Option<String>| s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    if let Some(id) = some(&a.emulator) {
        let Some(emu) = reg.emulator(&id) else {
            return Err(format!("the emulator it uses ({id}) isn't installed any more — pick another one"));
        };
        if emu.blocked {
            return Err(format!(
                "{} isn't allowed to run yet (Settings → Emulation → Allow it to run)",
                emu.label_in_text()
            ));
        }
        let folder = reg.config.rom_folders.get(&e.folder);
        return choose(reg, &e.platform, &e.rom, Some(&id), some(&a.profile).as_deref(), folder)
            .filter(|(x, _)| x.id == id)
            .ok_or_else(|| format!("{} has no profile for this console", emu.name));
    }
    let mut g = game.clone();
    if let Some(p) = some(&a.profile) {
        g.edits.emulator_profile = Some(p);
    }
    choose_for(reg, Some(&g), e)
        .ok_or_else(|| missing_reason_with(Some(reg), game).unwrap_or_else(|| "no emulator for it was found".into()))
}

/// An emulator action's command (Edit → Actions, type Emulator). Also a Program action with an
/// empty program on an emulated game: its arguments are the extra ones, the ROM (and its `--`)
/// it names is left out (Lodestar adds it) — unless it names another file, then its arguments
/// replace the profile's.
pub fn action_spec(
    reg: &Registry,
    game: &Game,
    a: &crate::model::GameAction,
    fullscreen: Option<bool>,
) -> Result<LaunchSpec, String> {
    let base = game.emu.as_ref().ok_or("this isn't an emulated game")?;
    let e = &super::roms::rom_ref(game, base, a.rom.as_deref())?;
    if let Some(why) = unplayable_ref(e) {
        return Err(why);
    }
    let a = if a.kind == crate::model::ActionKind::Emulator { a.clone() } else { program_as_emulator(e, a)? };
    let (emu, prof) = action_choice(reg, game, &a)?;
    build(reg, &emu, &prof, e, &BuildOpts { fullscreen, game: Some(game), action: Some(&a) })
}

/// [`action_spec`] against the published registry.
pub fn action_spec_now(game: &Game, a: &crate::model::GameAction) -> Result<LaunchSpec, String> {
    let reg = current().ok_or("emulators are still being looked for")?;
    action_spec(&reg, game, a, None)
}

/// A Program action without a program on an emulated game, as an emulator action.
fn program_as_emulator(e: &EmuRef, a: &crate::model::GameAction) -> Result<crate::model::GameAction, String> {
    let words = crate::launch::split_command(&a.args).map_err(|err| format!("arguments: {err}"))?;
    let home = dirs::home_dir();
    let norm = |w: &str| -> PathBuf {
        match (w.strip_prefix("~/"), &home) {
            (Some(rest), Some(h)) => h.join(rest),
            _ => PathBuf::from(w),
        }
    };
    let ours: Vec<PathBuf> = std::iter::once(e.rom.clone()).chain(e.files.iter().cloned()).collect();
    let is_ours = |w: &str| {
        ["{Rom}", "{rom}", "{ImagePath}"].contains(&w) || {
            let p = norm(w);
            ours.contains(&p) || (p.is_absolute() && ours.iter().any(|o| p.starts_with(o) && is_dir(o)))
        }
    };
    let mut kept: Vec<String> = vec![];
    let mut other_file = false;
    for w in &words {
        if is_ours(w) {
            if kept.last().is_some_and(|l| l == "--") {
                kept.pop();
            }
            continue;
        }
        if (w.starts_with('/') || w.starts_with("~/")) && !w.contains('{') {
            other_file = true;
        }
        kept.push(w.clone());
    }
    let mut out = crate::model::GameAction { kind: crate::model::ActionKind::Emulator, ..a.clone() };
    if other_file {
        // Another file: the user's arguments are the whole command.
        out.replace_args = true;
    } else {
        out.args = kept.iter().map(|w| crate::launch::shell_quote(w)).collect::<Vec<_>>().join(" ");
    }
    Ok(out)
}

fn spec_from_current(game: &Game, e: &EmuRef, fullscreen: Option<bool>) -> Option<LaunchSpec> {
    spec_with(&*current()?, game, e, fullscreen)
}

/// Play time: the game's command rebuilt from the published registry (None = keep
/// `game.launch`).
pub fn spec_for(game: &Game, e: &EmuRef) -> Option<LaunchSpec> {
    spec_from_current(game, e, None)
}

/// Play-time, windowed: like [`spec_for`] without the fullscreen words (Play ▾ → "Play in a window").
pub fn spec_windowed(game: &Game, e: &EmuRef) -> Option<LaunchSpec> {
    spec_from_current(game, e, Some(false))
}

/// Start the emulator itself with no game (its own settings / controller setup / firmware
/// install). RetroArch opens its menu.
pub fn open_spec(emu: &Emulator) -> LaunchSpec {
    let extra: Vec<String> = if emu.def == "retroarch" { vec!["--menu".into()] } else { vec![] };
    let (program, args) = match &emu.kind {
        InstallKind::Flatpak { app_id } => {
            let mut a = vec!["run".to_string(), app_id.clone()];
            a.extend(extra);
            ("flatpak".to_string(), a)
        }
        InstallKind::AppImage { path } | InstallKind::Binary { path } => (path.to_string_lossy().into_owned(), extra),
    };
    LaunchSpec::Command { program, args, cwd: program_dir(&emu.kind), env: vec![] }
}

/// The folder of an AppImage / program install (None for Flatpaks): emulators start there.
fn program_dir(kind: &InstallKind) -> Option<PathBuf> {
    match kind {
        InstallKind::AppImage { path } | InstallKind::Binary { path } => path
            .parent()
            .filter(|p| p.is_absolute())
            // System-wide programs (/usr/bin/retroarch …) aren't portable: leave them be.
            .filter(|p| {
                !["/usr", "/bin", "/sbin", "/nix/store", "/run/current-system", "/app"].iter().any(|s| p.starts_with(s))
            })
            .map(Path::to_path_buf),
        InstallKind::Flatpak { .. } => None,
    }
}

/// What to install for a console, in plain words: "PCSX2 (on Flathub: net.pcsx2.PCSX2)".
pub fn install_hint(platform: &str) -> Option<String> {
    let d = super::defs::suggest(platform)?;
    Some(match d.flathub {
        Some(id) => format!("{} (on Flathub: {id})", d.name),
        None => format!("{} ({})", d.name, d.website),
    })
}

/// Why an emulated game can't start, in plain words (None: not an emulated game, or an
/// emulator is ready for it).
pub fn missing_reason(game: &Game) -> Option<String> {
    missing_reason_with(current().as_deref(), game)
}

/// [`missing_reason`] against a given registry (None = nothing published yet).
pub fn missing_reason_with(reg: Option<&Registry>, game: &Game) -> Option<String> {
    let e = game.emu.as_ref()?;
    if let Some(why) = unplayable_ref(e) {
        return Some(why);
    }
    let console = super::platforms::by_id(&e.platform)
        .map(|p| super::console_label(p).to_string())
        .or_else(|| game.platform.clone())
        .unwrap_or_else(|| "this console".into());
    let setup = "set one up in Settings → Emulation";
    let Some(reg) = reg else {
        return Some(format!("No emulator for {console} yet — {setup}."));
    };
    if let Some(b) = blocked_choice(reg, game) {
        return Some(format!(
            "{} isn't allowed to run yet — allow it (Play asks, or Settings → Emulation → Allow it to run), or pick another emulator.",
            b.label_in_text()
        ));
    }
    if choose_for(reg, Some(game), e).is_some() {
        return None;
    }
    if let Some(chosen) = game.edits.emulator.as_deref().filter(|s| !s.is_empty())
        && reg.emulator(chosen).is_none()
    {
        return Some(format!(
            "The emulator chosen for this game ({chosen}) isn't installed any more — pick another in Edit → Installation or {setup}."
        ));
    }
    // Found, but not allowed to run yet (a downloaded AppImage without the executable bit).
    if let Some(b) = reg
        .emulators
        .iter()
        .find(|x| x.blocked && detect::profiles(x, &reg.config).iter().any(|p| p.supports(&e.platform)))
    {
        return Some(format!(
            "{} was found but isn't allowed to run yet — click “Allow it to run” next to it in Settings → Emulation.",
            b.name
        ));
    }
    Some(match install_hint(&e.platform) {
        Some(h) => format!("No emulator for {console} was found on this computer. Install {h}, or {setup}."),
        None => format!("No emulator for {console} was found on this computer — {setup}."),
    })
}

#[cfg(test)]
mod tests;
