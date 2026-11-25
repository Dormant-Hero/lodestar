//! `.desktop` entry scanner: every `Type=Application` entry with `Categories=…Game…` in the XDG
//! application directories (+ Flatpak exports and `scan.extra_desktop_dirs`).
//!
//! * Desktop file ids follow the XDG spec (path relative to the `applications` dir, `/` → `-`);
//!   an id found in an earlier directory shadows the same id in later ones (even when the earlier
//!   one is hidden — that's how users hide system entries).
//! * Entries that are shortcuts into another launcher (Steam, Faugus, Lutris, Heroic) get
//!   `duplicate_of` so the orchestrator can drop them and lend their icon to the real game.
//!   Such entries are never removed by `scan.desktop_ignore`.
//! * Emulators are skipped (`Categories=…Emulator…`, or a known emulator by Flatpak id / program /
//!   AppImage / name, whatever its Categories): they are set up in Settings → Emulation.
//!
//! CONTRACT: `pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>>` (see scan/mod.rs).

pub(crate) mod common;

use super::ScanContext;
use crate::model::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut icons: Option<IconIndex> = None;
    let mut out = vec![];
    for dir in application_dirs(ctx) {
        let mut files = vec![];
        collect_desktop_files(&dir, &dir, 0, &mut files);
        files.sort();
        for (id, path) in files {
            if !seen.insert(id.clone()) {
                continue; // shadowed by an earlier directory
            }
            let Ok(text) = read_small(&path) else {
                continue;
            };
            let Some(entry) = DesktopEntry::parse(&text) else {
                continue;
            };
            if let Some(g) = entry_to_game(ctx, &id, &entry, &mut icons) {
                out.push(g);
            }
        }
    }
    mark_stale_steam_shortcuts(ctx, &mut out);
    Ok(out)
}

/// Steam leaves `steam://rungameid/<id>` desktop shortcuts behind when a game is uninstalled.
/// Those never dedupe against the Steam scanner (it only reports installed games), so mark them
/// not installed when Steam exists but no library has the game's manifest.
fn mark_stale_steam_shortcuts(ctx: &ScanContext, games: &mut [ScannedGame]) {
    let wanted = |g: &ScannedGame| match &g.duplicate_of {
        Some((Source::Steam, id)) => id.parse::<u32>().ok(),
        _ => None,
    };
    if !games.iter().any(|g| wanted(g).is_some()) {
        return;
    }
    let libs: Vec<PathBuf> = super::steam::find_roots(ctx)
        .iter()
        .flat_map(|r| super::steam::library_folders(&r.path))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if libs.is_empty() {
        return;
    }
    for g in games.iter_mut() {
        if let Some(appid) = wanted(g) {
            let manifest = format!("steamapps/appmanifest_{appid}.acf");
            if !libs.iter().any(|l| l.join(&manifest).is_file()) {
                g.installed = false;
            }
        }
    }
}

/// Application directories in precedence order (earlier shadows later), de-duplicated.
pub fn application_dirs(ctx: &ScanContext) -> Vec<PathBuf> {
    let mut dirs =
        vec![ctx.data_home().join("applications"), ctx.data_home().join("flatpak/exports/share/applications")];
    dirs.extend(ctx.xdg_data_dirs.iter().map(|d| d.join("applications")));
    dirs.extend(ctx.config.extra_desktop_dirs.iter().cloned());
    let mut seen = HashSet::new();
    dirs.retain(|d| {
        let key = std::fs::canonicalize(d).unwrap_or_else(|_| d.clone());
        d.is_dir() && seen.insert(key)
    });
    dirs
}

/// Read one `.desktop` file (Add Game → Pick a program / Detect installed). `None` when it can't
/// be read or has no `[Desktop Entry]` group.
pub fn read_entry_file(path: &Path) -> Option<DesktopEntry> {
    DesktopEntry::parse(&read_small(path).ok()?)
}

/// The launch of an entry as the scanner builds it (`env VAR=… program args` split, `Path=` as the
/// working folder), plus whether it runs a Windows program. No category / ignore-list filters.
pub fn entry_launch(e: &DesktopEntry) -> Option<(LaunchSpec, bool)> {
    let argv = parse_exec(e.get("Exec")?);
    if argv.is_empty() {
        return None;
    }
    let windows = is_windows_command(&argv);
    let cwd = e.string("Path").map(PathBuf::from).filter(|p| p.is_absolute());
    let (program, args, env) = split_env(argv);
    Some((LaunchSpec::Command { program, args, cwd, env }, windows))
}

