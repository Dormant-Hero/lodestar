//! Walking a ROM folder and grouping its files into games: playlists first (m3u, then cue / gdi
//! / ccd / mds, hiding the files they list), folder games (PS4, PS3, Wii U, extracted PSP), multi-disc
//! sets, and one game per remaining ROM file whose extension belongs to the console.
//!
//! The walk is bounded: depth ≤ [`MAX_DEPTH`] below the root, at most [`MAX_ENTRIES`] directory
//! entries per scan, hidden / BIOS / save / media folders skipped, other roots and ignored
//! folders skipped.

use super::Role;
use super::identify::{self, Probe};
use super::names;
use super::roots::{Layout, Root};
use crate::emulation::platforms::{self, Platform};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const MAX_DEPTH: u32 = 4;
pub const MAX_ENTRIES: usize = 200_000;

/// Folders that never hold games.
const SKIP_DIRS: &[&str] = &[
    "bios",
    "system",
    "firmware",
    "keys",
    "saves",
    "save",
    "savestates",
    "states",
    "screenshots",
    "cheats",
    "patches",
    "textures",
    "shaders",
    "media",
    "images",
    "videos",
    "manuals",
    "downloaded_images",
    "downloaded_media",
    "snap",
    "snaps",
    "boxart",
    "covers",
    "lost+found",
    "pc",
    // Inside game folders (PS4 sce_module / sce_sys, PS3 USRDIR, Wii U code / content / meta,
    // update / patch / DLC folders): never games of their own.
    "sce_module",
    "sce_sys",
    "usrdir",
    "code",
    "content",
    "meta",
    "update",
    "patch",
    "dlc",
];

/// File names that say how a game starts, not what it is called (`eboot.bin`, `default.xbe`,
/// `game.iso` …): such a game is named after its folder.
const GENERIC_STEMS: &[&str] = &["eboot", "default", "game", "main", "boot", "start", "launcher", "index"];

/// Is this file stem a launch-file name rather than a title?
pub fn generic_stem(stem: &str) -> bool {
    GENERIC_STEMS.iter().any(|g| g.eq_ignore_ascii_case(stem.trim()))
}

/// Size of a PS1 BIOS dump (512 KiB): a `.bin` this size named like one is a BIOS, not a game.
const PS1_BIOS_SIZE: u64 = 512 * 1024;

/// Files that are never ROMs.
const IGNORED_EXTS: &[&str] = &[
    "sav", "srm", "eep", "fla", "mpk", "mcr", "mcd", "ps2", "state", "cht", "ips", "ups", "bps", "xdelta", "ppf",
    "aps", "txt", "nfo", "diz", "md", "pdf", "html", "htm", "jpg", "jpeg", "png", "gif", "bmp", "webp", "xml", "dat",
    "db", "ini", "cfg", "log", "sfv", "md5", "sha1", "torrent", "part", "tmp", "url", "lnk", "!qb", "h3", "tik",
    "cert", "json", "yml", "yaml", "sh", "exe", "dll", "lua",
];

const PATCH_EXTS: &[&str] = &["ips", "ups", "bps", "xdelta", "ppf", "aps"];

/// Extensions of disc images / disc playlists (a serial hit on them is trusted).
pub const DISC_EXTS: &[&str] = &[
    "iso", "bin", "img", "cue", "chd", "m3u", "gdi", "ccd", "mds", "mdf", "cso", "zso", "rvz", "wia", "gcm", "gcz",
    "wbfs", "ciso", "tgc", "wud", "wux", "wua", "pbp", "nrg", "ecm", "toc", "cdi",
];

#[derive(Clone, Debug)]
pub struct FileInfo {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub mtime: i64,
}

/// One game found by the walk, before identification.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub root: usize,
    pub platform: &'static Platform,
    /// What is launched: a file, a playlist, or the game's folder.
    pub rom: PathBuf,
    /// The other files (bins, other discs, attached updates / DLC).
    pub files: Vec<PathBuf>,
    /// The file whose content identifies the game (None: folder game, probed already).
    pub probe_file: Option<FileInfo>,
    /// File names to try as database ROM names.
    pub rom_names: Vec<String>,
    pub folder_game: bool,
    /// The name it is called by when the database doesn't know it (file stem / folder name).
    pub name_src: String,
    /// Not directly in the console folder.
    pub nested: bool,
    /// The console folder it was found in (ES-DE media are per console folder).
    pub console_dir: PathBuf,
    pub size: Option<u64>,
    pub probe: Probe,
    pub role: Option<Role>,
}

