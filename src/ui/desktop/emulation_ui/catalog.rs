//! What the emulation pages show, worked out from the published registry off the UI thread:
//! the emulators found with their profiles, which emulator each console gets automatically,
//! BIOS checks; plus the small pure helpers (labels, keys, folder roots) the pages share.
//!
//! Nothing here touches the filesystem on the UI thread: [`catalog`] and [`BiosJob`] run their
//! work on helper threads and hand back results the pages keep.

use crate::config::{EmulationConfig, EmulatorEntry, RomFolder};
use crate::emulation::bios::{self, BiosState, BiosStatus};
use crate::emulation::detect::{Emulator, InstallKind, Profile};
use crate::emulation::registry::{self, Registry};
use crate::emulation::{defs, platforms};
use crate::model::{Game, Source};
use crossbeam_channel::Receiver;
use parking_lot::Mutex;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// One emulator found, with the profiles it can use.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CatEmu {
    pub emu: Emulator,
    pub profiles: Vec<Profile>,
}

/// The emulators found and what they cover, for one published registry.
#[derive(Debug)]
pub(crate) struct Catalog {
    /// The registry this was built from (kept alive, so its address identifies it).
    pub reg: Arc<Registry>,
    pub emus: Vec<CatEmu>,
    /// Platform id → (emulator id, profile id) chosen when the user picked nothing for it.
    pub auto: BTreeMap<String, (String, String)>,
    /// Copies of the same emulator together (indexes into `emus`), in the order found.
    pub groups: Vec<Group>,
}

/// One emulator with its copies (an old AppImage next to the current one, a backup folder): the
/// same built-in emulator under the same name. Settings lists one row per group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Group {
    /// The copy the row stands for: the one a console uses, else the first that doesn't look
    /// like an old one ("_old", "backup", "test").
    pub main: usize,
    pub copies: Vec<usize>,
}

/// Sort emulators into [`Group`]s. `used`: ids a console starts its games with.
pub(crate) fn group(emus: &[CatEmu], used: &BTreeSet<String>) -> Vec<Group> {
    let key = |e: &Emulator| -> (String, String) {
        if e.def.is_empty() {
            (format!("custom:{}", e.id), String::new())
        } else {
            (e.def.clone(), e.name.trim().to_lowercase())
        }
    };
    let mut keys: Vec<(String, String)> = vec![];
    let mut members: Vec<Vec<usize>> = vec![];
    for (i, c) in emus.iter().enumerate() {
        let k = key(&c.emu);
        match keys.iter().position(|x| *x == k) {
            Some(g) => members[g].push(i),
            None => {
                keys.push(k);
                members.push(vec![i]);
            }
        }
    }
    members
        .into_iter()
        .map(|m| {
            let main = *m
                .iter()
                .min_by_key(|&&i| (!used.contains(&emus[i].emu.id), emus[i].emu.blocked, looks_old(&emus[i].emu), i))
                .expect("a group has members");
            Group { main, copies: m.into_iter().filter(|&i| i != main).collect() }
        })
        .collect()
}

/// A copy that looks like one kept aside ([`Emulator::looks_old`]).
pub(crate) fn looks_old(e: &Emulator) -> bool {
    e.looks_old()
}

impl Catalog {
    /// Gather everything from `reg` (calls the emulation API; may touch the filesystem).
    pub fn build(reg: Arc<Registry>) -> Catalog {
        let emus: Vec<CatEmu> = reg
            .emulators
            .iter()
            .map(|e| CatEmu { emu: e.clone(), profiles: crate::emulation::detect::profiles(e, &reg.config) })
            .collect();
        let covered: BTreeSet<String> =
            emus.iter().flat_map(|c| c.profiles.iter().flat_map(|p| p.platforms.iter().cloned())).collect();
        // "Automatic" = what Lodestar would pick without the user's per-console choice.
        let mut plain = (*reg).clone();
        let mut auto = BTreeMap::new();
        for p in covered {
            plain.config.platform_emulator.remove(&p);
            if let Some((e, pr)) = registry::choose(&plain, &p, Path::new(""), None, None, None) {
                auto.insert(p.clone(), (e.id, pr.id));
            }
            if let Some(v) = reg.config.platform_emulator.get(&p) {
                plain.config.platform_emulator.insert(p.clone(), v.clone());
            }
        }
        Catalog::from_parts_arc(reg, emus, auto)
    }

