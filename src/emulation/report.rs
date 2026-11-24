//! `lodestar --emulation-report`: emulators found and how, ROM folders and their consoles, which
//! emulator each console would use (with the exact command for an example game, including the
//! Flatpak access it needs) and the BIOS state. Read-only: it never writes anything.

use super::bios::{self, BiosState};
use super::detect::{self, InstallKind};
use super::platforms::{self, Platform};
use super::registry::{self, Registry};
use crate::model::{EmuRef, LaunchSpec};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// A ROM folder for the report: its path and the consoles in it (sub-folder name → platform).
struct Root {
    id: String,
    path: PathBuf,
    from: &'static str,
    consoles: Vec<(PathBuf, Option<&'static Platform>, String)>,
}

fn subdirs(p: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(p)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
        .collect();
    v.sort();
    v
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// A folder whose sub-folders are consoles (≥ 2 of them map to a platform).
fn console_root(p: &Path) -> Option<Vec<(PathBuf, Option<&'static Platform>, String)>> {
    let subs = subdirs(p);
    let mapped: Vec<_> = subs.iter().map(|s| (s.clone(), platforms::by_folder_name(&name_of(s)), name_of(s))).collect();
    (mapped.iter().filter(|(_, p, _)| p.is_some()).count() >= 2).then_some(mapped)
}

/// ROM folders: the configured ones, then (when finding folders automatically) a quick look at
/// the usual places, `/mnt/*`, `/mnt/*/*`, `/run/media/$USER/*(/*)` and `/media/*(/*)`.
fn rom_roots(ctx: &crate::scan::ScanContext) -> Vec<Root> {
    let cfg = &ctx.emulation;
    let mut out: Vec<Root> = vec![];
    for (id, f) in &cfg.rom_folders {
        if !f.enabled {
            continue;
        }
        let consoles = match platforms::by_id(f.platform.trim()) {
            Some(p) => vec![(f.path.clone(), Some(p), name_of(&f.path))],
            None => match platforms::by_folder_name(&name_of(&f.path)) {
                Some(p) => vec![(f.path.clone(), Some(p), name_of(&f.path))],
                None => console_root(&f.path).unwrap_or_default(),
            },
        };
        out.push(Root { id: id.clone(), path: f.path.clone(), from: "set up in Settings", consoles });
    }
    if !cfg.auto_folders {
        return out;
    }
    let h = &ctx.home;
    let mut cands: Vec<PathBuf> =
        ["ROMs", "roms", "Emulation/roms", "RetroPie/roms", "retrodeck/roms"].iter().map(|d| h.join(d)).collect();
    let user = h.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    for base in [PathBuf::from("/mnt"), Path::new("/run/media").join(&user), PathBuf::from("/media")] {
        for d in subdirs(&base) {
            cands.push(d.clone());
            cands.extend(subdirs(&d));
        }
    }
    for c in cands {
        if out.iter().any(|r| c.starts_with(&r.path)) || cfg.ignored_folders.contains(&c) {
            continue;
        }
        if let Some(consoles) = console_root(&c) {
            let id = format!("report-{}", out.len());
            out.push(Root { id, path: c, from: "found automatically", consoles });
        }
    }
    out
}

/// An example game in a console folder that the profile opens (for the "Will run" line).
fn example_rom(dir: &Path, prof: &detect::Profile, p: &Platform) -> PathBuf {
    let entries: Vec<PathBuf> = {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
        v.sort();
        v
    };
    if p.folder_games
        && let Some(d) = entries.iter().find(|e| {
            e.is_dir()
                && !name_of(e).contains("(Update)")
                && !name_of(e).contains("(DLC)")
                && !name_of(e).starts_with(['$', '＄'])
        })
    {
        return d.clone();
    }
    for ext in &prof.extensions {
        let mut all = entries.clone();
        for e in &entries {
            if e.is_dir() && !name_of(e).eq_ignore_ascii_case("bios") {
                all.extend(std::fs::read_dir(e).into_iter().flatten().flatten().map(|x| x.path()));
            }
        }
        all.sort();
        if let Some(f) = all.iter().find(|f| f.is_file() && f.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)))
        {
            return f.clone();
        }
    }
    if p.folder_games {
        return dir.join("Game");
    }
    dir.join(format!("Game.{}", prof.extensions.first().map(String::as_str).unwrap_or("rom")))
}