/// Resolves `Icon=` names like the scanner does; the hicolor index is built on first use and kept.
#[derive(Default)]
pub struct IconResolver {
    idx: Option<IconIndex>,
}

impl IconResolver {
    /// (file, is_large) for an icon name or absolute path.
    pub fn resolve(&mut self, ctx: &ScanContext, icon: &str) -> Option<(PathBuf, bool)> {
        resolve_icon(ctx, icon, &mut self.idx)
    }
}

fn read_small(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let meta = std::fs::metadata(path)?;
    // A FIFO/device named *.desktop would block (or never end) a plain read.
    if !meta.is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    if meta.len() > 512 * 1024 {
        return Err(std::io::Error::other("desktop file too large"));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    std::fs::File::open(path)?.take(512 * 1024).read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Recursively list `*.desktop` files with their desktop file id.
fn collect_desktop_files(root: &Path, dir: &Path, depth: usize, out: &mut Vec<(String, PathBuf)>) {
    if depth > 4 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        let is_dir = ft.is_dir() || (ft.is_symlink() && p.is_dir());
        if is_dir {
            collect_desktop_files(root, &p, depth + 1, out);
        } else if p.extension().is_some_and(|x| x == "desktop")
            && let Ok(rel) = p.strip_prefix(root)
        {
            let id = rel.to_string_lossy().replace('/', "-");
            out.push((id, p));
        }
    }
}

/// The `[Desktop Entry]` group of a desktop file (unlocalised keys only).
#[derive(Clone, Debug, Default)]
pub struct DesktopEntry {
    pub keys: HashMap<String, String>,
}

impl DesktopEntry {
    pub fn parse(text: &str) -> Option<DesktopEntry> {
        let mut in_main = false;
        let mut found = false;
        let mut keys = HashMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') {
                in_main = line == "[Desktop Entry]";
                found |= in_main;
                continue;
            }
            if !in_main {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let k = k.trim();
            if k.contains('[') {
                continue; // localised variant
            }
            keys.entry(k.to_string()).or_insert_with(|| v.trim().to_string());
        }
        found.then_some(DesktopEntry { keys })
    }

    pub fn get(&self, k: &str) -> Option<&str> {
        self.keys.get(k).map(|s| s.as_str()).filter(|s| !s.is_empty())
    }

    pub fn bool(&self, k: &str) -> bool {
        self.get(k).is_some_and(|v| v.eq_ignore_ascii_case("true"))
    }

    /// Unescaped string value (`\s \n \t \r \\`).
    pub fn string(&self, k: &str) -> Option<String> {
        self.get(k).map(unescape_string)
    }

    pub fn list(&self, k: &str) -> Vec<String> {
        self.get(k)
            .map(|v| v.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default()
    }
}

fn entry_to_game(ctx: &ScanContext, id: &str, e: &DesktopEntry, icons: &mut Option<IconIndex>) -> Option<ScannedGame> {
    if e.get("Type") != Some("Application") || e.bool("Hidden") || e.bool("NoDisplay") {
        return None;
    }
    let cats = e.list("Categories");
    if !cats.iter().any(|c| c.eq_ignore_ascii_case("Game")) {
        return None;
    }
    if let Some(try_exec) = e.string("TryExec") {
        let p = Path::new(&try_exec);
        if p.is_absolute() && !p.exists() {
            return None;
        }
    }
    let name = e.string("Name")?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let argv = parse_exec(e.get("Exec")?);
    if argv.is_empty() {
        return None;
    }
    let duplicate_of = shortcut_target(&argv);
    if duplicate_of.is_none() {
        // Store clients / launchers (itch, Moonlight, Prism …) are not games, whatever the
        // user's ignore list says.
        if super::clients::is_launcher_app_id(id) || flatpak_app(&argv).is_some_and(super::clients::is_launcher_app_id)
        {
            return None;
        }
        let lname = name.to_lowercase();
        if ctx.config.desktop_ignore.iter().any(|i| !i.trim().is_empty() && lname.contains(&i.trim().to_lowercase())) {
            return None;
        }
        if cats.iter().any(|c| c.eq_ignore_ascii_case("Emulator")) {
            return None;
        }
        // Known emulators are not games, whatever their Categories say (PPSSPP lists itself
        // as a game on some systems).
        let stem = id.strip_suffix(".desktop").unwrap_or(id);
        let stem = stem.rsplit('/').next().unwrap_or(stem);
        if crate::emulation::defs::emulator_app(stem, &argv, &name).is_some() {
            return None;
        }
    }

    let mut g = ScannedGame::new(Source::Desktop, id, name);
    g.duplicate_of = duplicate_of;
    let cwd = e.string("Path").map(PathBuf::from).filter(|p| p.is_absolute());
    g.install_dir = cwd.clone();
    g.platform = Some(if is_windows_command(&argv) { "windows" } else { "linux" }.into());
    let (program, args, env) = split_env(argv);
    let prog_path = Path::new(&program);
    if prog_path.is_absolute() && !prog_path.exists() {
        g.installed = false;
    }
    g.launch = Some(LaunchSpec::Command { program, args, cwd, env });
    if let Some(desc) = e.string("Comment")
        && !desc.starts_with("Play this game on Steam")
    {
        g.meta.description = Some(desc);
    }
    if let Some(icon) = e.string("Icon")
        && let Some((path, large)) = resolve_icon(ctx, &icon, icons)
    {
        if large {
            g.art.cover = Some(path.clone());
        }
        g.art.icon = Some(path);
    }
    Some(g)
}

/// Unescape a desktop-file `string` value. Unknown escapes are kept verbatim so the Exec-level
/// unescaping (`\"`, `` \` ``, `\$`, `\\`) still sees them.
pub fn unescape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.peek().copied() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => {
                out.push('\\');
                continue;
            }
        }
        it.next();
    }
    out
}