/// File extensions the console's emulators accept beyond the platform's own list.
pub type ExtraExts = HashMap<&'static str, HashSet<String>>;

pub fn accepts(p: &Platform, ext: &str, extra: &ExtraExts) -> bool {
    !ext.is_empty() && (p.extensions.contains(&ext) || extra.get(p.id).is_some_and(|s| s.contains(ext)))
}

pub struct Walker<'a> {
    pub entries: usize,
    pub capped: bool,
    /// Other roots and ignored folders.
    pub skip: &'a HashSet<PathBuf>,
    pub extra: &'a ExtraExts,
    pub out: Vec<Candidate>,
    root_idx: usize,
    root_path: PathBuf,
    /// The root's real folder (other roots in `skip` are real paths too).
    root_real: PathBuf,
    exclude: Vec<String>,
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn hidden(name: &str) -> bool {
    name.starts_with('.') || name.starts_with('$') || name.starts_with('＄')
}

/// Does one of a ROM folder's exclusions match this `/`-separated path relative to the folder?
/// Matched like `exclude::glob_match` (`*` = any run of characters, the whole text must match),
/// ignoring case; a pattern without `/` may also match any single part of the path
/// (`*(Beta)*` hides "ps2/Game (Beta).iso", `ps2/Unfinished/*` only that folder). The import
/// dialog and Settings use the same rule, so what they leave out is what the scan leaves out.
pub fn excluded_by(patterns: &[String], rel: &str) -> bool {
    let rel = rel.trim_matches('/').to_lowercase();
    patterns.iter().any(|pat| {
        let pat = pat.trim().trim_matches('/').to_lowercase();
        if pat.is_empty() || pat.chars().all(|c| c == '*') {
            return false;
        }
        crate::exclude::glob_match(&pat, &rel)
            || (!pat.contains('/') && rel.split('/').any(|c| crate::exclude::glob_match(&pat, c)))
    })
}