fn shell_line(spec: &LaunchSpec) -> String {
    match spec {
        LaunchSpec::Command { program, args, .. } => {
            let q = |s: &str| {
                if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:,+@%".contains(c)) {
                    s.to_string()
                } else {
                    format!("'{}'", s.replace('\'', "'\\''"))
                }
            };
            std::iter::once(q(program)).chain(args.iter().map(|a| q(a))).collect::<Vec<_>>().join(" ")
        }
        LaunchSpec::Url(u) => u.clone(),
        LaunchSpec::Steam { appid } => format!("steam://rungameid/{appid}"),
    }
}

fn yes(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

pub fn report(ctx: &crate::scan::ScanContext) -> String {
    let cfg = &ctx.emulation;
    let mut out = String::new();
    let _ = writeln!(out, "Lodestar emulation report");
    let _ = writeln!(
        out,
        "  find emulators automatically: {} · find ROM folders automatically: {} · start games fullscreen: {}",
        yes(cfg.auto_emulators),
        yes(cfg.auto_folders),
        yes(cfg.fullscreen)
    );

    let roots = rom_roots(ctx);
    registry::note_roots(&roots.iter().map(|r| (r.id.clone(), r.path.clone())).collect::<Vec<_>>());
    let emulators = detect::detect(ctx);
    let reg = Registry { emulators, config: cfg.clone(), home: ctx.home.clone() };

    emulators_section(&mut out, &reg);
    rom_folders_section(&mut out, &roots);
    let consoles = consoles_in(&roots);
    let _ = writeln!(out, "\nConsoles: {}", consoles.len());
    for p in consoles {
        let _ = writeln!(out, "  {} ({})", p.name, p.id);
        let dirs: Vec<(&Root, &PathBuf)> = roots
            .iter()
            .flat_map(|r| {
                r.consoles.iter().filter(|(_, x, _)| x.is_some_and(|x| x.id == p.id)).map(move |(d, _, _)| (r, d))
            })
            .collect();
        for (_, d) in &dirs {
            let _ = writeln!(out, "    ROMs: {}", d.display());
        }
        emulator_lines(&mut out, &reg, p, &dirs);
        bios_lines(&mut out, &reg, p);
    }
    out
}

/// "Emulators found": each install, how it was found, its sandbox access and its own files.
fn emulators_section(out: &mut String, reg: &Registry) {
    let _ = writeln!(out, "\nEmulators found: {}", reg.emulators.len());
    for e in &reg.emulators {
        let what = match &e.kind {
            InstallKind::Flatpak { app_id } => format!("flatpak {app_id}"),
            InstallKind::AppImage { path } | InstallKind::Binary { path } => path.display().to_string(),
        };
        let _ = writeln!(out, "  {:<20} {:<32} {} ({})", e.id, e.label(), what, e.how);
        if e.blocked {
            let _ = writeln!(
                out,
                "  {:<16} not allowed to run yet (no executable bit): never chosen automatically; Settings → Emulation → Allow it to run",
                ""
            );
        }
        if let Some(app) = e.flatpak_id() {
            let fs = detect::flatpak_access(&reg.home, app);
            let fs = if fs.is_empty() { "nothing outside its own folders".to_string() } else { fs.join(", ") };
            let _ = writeln!(out, "  {:<16} sandbox can see: {fs}", "");
        }
        if e.def == "retroarch" {
            let _ = writeln!(out, "  {:<16} cores: {}", "", e.cores.len());
        }
        if let Some(u) = super::roots::user_dir(e, &reg.home) {
            let there = if u.path.is_dir() { "" } else { ", not created yet" };
            let _ = writeln!(out, "  {:<16} its files: {} ({}{there})", "", u.path.display(), u.how.label());
        }
    }
    if reg.emulators.is_empty() {
        let _ = writeln!(out, "  (none)");
    }
}

/// "ROM folders": each folder and the console of each sub-folder.
fn rom_folders_section(out: &mut String, roots: &[Root]) {
    let _ = writeln!(out, "\nROM folders: {}", roots.len());
    for r in roots {
        let _ = writeln!(out, "  {} ({})", r.path.display(), r.from);
        for (dir, p, name) in &r.consoles {
            let what = match p {
                Some(p) => p.name.to_string(),
                None if name.eq_ignore_ascii_case("pc") => "not a console (PC games)".into(),
                None => "not a console".into(),
            };
            let _ = writeln!(out, "    {:<14} {what}", if dir == &r.path { "." } else { name.as_str() });
        }
    }
}

/// The consoles of the ROM folders, once each, by name.
fn consoles_in(roots: &[Root]) -> Vec<&'static Platform> {
    let mut ids: Vec<&'static Platform> = vec![];
    for r in roots {
        for (_, p, _) in &r.consoles {
            if let Some(p) = p
                && !ids.iter().any(|x| x.id == p.id)
            {
                ids.push(p);
            }
        }
    }
    ids.sort_by_key(|p| p.name);
    ids
}

