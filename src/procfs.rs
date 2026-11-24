//! Reading /proc (or a /proc-like folder in tests): process stat, command line and environment,
//! boot time and clock ticks. Callers keep their own matching; only the byte plumbing is here.

use std::path::Path;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: i32,
    pub ppid: i32,
    pub pgrp: i32,
    pub sid: i32,
    /// Start time in clock ticks since boot (disambiguates reused pids).
    pub start: u64,
    pub zombie: bool,
    pub cmdline: Option<Vec<String>>,
}

/// One process' stat (None if it's gone or unreadable).
pub fn read_stat(pid: i32) -> Option<ProcInfo> {
    read_stat_in(Path::new("/proc"), pid)
}

/// One process' stat under `root` (a /proc).
pub fn read_stat_in(root: &Path, pid: i32) -> Option<ProcInfo> {
    let s = std::fs::read_to_string(root.join(pid.to_string()).join("stat")).ok()?;
    parse_stat(pid, &s)
}

/// `pid` under `root` still runs and is the same process (same start time, not a zombie).
pub fn is_same_live(root: &Path, pid: i32, start: u64) -> bool {
    read_stat_in(root, pid).is_some_and(|p| p.start == start && !p.zombie)
}

pub fn read_procs(want_cmdline: bool) -> Vec<ProcInfo> {
    read_procs_in(Path::new("/proc"), want_cmdline)
}

pub fn read_procs_in(root: &Path, want_cmdline: bool) -> Vec<ProcInfo> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return vec![];
    };
    let mut out = vec![];
    for ent in rd.flatten() {
        let name = ent.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<i32>().ok()) else {
            continue;
        };
        let dir = ent.path();
        let Ok(stat) = std::fs::read_to_string(dir.join("stat")) else {
            continue;
        };
        let Some(mut p) = parse_stat(pid, &stat) else {
            continue;
        };
        if want_cmdline {
            p.cmdline = read_cmdline(&dir);
        }
        out.push(p);
    }
    out
}

/// Parse `/proc/<pid>/stat`. The command name may contain spaces and parens, so fields are
/// counted from the last ')'.
pub fn parse_stat(pid: i32, stat: &str) -> Option<ProcInfo> {
    let rest = &stat[stat.rfind(')')? + 1..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    // f[0]=state f[1]=ppid f[2]=pgrp f[3]=session … f[19]=starttime
    Some(ProcInfo {
        pid,
        zombie: matches!(*f.first()?, "Z" | "X"),
        ppid: f.get(1)?.parse().ok()?,
        pgrp: f.get(2)?.parse().ok()?,
        sid: f.get(3)?.parse().ok()?,
        start: f.get(19)?.parse().ok()?,
        cmdline: None,
    })
}

/// The command line in a process folder (`<proc>/<pid>`): its NUL-separated words, empty ones
/// skipped (None if it's gone or unreadable).
pub fn read_cmdline(dir: &Path) -> Option<Vec<String>> {
    std::fs::read(dir.join("cmdline")).ok().map(|b| {
        b.split(|&c| c == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect()
    })
}

/// Process `pid`'s raw environment under `root` (None: gone, or not readable: another user's).
pub fn read_environ(root: &Path, pid: i32) -> Option<Vec<u8>> {
    std::fs::read(root.join(pid.to_string()).join("environ")).ok()
}

/// The `KEY=value` entries of a raw environment, split at the first `=` (entries without one
/// are skipped).
pub fn env_entries(env: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
    env.split(|&b| b == 0).filter_map(|kv| {
        let eq = kv.iter().position(|&b| b == b'=')?;
        Some((&kv[..eq], &kv[eq + 1..]))
    })
}

/// Boot time (unix seconds, `btime` in /proc/stat).
pub fn boot_time(proc_root: &Path) -> Option<u64> {
    let s = std::fs::read_to_string(proc_root.join("stat")).ok()?;
    s.lines().find_map(|l| l.strip_prefix("btime ")?.trim().parse().ok())
}

/// Clock ticks per second (the unit of /proc/<pid>/stat's starttime).
pub fn clock_ticks() -> u64 {
    // SAFETY: sysconf is a plain query.
    let t = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if t > 0 { t as u64 } else { 100 }
}

/// How long ago a process that started `start` ticks after boot (`btime`, unix seconds) did.
pub fn started_ago(start: u64, btime: u64, hz: u64, now_unix: f64) -> Duration {
    let at = btime as f64 + start as f64 / hz.max(1) as f64;
    let ago = now_unix - at;
    if ago.is_finite() && ago > 0.0 { Duration::from_secs_f64(ago) } else { Duration::ZERO }
}

/// A fake /proc tree for tests (stat, cmdline, environ, optional cwd link, /proc/stat btime).
#[cfg(test)]
pub(crate) mod fake {
    use std::path::Path;

    pub struct P<'a> {
        pub pid: i32,
        pub ppid: i32,
        pub pgrp: i32,
        pub sid: i32,
        pub start: u64,
        pub cmdline: &'a [&'a str],
        pub env: &'a [&'a str],
        pub cwd: Option<&'a Path>,
    }

    impl<'a> P<'a> {
        pub fn new(pid: i32, start: u64, cmdline: &'a [&'a str]) -> P<'a> {
            P { pid, ppid: 1, pgrp: pid, sid: pid, start, cmdline, env: &[], cwd: None }
        }
        pub fn env(mut self, env: &'a [&'a str]) -> P<'a> {
            self.env = env;
            self
        }
        pub fn parent(mut self, ppid: i32, pgrp: i32, sid: i32) -> P<'a> {
            (self.ppid, self.pgrp, self.sid) = (ppid, pgrp, sid);
            self
        }
        pub fn cwd(mut self, cwd: &'a Path) -> P<'a> {
            self.cwd = Some(cwd);
            self
        }
    }

    pub fn boot(root: &Path, btime: u64) {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(root.join("stat"), format!("cpu 1 2 3\nbtime {btime}\nprocesses 9\n")).unwrap();
    }

    pub fn add(root: &Path, p: P) {
        let d = root.join(p.pid.to_string());
        std::fs::create_dir_all(&d).unwrap();
        let stat =
            format!("{} (x) S {} {} {} 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 {} 0 0", p.pid, p.ppid, p.pgrp, p.sid, p.start);
        std::fs::write(d.join("stat"), stat).unwrap();
        let nul = |v: &[&str]| v.iter().flat_map(|s| s.bytes().chain([0])).collect::<Vec<u8>>();
        std::fs::write(d.join("cmdline"), nul(p.cmdline)).unwrap();
        std::fs::write(d.join("environ"), nul(p.env)).unwrap();
        if let Some(c) = p.cwd {
            std::os::unix::fs::symlink(c, d.join("cwd")).unwrap();
        }
    }
}