impl<'a> Walker<'a> {
    pub fn new(skip: &'a HashSet<PathBuf>, extra: &'a ExtraExts) -> Walker<'a> {
        Walker {
            entries: 0,
            capped: false,
            skip,
            extra,
            out: vec![],
            root_idx: 0,
            root_path: PathBuf::new(),
            root_real: PathBuf::new(),
            exclude: vec![],
        }
    }

    fn excluded(&self, p: &Path) -> bool {
        if self.exclude.is_empty() {
            return false;
        }
        let rel = p.strip_prefix(&self.root_path).unwrap_or(p).to_string_lossy().into_owned();
        excluded_by(&self.exclude, &rel)
    }

    /// Walk one root.
    pub fn walk_root(&mut self, idx: usize, root: &Root) {
        self.root_idx = idx;
        self.root_path = root.path.clone();
        self.root_real = root.real.clone();
        self.exclude = root.folder.as_ref().map(|f| f.exclude.clone()).unwrap_or_default();
        match root.layout {
            Layout::Platform(p) => self.walk_dir(&root.path, Some(p), 0, &root.path.clone()),
            Layout::Consoles => {
                let Some((files, dirs)) = self.list(&root.path) else { return };
                for d in dirs {
                    let n = file_name(&d);
                    if self.skip_dir(&d, &n) {
                        continue;
                    }
                    if let Some(p) = platforms::by_folder_name(&n) {
                        self.walk_dir(&d, Some(p), 1, &d.clone());
                    }
                }
                // Loose files next to the console folders: only extensions that alone say the console.
                self.group(&root.path, files, None, &root.path.clone());
            }
        }
    }

    fn skip_dir(&self, d: &Path, name: &str) -> bool {
        hidden(name)
            || SKIP_DIRS.iter().any(|s| s.eq_ignore_ascii_case(name))
            || self.skip.contains(d)
            || self.skip.contains(&self.real_of(d))
            || self.excluded(d)
    }

    /// `d` (below the root) as a path below the root's real folder.
    fn real_of(&self, d: &Path) -> PathBuf {
        match d.strip_prefix(&self.root_path) {
            Ok(rest) if self.root_real != self.root_path => self.root_real.join(rest),
            _ => d.to_path_buf(),
        }
    }

    fn list(&mut self, dir: &Path) -> Option<(Vec<FileInfo>, Vec<PathBuf>)> {
        if self.capped {
            return None;
        }
        let rd = std::fs::read_dir(dir).ok()?;
        let (mut files, mut dirs) = (vec![], vec![]);
        for e in rd.flatten() {
            self.entries += 1;
            if self.entries > MAX_ENTRIES {
                if !self.capped {
                    log::warn!("emulation: stopped after {MAX_ENTRIES} folder entries (in {})", dir.display());
                }
                self.capped = true;
                break;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if hidden(&name) {
                continue;
            }
            let Ok(mut ft) = e.file_type() else { continue };
            let path = e.path();
            let meta = if ft.is_symlink() {
                let Ok(m) = std::fs::metadata(&path) else { continue };
                ft = m.file_type();
                Some(m)
            } else {
                None
            };
            if ft.is_dir() {
                dirs.push(path);
            } else if ft.is_file() {
                let ext = names::ext(&name);
                if IGNORED_EXTS.contains(&ext.as_str()) && !name.eq_ignore_ascii_case("meta.xml") {
                    // Keep patch files visible to the patch-kit check.
                    if PATCH_EXTS.contains(&ext.as_str()) {
                        files.push(FileInfo { path, name, size: 0, mtime: 0 });
                    }
                    continue;
                }
                let Some(m) = meta.or_else(|| e.metadata().ok()) else { continue };
                let mtime = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos() as i64)
                    .unwrap_or(0);
                files.push(FileInfo { path, name, size: m.len(), mtime });
            }
        }
        files.sort_by(|a, b| a.name.cmp(&b.name));
        dirs.sort();
        Some((files, dirs))
    }

    fn walk_dir(&mut self, dir: &Path, plat: Option<&'static Platform>, depth: u32, console_dir: &Path) {
        let Some((files, dirs)) = self.list(dir) else { return };
        if depth > 0
            && dir != console_dir
            && let Some(c) = self.folder_game(dir, &files, &dirs, plat, console_dir)
        {
            self.out.push(c);
            return;
        }
        // A ROM-hack patch kit (patch files next to a copy of the original ROM): not games,
        // except a ROM named like the patch or the folder (the patched result kept with its
        // patch: "Radical Red/Radical Red.gba" + "Radical Red.ups").
        if dir != console_dir && files.iter().any(|f| PATCH_EXTS.contains(&names::ext(&f.name).as_str())) {
            let keys: HashSet<String> = files
                .iter()
                .filter(|f| PATCH_EXTS.contains(&names::ext(&f.name).as_str()))
                .map(|f| names::key(names::stem(&f.name)))
                .chain(std::iter::once(names::key(&file_name(dir))))
                .collect();
            let kept: Vec<FileInfo> = files
                .into_iter()
                .filter(|f| {
                    !PATCH_EXTS.contains(&names::ext(&f.name).as_str())
                        && keys.contains(&names::key(names::stem(&f.name)))
                })
                .collect();
            self.group(dir, kept, plat, console_dir);
            return;
        }
        for d in dirs {
            let n = file_name(&d);
            if self.skip_dir(&d, &n) || depth + 1 > MAX_DEPTH {
                continue;
            }
            let before = self.out.len();
            self.walk_dir(&d, plat, depth + 1, console_dir);
            // A folder holding exactly one game (Name/Name.cue + Name.bin) is named after the folder.
            if self.out.len() == before + 1
                && !self.out[before].folder_game
                && self.out[before].rom.parent() == Some(d.as_path())
            {
                let g = &mut self.out[before];
                if names::key(&g.name_src) == names::key(&n) || names::disc(&g.name_src).is_some() {
                    g.name_src = n;
                }
            }
        }
        self.group(dir, files, plat, console_dir);
    }

    /// PS4 game folders, PS3 disc / HDD folders, Wii U (extracted or NUS) and extracted PSP games.
    fn folder_game(
        &self,
        dir: &Path,
        files: &[FileInfo],
        dirs: &[PathBuf],
        plat: Option<&'static Platform>,
        console_dir: &Path,
    ) -> Option<Candidate> {
        let has_dir = |n: &str| dirs.iter().any(|d| file_name(d).eq_ignore_ascii_case(n));
        let file = |n: &str| files.iter().find(|f| f.name.eq_ignore_ascii_case(n));
        let pick = |id: &str| platforms::by_id(id).or(plat.filter(|p| p.folder_games && p.id == id));
        let mut probe = Probe::default();
        let platform;
        let sce_sfo = || {
            let sys = dirs.iter().find(|d| file_name(d).eq_ignore_ascii_case("sce_sys"))?;
            std::fs::read_dir(sys)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .find(|p| file_name(p).eq_ignore_ascii_case("param.sfo"))
        };
        if file("eboot.bin").is_some()
            && let Some(sfo) = sce_sfo()
        {
            // PS4 (shadPS4): <folder>/eboot.bin + sce_sys/param.sfo; updates are CATEGORY "gp"
            // (often a "-UPDATE" / "-patch" folder next to the game).
            platform = pick("sony_playstation4")?;
            let v = identify::sfo_file(&sfo);
            probe.serial = v.get("TITLE_ID").cloned().filter(|s| !s.trim().is_empty());
            probe.title = v.get("TITLE").map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
            let lower = file_name(dir).to_ascii_lowercase();
            let cat = v.get("CATEGORY").map(|c| c.trim().to_ascii_lowercase()).unwrap_or_default();
            if cat == "gp" || lower.ends_with("-update") || lower.ends_with("-patch") || lower.ends_with("_update") {
                probe.role = Some(Role::Update);
            } else if cat == "ac" {
                probe.role = Some(Role::Dlc);
            }
        } else if has_dir("PS3_GAME") && dir.join("PS3_GAME/PARAM.SFO").is_file()
            || file("PARAM.SFO").is_some() && has_dir("USRDIR")
        {
            platform = pick("sony_playstation3")?;
            let sfo = if has_dir("PS3_GAME") { dir.join("PS3_GAME/PARAM.SFO") } else { dir.join("PARAM.SFO") };
            let v = identify::sfo_file(&sfo);
            probe.serial = v.get("TITLE_ID").cloned();
            probe.title = v.get("TITLE").cloned();
            if v.get("CATEGORY").is_some_and(|c| c == "GD") {
                probe.role = Some(Role::Update);
            }
        } else if has_dir("code") && has_dir("content") && has_dir("meta") {
            platform = pick("nintendo_wiiu")?;
            probe = identify::wiiu_meta(&dir.join("meta/meta.xml"));
        } else if let Some(tmd) = files.iter().find(|f| {
            let l = f.name.to_ascii_lowercase();
            l == "title.tmd"
                || l.strip_prefix("tmd.").is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        }) {
            platform = pick("nintendo_wiiu")?;
            if let Some((id, role)) = identify::wiiu_tmd(&tmd.path) {
                probe.title_id = Some(id);
                probe.role = role;
            }
        } else if has_dir("PSP_GAME") || file("EBOOT.PBP").is_some() && plat.is_none_or(|p| p.id == "sony_psp") {
            platform = pick("sony_psp")?;
            if has_dir("PSP_GAME") {
                let v = identify::sfo_file(&dir.join("PSP_GAME/PARAM.SFO"));
                probe.serial = v.get("DISC_ID").map(|s| identify::dash_serial(s));
                probe.title = v.get("TITLE").cloned();
            } else if let Some(f) = file("EBOOT.PBP") {
                probe = identify::probe_file(&f.path, f.size);
            }
        } else {
            return None;
        }
        let name = file_name(dir);
        Some(Candidate {
            root: self.root_idx,
            platform,
            rom: dir.to_path_buf(),
            files: vec![],
            probe_file: None,
            rom_names: vec![],
            folder_game: true,
            role: probe.role.or_else(|| names::addon_tag(&name)),
            name_src: name,
            nested: dir.parent() != Some(console_dir),
            console_dir: console_dir.to_path_buf(),
            size: None,
            probe,
        })
    }

    /// The files of a folder that can be (part of) a game: no BIOS dumps, ignored kinds or
    /// files the user excluded.
    fn game_files(&self, files: Vec<FileInfo>) -> Vec<FileInfo> {
        files
            .into_iter()
            .filter(|f| {
                let ext = names::ext(&f.name);
                let lower = f.name.to_ascii_lowercase();
                let bios = lower.contains("[bios]")
                    || ext == "bin"
                        && f.size == PS1_BIOS_SIZE
                        && ["scph", "ps-", "psx", "psone", "ps1"].iter().any(|p| lower.starts_with(p));
                !IGNORED_EXTS.contains(&ext.as_str()) && !bios && !self.excluded(&f.path)
            })
            .collect()
    }

    /// Group a folder's files into games.
    fn group(&mut self, dir: &Path, files: Vec<FileInfo>, plat: Option<&'static Platform>, console_dir: &Path) {
        let files = self.game_files(files);
        let by_name: HashMap<String, usize> =
            files.iter().enumerate().map(|(i, f)| (f.name.to_lowercase(), i)).collect();
        let mut used = vec![false; files.len()];
        let mut found: Vec<Candidate> = vec![];
        // Files an m3u lists in sub-folders (already walked): their games are the m3u's.
        let mut listed_elsewhere: HashSet<PathBuf> = HashSet::new();
        let resolve = |rel: &str| -> Option<PathBuf> {
            let rel = rel.trim().replace('\\', "/");
            if rel.is_empty() {
                return None;
            }
            let p = dir.join(&rel);
            if !rel.contains('/') {
                if let Some(&i) = by_name.get(&rel.to_lowercase()) {
                    return Some(files[i].path.clone());
                }
            } else if p.exists() {
                return Some(p);
            }
            p.exists().then_some(p)
        };
        let mark = |used: &mut Vec<bool>, p: &Path| {
            if p.parent() == Some(dir)
                && let Some(&i) = by_name.get(&file_name(p).to_lowercase())
            {
                used[i] = true;
            }
        };
        // Playlists: m3u first, then the sheets (cue / gdi / ccd / mds).
        for pass in [&["m3u"][..], &["cue", "gdi", "ccd", "mds"][..]] {
            for i in 0..files.len() {
                let ext = names::ext(&files[i].name);
                if used[i] || !pass.contains(&ext.as_str()) {
                    continue;
                }
                used[i] = true;
                let mut children: Vec<PathBuf> = vec![];
                let mut data: Option<PathBuf> = None;
                let refs = sheet_refs(&files[i].path, &ext, &resolve);
                for (child, is_data) in refs {
                    if children.contains(&child) {
                        continue;
                    }
                    mark(&mut used, &child);
                    if is_data && data.is_none() && ext != "m3u" {
                        data = Some(child.clone());
                    }
                    children.push(child.clone());
                    let cext = names::ext(&file_name(&child));
                    // An m3u that lists sheets: hide (and keep) the sheets' files too.
                    if ext == "m3u" && ["cue", "gdi", "ccd", "mds"].contains(&cext.as_str()) {
                        let cdir = child.parent().unwrap_or(dir).to_path_buf();
                        let sub = |r: &str| -> Option<PathBuf> {
                            if cdir == dir {
                                resolve(r)
                            } else {
                                let p = cdir.join(r.trim().replace('\\', "/"));
                                p.exists().then_some(p)
                            }
                        };
                        for (gc, gdata) in sheet_refs(&child, &cext, &sub) {
                            mark(&mut used, &gc);
                            if gdata && data.is_none() {
                                data = Some(gc.clone());
                            }
                            if !children.contains(&gc) {
                                children.push(gc);
                            }
                        }
                    } else if ext == "m3u" && data.is_none() {
                        data = Some(child.clone());
                    }
                }
                if ext == "m3u" {
                    listed_elsewhere.extend(children.iter().filter(|c| c.parent() != Some(dir)).cloned());
                }
                let lead = &files[i];
                let probe_path = data.clone().unwrap_or_else(|| lead.path.clone());
                found.push(self.candidate(dir, lead, children, Some(probe_path), plat, console_dir, &files));
            }
        }
        if !listed_elsewhere.is_empty() {
            self.out.retain(|c| !listed_elsewhere.contains(&c.rom));
        }
        for (i, f) in files.iter().enumerate() {
            if !used[i] && !PATCH_EXTS.contains(&names::ext(&f.name).as_str()) {
                found.push(self.candidate(dir, f, vec![], Some(f.path.clone()), plat, console_dir, &files));
            }
        }
        // Console per game: the folder's console when it takes the extension, else the one the
        // extension alone stands for (a stray .z64 in gc/ is an N64 game), else not a game.
        let mut games: Vec<Candidate> = found
            .into_iter()
            .filter_map(|mut c| {
                let ext = names::ext(&file_name(&c.rom));
                let p = match plat {
                    Some(p) if accepts(p, &ext, self.extra) => Some(p),
                    _ => platforms::by_unique_extension(&ext),
                }?;
                c.platform = p;
                Some(c)
            })
            .collect();
        merge_disc_sets(&mut games);
        self.out.extend(games);
    }

    #[allow(clippy::too_many_arguments)]
    fn candidate(
        &self,
        dir: &Path,
        lead: &FileInfo,
        children: Vec<PathBuf>,
        probe: Option<PathBuf>,
        plat: Option<&'static Platform>,
        console_dir: &Path,
        files: &[FileInfo],
    ) -> Candidate {
        let info = |p: &Path| -> Option<FileInfo> {
            files.iter().find(|f| f.path == p).cloned().or_else(|| {
                let m = std::fs::metadata(p).ok()?;
                let mtime = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos() as i64;
                Some(FileInfo { path: p.to_path_buf(), name: file_name(p), size: m.len(), mtime })
            })
        };
        let size: u64 = lead.size + children.iter().filter_map(|c| info(c)).map(|f| f.size).sum::<u64>();
        let mut rom_names = vec![lead.name.clone()];
        rom_names.extend(children.iter().map(|c| file_name(c)));
        let mut stem = names::stem(&lead.name).to_string();
        // "eboot.bin", "default.xbe", "game.iso": the folder says what it is.
        if generic_stem(&stem)
            && dir != console_dir
            && let Some(folder) = dir.file_name()
        {
            stem = folder.to_string_lossy().into_owned();
        }
        Candidate {
            root: self.root_idx,
            platform: plat.unwrap_or(&platforms::all()[0]),
            rom: lead.path.clone(),
            files: children,
            probe_file: probe.as_deref().and_then(info),
            rom_names,
            folder_game: false,
            role: names::addon_tag(&stem),
            name_src: stem,
            nested: dir != console_dir,
            console_dir: console_dir.to_path_buf(),
            size: Some(size),
            probe: Probe::default(),
        }
    }
}

