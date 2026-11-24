//! Checking one console's BIOS: which known files were found (and where), which are missing or
//! wrong, the firmware packages found, and what each installed emulator still lacks.
//!
//! [`check`] runs in steps: gather candidate files, decide a row per known file (PS3 / Switch /
//! Vita firmware have rows of their own), settle one-of groups and the PS2 placeholder rows, then
//! the console's overall state, its notes and the copy targets per emulator.
//!
//! Candidates are ranked, never sorted by path across drives: the console's emulators' own
//! folders first (a file there is the one shown, and the emulator is ready with it), then the
//! folders the user chose, other emulators' folders and the user's emulator folders, ROM folders,
//! the home folder's places, the drives' `Emulation/bios` ([`SearchDir::rank`]). Inside a rank
//! the shortest path wins (fewest folders, then fewest characters, then by name): a file near
//! the top of a BIOS folder over a copy deeper inside it. Modification times aren't used: copies
//! of a dump keep the date they were copied, which says nothing about which one is meant.

use super::*;

/// One row of [`BiosStatus::files`]: a known file, its state and where it was found.
type Row = (&'static BiosFile, BiosState, Option<PathBuf>);

/// At most this many files are collected from the search folders.
const MAX_CANDIDATES: usize = 20_000;
/// A folder or zip with at least this many `.nca` files is a Switch firmware package.
const MIN_FIRMWARE_NCAS: usize = 20;

/// PS1 BIOS files emulators like best, per region (then the table's order).
const PREFERRED: &[&str] =
    &["scph5501.bin", "scph5500.bin", "scph5502.bin", "scph7001.bin", "scph7000.bin", "scph7002.bin"];

/// What the rows of the known files gathered besides the rows themselves.
#[derive(Default)]
struct Findings {
    rows: Vec<Row>,
    packages: Vec<Package>,
    notes: Vec<String>,
    /// Items whose own "how to get it" note is replaced by a found package.
    packaged: Vec<&'static str>,
    /// PS2 dumps found beyond the one used per region.
    ps2_extra: usize,
    /// The folders (or zips) PS2 dumps were found in.
    ps2_places: Vec<PathBuf>,
}

/// Check a console's BIOS (MD5-identifies candidate files; cached by path + size + mtime).
pub fn check(reg: &Registry, platform: &str) -> BiosStatus {
    let known = files_for(platform);
    let mut st = BiosStatus {
        platform: platform.to_string(),
        state: BiosState::NotNeeded,
        files: vec![],
        targets: vec![],
        note: String::new(),
        packages: vec![],
    };
    if known.is_empty() {
        return st;
    }
    let emus = emulators_for(reg, platform);
    let ranked = candidates(reg, &emus);
    let rank: HashMap<PathBuf, u8> = ranked.iter().cloned().collect();
    let cands: Vec<PathBuf> = ranked.into_iter().map(|(p, _)| p).collect();
    let ids: Vec<(PathBuf, &'static BiosFile)> = cands
        .iter()
        .filter_map(|c| identify_file(c).filter(|b| b.platform == platform).map(|b| (c.clone(), b)))
        .collect();

    let mut f = Findings::default();
    for b in &known {
        let found: Vec<&PathBuf> = ids.iter().filter(|(_, x)| std::ptr::eq(*x, *b)).map(|(p, _)| p).collect();
        // PS3 firmware: installed through RPCS3; a PUP found is offered for installing.
        if b.platform == "sony_playstation3" && b.file == "PS3UPDAT.PUP" {
            ps3_firmware_row(reg, &emus, b, &found, &mut f);
        } else if b.platform == "nintendo_switch" && b.file == "firmware" {
            switch_firmware_row(reg, &emus, b, &cands, &mut f);
        } else if !found.is_empty() && b.file.is_empty() {
            ps2_dump_row(b, &found, &rank, &mut f);
        } else if !found.is_empty() {
            // The best-ranked copy decides (an emulator's own first); a real file before one
            // inside a zip of the same rank.
            let best = rank.get(found[0]).copied().unwrap_or(u8::MAX);
            let pick = found
                .iter()
                .take_while(|p| rank.get(**p).copied().unwrap_or(u8::MAX) == best)
                .find(|p| zipread::split(p).is_none())
                .unwrap_or(&found[0]);
            f.rows.push((b, BiosState::Ok, Some((*pick).clone())));
        } else if let Some(vs0) = (b.platform == "sony_vita" && b.file == "PSVUPDAT.PUP")
            .then(|| installed_vita_firmware(reg, &emus))
            .flatten()
        {
            f.rows.push((b, BiosState::Ok, Some(vs0)));
        } else {
            f.rows.push(not_found_row(b, &cands));
        }
    }

    let groups = settle_groups(&mut f.rows);
    tidy_ps2_rows(&mut f.rows, groups.get("ps2") == Some(&true), &rank);
    st.state = overall_state(&f.rows, &groups);
    let mut notes = std::mem::take(&mut f.notes);
    notes.extend(region_coverage_note(&known, &f.rows));
    if f.ps2_extra > 0 || f.ps2_places.len() > 1 {
        let places: Vec<String> = f
            .ps2_places
            .iter()
            .map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
            .collect();
        notes.push(format!(
            "PS2 BIOS files found in {}; one of each region is used (the emulator's own first, else the newest).",
            places.join(", ")
        ));
    }
    for (b, s, _) in &f.rows {
        if *s != BiosState::Ok
            && !b.note.is_empty()
            && b.required
            && b.md5.is_empty()
            && !b.file.is_empty()
            && !f.packaged.contains(&b.file)
        {
            notes.push(b.note.to_string());
        }
    }
    st.targets = emus.iter().filter_map(|e| copy_target(reg, e, platform, &f.rows, &ids)).collect();
    st.files = f.rows;
    st.packages = f.packages;
    st.note = notes.join(" · ");
    st
}

/// Candidate files with their rank (lower = preferred; see the module notes): the search
/// folders, plus the files the emulators' own settings point at (melonDS, xemu, mGBA BIOS paths:
/// rank 0). Best first, each file once (at its best rank).
fn candidates(reg: &Registry, emus: &[&Emulator]) -> Vec<(PathBuf, u8)> {
    let mut dirs = search_dirs_with_origin(reg);
    dirs.sort_by_key(|d| d.rank(emus));
    let mut cands: Vec<(PathBuf, u8)> = vec![];
    let mut cap = MAX_CANDIDATES;
    for d in &dirs {
        let own = matches!(d.origin, Origin::Emulator(_));
        let mut files = vec![];
        walk(&d.path, d.depth, d.origin != Origin::Chosen, &mut cap, &mut files);
        // A folder the user chose may hold one they removed (a whole drive, its backups inside);
        // an emulator's own folder is where it reads, whatever was removed.
        files.retain(|c| own || !super::removed_by_user(reg, c));
        let r = d.rank(emus);
        cands.extend(files.into_iter().map(|f| (f, r)));
    }
    for e in emus {
        cands.extend(super::super::roots::config_named_files(e, &reg.home).into_iter().map(|f| (f, 0)));
    }
    cands.sort_by(|(a, ra), (b, rb)| {
        (ra, a.components().count(), a.as_os_str().len(), a).cmp(&(rb, b.components().count(), b.as_os_str().len(), b))
    });
    let mut seen = std::collections::HashSet::new();
    cands.retain(|(p, _)| seen.insert(p.clone()));
    cands
}

/// The PS3 firmware row: Ok when RPCS3 has firmware installed. A `PS3UPDAT.PUP` found becomes a
/// package to install into each RPCS3.
fn ps3_firmware_row(reg: &Registry, emus: &[&Emulator], b: &'static BiosFile, found: &[&PathBuf], f: &mut Findings) {
    let installed = emus.iter().find_map(|e| ps3_firmware(e, &reg.home).map(|v| (e, v)));
    if let Some(pup) = found.iter().find(|p| p.is_file()) {
        let targets: Vec<PackageTarget> = emus
            .iter()
            .filter(|e| e.def == "rpcs3" && !e.blocked)
            .map(|e| PackageTarget {
                emulator: e.id.clone(),
                dir: rpcs3_flash(e, &reg.home),
                installed: ps3_firmware(e, &reg.home),
            })
            .collect();
        f.packages.push(Package {
            kind: PackageKind::Ps3Firmware,
            path: (*pup).clone(),
            version: pup_version(pup),
            files: 1,
            targets,
        });
        f.packaged.push(b.file);
    }
    // A firmware file found shows where it is installed with it (Settings: "Not installed
    // in RPCS3 yet" under the file): no second line saying the same.
    let shown = f.packages.iter().any(|k| k.kind == PackageKind::Ps3Firmware);
    match installed {
        Some((e, v)) => {
            if !shown {
                f.notes.push(format!("PS3 firmware {v} is installed in {}.", e.name));
            }
            f.rows.push((b, BiosState::Ok, Some(rpcs3_flash(e, &reg.home))));
        }
        None => f.rows.push((b, BiosState::Missing, None)),
    }
}

/// The Switch firmware row: Ok when an emulator's NAND has firmware. The newest firmware package
/// found (a folder over a zip: it copies faster) becomes a package to install into each emulator.
fn switch_firmware_row(reg: &Registry, emus: &[&Emulator], b: &'static BiosFile, cands: &[PathBuf], f: &mut Findings) {
    let fw = emus.iter().filter_map(|e| switch_firmware_dir(e, &reg.home)).find(|d| has_nca(d));
    let mut packs = switch_firmware_found(cands);
    // Not the emulators' own installed copies.
    packs.retain(|(p, _)| !emus.iter().filter_map(|e| switch_firmware_dir(e, &reg.home)).any(|d| *p == d));
    packs.sort_by_key(|(p, _)| {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let v: Vec<u32> =
            version_in_name(&name).unwrap_or_default().split('.').filter_map(|x| x.parse().ok()).collect();
        (std::cmp::Reverse(v), p.is_file())
    });
    if let Some((p, n)) = packs.into_iter().next() {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        // Firmware NCA names are content hashes: another version has other names.
        let probe =
            Package { kind: PackageKind::SwitchFirmware, path: p.clone(), version: None, files: n, targets: vec![] };
        let wanted: Vec<String> = firmware_files(&probe).into_iter().map(|(n, _)| n.to_lowercase()).collect();
        let targets = emus
            .iter()
            .filter(|e| !e.blocked)
            .filter_map(|e| {
                let d = switch_firmware_dir(e, &reg.home)?;
                let installed = has_nca(&d).then(|| {
                    let have: std::collections::HashSet<String> = std::fs::read_dir(&d)
                        .into_iter()
                        .flatten()
                        .flatten()
                        .map(|x| x.file_name().to_string_lossy().to_lowercase())
                        .collect();
                    if wanted.iter().all(|w| have.contains(w)) { "installed".to_string() } else { "other".to_string() }
                });
                Some(PackageTarget { emulator: e.id.clone(), dir: d, installed })
            })
            .collect();
        f.packages.push(Package {
            kind: PackageKind::SwitchFirmware,
            path: p,
            version: version_in_name(&name),
            files: n,
            targets,
        });
        f.packaged.push(b.file);
    }
    f.rows.push((b, if fw.is_some() { BiosState::Ok } else { BiosState::Missing }, fw));
}

/// A PS2 dump row (any file name): per region the best-ranked place (an emulator's own folder
/// first), there the newest dump, a real file over a zip entry.
fn ps2_dump_row(b: &'static BiosFile, found: &[&PathBuf], rank: &HashMap<PathBuf, u8>, f: &mut Findings) {
    let mut best: Vec<&PathBuf> = found.to_vec();
    best.sort_by_key(|p| {
        let v = ps2_romver_of(p).map(|r| ps2_version_key(&r)).unwrap_or_default();
        let r = rank.get(*p).copied().unwrap_or(u8::MAX);
        (r, std::cmp::Reverse(v), zipread::split(p).is_some(), p.as_os_str().len())
    });
    f.ps2_extra += best.len() - 1;
    for p in &best {
        let place = match zipread::split(p) {
            Some((z, _)) => z,
            None => p.parent().map(Path::to_path_buf).unwrap_or_default(),
        };
        if !f.ps2_places.contains(&place) {
            f.ps2_places.push(place);
        }
    }
    f.rows.push((b, BiosState::Ok, Some(best[0].clone())));
}

/// The Vita firmware Vita3K has installed (its `vs0` folder), if any.
fn installed_vita_firmware(reg: &Registry, emus: &[&Emulator]) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = emus
        .iter()
        .filter(|e| e.def == "vita3k")
        .filter_map(|e| super::super::roots::user_dir(e, &reg.home))
        .map(|u| u.path.join("vs0"))
        .collect();
    dirs.push(reg.home.join(".local/share/Vita3K/Vita3K/vs0"));
    dirs.into_iter().find(|v| v.join("vsh").is_dir() || v.join("sys").is_dir())
}

/// The row of a known file nothing matched: Wrong when a file with its name is there but isn't a
/// good dump, else Missing or Optional.
fn not_found_row(b: &'static BiosFile, cands: &[PathBuf]) -> Row {
    let wrong = (!b.file.is_empty() && !b.md5.is_empty())
        .then(|| cands.iter().find(|c| c.file_name().is_some_and(|n| name_eq(&n.to_string_lossy(), b.base_name()))))
        .flatten();
    let state = match (wrong, b.required) {
        (Some(_), _) => BiosState::Wrong,
        (None, true) => BiosState::Missing,
        (None, false) => BiosState::Optional,
    };
    (b, state, wrong.cloned())
}

/// One-of groups: satisfied when any member is Ok; then the others are just optional. Returns
/// whether each group is satisfied.
fn settle_groups(rows: &mut [Row]) -> HashMap<&'static str, bool> {
    let mut groups: HashMap<&'static str, bool> = HashMap::new();
    for (b, s, _) in rows.iter() {
        if let Some(g) = b.group {
            *groups.entry(g).or_default() |= *s == BiosState::Ok;
        }
    }
    for (b, s, _) in rows.iter_mut() {
        if let Some(g) = b.group
            && groups[g]
            && *s == BiosState::Missing
        {
            *s = BiosState::Optional;
        }
    }
    groups
}