/// Split a desktop `Exec` value into argv: string unescape, double-quote handling with the
/// spec's escapes, field codes removed (`%%` → `%`), Flatpak `@@`/`@@u` forwarding markers dropped.
pub fn parse_exec(raw: &str) -> Vec<String> {
    let s = unescape_string(raw);
    let mut args: Vec<String> = vec![];
    let mut cur = String::new();
    let mut in_token = false;
    let mut only_codes = true; // token consists solely of field codes so far
    let mut in_quotes = false;
    let mut it = s.chars().peekable();
    let flush = |cur: &mut String, in_token: &mut bool, only_codes: &mut bool, args: &mut Vec<String>| {
        if *in_token && !(*only_codes && cur.is_empty()) {
            args.push(std::mem::take(cur));
        }
        cur.clear();
        *in_token = false;
        *only_codes = true;
    };
    while let Some(c) = it.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                in_token = true;
                only_codes = false;
            }
            '\\' => {
                in_token = true;
                only_codes = false;
                match it.peek().copied() {
                    Some(n @ ('"' | '`' | '$' | '\\')) => {
                        cur.push(n);
                        it.next();
                    }
                    Some(' ') if !in_quotes => {
                        cur.push(' ');
                        it.next();
                    }
                    _ => cur.push('\\'),
                }
            }
            '%' => {
                in_token = true;
                match it.peek().copied() {
                    Some('%') => {
                        cur.push('%');
                        only_codes = false;
                        it.next();
                    }
                    Some('f' | 'F' | 'u' | 'U' | 'i' | 'c' | 'k' | 'd' | 'D' | 'n' | 'N' | 'v' | 'm') => {
                        it.next();
                    }
                    _ => {
                        cur.push('%');
                        only_codes = false;
                    }
                }
            }
            c if c.is_whitespace() && !in_quotes => {
                flush(&mut cur, &mut in_token, &mut only_codes, &mut args);
            }
            c => {
                cur.push(c);
                in_token = true;
                only_codes = false;
            }
        }
    }
    flush(&mut cur, &mut in_token, &mut only_codes, &mut args);
    if args.first().is_some_and(|a| basename(a) == "flatpak") {
        args.retain(|a| !(a.starts_with("@@") && a.len() <= 3 && a[2..].chars().all(|c| c.is_ascii_alphabetic())));
    }
    args
}