    /// Built from parts (tests, the screenshot demo).
    pub fn from_parts(reg: Registry, emus: Vec<CatEmu>, auto: BTreeMap<String, (String, String)>) -> Catalog {
        Catalog::from_parts_arc(Arc::new(reg), emus, auto)
    }

    fn from_parts_arc(reg: Arc<Registry>, emus: Vec<CatEmu>, auto: BTreeMap<String, (String, String)>) -> Catalog {
        let mut c = Catalog { reg, emus, auto, groups: vec![] };
        let used: BTreeSet<String> = c.covered().iter().filter_map(|p| c.uses(p)).map(|(e, _)| e).collect();
        c.groups = group(&c.emus, &used);
        c
    }

    pub fn emu(&self, id: &str) -> Option<&CatEmu> {
        self.emus.iter().find(|c| c.emu.id == id)
    }

    /// Is it the one its group's row stands for (not one of its other copies)?
    pub fn is_main(&self, id: &str) -> bool {
        self.groups.iter().any(|g| self.emus[g.main].emu.id == id)
    }

    /// An emulator's name in a list or a button: the name it was given ("DuckStation"), with its
    /// kind ("DuckStation — AppImage") when another row has the same name, or its folder
    /// ("DuckStation — Duckstation.backup") when it's one of the other copies of an emulator.
    pub fn name_of(&self, e: &Emulator) -> String {
        if !self.is_main(&e.id) && self.emu(&e.id).is_some() {
            return e.copy_label();
        }
        e.name_among(self.groups.iter().map(|g| &self.emus[g.main].emu))
    }

    /// What a console's games start with: the user's pick (when it's still there), else the
    /// automatic one.
    pub fn uses(&self, platform: &str) -> Option<(String, String)> {
        if let Some(v) = self.reg.config.platform_emulator.get(platform) {
            let (e, p) = split_choice(v);
            if let Some(c) = self.emu(e) {
                let prof = c
                    .profiles
                    .iter()
                    .find(|x| x.id == p)
                    .or_else(|| c.profiles.iter().find(|x| platform_matches(x, platform)))
                    .map(|x| x.id.clone())
                    .unwrap_or_default();
                return Some((e.to_string(), prof));
            }
        }
        self.auto.get(platform).cloned()
    }

