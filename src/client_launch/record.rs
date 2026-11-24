//! Store clients the helper had to start itself, so the launcher can close them again.
//!
//! CONTRACT (for launcher hygiene):
//! * A cold start by `lodestar --client-launch …` writes `<cache_dir>/clients/<prefix hash>-<client>.json`
//!   (cache dir from `config::Paths`, so `LODESTAR_HOME` works): [`StartedClient`]. A client the
//!   user opened themselves, or one already running when the helper ran, gets no record.
//! * [`started_clients`] lists the records whose client still runs (anything carrying the
//!   record's tag, or one of its recorded processes); records of clients that are gone are
//!   deleted on the way.
//! * [`close_client`] closes one: polite first (Wine's `taskkill /IM <main exe>` through the
//!   client's own wineserver, which sends its windows WM_CLOSE — clients that minimise to the tray
//!   ignore it), then SIGTERM, then SIGKILL after a grace period. It only ever signals processes of
//!   the record's prefix whose program lies in the client's own folders (never a `games` folder
//!   inside them, never wineserver, never another prefix), and only while the record is still
//!   alive. It never runs `wineserver -k`. The record is removed afterwards. Call it after the
//!   game has exited: Battle.net games don't need their client, but EA / Ubisoft games do.

use super::procs::{self, Proc, ProcTable};
use crate::config::Paths;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long the client gets to close its windows itself.
const POLITE_GRACE: Duration = Duration::from_secs(8);
/// After SIGTERM, how long before SIGKILL.
const TERM_GRACE: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedPid {
    pub pid: i32,
    /// Start time in clock ticks since boot (`/proc/<pid>/stat`), so a reused pid isn't taken
    /// for the client.
    pub start: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartedClient {
    /// The helper kind that started it: `battlenet`, `ea` or `generic`.
    pub kind: String,
    /// Which client: `battlenet`, `ea`, `ubisoft`, `rockstar`, `riot`, `hoyoplay` (see
    /// [`super::client_name`]).
    pub client: String,
    pub prefix: PathBuf,
    /// The program that was started (Unix path inside the prefix).
    pub exe: PathBuf,
    /// The client's own folders: its processes run programs from here (`games` sub-folders
    /// excepted).
    pub client_dirs: Vec<PathBuf>,
    /// The prefix's runner as `--runner` takes it (`proton:<dir>` / `wine:<bin>` / `unknown`).
    pub runner: String,
    /// The client's main process names (`Battle.net.exe`, `EADesktop.exe` …).
    pub main: Vec<String>,
    /// The process that was spawned (umu-run / proton / wine) and the client's main processes
    /// once they showed up.
    pub pids: Vec<RecordedPid>,
    /// `LODESTAR_CLIENT_TAG` value every process of that start inherits.
    pub tag: String,
    /// The launch (`LODESTAR_GAME_TAG`) the client was started for, if any.
    #[serde(default)]
    pub game_tag: Option<String>,
    /// Unix seconds.
    pub started_at: u64,
    /// The record file (not stored in it).
    #[serde(skip)]
    pub record: PathBuf,
}

/// What [`close_client`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CloseReport {
    /// Processes of the client found running.
    pub found: usize,
    /// Gone after the polite close.
    pub closed_politely: bool,
    pub terminated: Vec<i32>,
    pub killed: Vec<i32>,
}

pub fn records_dir(paths: &Paths) -> PathBuf {
    paths.cache_dir.join("clients")
}

pub fn record_path(dir: &Path, prefix: &Path, client: &str) -> PathBuf {
    dir.join(format!("{}-{client}.json", crate::scan::winprefix::prefix_hash(prefix)))
}

/// Write (replace) `c`'s record in `dir`; returns its path.
pub fn write_record(dir: &Path, c: &StartedClient) -> anyhow::Result<PathBuf> {
    let path = record_path(dir, &c.prefix, &c.client);
    crate::fsutil::write_atomic(&path, &serde_json::to_vec_pretty(c)?)?;
    Ok(path)
}

pub fn read_records(dir: &Path) -> Vec<StartedClient> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut v: Vec<StartedClient> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| {
            let mut c: StartedClient = serde_json::from_slice(&std::fs::read(&p).ok()?).ok()?;
            c.record = p;
            Some(c)
        })
        .collect();
    v.sort_by(|a, b| a.record.cmp(&b.record));
    v
}

/// Something of this start still runs: a process carrying its tag (in its prefix), or one of
/// its recorded processes.
pub fn is_alive(c: &StartedClient, table: &ProcTable, procs: &[Proc]) -> bool {
    procs.iter().any(|p| !c.tag.is_empty() && p.env(super::TAG_VAR) == Some(c.tag.as_str()))
        || c.pids.iter().any(|r| table.alive(r.pid, r.start))
}

/// Clients Lodestar started that still run (records of finished ones are deleted).
pub fn started_clients(paths: &Paths) -> Vec<StartedClient> {
    started_clients_in(&records_dir(paths), &ProcTable::real())
}

pub fn started_clients_in(dir: &Path, table: &ProcTable) -> Vec<StartedClient> {
    let procs = table.list();
    read_records(dir)
        .into_iter()
        .filter(|c| {
            let alive = is_alive(c, table, &procs);
            if !alive {
                let _ = std::fs::remove_file(&c.record);
            }
            alive
        })
        .collect()
}