/// `flatpak run [--opts] <app id> [args]` with no args (just the app) → the app id.
fn flatpak_app(argv: &[String]) -> Option<&str> {
    if argv.first().is_none_or(|a| basename(a) != "flatpak") {
        return None;
    }
    let mut it = argv[1..].iter().map(String::as_str);
    if it.next()? != "run" {
        return None;
    }
    let mut rest = it.skip_while(|a| a.starts_with('-'));
    let app = rest.next()?;
    rest.next().is_none().then_some(app)
}

fn basename(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}

/// `env VAR=value … program args` → (program, args, env). Anything else passes through.
fn split_env(argv: Vec<String>) -> (String, Vec<String>, Vec<(String, String)>) {
    let mut it = argv.into_iter();
    let first = it.next().unwrap_or_default();
    let rest: Vec<String> = it.collect();
    if basename(&first) != "env" {
        return (first, rest, vec![]);
    }
    let mut env = vec![];
    let mut i = 0;
    while i < rest.len() {
        match rest[i].split_once('=') {
            Some((k, v)) if is_env_name(k) => env.push((k.to_string(), v.to_string())),
            _ => break,
        }
        i += 1;
    }
    if i >= rest.len() || rest[i].starts_with('-') {
        return (first, rest, vec![]); // `env` with flags or nothing to run: keep as is
    }
    let program = rest[i].clone();
    (program, rest[i + 1..].to_vec(), env)
}

fn is_env_name(k: &str) -> bool {
    !k.is_empty()
        && !k.starts_with(|c: char| c.is_ascii_digit())
        && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_windows_command(argv: &[String]) -> bool {
    argv.iter().any(|a| {
        let b = basename(a).to_ascii_lowercase();
        b.starts_with("wine")
            || b == "proton"
            || b == "umu-run"
            || a.starts_with("WINEPREFIX=")
            || common::is_windows_exe(&b)
    })
}

/// If this command line is a shortcut into another launcher, return that source and its id.
pub fn shortcut_target(argv: &[String]) -> Option<(Source, String)> {
    for (i, a) in argv.iter().enumerate() {
        let lower = a.to_ascii_lowercase();
        for prefix in ["steam://rungameid/", "steam://run/", "steam://launch/"] {
            if let Some(rest) = lower.strip_prefix(prefix) {
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                let n: u64 = digits.parse().ok()?;
                let id = if n > u32::MAX as u64 { format!("shortcut:{}", n >> 32) } else { n.to_string() };
                return Some((Source::Steam, id));
            }
        }
        if lower.starts_with("heroic://") {
            return heroic_app_name(a).map(|n| (Source::Heroic, n));
        }
        if let Some(rest) = lower.strip_prefix("lutris:rungameid/") {
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() {
                return Some((Source::Lutris, digits));
            }
        }
        if a == "--game" && i > 0 {
            let is_faugus =
                argv[..i].iter().any(|p| basename(p) == "faugus-launcher" || p == "io.github.Faugus.faugus-launcher");
            if let Some(id) = argv.get(i + 1).filter(|_| is_faugus) {
                return Some((Source::Faugus, id.clone()));
            }
        }
        // The store CLIs: `legendary launch <app>` (Epic), `nile launch <id>` (Amazon).
        let store = match basename(a) {
            "legendary" => Some(Source::Epic),
            "nile" => Some(Source::Amazon),
            _ => None,
        };
        if let Some(src) = store
            && argv.get(i + 1).map(String::as_str) == Some("launch")
            && let Some(id) = argv.get(i + 2).filter(|id| !id.starts_with('-'))
        {
            return Some((src, id.clone()));
        }
    }
    None
}

/// `heroic://launch?appName=X&runner=Y`, `heroic://launch/?appName=X`, `heroic://launch/<runner>/<appName>`.
fn heroic_app_name(url: &str) -> Option<String> {
    let rest = url.get("heroic://".len()..)?;
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (rest, None),
    };
    if let Some(q) = query {
        for kv in q.split('&') {
            if let Some((k, v)) = kv.split_once('=')
                && k.eq_ignore_ascii_case("appName")
                && !v.is_empty()
            {
                return Some(common::percent_decode(v, true));
            }
        }
    }
    let mut parts = path.trim_matches('/').split('/');
    if !parts.next()?.eq_ignore_ascii_case("launch") {
        return None;
    }
    let segs: Vec<&str> = parts.filter(|s| !s.is_empty()).collect();
    segs.last().map(|s| common::percent_decode(s, false))
}

