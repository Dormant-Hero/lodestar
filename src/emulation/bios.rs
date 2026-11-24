//! BIOS / firmware a console needs: which files (by MD5), finding them, checking them and
//! copying them where each installed emulator looks. Lodestar never ships or downloads any.
//!
//! Files are recognised by content, not name (DuckStation and PCSX2 ignore names): MD5 against
//! the table in `bios/table.rs`, PS2 BIOS dumps by their ROMVER, keys / firmware packages by
//! name. Files inside .zip archives count too (`<zip>/<entry>` "virtual paths", see
//! `zipread`): BIOS packs are usually shared as zips, with macOS junk inside. MD5s are cached
//! per file (path + size + modification time; zip entries by zip + size + mtime + entry); files
//! over 64 MiB are never read.
//!
//! Where it looks ([`search_dirs_with_origin`]), best first: the emulators' own BIOS folders (per
//! install: portable copies, AppImage homes, Flatpaks — see `roots`) and files their settings
//! name, the chosen BIOS folders, folders named like "bios" in the user's emulator folders
//! (`<folder>/<emulator>/bios`), "bios" / "system" folders and "*bios*" folders in the ROM
//! folders, `~/Emulation/bios` and every folder (or zip) named like "*bios*" in the home folder,
//! Documents, Downloads and Emulation (3 levels down), and last `Emulation/bios` on the drives
//! (at most three folders below a drive). Nothing else on the drives: old emulator copies and
//! backups there held outdated dumps. Folders whose name says they were put aside ("Backups",
//! "unused BIOS", "_old") are skipped unless the user chose them; folders the user removed are
//! never searched (an emulator's own folder always is: it reads there). Found files are ranked
//! by where they are (`check`), so a file in the emulator's own folder is the one shown and used.
//!
//! An emulator that already has what it needs in its own folders is "ready" (`BiosTarget.ready`:
//! nothing to choose or copy); its installed firmware (PS3 version, Switch NAND) is read from its
//! own folders too. "Set up everything" (`bios/setup.rs`) puts whatever is still missing in place
//! after one confirmation.
//!
//! Copying ("Copy to <emulator>") takes a sensible set, not every variant found: one BIOS per
//! region for consoles with regional BIOS files (PS1: scph5501 / 5500 / 5502 first; PS2: the
//! newest version per region), so an emulator's BIOS list stays short.
//!
//! Switch keys are copied like BIOS files; Switch firmware (a folder or zip of `.nca` files) is
//! installed by copying the NCAs into the emulator's `nand/system/Contents/registered` (what the
//! emulators' own "Install Firmware" does); PS3 firmware (`PS3UPDAT.PUP`) is installed by RPCS3
//! itself (`rpcs3 --installfw`). Key contents are never shown or logged.

use super::detect::{Emulator, InstallKind};
use super::registry::Registry;
use super::zipread;
use md5::{Digest, Md5};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

mod check;
mod setup;
mod table;
pub use check::check;
pub use setup::{Mood, Step, join_and, run_setup, setup_plan, step_line, summary};
#[cfg(test)]
pub use setup::{Summary, what_of};

/// One known BIOS file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BiosFile {
    /// Platform id.
    pub platform: &'static str,
    /// File name emulators look for ("" = any name: PS2 BIOS dumps).
    pub file: &'static str,
    /// Good dumps (lower-case hex); empty = recognised by name.
    pub md5: &'static [&'static str],
    pub required: bool,
    /// One-of group (any one file of the group is enough).
    pub group: Option<&'static str>,
    pub region: Option<&'static str>,
    pub note: &'static str,
}

impl BiosFile {
    /// What to call it in the UI: its file name, else its note.
    pub fn label(&self) -> &'static str {
        if self.file.is_empty() { self.note } else { self.file }
    }

    /// The file name part ("dc/dc_boot.bin" → "dc_boot.bin").
    pub fn base_name(&self) -> &'static str {
        self.file.rsplit('/').next().unwrap_or(self.file)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BiosState {
    Ok,
    Missing,
    /// A file with the right name but not a known good dump.
    Wrong,
    NotNeeded,
    Optional,
}

/// A console's BIOS check.
#[derive(Clone, Debug, PartialEq)]
pub struct BiosStatus {
    pub platform: String,
    pub state: BiosState,
    /// Each known file, its state and where it was found.
    pub files: Vec<(&'static BiosFile, BiosState, Option<PathBuf>)>,
    /// Installed emulators of this console and what they still lack.
    pub targets: Vec<BiosTarget>,
    pub note: String,
    /// Firmware found on the computer that is installed through / into an emulator (Switch
    /// firmware, PS3 firmware).
    pub packages: Vec<Package>,
}

/// Where one emulator looks for BIOS files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BiosTarget {
    /// Emulator instance id.
    pub emulator: String,
    pub dir: PathBuf,
    /// Found files that aren't there yet.
    pub missing: Vec<PathBuf>,
    /// It already has what it needs in its own folders: the BIOS files (one of a "any one is
    /// enough" group), Switch keys and firmware, PS3 firmware. Nothing to choose or copy.
    pub ready: bool,
    /// Firmware installed in it: PS3 its version ("4.93" / "installed"), Switch "installed";
    /// None when it has none or the console has no firmware.
    pub firmware: Option<String>,
}

/// What kind of firmware a [`Package`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageKind {
    /// A folder or .zip of Switch firmware `.nca` files.
    SwitchFirmware,
    /// `PS3UPDAT.PUP`.
    Ps3Firmware,
}

/// Firmware found on the computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub kind: PackageKind,
    /// The folder, zip or file.
    pub path: PathBuf,
    /// "23.5.0" (from the folder / zip name, or the PUP's version entry).
    pub version: Option<String>,
    /// Files in it (NCAs).
    pub files: usize,
    /// Emulators it can go into, and what they have now.
    pub targets: Vec<PackageTarget>,
}

/// One emulator a [`Package`] can be installed into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageTarget {
    pub emulator: String,
    /// Where it goes (Switch: the `registered` folder; PS3: RPCS3's `dev_flash`).
    pub dir: PathBuf,
    /// What it has installed now: None = nothing, Some("installed") (Switch: this firmware),
    /// Some("other") (Switch: a different firmware), Some("4.92") (PS3: its version).
    pub installed: Option<String>,
}