/// With a PS2 dump found, drop the placeholder rows that found nothing, and a region's dump
/// when a known-good named one of that region was found as near (it is the one used; a dump in
/// an emulator's own folder stays). Without one, keep a single "PS2 BIOS" placeholder row.
fn tidy_ps2_rows(rows: &mut Vec<Row>, ps2_found: bool, rank: &HashMap<PathBuf, u8>) {
    let rank_of = |p: &Option<PathBuf>| p.as_ref().and_then(|p| rank.get(p)).copied().unwrap_or(u8::MAX);
    if ps2_found {
        let named: Vec<(Option<&str>, u8)> = rows
            .iter()
            .filter(|(b, s, _)| b.platform == "sony_playstation2" && !b.file.is_empty() && *s == BiosState::Ok)
            .map(|(b, _, p)| (b.region, rank_of(p)))
            .collect();
        rows.retain(|(b, s, p)| {
            !(b.file.is_empty()
                && (*s != BiosState::Ok || named.iter().any(|(r, nr)| *r == b.region && *nr <= rank_of(p))))
        });
    } else {
        let mut seen = false;
        rows.retain(|(b, _, _)| {
            if b.platform == "sony_playstation2" && b.file.is_empty() {
                let keep = !seen && b.region.is_none();
                seen |= keep;
                keep
            } else {
                true
            }
        });
    }
}

