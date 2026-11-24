//! Gamescope capabilities from its `--help` (run once per binary version, cached on disk).

use super::*;

// --------------------------------------------------------------------------------- gamescope

/// On-disk cache of one `gamescope --help` run.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct HelpCache {
    path: PathBuf,
    mtime_ns: u128,
    size: u64,
    text: String,
}

/// (mtime in ns since the epoch, size) of a file, following symlinks.
pub(super) fn file_stamp(p: &Path) -> Option<(u128, u64)> {
    let m = std::fs::metadata(p).ok()?;
    let t = m.modified().ok()?.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_nanos();
    Some((t, m.len()))
}

/// Gamescope's features from its `--help` text: cached per (path, mtime, size), else one run
/// with `r.gamescope_timeout`. None when the help couldn't be read (failed runs aren't cached,
/// so the next probe tries again).
pub(super) fn gamescope_caps(bin: &Path, r: &ProbeRoots) -> Option<GamescopeCaps> {
    let (mtime_ns, size) = file_stamp(bin)?;
    let cache_file = r.cache_dir.as_ref().map(|d| d.join(GAMESCOPE_CACHE_FILE));
    let cached = cache_file
        .as_deref()
        .and_then(read_small)
        .and_then(|b| serde_json::from_slice::<HelpCache>(&b).ok())
        .filter(|c| c.path == bin && c.mtime_ns == mtime_ns && c.size == size);
    let text = match cached {
        Some(c) => c.text,
        None => {
            let text = run_help(bin, r.gamescope_timeout)?;
            if let Some(f) = &cache_file {
                write_cache(f, &HelpCache { path: bin.to_path_buf(), mtime_ns, size, text: text.clone() });
            }
            text
        }
    };
    Some(parse_gamescope_help(&text))
}

/// Write the help cache atomically (temp file + rename); failures are only logged.
pub(super) fn write_cache(file: &Path, entry: &HelpCache) {
    let write = || crate::fsutil::write_atomic(file, &serde_json::to_vec(entry).map_err(std::io::Error::other)?);
    if let Err(e) = write() {
        log::debug!("gamescope help cache {}: {e}", file.display());
    }
}

/// Run `<bin> --help` and return stderr + stdout (gamescope prints its help to stderr). None
/// on spawn failure, timeout, or output without any option in it.
pub(super) fn run_help(bin: &Path, timeout: Duration) -> Option<String> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(bin);
    cmd.arg("--help").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = spawn_retrying(&mut cmd)?;
    let (tx, rx) = std::sync::mpsc::channel::<(u8, Vec<u8>)>();
    let pipes: [(u8, Option<Box<dyn Read + Send>>); 2] = [
        (0, child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>)),
        (1, child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>)),
    ];
    for (idx, pipe) in pipes {
        let Some(pipe) = pipe else { continue };
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let _ = pipe.take(1 << 20).read_to_end(&mut out);
            let _ = tx.send((idx, out));
        });
    }
    drop(tx);
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                log::warn!("{} --help timed out", bin.display());
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    // The process has exited; its pipes close unless it left a child holding them.
    let mut parts: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
    for _ in 0..2 {
        let left = deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(100));
        match rx.recv_timeout(left) {
            Ok((idx, out)) => parts[idx as usize] = out,
            Err(_) => break,
        }
    }
    let text = String::from_utf8_lossy(&[parts[0].as_slice(), b"\n", parts[1].as_slice()].concat()).into_owned();
    text.contains("--").then_some(text)
}

/// `Command::spawn`, retried briefly on ETXTBSY (a freshly written script can be "busy" while
/// another thread forks).
pub(super) fn spawn_retrying(cmd: &mut std::process::Command) -> Option<std::process::Child> {
    for _ in 0..10 {
        match cmd.spawn() {
            Ok(c) => return Some(c),
            Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                log::debug!("spawn {:?}: {e}", cmd.get_program());
                return None;
            }
        }
    }
    None
}

/// Remove ANSI escape sequences (CSI `ESC [ … final` and two-byte `ESC x`).
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        if it.peek() == Some(&'[') {
            it.next();
            for c in it.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    break;
                }
            }
        } else {
            it.next();
        }
    }
    out
}

/// Parse `gamescope --help`: long options (`--[a-z0-9-]+`), short options (`-W`), the filter
/// list from the `-F` / `--filter` entry, and the version (`version\s+v?N.N[.N]`, after
/// stripping ANSI). Fedora's tarball build prints no version, so `version` may be None.
pub fn parse_gamescope_help(raw: &str) -> GamescopeCaps {
    let text = strip_ansi(raw);
    GamescopeCaps {
        flags: help_flags(&text),
        filters: help_filters(&text),
        version: help_version(&text),
        options: help_options(&text),
    }
}