impl Package {
    /// The emulator has an older (PS3: lower version) or a different (Switch: other files)
    /// firmware than this one: worth offering to replace it.
    pub fn replaces(&self, t: &PackageTarget) -> bool {
        match (self.kind, t.installed.as_deref()) {
            (_, None) => false,
            (PackageKind::SwitchFirmware, Some(v)) => v == "other",
            (PackageKind::Ps3Firmware, Some(v)) => self.version.as_deref().is_some_and(|new| version_less(v, new)),
        }
    }

    /// "Firmware 23.5.0" / "PS3 firmware 4.92" / "Firmware".
    pub fn label(&self) -> String {
        let what = match self.kind {
            PackageKind::SwitchFirmware => "Firmware",
            PackageKind::Ps3Firmware => "PS3 firmware",
        };
        match &self.version {
            Some(v) => format!("{what} {v}"),
            None => what.to_string(),
        }
    }
}

#[cfg(test)]
pub(crate) static TEST_BIOS: &[BiosFile] = &[
    // md5("lodestar fake ps1 bios") — only in tests.
    BiosFile {
        platform: "sony_playstation",
        file: "scph-test.bin",
        md5: &["5f4cdec3d512806f5b867409a9ad8e97"],
        required: true,
        group: Some("psx"),
        region: Some("USA"),
        note: "",
    },
];

/// Every known BIOS file.
pub fn table() -> impl Iterator<Item = &'static BiosFile> {
    #[cfg(test)]
    let extra = TEST_BIOS.iter();
    #[cfg(not(test))]
    let extra = [].iter();
    table::BIOS.iter().chain(extra)
}

/// The known files of one console.
pub fn files_for(platform: &str) -> Vec<&'static BiosFile> {
    table().filter(|b| b.platform == platform).collect()
}

/// Does this platform need a BIOS?
pub fn needs_bios(platform: &str) -> bool {
    table().any(|b| b.platform == platform && b.required)
}

// ---------------------------------------------------------------------------------------------
// Identifying files
// ---------------------------------------------------------------------------------------------

const MAX_HASH: u64 = 64 * 1024 * 1024;

/// Never BIOS files (game files, firmware content): skipped without reading.
const NOT_BIOS: &[&str] =
    &["nca", "nsp", "xci", "nsz", "iso", "chd", "cso", "rvz", "wbfs", "cue", "m3u", "txt", "nfo", "png", "jpg"];

type Md5Cache = HashMap<PathBuf, (u64, Option<SystemTime>, String)>;
static MD5S: Mutex<Option<Md5Cache>> = Mutex::new(None);

/// Facts about a zip entry: (zip size, zip mtime, md5, ROMVER).
type ZipFacts = HashMap<PathBuf, (u64, Option<SystemTime>, String, Option<String>)>;
static ZIP_FACTS: Mutex<Option<ZipFacts>> = Mutex::new(None);

/// MD5 + ROMVER of a file inside a zip (one decompression; cached by zip + size + mtime + entry).
fn zip_facts(p: &Path) -> Option<(String, Option<String>)> {
    let (zip, e) = zipread::entry_of(p)?;
    let m = std::fs::metadata(&zip).ok()?;
    let mtime = m.modified().ok();
    if let Some((l, t, h, v)) = ZIP_FACTS.lock().as_ref().and_then(|c| c.get(p))
        && *l == m.len()
        && *t == mtime
    {
        return Some((h.clone(), v.clone()));
    }
    let data = zipread::read(&zip, &e, MAX_HASH)?;
    let hex: String = Md5::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
    let romver = ps2_romver(&data);
    ZIP_FACTS
        .lock()
        .get_or_insert_with(HashMap::new)
        .insert(p.to_path_buf(), (m.len(), mtime, hex.clone(), romver.clone()));
    Some((hex, romver))
}

/// Lower-case hex MD5 of a file (cached by path + size + mtime), or of a file inside a zip;
/// None for folders, unreadable files and files over 64 MiB.
pub fn md5_of(p: &Path) -> Option<String> {
    use std::io::Read;
    let m = match std::fs::metadata(p) {
        Ok(m) => m,
        Err(_) => return zip_facts(p).map(|(h, _)| h),
    };
    if !m.is_file() || m.len() > MAX_HASH {
        return None;
    }
    let mtime = m.modified().ok();
    if let Some((len, t, h)) = MD5S.lock().as_ref().and_then(|c| c.get(p))
        && *len == m.len()
        && *t == mtime
    {
        return Some(h.clone());
    }
    let mut f = std::fs::File::open(p).ok()?;
    let mut h = Md5::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    MD5S.lock().get_or_insert_with(HashMap::new).insert(p.to_path_buf(), (m.len(), mtime, hex.clone()));
    Some(hex)
}

/// A PS2 BIOS dump's ROMVER ("0230AC20080220"): a 4–8 MiB file with a romdir whose first entry
/// is RESET (PCSX2's BiosTools.cpp).
pub fn ps2_romver(data: &[u8]) -> Option<String> {
    if !(4 * 1024 * 1024..=8 * 1024 * 1024).contains(&data.len()) {
        return None;
    }
    let scan = &data[..data.len().min(512 * 1024)];
    let start = (0..scan.len().saturating_sub(16)).step_by(16).find(|&i| &scan[i..i + 10] == b"RESET\0\0\0\0\0")?;
    let mut off: usize = 0;
    let mut i = start;
    while i + 16 <= data.len() {
        let e = &data[i..i + 16];
        if e[0] == 0 {
            break;
        }
        let name_end = e[..10].iter().position(|&b| b == 0).unwrap_or(10);
        let name = &e[..name_end];
        let size = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
        if name == b"ROMVER" {
            let v = data.get(off..off + 14)?;
            return v.iter().all(|b| b.is_ascii_alphanumeric()).then(|| String::from_utf8_lossy(v).into_owned());
        }
        off = off.checked_add(size.div_ceil(16) * 16)?;
        i += 16;
    }
    None
}

