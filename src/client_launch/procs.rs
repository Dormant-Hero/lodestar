//! The processes of a Wine prefix, read from a `/proc`-like folder (a temp fixture in tests).
//!
//! Wine processes show their Windows image as `argv[0]` (`C:\Program Files (x86)\Battle.net\
//! Battle.net.exe`), sometimes behind the loader (`wine64 C:\…`), and carry the `WINEPREFIX` they
//! were started with (umu / Proton: the compat root or `<root>/pfx`).

use crate::procfs;
use std::path::{Path, PathBuf};

/// Environment variables read from `/proc/<pid>/environ` (the rest is skipped).
const WANTED_ENV: &[&str] = &["WINEPREFIX", "WINEFSYNC", "WINEESYNC", "WINENTSYNC", "PROTON_NO_NTSYNC", super::TAG_VAR];

/// The Wine sync switches a client joining a running wineserver must match (`PROTON_NO_NTSYNC`:
/// the NTSync game option, read by Proton's wineserver).
pub const SYNC_VARS: &[&str] = &["WINEFSYNC", "WINEESYNC", "WINENTSYNC", "PROTON_NO_NTSYNC"];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Proc {
    pub pid: i32,
    /// Start time in clock ticks since boot (tells a reused pid apart).
    pub start: u64,
    pub cmdline: Vec<String>,
    /// Only [`WANTED_ENV`].
    pub env: Vec<(String, String)>,
}

impl Proc {
    pub fn env(&self, k: &str) -> Option<&str> {
        self.env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }

    /// The Windows (or Unix) path of the program this process runs, normalised by [`norm`]:
    /// `argv[0]`, or the argument after a Wine loader (`wine64 C:\x.exe`).
    pub fn image(&self) -> Option<String> {
        let mut it = self.cmdline.iter();
        let mut first = it.next()?;
        for _ in 0..2 {
            let base = base_name(first);
            let loader = base.starts_with("wine") && !base.ends_with(".exe");
            match (loader, it.next()) {
                (true, Some(next)) => first = next,
                _ => break,
            }
        }
        Some(norm(first))
    }

    /// The image's file name, lower case (`battle.net.exe`).
    pub fn image_name(&self) -> Option<String> {
        self.image().map(|i| base_name(&i).to_string())
    }

    /// Runs in `prefix` (its `WINEPREFIX` is the prefix, its compat root or `<root>/pfx`).
    pub fn in_prefix(&self, prefix: &Path) -> bool {
        self.env("WINEPREFIX").is_some_and(|v| same_prefix(Path::new(v), prefix))
    }

    /// The Wine sync switches this process runs with (what a request must match to reach it).
    pub fn sync_env(&self) -> Vec<(String, String)> {
        SYNC_VARS.iter().filter_map(|k| self.env(k).map(|v| (k.to_string(), v.to_string()))).collect()
    }
}

/// Lower case, `\` → `/`, quotes trimmed: `"C:\Program Files\X.exe"` → `c:/program files/x.exe`.
pub fn norm(s: &str) -> String {
    s.trim().trim_matches('"').replace('\\', "/").to_lowercase()
}

/// After the last `/` or `\`.
pub fn base_name(s: &str) -> &str {
    s.rsplit(['/', '\\']).next().unwrap_or(s)
}

/// `/x/pfx/`, `/x/pfx` and `/x` are the same prefix (umu / Proton pass the compat root,
/// Wine itself the `pfx` folder).
pub fn same_prefix(a: &Path, b: &Path) -> bool {
    fn key(p: &Path) -> PathBuf {
        let s = p.to_string_lossy();
        let s = s.trim_end_matches('/');
        PathBuf::from(s.strip_suffix("/pfx").unwrap_or(s).trim_end_matches('/'))
    }
    key(a) == key(b)
}

/// Reads processes from a `/proc`-like folder.
#[derive(Clone, Debug)]
pub struct ProcTable {
    pub root: PathBuf,
}

impl ProcTable {
    pub fn real() -> ProcTable {
        ProcTable { root: PathBuf::from("/proc") }
    }

    /// Every live process, by pid.
    pub fn list(&self) -> Vec<Proc> {
        let mut v: Vec<Proc> = procfs::read_procs_in(&self.root, true)
            .into_iter()
            .filter(|p| !p.zombie)
            .map(|p| Proc {
                pid: p.pid,
                start: p.start,
                cmdline: p.cmdline.unwrap_or_default(),
                env: read_env(&self.root, p.pid),
            })
            .collect();
        v.sort_by_key(|p| p.pid);
        v
    }

    /// `pid` still runs and is the same process (same start time).
    pub fn alive(&self, pid: i32, start: u64) -> bool {
        procfs::is_same_live(&self.root, pid, start)
    }
}

fn read_env(root: &Path, pid: i32) -> Vec<(String, String)> {
    let Some(bytes) = procfs::read_environ(root, pid) else {
        return vec![];
    };
    bytes
        .split(|&b| b == 0)
        .filter_map(|kv| {
            let kv = String::from_utf8_lossy(kv);
            let (k, v) = kv.split_once('=')?;
            WANTED_ENV.contains(&k).then(|| (k.to_string(), v.to_string()))
        })
        .collect()
}

/// The client's main processes in `prefix`: image file name in `mains` (case-insensitive),
/// not one of its helper processes (`--type=renderer` etc.).
pub fn main_procs<'a>(procs: &'a [Proc], prefix: &Path, mains: &[String]) -> Vec<&'a Proc> {
    procs
        .iter()
        .filter(|p| {
            p.in_prefix(prefix)
                && p.image_name().is_some_and(|n| mains.iter().any(|m| m.eq_ignore_ascii_case(&n)))
                && !p.cmdline.iter().any(|a| a.starts_with("--type="))
        })
        .collect()
}

/// `dir` (a Unix path inside `prefix`) in the forms a process image may show it: `c:/…`,
/// `z:/home/…` and `/home/…`, normalised, without a trailing `/`.
pub fn dir_forms(prefix: &Path, dir: &Path) -> Vec<String> {
    let win = norm(&crate::scan::winprefix::unix_to_win(prefix, dir));
    let unix = norm(&dir.to_string_lossy());
    let mut v = vec![
        win.trim_end_matches('/').to_string(),
        format!("z:{}", unix.trim_end_matches('/')),
        unix.trim_end_matches('/').to_string(),
    ];
    v.dedup();
    v
}

/// `image` (normalised) lies inside one of the `dirs` forms.
pub fn image_under(image: &str, dirs: &[String]) -> bool {
    dirs.iter().any(|d| image.strip_prefix(d.as_str()).is_some_and(|r| r.starts_with('/')))
}