    /// The (emulator, profile) choices for a console: each emulator row's profiles for it (not
    /// the other copies), plus the user's pick when it is one of those.
    pub fn choices(&self, platform: &str) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = vec![];
        for g in &self.groups {
            let c = &self.emus[g.main];
            for p in c.profiles.iter().filter(|p| platform_matches(p, platform)) {
                v.push((c.emu.id.clone(), p.id.clone()));
            }
        }
        if let Some(u) = self.uses(platform)
            && !v.contains(&u)
        {
            v.push(u);
        }
        v
    }

    /// "Eden (automatic)": a choice's label in a console's picker.
    pub fn automatic_label(label: &str) -> String {
        format!("{label} (automatic)")
    }

    /// The picker's text when room is short: "DuckStation (installed)" for "DuckStation —
    /// installed program" (the full text is the hover text).
    pub fn short_label(label: &str) -> String {
        label.replace("installed program", "installed")
    }

    /// [`Catalog::name_of`] by id (the id itself when it isn't found).
    pub fn name_by_id(&self, id: &str) -> String {
        self.emu(id).map_or_else(|| id.to_string(), |c| self.name_of(&c.emu))
    }

    /// (emulator, profile) pairs that run `platform`, in the order found.
    pub fn compatible(&self, platform: &str) -> Vec<(&CatEmu, &Profile)> {
        let mut v = vec![];
        for c in &self.emus {
            for p in &c.profiles {
                if p.platforms.iter().any(|x| x == platform) {
                    v.push((c, p));
                }
            }
        }
        v
    }

    /// Emulators (once each) that run `platform`.
    pub fn emulators_for(&self, platform: &str) -> Vec<&CatEmu> {
        self.emus.iter().filter(|c| c.profiles.iter().any(|p| p.platforms.iter().any(|x| x == platform))).collect()
    }

    /// Every platform some emulator runs.
    pub fn covered(&self) -> BTreeSet<String> {
        self.emus.iter().flat_map(|c| c.profiles.iter().flat_map(|p| p.platforms.iter().cloned())).collect()
    }

    /// "PCSX2" or "RetroArch — Beetle PSX" (the profile only when the emulator has several for
    /// this console).
    pub fn choice_label(&self, emu: &str, profile: &str, platform: &str) -> Option<String> {
        let c = self.emu(emu)?;
        let n = c.profiles.iter().filter(|p| p.platforms.iter().any(|x| x == platform)).count();
        let p = c.profiles.iter().find(|p| p.id == profile);
        let name = self.name_of(&c.emu);
        // With the profile too: "RetroArch (Flatpak) — Beetle PSX".
        let with_profile = |prof: &str| match name.split_once(" — ") {
            Some((n, kind)) => format!("{n} ({kind}) — {prof}"),
            None => format!("{name} — {prof}"),
        };
        Some(match p {
            Some(p) if n > 1 || !platform_matches(p, platform) => with_profile(&p.name),
            Some(_) => name,
            None if profile.is_empty() => name,
            None => with_profile(profile),
        })
    }

    /// The emulator a game starts with now (its own choice > its folder's > the console's >
    /// automatic), else the one the scan picked.
    pub fn game_emulator(&self, g: &Game) -> Option<(Emulator, Option<Profile>)> {
        let e = g.emu.as_ref()?;
        let folder = self.reg.config.rom_folders.get(&e.folder);
        if let Some((emu, p)) = registry::choose(
            &self.reg,
            &e.platform,
            &e.rom,
            g.edits.emulator.as_deref().filter(|s| !s.is_empty()),
            g.edits.emulator_profile.as_deref().filter(|s| !s.is_empty()),
            folder,
        ) {
            return Some((emu, Some(p)));
        }
        let want = g.edits.emulator.as_deref().filter(|s| !s.is_empty()).unwrap_or(&e.emulator);
        self.emu(want).map(|c| (c.emu.clone(), c.profiles.iter().find(|p| p.id == e.profile).cloned()))
    }
}

fn platform_matches(p: &Profile, platform: &str) -> bool {
    p.platforms.iter().any(|x| x == platform)
}

/// "PS1, PS2, PSP +3".
pub(crate) fn join_limited(names: &[String], max: usize) -> String {
    if names.len() <= max {
        return names.join(", ");
    }
    format!("{} +{}", names[..max].join(", "), names.len() - max)
}

/// A console's short name ("PS2"), or the id when unknown.
pub(crate) fn short_name(id: &str) -> String {
    platforms::by_id(id).map_or_else(|| id.to_string(), |p| p.short.to_string())
}

/// A console's name in running text ("PlayStation 2", "GameCube", "Nintendo 64"), or the id
/// when unknown: the same form the game page and controller mode use, so one console doesn't
/// appear as "PS2", "PlayStation 2" and "Sony PlayStation 2" across the app. The short form
/// ([`short_name`]) is for chips and lists where space is tight.
pub(crate) fn console_name(id: &str) -> String {
    crate::ui::platform_icons::console_name(id)
}

// ------------------------------------------------------------------------- catalog cache

struct Cache {
    cat: Option<Arc<Catalog>>,
    /// Registry address being built now.
    pending: Option<(usize, Receiver<Arc<Catalog>>)>,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache { cat: None, pending: None });

fn addr(reg: &Arc<Registry>) -> usize {
    Arc::as_ptr(reg) as usize
}

/// The catalog for the published registry (None while nothing is published yet). When a new
/// registry is published the previous catalog is returned until the new one is built on a
/// helper thread (repaints when ready).
pub(crate) fn catalog(ctx: &egui::Context) -> Option<Arc<Catalog>> {
    let reg = registry::current();
    let mut c = CACHE.lock();
    if let Some((_, rx)) = &c.pending
        && let Ok(cat) = rx.try_recv()
    {
        c.cat = Some(cat);
        c.pending = None;
    }
    let Some(reg) = reg else {
        return c.cat.clone();
    };
    let a = addr(&reg);
    let fresh = c.cat.as_ref().is_some_and(|cat| addr(&cat.reg) == a);
    let building = c.pending.as_ref().is_some_and(|(p, _)| *p == a);
    if !fresh && !building {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("emulation-catalog".into()).spawn(move || {
            let _ = tx.send(Arc::new(Catalog::build(reg)));
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => c.pending = Some((a, rx)),
            Err(e) => log::warn!("emulation catalog thread: {e}"),
        }
    }
    c.cat.clone()
}

// ------------------------------------------------------------------------- BIOS checks

/// One console's BIOS check plus, per emulator, the files that would replace a different one.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BiosInfo {
    pub status: BiosStatus,
    /// Emulator id → target files that already exist (and differ: they are listed as missing).
    pub conflicts: BTreeMap<String, Vec<PathBuf>>,
    /// What "Set up everything" would do (worked out here: it looks at the disk).
    pub plan: Vec<bios::Step>,
}