fn ps2_entry(romver: &str) -> Option<&'static BiosFile> {
    let region = match romver.as_bytes().get(4) {
        Some(b'J') => Some("Japan"),
        Some(b'A') => Some("USA"),
        Some(b'E') => Some("Europe"),
        Some(b'H') => Some("Asia"),
        _ => None,
    };
    table().find(|b| b.platform == "sony_playstation2" && b.file.is_empty() && b.region == region)
}

fn name_eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Size of a file, or of a file inside a zip (None: neither).
fn size_of(p: &Path) -> Option<u64> {
    match std::fs::metadata(p) {
        Ok(m) if m.is_file() => Some(m.len()),
        Ok(_) => None,
        Err(_) => zipread::entry_of(p).map(|(_, e)| e.size),
    }
}

/// Does a file (or zip entry) exist?
pub fn source_exists(p: &Path) -> bool {
    p.is_file() || zipread::entry_of(p).is_some()
}

/// The first bytes of a file or zip entry (small zip entries only).
fn head_of(p: &Path, n: usize) -> Vec<u8> {
    use std::io::Read;
    if let Ok(mut f) = std::fs::File::open(p) {
        let mut head = vec![0u8; n];
        let got = f.read(&mut head).unwrap_or(0);
        head.truncate(got);
        return head;
    }
    match zipread::entry_of(p) {
        Some((z, e)) => {
            let mut d = zipread::read(&z, &e, MAX_HASH).unwrap_or_default();
            d.truncate(n);
            d
        }
        None => vec![],
    }
}

/// The whole content of a small file or zip entry.
fn read_small(p: &Path, limit: u64) -> Option<Vec<u8>> {
    match std::fs::metadata(p) {
        Ok(m) if m.is_file() && m.len() <= limit => std::fs::read(p).ok(),
        Ok(_) => None,
        Err(_) => zipread::entry_of(p).and_then(|(z, e)| zipread::read(&z, &e, limit)),
    }
}

/// Which BIOS a single file (or file inside a zip) is.
pub fn identify_file(p: &Path) -> Option<&'static BiosFile> {
    let name = p.file_name()?.to_string_lossy().into_owned();
    let by_name = table().find(|b| b.md5.is_empty() && !b.file.is_empty() && name_eq(b.base_name(), &name));
    let ext = name.rsplit_once('.').map(|(_, x)| x.to_ascii_lowercase()).unwrap_or_default();
    if by_name.is_none() && NOT_BIOS.contains(&ext.as_str()) {
        return None;
    }
    let len = size_of(p)?;
    if len == 0 {
        return None;
    }
    // Known by name (no fixed checksum).
    if let Some(b) = by_name {
        return name_check(b, p).then_some(b);
    }
    if len <= MAX_HASH
        && let Some(h) = md5_of(p)
        && let Some(b) = table().find(|b| b.md5.contains(&h.as_str()))
    {
        return Some(b);
    }
    if (4 * 1024 * 1024..=8 * 1024 * 1024).contains(&len)
        && let Some(v) = ps2_romver_of(p)
    {
        return ps2_entry(&v);
    }
    None
}

type RomverCache = HashMap<PathBuf, (u64, Option<SystemTime>, Option<String>)>;
static ROMVERS: Mutex<Option<RomverCache>> = Mutex::new(None);

/// [`ps2_romver`] of a file (or zip entry), cached by path + size + mtime (BIOS checks run per
/// console and would otherwise read every 4–8 MiB file in the BIOS folders again each time).
pub fn ps2_romver_of(p: &Path) -> Option<String> {
    let Ok(m) = std::fs::metadata(p) else {
        return zip_facts(p).and_then(|(_, v)| v);
    };
    let mtime = m.modified().ok();
    if let Some((len, t, v)) = ROMVERS.lock().as_ref().and_then(|c| c.get(p))
        && *len == m.len()
        && *t == mtime
    {
        return v.clone();
    }
    let v = if (4 * 1024 * 1024..=8 * 1024 * 1024).contains(&m.len()) {
        std::fs::read(p).ok().and_then(|data| ps2_romver(&data))
    } else {
        None
    };
    ROMVERS.lock().get_or_insert_with(HashMap::new).insert(p.to_path_buf(), (m.len(), mtime, v.clone()));
    v
}

/// Cheap content checks for files known by name.
fn name_check(b: &BiosFile, p: &Path) -> bool {
    let head = head_of(p, 16);
    match b.base_name().to_ascii_lowercase().as_str() {
        "prod.keys" | "title.keys" | "keys.txt" => {
            read_small(p, 4 * 1024 * 1024).is_some_and(|t| t.contains(&b'=') || t.contains(&b' '))
        }
        "ps3updat.pup" | "psvupdat.pup" | "psp2updat.pup" => head.starts_with(b"SCEUF"),
        "xbox_hdd.qcow2" => head.starts_with(b"QFI\xfb"),
        n if n.ends_with(".zip") => head.starts_with(b"PK"),
        _ => true,
    }
}