/// Every documented option with its help text and section, in help order. Lines look like
/// `  -W, --output-width             output width` (names, two+ spaces, help); indented lines
/// without a leading `-` continue the previous help; `Section name:` lines start a section; the
/// keyboard-shortcut list at the end isn't options.
pub(super) fn help_options(text: &str) -> Vec<GsHelpOption> {
    let mut out: Vec<GsHelpOption> = Vec::new();
    let mut section = String::from("Options");
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if !line.starts_with([' ', '\t']) {
            if let Some(name) = t.strip_suffix(':') {
                if name.eq_ignore_ascii_case("Keyboard shortcuts") {
                    break;
                }
                section = name.to_string();
            }
            continue;
        }
        if t.starts_with('-') {
            // Names end at the first run of two spaces or a tab.
            let split = t.find("  ").into_iter().chain(t.find('\t')).min().unwrap_or(t.len());
            let (names, help) = t.split_at(split);
            let mut long = None;
            let mut short = None;
            for n in names.split(',').map(str::trim) {
                let valid = n.len() > 1 && n[1..].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
                if !valid {
                    continue;
                }
                if n.starts_with("--") {
                    long.get_or_insert_with(|| n.to_string());
                } else if n.len() == 2 {
                    short.get_or_insert_with(|| n.to_string());
                }
            }
            let Some(long) = long else { continue };
            if long == "--help" || out.iter().any(|o| o.long == long) {
                continue;
            }
            out.push(GsHelpOption { long, short, help: help.trim().to_string(), section: section.clone() });
        } else if let Some(last) = out.last_mut()
            && last.help.len() < 600
        {
            if !last.help.is_empty() {
                last.help.push(' ');
            }
            last.help.push_str(t);
        }
    }
    out
}

/// Every `--long` and `-X` option token in the text.
pub(super) fn help_flags(text: &str) -> HashSet<String> {
    let b = text.as_bytes();
    let mut flags = HashSet::new();
    let boundary = |i: usize| i == 0 || matches!(b[i - 1], b' ' | b'\t' | b'\n' | b',' | b'[' | b'(' | b'|');
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'-' && boundary(i) {
            if b.get(i + 1) == Some(&b'-') {
                let start = i + 2;
                let mut end = start;
                while end < b.len() && (b[end].is_ascii_lowercase() || b[end].is_ascii_digit() || b[end] == b'-') {
                    end += 1;
                }
                if end > start && b[start] != b'-' {
                    flags.insert(text[i..end].trim_end_matches('-').to_string());
                }
                i = end.max(i + 2);
                continue;
            }
            if let Some(c) = b.get(i + 1).filter(|c| c.is_ascii_alphabetic())
                && b.get(i + 2).is_none_or(|n| !n.is_ascii_alphanumeric() && *n != b'-')
            {
                flags.insert(format!("-{}", *c as char));
            }
        }
        i += 1;
    }
    flags
}

/// Filter names from the `-F, --filter` entry: the parenthesised list on its line, plus
/// `name =>` explanation lines that follow it.
pub(super) fn help_filters(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |w: &str| {
        let w = w.trim().to_ascii_lowercase();
        if !w.is_empty() && w.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') && !out.contains(&w) {
            out.push(w);
        }
    };
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let t = line.trim_start();
        if !(t.starts_with("-F,") || t.starts_with("-F ") || t.starts_with("--filter")) {
            continue;
        }
        if let Some(open) = line.find('(')
            && let Some(close) = line[open..].find(')')
        {
            line[open + 1..open + close].split(',').for_each(&mut push);
        }
        while let Some(next) = lines.peek() {
            let nt = next.trim_start();
            if nt.starts_with('-') || nt.is_empty() {
                break;
            }
            if let Some((name, _)) = nt.split_once("=>") {
                push(name);
            }
            lines.next();
        }
        break;
    }
    out
}

/// `version 3.16.17`, `version v3.16`, … → (3, 16, 17); a missing patch is 0.
pub(super) fn help_version(text: &str) -> Option<(u32, u32, u32)> {
    let lower = text.to_ascii_lowercase();
    lower.match_indices("version").find_map(|(i, _)| {
        let rest = &lower[i + "version".len()..];
        let trimmed = rest.trim_start();
        if trimmed.len() == rest.len() {
            return None; // needs whitespace after "version"
        }
        let num = trimmed.strip_prefix('v').unwrap_or(trimmed);
        let end = num.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(num.len());
        let parts: Vec<u32> = num[..end].trim_end_matches('.').split('.').map_while(|p| p.parse().ok()).collect();
        (parts.len() >= 2).then(|| (parts[0], parts[1], parts.get(2).copied().unwrap_or(0)))
    })
}