/// Results of one BIOS job.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct BiosData {
    pub by_platform: BTreeMap<String, BiosInfo>,
    /// Folders searched (chosen + found automatically).
    pub search_dirs: Vec<PathBuf>,
    /// The ones that are an emulator's own BIOS folder: folder → the emulator's name.
    pub own: BTreeMap<PathBuf, String>,
}

/// Consoles whose BIOS / keys / firmware are worth checking.
pub(crate) fn wants_check(platform: &str) -> bool {
    bios::needs_bios(platform) || is_firmware_console(platform)
}

/// Switch and PS3: keys / firmware are installed through the emulator.
pub(crate) fn is_firmware_console(platform: &str) -> bool {
    matches!(platform, "nintendo_switch" | "sony_playstation3")
}

/// Check these consoles (MD5s, file listings: call off the UI thread).
pub(crate) fn check_bios(reg: &Registry, consoles: &[String]) -> BiosData {
    let dirs = bios::search_dirs_with_origin(reg);
    let own = dirs
        .iter()
        .filter_map(|d| match &d.origin {
            bios::Origin::Emulator(id) => {
                reg.emulator(id).map(|e| (d.path.clone(), e.name_among(reg.emulators.iter())))
            }
            _ => None,
        })
        .collect();
    let mut out = BiosData { search_dirs: dirs.into_iter().map(|d| d.path).collect(), own, ..Default::default() };
    for p in consoles.iter().filter(|p| wants_check(p)) {
        let status = bios::check(reg, p);
        let mut conflicts = BTreeMap::new();
        for t in &status.targets {
            let there: Vec<PathBuf> = t
                .missing
                .iter()
                .filter_map(|src| bios::dest_for(reg, &status, &t.emulator, src))
                .filter(|dst| dst.exists())
                .collect();
            if !there.is_empty() {
                conflicts.insert(t.emulator.clone(), there);
            }
        }
        let prefer = reg.config.platform_emulator.get(p).map(|v| split_choice(v).0.to_string());
        let plan = bios::setup_plan(reg, &status, prefer.as_deref());
        out.by_platform.insert(p.clone(), BiosInfo { status, conflicts, plan });
    }
    // Shown: the folders the user chose, and the found ones that really hold a BIOS, keys or
    // firmware Lodestar recognised (not every folder whose name says "bios").
    let held: Vec<PathBuf> = out
        .by_platform
        .values()
        .flat_map(|i| {
            i.status
                .files
                .iter()
                .filter_map(|(_, _, p)| p.clone())
                .chain(i.status.packages.iter().map(|k| k.path.clone()))
        })
        .map(|p| bios::tidy(&p))
        .collect();
    let chosen: Vec<PathBuf> = reg.config.bios_dirs.iter().map(|d| bios::tidy(d)).collect();
    out.search_dirs.retain(|d| chosen.contains(d) || held.iter().any(|f| f.starts_with(d)));
    out
}

/// (registry address, consoles) a BIOS job ran for.
type JobKey = (usize, Vec<String>);

/// A background BIOS check, re-run when the registry or the consoles change.
#[derive(Default)]
pub(crate) struct BiosJob {
    pub data: Option<BiosData>,
    key: Option<(usize, Vec<String>)>,
    rx: Option<(Receiver<BiosData>, JobKey)>,
}