/// Every file under `dir` (depth ≤ `depth`, at most `cap`), skipping hidden ones (and with
/// `skip_aside`, folders whose name says they were put aside: "old", "backup", "unused"); the
/// files inside .zip archives are listed as `<zip>/<entry>` after the zip itself. `dir` may be a
/// single file (a BIOS zip found by name).
fn walk(dir: &Path, depth: usize, skip_aside: bool, cap: &mut usize, out: &mut Vec<PathBuf>) {
    let push_file = |p: PathBuf, cap: &mut usize, out: &mut Vec<PathBuf>| {
        if *cap == 0 {
            return;
        }
        *cap -= 1;
        let is_zip = p.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip"));
        out.push(p.clone());
        if is_zip && let Some(entries) = zipread::list(&p) {
            for e in entries {
                if *cap == 0 {
                    return;
                }
                *cap -= 1;
                out.push(p.join(&e.name));
            }
        }
    };
    let Ok(rd) = std::fs::read_dir(dir) else {
        if dir.is_file() {
            push_file(dir.to_path_buf(), cap, out);
        }
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if *cap == 0 {
            return;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "__MACOSX" {
            continue;
        }
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        let is_dir = ft.is_dir() || (ft.is_symlink() && p.is_dir());
        if is_dir {
            if depth > 0 && !(skip_aside && set_aside(&name)) {
                walk(&p, depth - 1, skip_aside, cap, out);
            }
        } else {
            push_file(p, cap, out);
        }
    }
}

/// Which BIOS each picked file is (unknown files are left out). Folders are searched, zips are
/// looked into.
pub fn identify(paths: &[PathBuf]) -> Vec<(PathBuf, &'static BiosFile)> {
    let mut files = vec![];
    let mut cap = 20_000;
    for p in paths {
        walk(p, 3, false, &mut cap, &mut files);
    }
    files.into_iter().filter_map(|f| identify_file(&f).map(|b| (f, b))).collect()
}

/// "SCPH-39001.bin in ~/Downloads/ps2-bios-usa.zip" for a file inside a zip, else the path.
/// Reads nothing from the disk (Settings draws it every frame).
pub fn describe(p: &Path, home: &Path) -> String {
    let tilde = |p: &Path| match p.strip_prefix(home) {
        Ok(r) if !home.as_os_str().is_empty() && home != Path::new("/") => format!("~/{}", r.display()),
        _ => p.display().to_string(),
    };
    match zipread::split_by_name(p) {
        Some((zip, entry)) => {
            let base = entry.rsplit('/').next().unwrap_or(&entry).to_string();
            format!("{base} in {}", tilde(&zip))
        }
        None => tilde(p),
    }
}

// ---------------------------------------------------------------------------------------------
// Where emulators keep BIOS files
// ---------------------------------------------------------------------------------------------

/// `key = value` in `[section]` of an ini file.
fn ini_value(path: &Path, section: &str, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_sec = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_sec = l.trim_matches(|c| c == '[' || c == ']') == section;
        } else if in_sec
            && let Some((k, v)) = l.split_once('=')
            && k.trim() == key
        {
            let v = v.trim();
            return (!v.is_empty()).then(|| v.to_string());
        }
    }
    None
}

/// The folder an emulator reads BIOS files (or Switch keys) from, for this install (Flatpak,
/// AppImage with its own home, portable copy: see `roots`), plus what its own settings name.
pub fn bios_dir(emu: &Emulator, home: &Path) -> Option<PathBuf> {
    let rel = |root: PathBuf, v: Option<String>, default: &str| -> PathBuf {
        match v {
            Some(v) if Path::new(&v).is_absolute() => PathBuf::from(v),
            Some(v) if v.starts_with('~') || v.starts_with("$HOME") => super::detect::expand_home(&v, home),
            Some(v) => root.join(v),
            None => root.join(default),
        }
    };
    let user = super::roots::user_dir(emu, home)?.path;
    Some(match emu.def.as_str() {
        "duckstation" => {
            let v = ini_value(&user.join("settings.ini"), "BIOS", "SearchDirectory");
            rel(user, v, "bios")
        }
        "pcsx2" => {
            let v = ini_value(&user.join("inis/PCSX2.ini"), "Folders", "Bios");
            rel(user, v, "bios")
        }
        "dolphin" | "primehack" | "flycast" | "cemu" => user,
        "mednafen" => user.join("firmware"),
        "eden" | "citron" | "sudachi" => user.join("keys"),
        "ryujinx" => user.join("system"),
        "retroarch" => {
            let cfg = super::detect::retroarch_cfg(&emu.kind, home);
            let v = std::fs::read_to_string(&cfg).ok().and_then(|t| {
                t.lines().find_map(|l| {
                    let (k, v) = l.split_once('=')?;
                    (k.trim() == "system_directory").then(|| v.trim().trim_matches('"').to_string())
                })
            });
            match v.filter(|v| !v.is_empty() && v != "default") {
                Some(v) => super::detect::ra_path(&v, &cfg, home),
                None => cfg.parent().unwrap_or(home).join("system"),
            }
        }
        _ => return None,
    })
}

/// "SCPH-90001_BIOS_V18_USA_230.ROM0" → "SCPH-90001_BIOS_V18_USA_230.bin" (".bin" names stay).
fn bin_name(name: &Path) -> PathBuf {
    match name.extension() {
        Some(x) if x.eq_ignore_ascii_case("bin") => name.to_path_buf(),
        _ => name.with_extension("bin"),
    }
}

/// Where a BIOS file goes inside an emulator's BIOS folder (None: it doesn't use that file, or
/// it can't be copied — firmware is installed through the emulator).
fn target_rel(def: &str, b: &BiosFile, src: &Path) -> Option<PathBuf> {
    let src_name = src.file_name().map(PathBuf::from)?;
    let named = || if b.file.is_empty() { src_name.clone() } else { PathBuf::from(b.file) };
    match (def, b.platform) {
        (_, "nintendo_switch") if b.file == "firmware" => None,
        ("eden" | "citron" | "sudachi" | "ryujinx", "nintendo_switch") => Some(named()),
        ("duckstation", "sony_playstation") => Some(named()),
        // PCSX2 reads any name; a dump packed as "….ROM0" is saved as "….bin" like the rest.
        ("pcsx2", "sony_playstation2") => Some(bin_name(&src_name)),
        ("dolphin" | "primehack", "nintendo_gamecube") => Some(named()),
        ("flycast", "sega_dreamcast" | "arcade") if b.file.starts_with("dc/") => Some(PathBuf::from(b.base_name())),
        ("cemu", "nintendo_wiiu") => Some(named()),
        ("mednafen", p) => {
            let ok = matches!(
                b.file,
                "scph5500.bin"
                    | "scph5501.bin"
                    | "scph5502.bin"
                    | "sega_101.bin"
                    | "mpr-17933.bin"
                    | "syscard3.pce"
                    | "pcfx.rom"
                    | "lynxboot.img"
                    | "gba_bios.bin"
                    | "disksys.rom"
            );
            (ok && p != "arcade").then(named)
        }
        ("retroarch", p) => match p {
            "sony_playstation2" => Some(Path::new("pcsx2/bios").join(bin_name(&src_name))),
            "nintendo_switch" | "sony_playstation3" | "sony_vita" | "xbox" | "nintendo_gamecube" | "nintendo_wiiu" => {
                None
            }
            _ if b.file.is_empty() => None,
            _ => Some(PathBuf::from(b.file)),
        },
        _ => None,
    }
}