/// Files a playlist / cue sheet lists, with "is a data track" (the serial source).
fn sheet_refs(path: &Path, ext: &str, resolve: &dyn Fn(&str) -> Option<PathBuf>) -> Vec<(PathBuf, bool)> {
    let Ok(text) = crate::scan::util::read_text(path) else { return vec![] };
    if text.len() > 256 * 1024 {
        return vec![];
    }
    let mut out: Vec<(PathBuf, bool)> = vec![];
    match ext {
        "m3u" => {
            for l in text.lines().map(|l| l.trim().trim_start_matches('\u{feff}')) {
                if l.is_empty() || l.starts_with('#') {
                    continue;
                }
                if let Some(p) = resolve(l) {
                    out.push((p, true));
                } else {
                    log::info!("emulation: {} lists a missing file {l}", path.display());
                }
            }
        }
        "cue" => {
            let mut current: Option<usize> = None;
            for l in text.lines().map(str::trim) {
                let upper = l.to_ascii_uppercase();
                if let Some(rest) = upper.strip_prefix("FILE ") {
                    let raw = &l[l.len() - rest.len()..];
                    let name = if let Some(q) = raw.strip_prefix('"') {
                        q.split('"').next().unwrap_or("")
                    } else {
                        raw.rsplit_once(' ').map(|(n, _)| n).unwrap_or(raw)
                    };
                    current = resolve(name).map(|p| {
                        out.push((p, false));
                        out.len() - 1
                    });
                } else if upper.starts_with("TRACK ")
                    && upper.contains("MODE")
                    && let Some(i) = current
                    && !out.iter().any(|(_, d)| *d)
                {
                    out[i].1 = true;
                }
            }
        }
        "gdi" => {
            for l in text.lines().skip(1) {
                let parts = shell_words(l);
                if parts.len() >= 5
                    && let Some(p) = resolve(&parts[4])
                {
                    let data = parts[2] == "4";
                    out.push((p, data && !out.iter().any(|(_, d)| *d)));
                }
            }
        }
        _ => {}
    }
    // ccd / mds: their companions share the stem.
    if ext == "ccd" || ext == "mds" {
        let stem = names::stem(&file_name(path)).to_string();
        let comps: &[&str] = if ext == "ccd" { &["img", "sub"] } else { &["mdf"] };
        for (i, c) in comps.iter().enumerate() {
            if let Some(p) = resolve(&format!("{stem}.{c}")) {
                out.push((p, i == 0));
            }
        }
    }
    out
}