impl BiosJob {
    /// Poll / start. `force`: check again even when nothing changed (after copying files).
    pub fn update(&mut self, ctx: &egui::Context, reg: &Arc<Registry>, consoles: &[String], force: bool) {
        if let Some((rx, key)) = &self.rx
            && let Ok(d) = rx.try_recv()
        {
            self.data = Some(d);
            self.key = Some(key.clone());
            self.rx = None;
        }
        let key = (addr(reg), consoles.to_vec());
        let running = self.rx.as_ref().is_some_and(|(_, k)| *k == key);
        if (self.key.as_ref() == Some(&key) && !force) || running {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        let reg = reg.clone();
        let list = consoles.to_vec();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("emulation-bios".into()).spawn(move || {
            let _ = tx.send(check_bios(&reg, &list));
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => self.rx = Some((rx, key)),
            Err(e) => log::warn!("bios check thread: {e}"),
        }
    }

    pub fn running(&self) -> bool {
        self.rx.is_some()
    }

    /// Show this instead of checking (screenshot demo, tests).
    pub fn fixed(data: BiosData) -> BiosJob {
        BiosJob { data: Some(data), key: Some((usize::MAX, vec![])), rx: None }
    }

    pub fn is_fixed(&self) -> bool {
        self.key.as_ref().is_some_and(|(a, _)| *a == usize::MAX)
    }
}

/// The chip text for a console's BIOS state (None: nothing to say). Switch says "Keys", PS3
/// "Firmware".
pub(crate) fn bios_chip(platform: &str, state: BiosState) -> Option<(String, ChipTone)> {
    let what = match platform {
        "nintendo_switch" => "Keys",
        "sony_playstation3" => "Firmware",
        _ => "BIOS",
    };
    match state {
        BiosState::Ok => Some((format!("{what} OK"), ChipTone::Good)),
        BiosState::Missing => Some((format!("{what} missing"), ChipTone::Bad)),
        BiosState::Wrong if what == "BIOS" => Some(("Wrong BIOS file".into(), ChipTone::Warn)),
        BiosState::Wrong => Some((format!("{what}: wrong file"), ChipTone::Warn)),
        BiosState::Optional => Some((format!("{what} optional"), ChipTone::Dim)),
        BiosState::NotNeeded => None,
    }
}

/// The chip for a console's whole check: like [`bios_chip`], but files that were found and only
/// need copying / installing into the emulator say so (amber) instead of "missing" / "OK".
pub(crate) fn bios_chip_for(s: &BiosStatus) -> Option<(String, ChipTone)> {
    let pf = s.platform.as_str();
    // An emulator already has everything in its own folders.
    if s.targets.iter().any(|t| t.ready) {
        let text = match pf {
            "nintendo_switch" => "Keys & firmware OK",
            "sony_playstation3" => "Firmware OK",
            _ => "BIOS OK",
        };
        return Some((text.into(), ChipTone::Good));
    }
    // No emulator has the found files / firmware yet.
    let not_copied = !s.targets.is_empty() && s.targets.iter().all(|t| !t.missing.is_empty());
    let not_installed =
        s.packages.iter().any(|k| !k.targets.is_empty() && k.targets.iter().all(|t| t.installed.is_none()));
    // Missing things that are not covered by a found package.
    let really_missing = s.files.iter().any(|(b, st, _)| {
        b.required
            && *st == BiosState::Missing
            && !((b.file == "firmware" || b.file == "PS3UPDAT.PUP") && !s.packages.is_empty())
            && b.group.is_none_or(|g| !s.files.iter().any(|(x, xs, _)| x.group == Some(g) && *xs == BiosState::Ok))
    });
    if s.state == BiosState::Wrong || really_missing {
        return bios_chip(pf, s.state);
    }
    match pf {
        "nintendo_switch" if not_copied || not_installed => Some(("Not set up yet".into(), ChipTone::Warn)),
        "sony_playstation3" if not_installed => Some(("Firmware not installed".into(), ChipTone::Warn)),
        _ if s.state == BiosState::Ok && not_copied => Some(("BIOS not copied".into(), ChipTone::Warn)),
        "nintendo_switch" if s.state == BiosState::Ok => Some(("Keys & firmware OK".into(), ChipTone::Good)),
        _ => bios_chip(pf, s.state),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChipTone {
    Good,
    Bad,
    Warn,
    Dim,
}

pub(crate) fn state_word(s: BiosState) -> &'static str {
    match s {
        BiosState::Ok => "OK",
        BiosState::Missing => "Missing",
        BiosState::Wrong => "Wrong file",
        BiosState::Optional => "Optional",
        BiosState::NotNeeded => "Not needed",
    }
}

// ------------------------------------------------------------------------- library facts

/// Per console and per ROM folder: how many games the library has (visible ones).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct LibFacts {
    pub generation: u64,
    /// Platform id → games.
    pub per_platform: BTreeMap<String, usize>,
    /// ROM folder id → games.
    pub per_folder: BTreeMap<String, usize>,
    /// ROM folder id → its consoles.
    pub folder_platforms: BTreeMap<String, BTreeSet<String>>,
    /// Folders found automatically: id → root (worked out from the games' paths).
    pub auto_roots: BTreeMap<String, PathBuf>,
}

impl LibFacts {
    pub fn compute(games: &[Game], generation: u64) -> LibFacts {
        let mut f = LibFacts { generation, ..Default::default() };
        for g in games.iter().filter(|g| g.source == Source::Emulator && !g.hidden) {
            let Some(e) = &g.emu else { continue };
            if !e.platform.is_empty() {
                *f.per_platform.entry(e.platform.clone()).or_default() += 1;
            }
            *f.per_folder.entry(e.folder.clone()).or_default() += 1;
            f.folder_platforms.entry(e.folder.clone()).or_default().insert(e.platform.clone());
            if e.folder.starts_with("auto-")
                && !f.auto_roots.contains_key(&e.folder)
                && let Some(root) = root_of(&g.source_id, &e.rom)
            {
                f.auto_roots.insert(e.folder.clone(), root);
            }
        }
        f
    }
}

/// The ROM folder a game was found in: its ROM path minus the path relative to the folder
/// (`source_id` = `<folder id>/<relative path>`).
pub(crate) fn root_of(source_id: &str, rom: &Path) -> Option<PathBuf> {
    let (_, rel) = source_id.split_once('/')?;
    let n = Path::new(rel).components().count();
    if n == 0 || !rom.ends_with(rel) {
        return None;
    }
    let mut root = rom.to_path_buf();
    for _ in 0..n {
        root = root.parent()?.to_path_buf();
    }
    Some(root)
}

// ------------------------------------------------------------------------- config helpers

/// The next free ROM folder key ("f1", "f2", …).
pub(crate) fn next_folder_key(folders: &BTreeMap<String, RomFolder>) -> String {
    (1..).map(|n| format!("f{n}")).find(|k| !folders.contains_key(k)).unwrap_or_else(|| "f0".into())
}

/// A key for a new emulator entry: `base` if free, else `base-2`, `base-3` …
pub(crate) fn new_emulator_key(base: &str, cfg: &EmulationConfig, found: &[String]) -> String {
    let base = if base.is_empty() { "custom" } else { base };
    let taken = |k: &str| cfg.emulators.contains_key(k) || found.iter().any(|f| f == k);
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|n| format!("{base}-{n}")).find(|k| !taken(k)).unwrap_or_else(|| base.to_string())
}

/// A key for a new user profile of `emu` ("<emu>/custom", "<emu>/custom-2" …).
pub(crate) fn new_profile_key(emu: &str, cfg: &EmulationConfig) -> String {
    let base = format!("{emu}/custom");
    if !cfg.profiles.contains_key(&base) {
        return base;
    }
    (2..).map(|n| format!("{base}-{n}")).find(|k| !cfg.profiles.contains_key(k)).unwrap_or(base)
}

/// How a picked program runs: "appimage" for `*.AppImage`, else "binary".
pub(crate) fn program_kind(path: &Path) -> &'static str {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ext == "appimage" { "appimage" } else { "binary" }
}