/// Where a searched folder comes from. For one console's check the found files are ranked by
/// it ([`SearchDir::rank`]): the console's emulators' own folders, the folders the user chose,
/// other emulators' own folders and "bios" folders in the user's emulator folders, ROM folders,
/// the home folder's places, then the drives' Emulation folders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// An emulator's own BIOS folder, for this install (emulator instance id).
    Emulator(String),
    /// A folder the user chose (Settings → Emulation → BIOS folders).
    Chosen,
    /// A folder named like "bios" in one of the user's emulator folders (`emulator_dirs`:
    /// `<folder>/bios`, `<folder>/<emulator>/bios`).
    EmulatorFolder,
    /// "bios" / "system" folders in a ROM folder (and `<console>/BIOS`), "*bios*" folders in it.
    RomFolder,
    /// `~/Emulation/bios`, "*bios*" folders and zips in the home folder, Documents, Downloads,
    /// Emulation.
    Home,
    /// `<drive>/…/Emulation/bios` (at most three folders below the drive).
    Drive,
}

/// One folder (or zip) searched for BIOS files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchDir {
    pub path: PathBuf,
    /// How many folder levels inside it are looked at.
    pub depth: usize,
    pub origin: Origin,
}

impl SearchDir {
    /// Its rank for a console whose emulators are `emus` (lower = preferred). Inside one rank the
    /// shortest path wins (see `check::candidates`).
    pub fn rank(&self, emus: &[&Emulator]) -> u8 {
        match &self.origin {
            Origin::Emulator(id) if emus.iter().any(|e| e.id == *id) => 0,
            Origin::Chosen => 1,
            Origin::Emulator(_) | Origin::EmulatorFolder => 2,
            Origin::RomFolder => 3,
            Origin::Home => 4,
            Origin::Drive => 5,
        }
    }
}

/// Folders searched for BIOS files (see [`search_dirs_with_origin`]).
#[cfg(test)]
pub fn search_dirs(reg: &Registry) -> Vec<PathBuf> {
    search_dirs_with_origin(reg).into_iter().map(|d| d.path).collect()
}

#[cfg(test)]
fn search_dirs_depth(reg: &Registry) -> Vec<(PathBuf, usize)> {
    search_dirs_with_origin(reg).into_iter().map(|d| (d.path, d.depth)).collect()
}

/// Folders searched for BIOS files, each once, with how deep it is searched and where it comes
/// from. In this order: the emulators' own BIOS folders (per install: portable copies, AppImage
/// homes, Flatpaks, what their settings name — never left out: they are where the emulator
/// reads), the chosen BIOS folders, folders named like "bios" in the user's emulator folders,
/// "bios" / "system" folders in ROM folders, the home folder's places, and `Emulation/bios`
/// folders on the drives. Folders the user removed are left out, and so is anything found by
/// itself under a folder whose name says it was put aside ("Backups", "unused BIOS", "_old":
/// [`detect::name_says_set_aside`](super::detect::name_says_set_aside)).
///
/// An emulator's own folder is searched only where its BIOS files live: Dolphin's user folder
/// holds texture packs, caches and the Wii NAND (`GC/` is enough), Cemu's holds the installed
/// games (`keys.txt` is at the top).
pub fn search_dirs_with_origin(reg: &Registry) -> Vec<SearchDir> {
    let mut out: Vec<SearchDir> = vec![];
    let mut push = |p: PathBuf, depth: usize, origin: Origin| {
        let p = tidy(&p);
        if (p.is_dir() || p.is_file()) && !out.iter().any(|x| x.path == p) {
            out.push(SearchDir { path: p, depth, origin });
        }
    };
    let h = &reg.home;
    for e in &reg.emulators {
        if let Some(d) = bios_dir(e, h) {
            let own = Origin::Emulator(e.id.clone());
            match e.def.as_str() {
                "dolphin" | "primehack" => push(d.join("GC"), 2, own),
                "cemu" => push(d, 0, own),
                _ => push(d, 3, own),
            }
        }
    }
    for d in &reg.config.bios_dirs {
        push(d.clone(), 3, Origin::Chosen);
    }
    // The user's emulator folders: `<folder>/bios`, `<folder>/<emulator>/bios`.
    for d in &reg.config.emulator_dirs {
        for p in bios_named_cached(std::slice::from_ref(d), 2) {
            push(p, 3, Origin::EmulatorFolder);
        }
    }
    let mut roots: Vec<PathBuf> =
        reg.config.rom_folders.values().filter(|f| f.enabled).map(|f| f.path.clone()).collect();
    roots.extend(super::registry::noted_roots());
    roots.retain(|r| !reg.config.ignored_folders.contains(r));
    const NAMES: &[&str] = &["bios", "BIOS", "Bios", "system", "firmware"];
    for r in &roots {
        for n in NAMES {
            push(r.join(n), 3, Origin::RomFolder);
        }
        // <root>/<console>/BIOS (e.g. ps1/BIOS).
        for sub in std::fs::read_dir(r).into_iter().flatten().flatten() {
            let p = sub.path();
            if p.is_dir() && !set_aside(&sub.file_name().to_string_lossy()) {
                for n in &NAMES[..3] {
                    push(p.join(n), 3, Origin::RomFolder);
                }
            }
        }
    }
    // Folders / zips named like "*bios*" in the ROM folders.
    for p in bios_named_cached(&roots, 3) {
        push(p, 3, Origin::RomFolder);
    }
    for d in ["Emulation/bios", "RetroPie/BIOS", "retrodeck/bios"] {
        push(h.join(d), 3, Origin::Home);
    }
    // Folders / zips named like "*bios*" in the usual places ("~/Documents/bios files",
    // "~/Downloads/ps2-bios-usa.zip").
    let bases: Vec<PathBuf> = ["", "Documents", "Downloads", "Emulation"].iter().map(|d| h.join(d)).collect();
    for p in bios_named_cached(&bases, 3) {
        push(p, 3, Origin::Home);
    }
    // A games drive's `Emulation/bios` (/mnt/X/Gaming/Emulation/bios).
    for p in drive_bios_cached() {
        push(p, 3, Origin::Drive);
    }
    // The emulators' own folders are where they read: always looked at.
    out.retain(|d| matches!(d.origin, Origin::Emulator(_)) || !removed_by_user(reg, &d.path));
    out
}