/// The console's state: problems that count are required files outside a satisfied one-of group.
fn overall_state(rows: &[Row], groups: &HashMap<&'static str, bool>) -> BiosState {
    let bad: Vec<BiosState> = rows
        .iter()
        .filter(|(b, s, _)| {
            b.required && matches!(s, BiosState::Missing | BiosState::Wrong) && b.group.is_none_or(|g| !groups[g])
        })
        .map(|(_, s, _)| *s)
        .collect();
    let any_required = rows.iter().any(|(b, _, _)| b.required);
    if bad.contains(&BiosState::Wrong) {
        BiosState::Wrong
    } else if !bad.is_empty() {
        BiosState::Missing
    } else if any_required || rows.iter().any(|(_, s, _)| *s == BiosState::Ok) {
        BiosState::Ok
    } else {
        BiosState::Optional
    }
}

/// Region coverage of a regional one-of group (PS1, Sega CD, Saturn …): "USA ✓  Europe ✗ …".
fn region_coverage_note(known: &[&'static BiosFile], rows: &[Row]) -> Option<String> {
    let g = known.iter().find_map(|b| b.group.filter(|_| b.region.is_some())).filter(|g| *g != "ps2")?;
    let mut regions: Vec<&str> = vec![];
    for b in known.iter().filter(|b| b.group == Some(g)) {
        if let Some(r) = b.region
            && !regions.contains(&r)
        {
            regions.push(r);
        }
    }
    if regions.len() <= 1 {
        return None;
    }
    let cover: Vec<String> = regions
        .iter()
        .map(|r| {
            let ok = rows.iter().any(|(b, s, _)| b.group == Some(g) && b.region == Some(r) && *s == BiosState::Ok);
            format!("{r} {}", if ok { "✓" } else { "✗" })
        })
        .collect();
    Some(cover.join("  "))
}

/// What emulator `e` still lacks: per regional BIOS group one file (the preferred one of each
/// region), every other found file as found. A region (or file) it already has a good dump of in
/// its own folders is never offered again, whatever else was found. None when it has no BIOS
/// (or firmware) folder.
fn copy_target(
    reg: &Registry,
    e: &Emulator,
    platform: &str,
    rows: &[Row],
    ids: &[(PathBuf, &'static BiosFile)],
) -> Option<BiosTarget> {
    let dir = bios_dir(e, &reg.home).or_else(|| firmware_home(e, &reg.home))?;
    let named = super::super::roots::config_named_files(e, &reg.home);
    let firmware = installed_firmware(e, &reg.home);
    let there = |src: &Path, rel: &Path| -> bool {
        let dest = dir.join(rel);
        same_file(src, &dest)
            || (matches!(e.def.as_str(), "duckstation" | "pcsx2")
                && std::fs::read_dir(dest.parent().unwrap_or(&dir))
                    .into_iter()
                    .flatten()
                    .flatten()
                    .any(|f| same_file(src, &f.path())))
    };
    let own = |p: &Path| p.starts_with(&dir) || named.iter().any(|n| n == p);
    // It has this file, or one of the same region of its group, in its own folders.
    let has_own = |b: &BiosFile| {
        ids.iter().any(|(p, x)| {
            own(p)
                && target_rel(&e.def, x, p).is_some()
                && match (b.group, b.region) {
                    (Some(g), Some(r)) => x.group == Some(g) && x.region == Some(r),
                    _ => std::ptr::eq(*x, b),
                }
        })
    };
    // (bucket key, [(preference, src, rel)])
    type Bucket = (String, Vec<(usize, PathBuf, PathBuf)>);
    let mut buckets: Vec<Bucket> = vec![];
    for (i, (b, s, p)) in rows.iter().enumerate() {
        let (BiosState::Ok, Some(src)) = (s, p) else { continue };
        let Some(rel) = target_rel(&e.def, b, src) else { continue };
        if !source_exists(src) || own(src) || has_own(b) {
            continue;
        }
        let key = match (b.group, b.region) {
            (Some(g), Some(r)) => format!("{g}/{r}"),
            _ => format!("#{i}"),
        };
        let item = (preference(b), src.clone(), rel);
        match buckets.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => v.push(item),
            None => buckets.push((key, vec![item])),
        }
    }
    let mut missing: Vec<PathBuf> = vec![];
    for (_, mut items) in buckets {
        items.sort_by_key(|(pref, _, _)| *pref);
        if items.iter().any(|(_, src, rel)| there(src, rel)) {
            continue;
        }
        let src = items[0].1.clone();
        if !missing.contains(&src) {
            missing.push(src);
        }
    }
    let ready = is_ready(e, platform, &dir, &named, ids, firmware.is_some());
    Some(BiosTarget { emulator: e.id.clone(), dir, missing, ready, firmware })
}

/// Folders and zips holding Switch firmware (≥ [`MIN_FIRMWARE_NCAS`] `.nca` files): (path, NCA count).
fn switch_firmware_found(cands: &[PathBuf]) -> Vec<(PathBuf, usize)> {
    let mut counts: Vec<(PathBuf, usize)> = vec![];
    for c in cands {
        let Some(name) = c.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        if !is_nca(&name) {
            continue;
        }
        let key = match zipread::split(c) {
            Some((zip, _)) => zip,
            None => c.parent().map(Path::to_path_buf).unwrap_or_default(),
        };
        match counts.iter_mut().find(|(k, _)| *k == key) {
            Some((_, n)) => *n += 1,
            None => counts.push((key, 1)),
        }
    }
    counts.retain(|(_, n)| *n >= MIN_FIRMWARE_NCAS);
    counts
}

/// (version, date) of a PS2 ROMVER, for "newest first".
fn ps2_version_key(romver: &str) -> (u32, String) {
    (romver.get(..4).and_then(|v| v.parse().ok()).unwrap_or(0), romver.get(6..).unwrap_or("").to_string())
}

fn preference(b: &BiosFile) -> usize {
    PREFERRED.iter().position(|f| name_eq(f, b.file)).unwrap_or(PREFERRED.len())
}

/// For a firmware-only emulator (RPCS3): where its firmware goes, as its "target" folder.
fn firmware_home(e: &Emulator, home: &Path) -> Option<PathBuf> {
    (e.def == "rpcs3").then(|| rpcs3_flash(e, home))
}

/// Firmware installed in an emulator: PS3 its version; Switch "installed" when its NAND has
/// firmware files.
fn installed_firmware(e: &Emulator, home: &Path) -> Option<String> {
    if e.def == "rpcs3" {
        return ps3_firmware(e, home);
    }
    switch_firmware_dir(e, home).filter(|d| has_nca(d)).map(|_| "installed".to_string())
}

/// Does `e` have, in its own folders (`dir`, or files its settings name), what it needs for
/// `platform`? Required files must be there (one member of a one-of group is enough); with no
/// required file it uses, any file it uses counts. Switch also needs firmware, PS3 only firmware.
fn is_ready(
    e: &Emulator,
    platform: &str,
    dir: &Path,
    named: &[PathBuf],
    ids: &[(PathBuf, &'static BiosFile)],
    firmware: bool,
) -> bool {
    if platform == "sony_playstation3" {
        return firmware;
    }
    let inside =
        |b: &BiosFile| ids.iter().any(|(p, x)| std::ptr::eq(*x, b) && (p.starts_with(dir) || named.contains(p)));
    let probe = Path::new("x.bin");
    let used: Vec<&'static BiosFile> = files_for(platform)
        .into_iter()
        .filter(|b| b.file != "firmware" && target_rel(&e.def, b, probe).is_some())
        .collect();
    let used_named: Vec<&'static BiosFile> = files_for(platform)
        .into_iter()
        .filter(|b| ids.iter().any(|(p, x)| std::ptr::eq(*x, *b) && named.contains(p)))
        .collect();
    let mut need_ok = true;
    let mut any_need = false;
    for b in used.iter().filter(|b| b.required) {
        any_need = true;
        let ok = match b.group {
            Some(g) => used.iter().chain(&used_named).filter(|x| x.group == Some(g)).any(|x| inside(x)),
            None => inside(b),
        };
        need_ok &= ok;
    }
    let files_ok = if any_need { need_ok } else { used.iter().chain(&used_named).any(|b| inside(b)) };
    if platform == "nintendo_switch" {
        return files_ok && firmware;
    }
    files_ok
}