// ---------------------------------------------------------------------------------------------
// Icons

/// hicolor icons by name, best candidate per name. Built once per scan, on first use.
struct IconIndex {
    best: HashMap<String, (u32, PathBuf)>,
}

/// Sizes at or above this count as "large" (usable as a cover).
const LARGE: u32 = 256;
const SCALABLE_RANK: u32 = 4096;

impl IconIndex {
    fn build(ctx: &ScanContext) -> IconIndex {
        let mut idx = IconIndex { best: HashMap::new() };
        for base in icon_base_dirs(ctx) {
            if let Ok(rd) = std::fs::read_dir(base.join("hicolor")) {
                for e in rd.flatten() {
                    let dname = e.file_name().to_string_lossy().into_owned();
                    let Some(size) = size_rank(&dname) else {
                        continue;
                    };
                    for ctxdir in ["apps", "games"] {
                        idx.add_dir(&e.path().join(ctxdir), size);
                    }
                }
            }
            idx.add_dir(&base, 1); // loose icons directly in ~/.icons etc.
        }
        idx.add_dir(Path::new("/usr/share/pixmaps"), 1);
        idx
    }

    fn add_dir(&mut self, dir: &Path, size: u32) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            let Some(ext) = p.extension().and_then(|x| x.to_str()) else {
                continue;
            };
            let is_svg = ext.eq_ignore_ascii_case("svg");
            if !(is_svg || ext.eq_ignore_ascii_case("png")) {
                continue;
            }
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            // Same nominal size: PNG beats SVG (cheaper to decode).
            let rank = size * 2 + u32::from(!is_svg);
            let slot = self.best.entry(stem.to_string()).or_insert((0, PathBuf::new()));
            if rank > slot.0 {
                *slot = (rank, p);
            }
        }
    }

    fn lookup(&self, name: &str) -> Option<(PathBuf, bool)> {
        let (rank, p) = self.best.get(name)?;
        let size = rank / 2;
        let large =
            if size == 1 { common::png_dimensions(p).is_some_and(|(w, h)| w.min(h) >= LARGE) } else { size >= LARGE };
        super::util::file_if_exists(p).map(|p| (p, large))
    }
}

fn size_rank(dir_name: &str) -> Option<u32> {
    if dir_name == "scalable" {
        return Some(SCALABLE_RANK);
    }
    let base = dir_name.split('@').next()?;
    let (w, h) = base.split_once('x')?;
    let (w, h): (u32, u32) = (w.parse().ok()?, h.parse().ok()?);
    Some(w.min(h).clamp(2, SCALABLE_RANK - 1))
}

fn icon_base_dirs(ctx: &ScanContext) -> Vec<PathBuf> {
    let mut v = vec![
        ctx.data_home().join("icons"),
        ctx.home.join(".icons"),
        ctx.data_home().join("flatpak/exports/share/icons"),
    ];
    v.extend(ctx.xdg_data_dirs.iter().map(|d| d.join("icons")));
    v.push(PathBuf::from("/var/lib/flatpak/exports/share/icons"));
    let mut seen = HashSet::new();
    v.retain(|d| seen.insert(d.clone()) && d.is_dir());
    v
}