/// A folder name saying its contents were put aside (backups, unused, old): never searched by
/// itself.
fn set_aside(name: &str) -> bool {
    super::detect::name_says_set_aside(name)
}

/// Inside (or one of) the BIOS folders the user removed, and not inside a folder they chose
/// that lies within that one (the closer folder decides): never searched.
pub(crate) fn removed_by_user(reg: &Registry, p: &Path) -> bool {
    let deepest = |dirs: &[PathBuf]| {
        dirs.iter().map(|d| tidy(d)).filter(|d| p.starts_with(d)).map(|d| d.components().count()).max()
    };
    match (deepest(&reg.config.ignored_bios_dirs), deepest(&reg.config.bios_dirs)) {
        (Some(removed), Some(chosen)) => removed > chosen,
        (Some(_), None) => true,
        _ => false,
    }
}

/// A folder / zip name that says "BIOS": "bios" as a word of its own ("PS2 BIOS", "nds bios",
/// "ps2-bios-usa.zip", "bios2"), or a word ending in it ("psxbios", "ps2bios") — not just the
/// letters inside another word ("Bioshock", "Biostar").
pub(crate) fn names_bios(name: &str) -> bool {
    name.to_lowercase().split(|c: char| !c.is_alphanumeric()).any(|w| {
        w.ends_with("bios")
            || w == "bioses"
            || w.strip_prefix("bios").is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
    })
}

/// `a/b/../c` → `a/c` (lexically, for showing and comparing folders).
pub(crate) fn tidy(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir if out.file_name().is_some() => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

/// Program / cache folders never searched for "*bios*" folders.
const SKIP_DIRS: &[&str] =
    &["node_modules", "target", "steamapps", "compatdata", "drive_c", "pfx", "snap", "__MACOSX", "proc"];

/// Folders (and .zip files) whose name says "bios" ([`names_bios`]), up to `levels` below `base`
/// (hidden, program and put-aside folders skipped; at most `budget` folders looked at). A match
/// isn't looked into.
fn bios_named(base: &Path, levels: usize, budget: &mut usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(base) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if *budget == 0 {
            return;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || SKIP_DIRS.iter().any(|s| name.eq_ignore_ascii_case(s)) || set_aside(&name) {
            continue;
        }
        let lname = name.to_lowercase();
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() || (ft.is_symlink() && p.is_dir()) {
            *budget -= 1;
            if names_bios(&lname) {
                out.push(p);
            } else if levels > 1 && !ft.is_symlink() {
                bios_named(&p, levels - 1, budget, out);
            }
        } else if names_bios(&lname) && lname.ends_with(".zip") {
            out.push(p);
        }
    }
}

type NamedCache = (Vec<PathBuf>, usize, std::time::Instant, Vec<PathBuf>);
static NAMED: Mutex<Vec<NamedCache>> = Mutex::new(vec![]);

/// [`bios_named`] (`levels` deep) over `bases`, cached for a few seconds (a BIOS check runs once
/// per console, all looking at the same folders).
fn bios_named_cached(bases: &[PathBuf], levels: usize) -> Vec<PathBuf> {
    let fresh = |t: &std::time::Instant| t.elapsed() < std::time::Duration::from_secs(10);
    if let Some((_, _, _, v)) = NAMED.lock().iter().find(|(b, l, t, _)| b == bases && *l == levels && fresh(t)) {
        return v.clone();
    }
    let mut out = vec![];
    for b in bases {
        let mut budget = 3000;
        bios_named(b, levels, &mut budget, &mut out);
    }
    out.sort();
    out.dedup();
    let mut cache = NAMED.lock();
    cache.retain(|(b, l, t, _)| fresh(t) && !(b == bases && *l == levels));
    cache.push((bases.to_vec(), levels, std::time::Instant::now(), out.clone()));
    out
}

/// `Emulation/bios` folders on the drives: a folder named "Emulation" at most three folders below
/// a drive (`/mnt/FastSSD/Gaming/Emulation`) with a "bios" folder in it, not under a folder put
/// aside ("Backups", "old"). Nothing else on the drives is searched by itself any more: old
/// emulator copies and backups there held outdated BIOS files that won over the emulator's own.
/// Cached for a few seconds like [`bios_named_cached`].
fn drive_bios_cached() -> Vec<PathBuf> {
    type DriveCache = (Vec<PathBuf>, std::time::Instant, Vec<PathBuf>);
    static DRIVES: Mutex<Option<DriveCache>> = Mutex::new(None);
    let drives = super::roots::drive_bases();
    if let Some((k, t, v)) = DRIVES.lock().as_ref()
        && *k == drives
        && t.elapsed() < std::time::Duration::from_secs(10)
    {
        return v.clone();
    }
    let emulation = super::roots::find_on_drives(&drives, 4, 800, &|n: &str| n == "emulation");
    let mut out = vec![];
    for e in emulation {
        let below = drives.iter().find_map(|d| e.strip_prefix(d).ok()).unwrap_or(&e);
        if below.components().any(|c| set_aside(&c.as_os_str().to_string_lossy())) {
            continue;
        }
        for n in ["bios", "BIOS", "Bios"] {
            let p = e.join(n);
            if p.is_dir() && !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out.sort();
    *DRIVES.lock() = Some((drives, std::time::Instant::now(), out.clone()));
    out
}

// ---------------------------------------------------------------------------------------------
// Checking
// ---------------------------------------------------------------------------------------------

/// RPCS3's `dev_flash` for this install (`roots::rpcs3_dev_flash`).
fn rpcs3_flash(emu: &Emulator, home: &Path) -> PathBuf {
    super::roots::rpcs3_dev_flash(emu, home).unwrap_or_else(|| home.join(".config/rpcs3/dev_flash"))
}

/// RPCS3's firmware version (`dev_flash/vsh/etc/version.txt`: "release:04.92:…"), for this
/// install (portable `portable/`, AppImage home, Flatpak, the `dev_flash` its vfs.yml names).
pub fn ps3_firmware(emu: &Emulator, home: &Path) -> Option<String> {
    if emu.def != "rpcs3" {
        return None;
    }
    let flash = rpcs3_flash(emu, home);
    match std::fs::read_to_string(flash.join("vsh/etc/version.txt")) {
        Ok(text) => {
            let v = text.split(':').nth(1).map(str::trim).unwrap_or("").to_string();
            Some(if v.is_empty() { "installed".into() } else { v })
        }
        // Installed without a readable version file: the system modules are there.
        Err(_) => (flash.join("sys/external").is_dir() || flash.join("vsh/module/vsh.self").is_file())
            .then(|| "installed".into()),
    }
}

/// Switch firmware folder of an emulator (`…/nand/system/Contents/registered`), for this install.
pub fn switch_firmware_dir(emu: &Emulator, home: &Path) -> Option<PathBuf> {
    Some(match emu.def.as_str() {
        "eden" | "citron" | "sudachi" => super::roots::yuzu_nand(emu, home)?.join("system/Contents/registered"),
        "ryujinx" => super::roots::user_dir(emu, home)?.path.join("bis/system/Contents/registered"),
        _ => return None,
    })
}

fn is_nca(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".nca")
}

fn nca_count(dir: &Path) -> usize {
    std::fs::read_dir(dir).into_iter().flatten().flatten().filter(|e| is_nca(&e.file_name().to_string_lossy())).count()
}

fn has_nca(dir: &Path) -> bool {
    nca_count(dir) > 0
}

fn emulators_for<'a>(reg: &'a Registry, platform: &str) -> Vec<&'a Emulator> {
    reg.emulators
        .iter()
        .filter(|e| super::detect::profiles(e, &reg.config).iter().any(|p| p.supports(platform)))
        .collect()
}