/// The emulator a console would use, and per ROM folder its sandbox access and the command an
/// example game would run.
fn emulator_lines(out: &mut String, reg: &Registry, p: &'static Platform, dirs: &[(&Root, &PathBuf)]) {
    let probe = dirs.first().map(|(_, d)| d.join("game")).unwrap_or_default();
    let Some((emu, prof)) = registry::choose(reg, p.id, &probe, None, None, None) else {
        let hint = registry::install_hint(p.id).map(|h| format!(" — install {h}")).unwrap_or_default();
        let _ = writeln!(out, "    emulator: none found{hint}");
        return;
    };
    let _ = writeln!(out, "    emulator: {} ({}, profile {})", emu.label(), emu.id, prof.id);
    for (root, d) in dirs {
        let rom = example_rom(d, &prof, p);
        let e = EmuRef {
            platform: p.id.to_string(),
            folder: root.id.clone(),
            rom,
            emulator: emu.id.clone(),
            profile: prof.id.clone(),
            ..Default::default()
        };
        if emu.flatpak_id().is_some() {
            match registry::needs_filesystem(reg, &emu, &prof, &e) {
                Some(dir) => {
                    let _ =
                        writeln!(out, "    sandbox: can't see {} → adds --filesystem={}", d.display(), dir.display());
                }
                None => {
                    let _ = writeln!(out, "    sandbox: already sees {}", d.display());
                }
            }
        }
        if let Some(why) = registry::unplayable_ref(&e) {
            let _ = writeln!(out, "    can't start {}: {why}", e.rom.display());
        } else if let Some(spec) = registry::build_spec(reg, &emu, &prof, &e) {
            let _ = writeln!(out, "    will run: {}", shell_line(&spec));
        }
    }
}