/// Resolve an `Icon=` value to (file, is_large).
fn resolve_icon(ctx: &ScanContext, icon: &str, idx: &mut Option<IconIndex>) -> Option<(PathBuf, bool)> {
    let icon = icon.trim();
    let p = Path::new(icon);
    if p.is_absolute() {
        let f = super::util::file_if_exists(p)?;
        let is_svg = f.extension().is_some_and(|x| x.eq_ignore_ascii_case("svg"));
        let large = is_svg || common::png_dimensions(&f).is_some_and(|(w, h)| w.min(h) >= LARGE);
        return Some((f, large));
    }
    let idx = idx.get_or_insert_with(|| IconIndex::build(ctx));
    idx.lookup(icon).or_else(|| {
        let stem = icon.strip_suffix(".png").or_else(|| icon.strip_suffix(".svg"))?;
        idx.lookup(stem)
    })
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::Path;

    /// Copy a fixture tree into `dst` (fixtures are checked in under tests/fixtures/scanners).
    pub fn copy_tree(src: &Path, dst: &Path) {
        std::fs::create_dir_all(dst).unwrap();
        for e in std::fs::read_dir(src).unwrap().flatten() {
            let to = dst.join(e.file_name());
            if e.file_type().unwrap().is_dir() {
                copy_tree(&e.path(), &to);
            } else {
                std::fs::copy(e.path(), &to).unwrap();
            }
        }
    }

    pub fn fixture(rel: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scanners").join(rel)
    }

    /// A file with a valid PNG header of the given size (enough for header sniffing).
    pub fn fake_png(path: &Path, w: u32, h: u32) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        std::fs::write(path, b).unwrap();
    }

    pub fn ctx(home: &Path) -> super::ScanContext {
        super::ScanContext {
            home: home.to_path_buf(),
            xdg_data_dirs: vec![home.join("usr/share")],
            // Tests state which games they expect; the user-facing default may change.
            config: crate::config::ScanConfig { include_uninstalled: false, ..Default::default() },
            data_dir: Default::default(),
            addons: Default::default(),
            emulation: crate::config::EmulationConfig::detect_nothing(),
            steam_names: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::test_support::sv;

    #[test]
    fn exec_parsing() {
        assert_eq!(parse_exec("steam steam://rungameid/1245620"), sv(&["steam", "steam://rungameid/1245620"]));
        assert_eq!(
            parse_exec(r#""/home/u/.local/bin/Epic Games.AppImage" %u"#),
            sv(&["/home/u/.local/bin/Epic Games.AppImage"])
        );
        assert_eq!(
            parse_exec(r#"game --file=%f --pct=100%% "a \"q\" b""#),
            sv(&["game", "--file=", "--pct=100%", "a \"q\" b"])
        );
        assert_eq!(parse_exec(r#""my\sgame" -x"#), sv(&["my game", "-x"]));
        assert_eq!(parse_exec(r"my\sgame"), sv(&["my", "game"]));
        assert_eq!(parse_exec(r#"sh -c "echo \\$HOME""#), sv(&["sh", "-c", "echo $HOME"]));
        assert_eq!(
            parse_exec(r#"env WINEPREFIX="/home/u/.wine" wine C:\\\\Games\\\\x.exe"#),
            sv(&["env", "WINEPREFIX=/home/u/.wine", "wine", r"C:\Games\x.exe"])
        );
        assert_eq!(
            parse_exec(
                "/usr/bin/flatpak run --branch=stable --command=PPSSPPSDL --file-forwarding org.ppsspp.PPSSPP @@u %U @@"
            ),
            sv(&[
                "/usr/bin/flatpak",
                "run",
                "--branch=stable",
                "--command=PPSSPPSDL",
                "--file-forwarding",
                "org.ppsspp.PPSSPP"
            ])
        );
        assert_eq!(parse_exec(r#"game """#), sv(&["game", ""]));
        assert!(parse_exec("   ").is_empty());
    }

    #[test]
    fn env_split() {
        let (p, a, e) = split_env(sv(&["env", "WINEPREFIX=/p", "DXVK_HUD=1", "wine", "game.exe"]));
        assert_eq!(p, "wine");
        assert_eq!(a, sv(&["game.exe"]));
        assert_eq!(e, vec![("WINEPREFIX".into(), "/p".into()), ("DXVK_HUD".into(), "1".into())]);
        let (p, a, e) = split_env(sv(&["env", "-u", "X", "game"]));
        assert_eq!((p.as_str(), a.len(), e.len()), ("env", 3, 0));
    }

    #[test]
    fn shortcut_targets() {
        let t = |v: &[&str]| shortcut_target(&sv(v));
        assert_eq!(t(&["steam", "steam://rungameid/1245620"]), Some((Source::Steam, "1245620".into())));
        assert_eq!(t(&["xdg-open", "steam://run/570"]), Some((Source::Steam, "570".into())));
        assert_eq!(
            t(&["flatpak", "run", "com.valvesoftware.Steam", "steam://rungameid/11674088718291271680"]),
            Some((Source::Steam, format!("shortcut:{}", 11674088718291271680u64 >> 32)))
        );
        assert_eq!(t(&["/usr/bin/faugus-launcher", "--game", "fl-2026"]), Some((Source::Faugus, "fl-2026".into())));
        assert_eq!(
            t(&["flatpak", "run", "io.github.Faugus.faugus-launcher", "--game", "x"]),
            Some((Source::Faugus, "x".into()))
        );
        assert_eq!(t(&["mygame", "--game", "x"]), None);
        assert_eq!(t(&["lutris", "lutris:rungameid/42"]), Some((Source::Lutris, "42".into())));
        assert_eq!(t(&["xdg-open", "heroic://launch/legendary/Fortnite"]), Some((Source::Heroic, "Fortnite".into())));
        assert_eq!(
            t(&["xdg-open", "heroic://launch?appName=1207658924&runner=gog"]),
            Some((Source::Heroic, "1207658924".into()))
        );
        assert_eq!(t(&["legendary", "launch", "Sugar"]), Some((Source::Epic, "Sugar".into())));
        assert_eq!(
            t(&["/usr/bin/nile", "launch", "amzn1.adg.product.x"]),
            Some((Source::Amazon, "amzn1.adg.product.x".into()))
        );
        assert_eq!(t(&["legendary", "launch", "--help"]), None);
        assert_eq!(t(&["supertuxkart"]), None);
    }

    fn scan_fixture() -> (tempfile::TempDir, Vec<ScannedGame>) {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        copy_tree(&fixture("desktop/home"), home);
        // Icons (binary, so generated here).
        let hi = home.join(".local/share/icons/hicolor");
        fake_png(&hi.join("128x128/apps/steam_icon_1245620.png"), 128, 128);
        fake_png(&hi.join("256x256/apps/runelite.png"), 256, 256);
        fake_png(&hi.join("32x32/apps/runelite.png"), 32, 32);
        fake_png(&home.join("usr/share/icons/hicolor/48x48/apps/supertuxkart.png"), 48, 48);
        std::fs::create_dir_all(home.join("usr/share/icons/hicolor/scalable/apps")).unwrap();
        std::fs::write(home.join("usr/share/icons/hicolor/scalable/apps/supertuxkart.svg"), "<svg/>").unwrap();
        fake_png(&home.join(".local/share/faugus-launcher/icons/fl-2026.png"), 64, 64);
        let bin = home.join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("RuneLite.AppImage"), "x").unwrap();
        // Fixture desktop files reference $HOME as /HOME; point them at the temp home.
        for dir in [home.join(".local/share/applications"), home.join("usr/share/applications")] {
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                if e.path().is_file() {
                    let t = std::fs::read_to_string(e.path()).unwrap();
                    std::fs::write(e.path(), t.replace("/HOME", &home.to_string_lossy())).unwrap();
                }
            }
        }
        let mut games = scan(&ctx(home)).unwrap();
        games.sort_by(|a, b| a.source_id.cmp(&b.source_id));
        (tmp, games)
    }

    #[test]
    fn scans_fixture_tree() {
        let (tmp, games) = scan_fixture();
        let home = tmp.path();
        let ids: Vec<&str> = games.iter().map(|g| g.source_id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "ELDEN RING.desktop",
                "Shortcut To Lutris.desktop",
                "Wind Waker.desktop",
                "fl-2026.desktop",
                "games-wine-thing.desktop",
                "runelite.desktop",
                "supertuxkart.desktop",
            ],
            "{games:#?}"
        );
        let by = |id: &str| games.iter().find(|g| g.source_id == id).unwrap();
        // Known emulators are never games, whatever their Categories (PPSSPP as Game;
        // ArcadeGame, a DuckStation AppImage, Dolphin's Flatpak); an entry that starts an
        // emulator with a game file is a game shortcut and stays.
        for id in [
            "ppsspp-tester.desktop",
            "duckstation-appimage.desktop",
            "dolphin-flatpak.desktop",
            "org.ppsspp.PPSSPP.desktop",
        ] {
            assert!(!ids.contains(&id), "{id} is an emulator");
        }

        let elden = by("ELDEN RING.desktop");
        assert_eq!(elden.duplicate_of, Some((Source::Steam, "1245620".into())));
        assert_eq!(elden.art.icon, Some(home.join(".local/share/icons/hicolor/128x128/apps/steam_icon_1245620.png")));
        assert_eq!(elden.art.cover, None);
        assert_eq!(elden.meta.description, None);

        let rl = by("runelite.desktop");
        assert_eq!(rl.duplicate_of, None);
        assert_eq!(rl.art.icon, Some(home.join(".local/share/icons/hicolor/256x256/apps/runelite.png")));
        assert_eq!(rl.art.cover, rl.art.icon);
        assert_eq!(rl.platform.as_deref(), Some("linux"));
        match &rl.launch {
            Some(LaunchSpec::Command { program, args, .. }) => {
                assert_eq!(program, &home.join(".local/bin/RuneLite.AppImage").to_string_lossy());
                assert!(args.is_empty());
            }
            other => panic!("{other:?}"),
        }

        // "Faugus" is in desktop_ignore, but a shortcut with duplicate_of must survive.
        let fl = by("fl-2026.desktop");
        assert_eq!(fl.duplicate_of, Some((Source::Faugus, "fl-2026".into())));
        assert_eq!(fl.art.icon, Some(home.join(".local/share/faugus-launcher/icons/fl-2026.png")));
        assert_eq!(fl.art.cover, None, "64px icon is not a cover");

        // Subdirectory id, env + wine, Path=.
        let w = by("games-wine-thing.desktop");
        assert_eq!(w.platform.as_deref(), Some("windows"));
        match &w.launch {
            Some(LaunchSpec::Command { program, args, cwd, env }) => {
                assert_eq!(program, "wine");
                assert_eq!(args, &vec![r"C:\Games\Thing\thing.exe".to_string()]);
                assert_eq!(cwd.as_deref(), Some(Path::new("/opt/thing")));
                assert_eq!(env, &vec![("WINEPREFIX".to_string(), "/opt/prefix".to_string())]);
            }
            other => panic!("{other:?}"),
        }

        // System entry shadowed by the user's copy (user copy wins with its own Name).
        let stk = by("supertuxkart.desktop");
        assert_eq!(stk.name, "SuperTuxKart (mine)");
        assert!(stk.art.icon.as_ref().unwrap().ends_with("scalable/apps/supertuxkart.svg"));
        assert_eq!(stk.art.cover, stk.art.icon);
        assert_eq!(stk.meta.description.as_deref(), Some("Kart racing"));

        assert_eq!(by("Shortcut To Lutris.desktop").duplicate_of, Some((Source::Lutris, "7".into())));
        assert!(elden.installed, "no Steam on this fixture home: leave as is");
    }

    #[test]
    fn stale_steam_shortcuts_marked_uninstalled() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let apps = home.join(".local/share/applications");
        std::fs::create_dir_all(&apps).unwrap();
        for (file, id) in [("Installed.desktop", 1245620), ("Gone.desktop", 377160)] {
            let text = format!(
                "[Desktop Entry]\nName={file}\nExec=steam steam://rungameid/{id}\nType=Application\nCategories=Game;\n"
            );
            std::fs::write(apps.join(file), text).unwrap();
        }
        let steamapps = home.join(".local/share/Steam/steamapps");
        std::fs::create_dir_all(&steamapps).unwrap();
        std::fs::write(steamapps.join("appmanifest_1245620.acf"), "\"AppState\"\n{\n}\n").unwrap();
        let games = scan(&ctx(home)).unwrap();
        let get = |n: &str| games.iter().find(|g| g.source_id == n).unwrap();
        assert!(get("Installed.desktop").installed);
        assert!(!get("Gone.desktop").installed);
        assert_eq!(get("Gone.desktop").duplicate_of, Some((Source::Steam, "377160".into())));
    }

    /// Prints what the scanner finds in the real home. `cargo test real_desktop -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_desktop() {
        let ctx = ScanContext::from_config(&crate::config::Config::default());
        let t = std::time::Instant::now();
        let games = scan(&ctx).unwrap();
        println!("{} desktop games in {:?}", games.len(), t.elapsed());
        for g in games {
            println!(
                "{:45} dup={:?} plat={:?} icon={:?} cover={} launch={:?}",
                g.source_id,
                g.duplicate_of,
                g.platform,
                g.art.icon,
                g.art.cover.is_some(),
                g.launch
            );
        }
    }
}