/// Same content (files or zip entries)?
fn same_file(a: &Path, b: &Path) -> bool {
    match (size_of(a), size_of(b)) {
        (Some(x), Some(y)) if x == y => x > MAX_HASH || md5_of(a) == md5_of(b),
        _ => false,
    }
}

/// Is version `a` lower than `b`? Numeric parts compared ("04.92" < "4.93", "4.9" < "4.10").
pub fn version_less(a: &str, b: &str) -> bool {
    let parts = |s: &str| -> Vec<u32> { s.split(['.', '-', '_']).map(|x| x.trim().parse().unwrap_or(0)).collect() };
    let (mut x, mut y) = (parts(a), parts(b));
    let n = x.len().max(y.len());
    x.resize(n, 0);
    y.resize(n, 0);
    x < y
}

/// A version in a folder / zip name: "Firmware.23.5.0" → "23.5.0", "ProdKeys.NET-v22.5.0" →
/// "22.5.0" (the first run of at least two dot-separated numbers).
pub fn version_in_name(name: &str) -> Option<String> {
    let b = name.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() && (i == 0 || !b[i - 1].is_ascii_digit()) {
            let mut j = i;
            let mut parts = 1;
            while j < b.len()
                && (b[j].is_ascii_digit() || (b[j] == b'.' && j + 1 < b.len() && b[j + 1].is_ascii_digit()))
            {
                if b[j] == b'.' {
                    parts += 1;
                }
                j += 1;
            }
            if parts >= 2 {
                return Some(name[i..j].to_string());
            }
            i = j;
        }
        i += 1;
    }
    None
}

/// The version inside a `PS3UPDAT.PUP` (its version.txt entry, "4.92").
pub fn pup_version(p: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(p).ok()?;
    let mut head = [0u8; 0x30];
    f.read_exact(&mut head).ok()?;
    if !head.starts_with(b"SCEUF") {
        return None;
    }
    let be = |b: &[u8]| u64::from_be_bytes(b.try_into().unwrap_or([0; 8]));
    let count = be(&head[0x18..0x20]).min(64);
    for i in 0..count {
        let mut e = [0u8; 0x20];
        f.seek(SeekFrom::Start(0x30 + i * 0x20)).ok()?;
        f.read_exact(&mut e).ok()?;
        if be(&e[0..8]) == 0x100 {
            let (off, len) = (be(&e[8..16]), be(&e[16..24]));
            if len == 0 || len > 64 {
                return None;
            }
            let mut v = vec![0u8; len as usize];
            f.seek(SeekFrom::Start(off)).ok()?;
            f.read_exact(&mut v).ok()?;
            let s = String::from_utf8_lossy(&v).trim().to_string();
            return (!s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '.')).then_some(s);
        }
    }
    None
}

/// Write `src` (a file or a file inside a zip) to `dest` through a temporary file.
fn copy_source(src: &Path, dest: &Path) -> anyhow::Result<()> {
    use anyhow::Context;
    let parent = dest.parent().unwrap_or(Path::new("/"));
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let tmp = PathBuf::from(format!("{}.lodestar-part", dest.display()));
    let r = match zipread::entry_of(src) {
        Some((zip, e)) => {
            let data = zipread::read(&zip, &e, 1 << 30).with_context(|| format!("reading {}", src.display()))?;
            std::fs::write(&tmp, data).with_context(|| format!("writing {}", tmp.display()))
        }
        None => std::fs::copy(src, &tmp)
            .map(|_| ())
            .with_context(|| format!("copying {} to {}", src.display(), dest.display())),
    };
    if let Err(e) = r {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, dest).with_context(|| format!("writing {}", dest.display()))
}

/// Where a found file goes for `emulator` (None: it doesn't take it).
pub fn dest_for(reg: &Registry, status: &BiosStatus, emulator: &str, src: &Path) -> Option<PathBuf> {
    let target = status.targets.iter().find(|t| t.emulator == emulator)?;
    let def = reg.emulator(emulator).map(|e| e.def.clone()).unwrap_or_default();
    let (b, _, _) = status.files.iter().find(|(_, s, p)| *s == BiosState::Ok && p.as_deref() == Some(src))?;
    target_rel(&def, b, src).map(|rel| target.dir.join(rel))
}