/// The config entry that describes `e` as it is installed now (to override one setting of an
/// emulator that was found by itself).
pub(crate) fn entry_for(e: &Emulator, cfg: &EmulationConfig) -> EmulatorEntry {
    if let Some(x) = cfg.emulators.get(&e.id) {
        return x.clone();
    }
    let mut x =
        EmulatorEntry { name: e.name.clone(), follows: e.def.clone(), args: e.args.clone(), ..Default::default() };
    set_program(&mut x, &e.kind);
    x
}

pub(crate) fn set_program(x: &mut EmulatorEntry, kind: &InstallKind) {
    match kind {
        InstallKind::Flatpak { app_id } => {
            x.kind = "flatpak".into();
            x.app_id = app_id.clone();
            x.path = PathBuf::new();
        }
        InstallKind::AppImage { path } => {
            x.kind = "appimage".into();
            x.app_id.clear();
            x.path = path.clone();
        }
        InstallKind::Binary { path } => {
            x.kind = "binary".into();
            x.app_id.clear();
            x.path = path.clone();
        }
    }
}

/// A picked program file or Flatpak as an install kind.
pub(crate) fn picked_kind(p: &Picked) -> InstallKind {
    match p {
        Picked::Flatpak { id, .. } => InstallKind::Flatpak { app_id: id.clone() },
        Picked::File(path) if program_kind(path) == "appimage" => InstallKind::AppImage { path: path.clone() },
        Picked::File(path) => InstallKind::Binary { path: path.clone() },
    }
}