/// A console's BIOS / firmware: its state and the files found, the firmware packages, what each
/// emulator has or lacks, and what "Set up everything" would do.
fn bios_lines(out: &mut String, reg: &Registry, p: &'static Platform) {
    let st = bios::check(reg, p.id);
    let _ = writeln!(out, "{}", bios_state_line(reg, p, &st));
    package_lines(out, reg, &st);
    for t in &st.targets {
        if t.ready {
            let fw = t
                .firmware
                .as_deref()
                .filter(|v| *v != "installed")
                .map(|v| format!(" (firmware {v})"))
                .unwrap_or_default();
            let _ = writeln!(out, "    ready in {}{fw}: has what it needs in {}", t.emulator, t.dir.display());
        } else if t.firmware.is_some() {
            let _ = writeln!(out, "    firmware in {}: installed", t.emulator);
        }
        if !t.missing.is_empty() {
            let names: Vec<String> = t
                .missing
                .iter()
                .map(|m| m.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
                .collect();
            let _ = writeln!(
                out,
                "    BIOS for {}: {} file(s) not yet in {} (Settings → Emulation can copy them): {}",
                t.emulator,
                t.missing.len(),
                t.dir.display(),
                names.join(", ")
            );
        }
    }
    let plan = bios::setup_plan(reg, &st, reg.config.platform_emulator.get(p.id).and_then(|v| v.split('/').next()));
    if !plan.is_empty() {
        let name = |id: &str| reg.emulator(id).map_or(id.to_string(), |e| e.name.clone());
        let _ = writeln!(out, "    Set up everything would:");
        for s in &plan {
            let _ = writeln!(out, "      {}", bios::step_line(s, &name, &reg.home));
        }
    }
}

/// "BIOS: OK — scph5501.bin ✓ (…)": the state, the files found and the check's note.
fn bios_state_line(reg: &Registry, p: &Platform, st: &bios::BiosStatus) -> String {
    let state = match st.state {
        BiosState::Ok => "OK",
        BiosState::Missing => "missing",
        BiosState::Wrong => "wrong file (not a known good dump)",
        BiosState::NotNeeded => "not needed",
        BiosState::Optional => "optional, none found",
    };
    // Firmware that is found but goes in through the emulator: say so instead of "missing".
    let waiting = st.state == BiosState::Missing
        && st.files.iter().filter(|(b, s, _)| b.required && *s == BiosState::Missing).all(|(b, _, _)| {
            (b.file == "firmware" && st.packages.iter().any(|p| p.kind == bios::PackageKind::SwitchFirmware))
                || (b.file == "PS3UPDAT.PUP" && st.packages.iter().any(|p| p.kind == bios::PackageKind::Ps3Firmware))
        });
    let state = if waiting { "found, not installed in the emulator yet" } else { state };
    let what = match p.id {
        "nintendo_switch" => "Keys & firmware",
        "sony_playstation3" => "Firmware",
        _ => "BIOS",
    };
    let mut line = format!("    {what}: {state}");
    let found: Vec<String> = st
        .files
        .iter()
        .filter(|(_, s, _)| *s == BiosState::Ok)
        .filter_map(|(b, _, path)| {
            path.as_ref().map(|pa| format!("{} ✓ ({})", b.label(), bios::describe(pa, &reg.home)))
        })
        .collect();
    if !found.is_empty() {
        line.push_str(&format!(" — {}", found.join("; ")));
    }
    if !st.note.is_empty() {
        line.push_str(&format!(" ({})", st.note));
    }
    line
}

/// The firmware packages found and whether each emulator has them installed.
fn package_lines(out: &mut String, reg: &Registry, st: &bios::BiosStatus) {
    for pk in &st.packages {
        let mut line = format!("    {} ✓ found ({})", pk.label(), bios::describe(&pk.path, &reg.home));
        if pk.kind == bios::PackageKind::SwitchFirmware {
            line.push_str(&format!(", {} files", pk.files));
        }
        let _ = writeln!(out, "{line}");
        for t in &pk.targets {
            let name = reg.emulator(&t.emulator).map_or(t.emulator.as_str(), |e| e.name.as_str());
            let what = match (&t.installed, pk.kind) {
                (None, bios::PackageKind::SwitchFirmware) => format!(
                    "not installed yet → Settings → Emulation: Install firmware into {name} (copies the files into {})",
                    t.dir.display()
                ),
                (None, bios::PackageKind::Ps3Firmware) => {
                    format!("not installed yet → Settings → Emulation: Install into {name} (runs rpcs3 --installfw)")
                }
                (Some(v), _) => format!("{v} installed"),
            };
            let _ = writeln!(out, "      in {}: {what}", t.emulator);
        }
    }
}