/// Copy the found BIOS files into `emulator`'s BIOS folder; returns how many were copied.
/// Never replaces a different existing file unless `overwrite`. Files inside zips are
/// extracted (only the one needed).
pub fn install(reg: &Registry, status: &BiosStatus, emulator: &str, overwrite: bool) -> anyhow::Result<usize> {
    let Some(target) = status.targets.iter().find(|t| t.emulator == emulator) else {
        anyhow::bail!("{emulator} has no BIOS folder for this console");
    };
    let def = reg.emulator(emulator).map(|e| e.def.clone()).unwrap_or_default();
    let mut n = 0;
    for src in &target.missing {
        let Some((b, _, _)) = status.files.iter().find(|(_, s, p)| *s == BiosState::Ok && p.as_ref() == Some(src))
        else {
            continue;
        };
        let Some(rel) = target_rel(&def, b, src) else { continue };
        let dest = target.dir.join(rel);
        if dest.exists() && (same_file(src, &dest) || !overwrite) {
            continue;
        }
        copy_source(src, &dest)?;
        n += 1;
    }
    Ok(n)
}

/// The NCA files of a Switch firmware package: (file name, source path or zip entry path).
fn firmware_files(pkg: &Package) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = vec![];
    if pkg.path.is_dir() {
        for e in std::fs::read_dir(&pkg.path).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if is_nca(&name) && e.path().is_file() {
                out.push((name, e.path()));
            }
        }
    } else if let Some(entries) = zipread::list(&pkg.path) {
        for e in entries {
            if is_nca(e.base_name()) && !out.iter().any(|(n, _)| n == e.base_name()) {
                out.push((e.base_name().to_string(), pkg.path.join(&e.name)));
            }
        }
    }
    out.sort();
    out
}

/// Install Switch firmware into an emulator: copy the package's NCA files into its
/// `nand/system/Contents/registered` folder (what its own Tools → Install Firmware does).
/// When it already has firmware, `replace` must be set; its old NCAs are removed first (only
/// after every new file was written). Returns how many files were installed.
pub fn install_firmware(status: &BiosStatus, emulator: &str, replace: bool) -> anyhow::Result<usize> {
    use anyhow::Context;
    let Some(pkg) = status.packages.iter().find(|p| p.kind == PackageKind::SwitchFirmware) else {
        anyhow::bail!("no Switch firmware was found");
    };
    let Some(t) = pkg.targets.iter().find(|t| t.emulator == emulator) else {
        anyhow::bail!("{emulator} can't take Switch firmware");
    };
    let old: Vec<PathBuf> = std::fs::read_dir(&t.dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| is_nca(&e.file_name().to_string_lossy()))
        .map(|e| e.path())
        .collect();
    if !old.is_empty() && !replace {
        anyhow::bail!("it already has firmware installed");
    }
    let files = firmware_files(pkg);
    if files.is_empty() {
        anyhow::bail!("no firmware files in {}", pkg.path.display());
    }
    std::fs::create_dir_all(&t.dir).with_context(|| format!("creating {}", t.dir.display()))?;
    // Write everything next to the target first; only then swap.
    let mut parts: Vec<(PathBuf, PathBuf)> = vec![];
    for (name, src) in &files {
        let dest = t.dir.join(name);
        let part = t.dir.join(format!("{name}.lodestar-part"));
        let r = match zipread::entry_of(src) {
            Some((zip, e)) => zipread::read(&zip, &e, 1 << 30)
                .with_context(|| format!("reading {name} from {}", zip.display()))
                .and_then(|d| std::fs::write(&part, d).with_context(|| format!("writing {}", part.display()))),
            None => std::fs::copy(src, &part).map(|_| ()).with_context(|| format!("copying {}", src.display())),
        };
        if let Err(e) = r {
            let _ = std::fs::remove_file(&part);
            for (p, _) in &parts {
                let _ = std::fs::remove_file(p);
            }
            return Err(e);
        }
        parts.push((part, dest));
    }
    for o in &old {
        let _ = std::fs::remove_file(o);
    }
    for (part, dest) in &parts {
        std::fs::rename(part, dest).with_context(|| format!("writing {}", dest.display()))?;
    }
    Ok(parts.len())
}

/// The command that installs a PS3 firmware file into RPCS3 (`rpcs3 --installfw <PUP>`; the
/// Flatpak is allowed to read the file's folder when it can't already). RPCS3 shows its own
/// progress window; start it only when the user asks.
pub fn ps3_install_spec(emu: &Emulator, home: &Path, pup: &Path) -> crate::model::LaunchSpec {
    let pup_s = pup.to_string_lossy().into_owned();
    let (program, args) = match &emu.kind {
        InstallKind::Flatpak { app_id } => {
            let mut a = vec!["run".to_string()];
            let dir = pup.parent().unwrap_or(Path::new("/"));
            let access = super::detect::flatpak_access(home, app_id);
            if !super::detect::fs_covers(&access, home, pup) {
                a.push(format!("--filesystem={}:ro", dir.display()));
            }
            a.push(app_id.clone());
            a.push("--installfw".into());
            a.push(pup_s);
            ("flatpak".to_string(), a)
        }
        InstallKind::AppImage { path } | InstallKind::Binary { path } => {
            (path.to_string_lossy().into_owned(), vec!["--installfw".to_string(), pup_s])
        }
    };
    crate::model::LaunchSpec::Command { program, args, cwd: None, env: vec![] }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn bios_is_a_word_not_letters_in_another_word() {
        for yes in [
            "bios",
            "BIOS",
            "PS2 BIOS",
            "nds bios",
            "unused BIOS",
            "ps2-bios-usa.zip",
            "psxbios",
            "ps2bios",
            "bios2",
            "BIOSes",
            "bios files",
        ] {
            assert!(names_bios(yes), "{yes}");
        }
        for no in ["Bioshock", "BioShock Infinite", "Biostar", "biosphere", "symbiosis", "Games"] {
            assert!(!names_bios(no), "{no}");
        }
    }

    #[test]
    fn tidy_removes_parent_steps() {
        assert_eq!(tidy(Path::new("/mnt/x/PCSX2/PCSX2.config/PCSX2/../../bios")), PathBuf::from("/mnt/x/PCSX2/bios"));
        assert_eq!(tidy(Path::new("/a/./b")), PathBuf::from("/a/b"));
    }
}