/// The processes [`close_client`] may stop: in the record's prefix, running a program from the
/// client's folders (not their `games` sub-folders), not wineserver, not this process. Empty
/// when the record isn't alive any more (a client the user restarted isn't ours).
pub fn client_processes<'a>(c: &StartedClient, table: &ProcTable, procs: &'a [Proc]) -> Vec<&'a Proc> {
    if !is_alive(c, table, procs) {
        return vec![];
    }
    let dirs: Vec<String> = c.client_dirs.iter().flat_map(|d| procs::dir_forms(&c.prefix, d)).collect();
    let games: Vec<String> = dirs.iter().map(|d| format!("{d}/games")).collect();
    let me = std::process::id() as i32;
    procs
        .iter()
        .filter(|p| {
            p.pid != me
                && p.in_prefix(&c.prefix)
                && p.image().is_some_and(|img| {
                    procs::image_under(&img, &dirs)
                        && !procs::image_under(&img, &games)
                        && !procs::base_name(&img).starts_with("wineserver")
                })
        })
        .collect()
}

/// What closing needs from the system (faked in tests).
pub(crate) trait CloseSys {
    fn table(&self) -> &ProcTable;
    /// Ask the client to close itself.
    fn polite(&mut self, c: &StartedClient);
    fn signal(&mut self, p: &Proc, sig: i32);
    fn sleep(&mut self, d: Duration);
}

struct RealSys {
    table: ProcTable,
}

impl CloseSys for RealSys {
    fn table(&self) -> &ProcTable {
        &self.table
    }
    fn polite(&mut self, c: &StartedClient) {
        polite_close(c, &self.table);
    }
    fn signal(&mut self, p: &Proc, sig: i32) {
        // Same process still (pid not reused)?
        if self.table.alive(p.pid, p.start) {
            // SAFETY: plain kill(2) on a pid just checked to be the recorded process.
            unsafe {
                libc::kill(p.pid, sig);
            }
        }
    }
    fn sleep(&mut self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// Close a client Lodestar started (see the module docs). Removes its record.
pub fn close_client(c: &StartedClient) -> anyhow::Result<CloseReport> {
    close_with(c, &mut RealSys { table: ProcTable::real() })
}

pub(crate) fn close_with(c: &StartedClient, sys: &mut dyn CloseSys) -> anyhow::Result<CloseReport> {
    let mut report = CloseReport::default();
    let current = |sys: &dyn CloseSys| -> Vec<Proc> {
        let procs = sys.table().list();
        // Once the record has been seen alive, keep following the same processes by pid (the
        // tag may be gone with the process that held it).
        client_processes(c, sys.table(), &procs).into_iter().cloned().collect()
    };
    let first = current(sys);
    report.found = first.len();
    if first.is_empty() {
        remove_record(c);
        return Ok(report);
    }
    let still = |sys: &dyn CloseSys, set: &[Proc]| -> Vec<Proc> {
        set.iter().filter(|p| sys.table().alive(p.pid, p.start)).cloned().collect()
    };
    let wait_gone = |sys: &mut dyn CloseSys, set: &[Proc], grace: Duration| -> Vec<Proc> {
        let mut waited = Duration::ZERO;
        loop {
            let left = still(sys, set);
            if left.is_empty() || waited >= grace {
                return left;
            }
            sys.sleep(POLL);
            waited += POLL;
        }
    };
    sys.polite(c);
    let left = wait_gone(sys, &first, POLITE_GRACE);
    if left.is_empty() {
        report.closed_politely = true;
        remove_record(c);
        return Ok(report);
    }
    for p in &left {
        sys.signal(p, libc::SIGTERM);
        report.terminated.push(p.pid);
    }
    let left = wait_gone(sys, &left, TERM_GRACE);
    for p in &left {
        sys.signal(p, libc::SIGKILL);
        report.killed.push(p.pid);
    }
    remove_record(c);
    Ok(report)
}

fn remove_record(c: &StartedClient) {
    if !c.record.as_os_str().is_empty() {
        let _ = std::fs::remove_file(&c.record);
    }
}

/// `taskkill /IM <main>` (no `/F`: WM_CLOSE to the process' windows) through the client's own
/// wineserver, with the sync switches its main process runs with.
fn polite_close(c: &StartedClient, table: &ProcTable) {
    let runner = super::parse_runner(&c.runner);
    let Some(wine) = super::wine_binary(&runner) else {
        return;
    };
    let procs = table.list();
    let sync = procs::main_procs(&procs, &c.prefix, &c.main).first().map(|p| p.sync_env()).unwrap_or_default();
    for name in &c.main {
        let mut cmd = std::process::Command::new(&wine);
        cmd.arg("taskkill")
            .arg("/IM")
            .arg(name)
            .env("WINEPREFIX", &c.prefix)
            .env("WINEDEBUG", "-all")
            .env_remove(super::TAG_VAR)
            .env_remove(crate::launch::track::TAG_VAR)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        for (k, v) in &sync {
            cmd.env(k, v);
        }
        if let Ok(mut child) = cmd.spawn() {
            for _ in 0..40 {
                if child.try_wait().ok().flatten().is_some() {
                    break;
                }
                std::thread::sleep(POLL);
            }
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}