fn shell_words(l: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    for c in l.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn is_disc(c: &Candidate) -> bool {
    c.folder_game || DISC_EXTS.contains(&names::ext(&file_name(&c.rom)).as_str())
}

/// Multi-disc sets without an m3u become one game, led by its first disc: "(Disc 1)" +
/// "(Disc 2)" … with the same title and the same release (region, version …). Other tags may
/// differ ("Gran Turismo 2 (Gran Turismo Mode) (Disc 1)" + "(Arcade Mode) (Disc 2)") as long as
/// the disc numbers do; else only discs with all the same tags.
fn merge_disc_sets(games: &mut Vec<Candidate>) {
    let mut loose: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, g) in games.iter().enumerate() {
        if names::ext(&file_name(&g.rom)) != "m3u" && names::disc(&g.name_src).is_some() {
            loose.entry((g.platform.id.to_string(), names::disc_release_key(&g.name_src))).or_default().push(i);
        }
    }
    let mut sets: Vec<Vec<usize>> = vec![];
    for idx in loose.into_values().filter(|v| v.len() > 1) {
        let mut nums: Vec<u32> = idx.iter().map(|&i| names::disc(&games[i].name_src).unwrap_or(0)).collect();
        nums.sort_unstable();
        nums.dedup();
        if nums.len() == idx.len() {
            sets.push(idx);
            continue;
        }
        let mut strict: HashMap<String, Vec<usize>> = HashMap::new();
        for i in idx {
            strict.entry(names::disc_group_key(&games[i].name_src)).or_default().push(i);
        }
        sets.extend(strict.into_values().filter(|v| v.len() > 1));
    }
    let mut drop = vec![false; games.len()];
    for idx in sets {
        let mut idx = idx;
        idx.sort_by_key(|&i| names::disc(&games[i].name_src).unwrap_or(99));
        let lead = idx[0];
        for &i in &idx[1..] {
            let (rom, files, size) = (games[i].rom.clone(), games[i].files.clone(), games[i].size);
            let g = &mut games[lead];
            g.files.push(rom);
            g.files.extend(files);
            g.size = g.size.zip(size).map(|(a, b)| a + b);
            drop[i] = true;
        }
    }
    let mut k = 0;
    games.retain(|_| {
        k += 1;
        !drop[k - 1]
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusion_globs() {
        let p = |x: &str| vec![x.to_string()];
        assert!(excluded_by(&p("*.txt"), "a.TXT"));
        assert!(excluded_by(&p("hacks/*"), "hacks/x.gba"));
        assert!(!excluded_by(&p("*.gba"), "x.gb"));
    }
}