/// A program the user picked for an emulator.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Picked {
    File(PathBuf),
    Flatpak { id: String, name: String },
}

impl Picked {
    pub fn label(&self) -> String {
        match self {
            Picked::File(p) => p.display().to_string(),
            Picked::Flatpak { id, name } => format!("{name} (Flatpak {id})"),
        }
    }

    /// The built-in emulator this program is (Ryujinx for "Ryujinx-1.2.3-x64.AppImage").
    pub fn identify(&self) -> Option<&'static defs::EmulatorDef> {
        crate::emulation::detect::identify(&picked_kind(self))
    }

    /// A name for the new emulator: the emulator the program is ("Ryujinx"), else the Flatpak's
    /// name, else the file's ("pcsx2-v2.2.0-linux-appimage-x64-Qt" → "pcsx2").
    pub fn name_guess(&self) -> String {
        if let Some(d) = self.identify() {
            return d.name.to_string();
        }
        match self {
            Picked::Flatpak { name, .. } => name.clone(),
            Picked::File(_) => crate::emulation::detect::name_from_program(&picked_kind(self)),
        }
    }
}

/// The built-in emulator to suggest for a console with none installed (one on Flathub first).
pub(crate) fn suggested_def(platform: &str) -> Option<&'static defs::EmulatorDef> {
    let runs = |d: &&defs::EmulatorDef| d.profiles.iter().any(|p| p.platforms.contains(&platform));
    defs::all().iter().filter(runs).find(|d| d.flathub.is_some()).or_else(|| defs::all().iter().find(runs))
}

/// Flathub's page for an app id (a full URL is kept).
pub(crate) fn flathub_url(id: &str) -> String {
    if id.starts_with("http://") || id.starts_with("https://") {
        id.to_string()
    } else {
        format!("https://flathub.org/apps/{id}")
    }
}

/// Exclusion patterns (paths relative to `root`, `/`-separated) for games the user unticked:
/// each game's ROM and its other files.
pub(crate) fn exclusions_for<'a>(
    root: &Path,
    unticked: impl IntoIterator<Item = &'a crate::model::EmuRef>,
) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for e in unticked {
        for p in std::iter::once(&e.rom).chain(e.files.iter()) {
            if let Ok(rel) = p.strip_prefix(root) {
                let s = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
                if !s.is_empty() && !out.contains(&s) {
                    out.push(s);
                }
            }
        }
    }
    out
}

/// Words of a comma / space separated list, lower-case, without dots ("ISO, .chd" → iso chd).
pub(crate) fn parse_extensions(s: &str) -> Vec<String> {
    let mut v: Vec<String> = vec![];
    for w in s.split([',', ' ', ';']).map(|w| w.trim().trim_start_matches('.').to_lowercase()) {
        if !w.is_empty() && !v.contains(&w) {
            v.push(w);
        }
    }
    v
}

/// The "<emulator>/<profile>" value a per-console choice is saved as.
pub(crate) fn choice_value(emu: &str, profile: &str) -> String {
    format!("{emu}/{profile}")
}

/// Split a saved choice ("pcsx2/default" → ("pcsx2", "default"); no `/` → whole emulator).
pub(crate) fn split_choice(v: &str) -> (&str, &str) {
    v.split_once('/').unwrap_or((v, ""))
}

/// Size in plain words (the details panel's format).
pub(crate) fn size_text(bytes: u64) -> String {
    super::super::details::format_size(bytes)
}
