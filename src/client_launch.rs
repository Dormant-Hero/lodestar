//! `lodestar --client-launch <kind> --prefix P --exe E --runner R [--arg A]… [kind options]`
//!
//! Starts a game through a Windows store client living in a Wine prefix, the way pressing Play
//! does. Every wine-launcher game's LaunchSpec is `@lodestar --client-launch …` (built by
//! `scan::wine_launchers`). Kinds:
//!
//! * `battlenet --code C` — `Battle.net.exe --exec="launch C"`. Not running: start the client
//!   bare, wait (≤ 5 min: login, self-update) for its log to say it's ready, then deliver.
//!   Confirmed by `LaunchBinary: uid=… binaryType=game` in its log; up to 3 deliveries.
//! * `ea --id ID [--id ID]…` — `EALauncher.exe origin2://game/launch?offerIds=<ID>&autoDownload=1`.
//!   IDs in the order to try: the game's content id first, then its offer id (the EA app looks a
//!   link's ids up as *content* ids: `findByContentId … Could not find entitlement for contentId
//!   [OFB-EAST:…]` when handed the offer id, on a real machine). Not running: start it with the
//!   link (it keeps it as a pending action and runs it once ready); running: wait until its log
//!   says `Content is now ready (entitlement store loaded)` for the current session, then
//!   deliver. Confirmed by `Successful launch` in `EADesktop.log`; on `Launch failed with error
//!   [EntitlementNotFound]` ("You don't have access": sent before its entitlements were loaded)
//!   wait for the next ready line and retry, at most 3 times; other errors end it at once.
//! * `generic --client NAME [--main PROC]… [--exe-is-game]` — Ubisoft Connect, Rockstar, Riot,
//!   HoYoPlay, an EA game's own launcher. When the client's main process (`--main`, else the
//!   client's known names, else the exe's file name) runs in the prefix, the request goes through
//!   its own wineserver; otherwise E starts with the args via the prefix's runner (umu-run /
//!   proton / wine), which works from a cold start (Ubisoft verified).
//!
//! Common options: `--prefix` the prefix (the folder with `drive_c`), `--exe` the Windows
//! program (Unix path), `--runner` `proton:<dir>` / `wine:<binary>` / `unknown`, `--arg` one
//! argument for E (repeatable). Exit code 0 = handed over, 1 = failed (reason on stderr),
//! 2 = bad arguments.
//!
//! Found on a real machine (CachyOS, Proton, Wine clients under XWayland):
//! * A request handed over through a NEW `umu-run` / `proton run` is lost when the client already
//!   runs: each starts its own pressure-vessel container that can't reach the running client.
//!   Running the prefix's own Wine build directly (`<proton>/files/bin/wine`, `WINEPREFIX`, the
//!   client's sync switches — fallbacks fsync / esync / none) reaches the running wineserver.
//! * Requests sent while a client is still starting are ignored (Battle.net only selects the
//!   game) or fail (EA app: EntitlementNotFound), so: wait for the ready line, deliver, confirm.
//!
//! When the helper had to start the client itself it leaves a record so the launcher can close
//! it later: see [`record`] (`started_clients`, `close_client`).

pub mod procs;
pub mod record;

pub use record::{StartedClient, close_client, started_clients};

use crate::model::{LaunchSpec, Source};
use crate::scan::winprefix::{self, Prefix, Runner};
use procs::{Proc, ProcTable};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The LaunchSpec program name that means "this Lodestar binary" (resolved at launch time, so it
/// survives Lodestar being reinstalled elsewhere).
pub const SELF_PROGRAM: &str = "@lodestar";

/// Set on a client the helper starts (inherited by all its processes): identifies that start.
pub const TAG_VAR: &str = "LODESTAR_CLIENT_TAG";

/// How long to wait for the client to be ready (first start, login, self-update).
const READY_TIMEOUT: Duration = Duration::from_secs(300);
const READY_POLL: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_secs(1);
/// After delivering, how long to wait for Battle.net's `LaunchBinary` before trying again.
const BNET_CONFIRM: Duration = Duration::from_secs(25);
/// After a request, how long to wait for the EA app's answer.
const EA_CONFIRM: Duration = Duration::from_secs(30);
/// After an EntitlementNotFound, how long to wait for the EA app to (re)load its store.
const EA_RETRY_READY: Duration = Duration::from_secs(60);
/// Retries after the first request.
const ATTEMPTS: usize = 3;
/// How long a delivered request may take to come back (the second instance hands over and
/// exits within a few seconds).
const DELIVER_WAIT: Duration = Duration::from_secs(15);
/// For a program that may keep running (a game, a client that didn't hand over): long enough to
/// see a Wine sync mismatch (it exits at once).
const DELIVER_KEEP_WAIT: Duration = Duration::from_secs(6);

// ---------------------------------------------------------------------------------------------
// Arguments

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    BattleNet,
    Ea,
    Generic,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::BattleNet => "battlenet",
            Kind::Ea => "ea",
            Kind::Generic => "generic",
        }
    }
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "battlenet" => Kind::BattleNet,
            "ea" => Kind::Ea,
            "generic" => Kind::Generic,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Args {
    pub kind: Kind,
    pub prefix: PathBuf,
    pub exe: PathBuf,
    pub runner: Runner,
    /// `--arg`: arguments for `exe` (generic: the request / cold-start arguments).
    pub args: Vec<String>,
    /// battlenet `--code`.
    pub code: String,
    /// ea `--id`: content id, then offer id.
    pub ids: Vec<String>,
    /// `--client`: `battlenet`, `ea`, `ubisoft`, `rockstar`, `riot`, `hoyoplay` (fixed for the
    /// battlenet / ea kinds).
    pub client: String,
    /// `--main`: the client's main process names (default: [`default_main`]).
    pub main: Vec<String>,
    /// `--exe-is-game`: `exe` is the game itself (HoYoPlay, an EA game's own launcher): started
    /// in the launch's own process group and never recorded as a client to close.
    pub exe_is_game: bool,
}

impl Args {
    pub fn new(kind: Kind, prefix: &Path, exe: &Path, runner: &Runner) -> Args {
        Args {
            kind,
            prefix: prefix.to_path_buf(),
            exe: exe.to_path_buf(),
            runner: runner.clone(),
            args: vec![],
            code: String::new(),
            ids: vec![],
            client: match kind {
                Kind::BattleNet => "battlenet".into(),
                Kind::Ea => "ea".into(),
                Kind::Generic => String::new(),
            },
            main: vec![],
            exe_is_game: false,
        }
    }

    /// The arguments after `lodestar` (starting with `--client-launch <kind>`).
    pub fn to_cli(&self) -> Vec<String> {
        let mut v: Vec<String> = vec!["--client-launch".into(), self.kind.as_str().into()];
        let mut opt = |k: &str, val: String| {
            v.push(k.into());
            v.push(val);
        };
        opt("--prefix", self.prefix.display().to_string());
        opt("--exe", self.exe.display().to_string());
        opt("--runner", runner_arg(&self.runner));
        for a in &self.args {
            opt("--arg", a.clone());
        }
        if self.kind == Kind::BattleNet {
            opt("--code", self.code.clone());
        }
        for i in &self.ids {
            opt("--id", i.clone());
        }
        if self.kind == Kind::Generic && !self.client.is_empty() {
            opt("--client", self.client.clone());
        }
        for m in &self.main {
            opt("--main", m.clone());
        }
        if self.exe_is_game {
            v.push("--exe-is-game".into());
        }
        v
    }

    /// The LaunchSpec that runs this through Lodestar itself.
    pub fn spec(&self) -> LaunchSpec {
        LaunchSpec::Command { program: SELF_PROGRAM.into(), args: self.to_cli(), cwd: None, env: vec![] }
    }

    /// The client's main process names.
    pub fn mains(&self) -> Vec<String> {
        if !self.main.is_empty() {
            return self.main.clone();
        }
        let d = default_main(&self.client);
        if !d.is_empty() {
            return d.iter().map(|s| s.to_string()).collect();
        }
        self.exe.file_name().map(|n| vec![n.to_string_lossy().into_owned()]).unwrap_or_default()
    }

    /// For messages: "Battle.net", "the EA app", "Ubisoft Connect" …
    pub fn label(&self) -> String {
        match self.client.as_str() {
            "battlenet" => "Battle.net".into(),
            "ea" => "the EA app".into(),
            "ubisoft" => "Ubisoft Connect".into(),
            "rockstar" => "the Rockstar Games Launcher".into(),
            "riot" => "the Riot Client".into(),
            "hoyoplay" => "HoYoPlay".into(),
            _ => self.exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "the client".into()),
        }
    }

    /// Folders the client's own programs run from (for closing it): the exe's folder, plus
    /// Battle.net's `ProgramData\Battle.net` (Agent.exe) and the EA app's whole install folder.
    pub fn client_dirs(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.exe.parent().map(Path::to_path_buf).into_iter().collect();
        match self.client.as_str() {
            "battlenet" => v.extend(winprefix::win_to_unix(&self.prefix, "C:\\ProgramData\\Battle.net")),
            "ea" => v.extend(
                self.exe
                    .parent()
                    .and_then(Path::parent)
                    .filter(|g| g.file_name().is_some_and(|n| n.eq_ignore_ascii_case("EA Desktop")))
                    .map(Path::to_path_buf),
            ),
            _ => {}
        }
        v
    }
}

/// The client name for a store (`--client`, record files).
pub fn client_name(source: Source) -> Option<&'static str> {
    Some(match source {
        Source::BattleNet => "battlenet",
        Source::Ea => "ea",
        Source::Ubisoft => "ubisoft",
        Source::Rockstar => "rockstar",
        Source::Riot => "riot",
        Source::HoYoPlay => "hoyoplay",
        _ => return None,
    })
}

/// Known main process names of a client.
pub fn default_main(client: &str) -> &'static [&'static str] {
    match client {
        "battlenet" => &["Battle.net.exe"],
        "ea" => &["EADesktop.exe", "Origin.exe"],
        "ubisoft" => &["upc.exe", "UbisoftConnect.exe"],
        "rockstar" => &["Launcher.exe"],
        "riot" => &["RiotClientServices.exe"],
        "hoyoplay" => &["HYP.exe", "launcher.exe"],
        _ => &[],
    }
}

/// `proton:<dir>` / `wine:<binary>` / `unknown`.
pub fn runner_arg(r: &Runner) -> String {
    match r {
        Runner::Proton { path } => format!("proton:{}", path.display()),
        Runner::Wine { path } => format!("wine:{}", path.display()),
        Runner::Unknown => "unknown".into(),
    }
}

pub fn parse_runner(s: &str) -> Runner {
    if let Some(p) = s.strip_prefix("proton:") {
        Runner::Proton { path: PathBuf::from(p) }
    } else if let Some(p) = s.strip_prefix("wine:") {
        Runner::Wine { path: PathBuf::from(p) }
    } else {
        Runner::Unknown
    }
}

/// The launch args Lodestar stores for a Battle.net game.
#[cfg(test)]
pub fn battlenet_args(prefix: &Path, exe: &Path, code: &str, runner: &Runner) -> Vec<String> {
    let mut a = Args::new(Kind::BattleNet, prefix, exe, runner);
    a.code = code.into();
    a.to_cli()
}

/// The prefix a `@lodestar --client-launch …` spec runs in (for matching a game to a
/// [`StartedClient`]).
#[cfg(test)]
pub fn spec_prefix(args: &[String]) -> Option<PathBuf> {
    args.windows(2).find(|w| w[0] == "--prefix").map(|w| PathBuf::from(&w[1]))
}

/// Parse the options after `--client-launch <kind>`.
pub fn parse_kind_args(kind: Kind, it: &mut dyn Iterator<Item = String>) -> anyhow::Result<Args> {
    let mut a = Args::new(kind, Path::new(""), Path::new(""), &Runner::Unknown);
    let (mut prefix, mut exe) = (None, None);
    while let Some(o) = it.next() {
        let mut val = || it.next().ok_or_else(|| anyhow::anyhow!("{o} needs a value"));
        match o.as_str() {
            "--prefix" => prefix = Some(PathBuf::from(val()?)),
            "--exe" => exe = Some(PathBuf::from(val()?)),
            "--runner" => a.runner = parse_runner(&val()?),
            "--arg" => a.args.push(val()?),
            "--code" if kind == Kind::BattleNet => a.code = val()?,
            "--id" if kind == Kind::Ea => a.ids.push(val()?),
            "--client" if kind == Kind::Generic => a.client = val()?,
            "--main" => a.main.push(val()?),
            "--exe-is-game" if kind == Kind::Generic => a.exe_is_game = true,
            other => anyhow::bail!("unknown argument {other} for {}", kind.as_str()),
        }
    }
    a.prefix = prefix.ok_or_else(|| anyhow::anyhow!("--prefix missing"))?;
    a.exe = exe.ok_or_else(|| anyhow::anyhow!("--exe missing"))?;
    a.ids.retain(|i| !i.trim().is_empty());
    match kind {
        Kind::BattleNet if a.code.trim().is_empty() => anyhow::bail!("--code missing"),
        Kind::Ea if a.ids.is_empty() => anyhow::bail!("--id missing"),
        _ => {}
    }
    Ok(a)
}

/// The battlenet kind's options (tests).
#[cfg(test)]
pub fn parse_args(it: &mut dyn Iterator<Item = String>) -> anyhow::Result<Args> {
    parse_kind_args(Kind::BattleNet, it)
}

/// `lodestar --client-launch <kind> …`: returns the process exit code.
pub fn cli(kind: &str, it: &mut dyn Iterator<Item = String>) -> i32 {
    let Some(k) = Kind::parse(kind) else {
        eprintln!("--client-launch: unknown client {kind:?} (battlenet, ea, generic)");
        return 2;
    };
    match parse_kind_args(k, it) {
        Ok(a) => run(&a),
        Err(e) => {
            eprintln!("--client-launch {kind}: {e:#}");
            2
        }
    }
}

/// Run a parsed request on this machine.
pub fn run(a: &Args) -> i32 {
    let mut ops = RealOps::new();
    match launch(a, &mut ops) {
        Ok(msg) => {
            eprintln!("{msg}");
            0
        }
        Err(e) => {
            eprintln!("{} launch: {e:#}", a.label());
            1
        }
    }
}

// ---------------------------------------------------------------------------------------------
// What the flows need from the system (faked in tests)

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Delivery {
    /// The program exited (code if it had one).
    Exited(Option<i32>),
    /// Still running after the wait and left running (`keep`).
    Running,
    /// It couldn't join the running wineserver (sync switches or Wine version differ).
    Unreachable(String),
}

pub(crate) trait Ops {
    fn procs(&self) -> Vec<Proc>;
    /// Start `a.exe` with `args` through the prefix's runner. `tag`: it's the client (set
    /// [`TAG_VAR`], detach it from the launch). Returns the spawned pid.
    fn cold_start(&mut self, a: &Args, args: &[String], tag: Option<&str>) -> anyhow::Result<i32>;
    /// Run `a.exe args` with the prefix's own Wine build (same wineserver) and `sync`.
    fn deliver(&mut self, a: &Args, args: &[String], sync: &[(String, String)], keep: bool)
    -> anyhow::Result<Delivery>;
    fn sleep(&mut self, d: Duration);
    /// Monotonic time.
    fn now(&self) -> Duration;
    fn records_dir(&self) -> PathBuf;
}

struct RealOps {
    t0: Instant,
    table: ProcTable,
    records: PathBuf,
    n: usize,
}

impl RealOps {
    fn new() -> RealOps {
        RealOps {
            t0: Instant::now(),
            table: ProcTable::real(),
            records: record::records_dir(&crate::config::Paths::detect()),
            n: 0,
        }
    }
}

impl Ops for RealOps {
    fn procs(&self) -> Vec<Proc> {
        self.table.list()
    }

    fn cold_start(&mut self, a: &Args, args: &[String], tag: Option<&str>) -> anyhow::Result<i32> {
        use std::os::unix::process::CommandExt;
        let p = Prefix { path: a.prefix.clone(), origin: String::new(), runner: a.runner.clone() };
        let LaunchSpec::Command { program, args, cwd, env } = winprefix::run_in_prefix(&p, &a.exe, args) else {
            anyhow::bail!("unexpected launch for {}", a.label());
        };
        let mut cmd = std::process::Command::new(program);
        cmd.args(args)
            .envs(env.iter().map(|(k, v)| (k, v)))
            // Only this helper needs Lodestar's root (for its records).
            .env_remove("LODESTAR_HOME")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        if let Some(t) = tag {
            // The client outlives the game: not part of the game's launch (Stop / tracking),
            // closed through its record instead.
            cmd.env(TAG_VAR, t).env_remove(crate::launch::track::TAG_VAR).process_group(0);
        }
        if let Some(d) = cwd {
            cmd.current_dir(d);
        }
        let mut child = cmd.spawn()?;
        let pid = child.id() as i32;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(pid)
    }

    fn deliver(
        &mut self,
        a: &Args,
        args: &[String],
        sync: &[(String, String)],
        keep: bool,
    ) -> anyhow::Result<Delivery> {
        let wine = wine_binary(&a.runner).ok_or_else(|| anyhow::anyhow!("no Wine binary for this prefix's runner"))?;
        std::fs::create_dir_all(&self.records)?;
        self.n += 1;
        let err_path = self.records.join(format!("deliver-{}-{}.err", std::process::id(), self.n));
        let err_file = std::fs::File::create(&err_path)?;
        let mut cmd = std::process::Command::new(&wine);
        cmd.arg(winprefix::unix_to_win(&a.prefix, &a.exe))
            .args(args)
            .env("WINEPREFIX", &a.prefix)
            // Errors of the sync code stay on: they tell a mismatch apart.
            .env("WINEDEBUG", "-all,err+esync,err+fsync,err+ntsync")
            .env_remove(TAG_VAR)
            .env_remove("LODESTAR_HOME")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(err_file);
        for k in procs::SYNC_VARS {
            cmd.env_remove(k);
        }
        for (k, v) in sync {
            cmd.env(k, v);
        }
        if let Some(d) = a.exe.parent().filter(|d| d.is_dir()) {
            cmd.current_dir(d);
        }
        let mut child = cmd.spawn()?;
        let wait = if keep { DELIVER_KEEP_WAIT } else { DELIVER_WAIT };
        let t0 = Instant::now();
        let mut status = None;
        while t0.elapsed() < wait {
            if let Some(s) = child.try_wait()? {
                status = Some(s);
                break;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        let err = std::fs::read_to_string(&err_path).unwrap_or_default();
        let _ = std::fs::remove_file(&err_path);
        if let Some(why) = unreachable_reason(&err) {
            if status.is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
            return Ok(Delivery::Unreachable(why));
        }
        Ok(match status {
            Some(s) => Delivery::Exited(s.code()),
            None if keep => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                Delivery::Running
            }
            None => {
                let _ = child.kill();
                let _ = child.wait();
                Delivery::Exited(None)
            }
        })
    }

    fn sleep(&mut self, d: Duration) {
        std::thread::sleep(d);
    }

    fn now(&self) -> Duration {
        self.t0.elapsed()
    }

    fn records_dir(&self) -> PathBuf {
        self.records.clone()
    }
}

/// Wine's own complaint when a process can't join the running wineserver: a different sync
/// method (`Server is running with WINEFSYNC but this process is not …`, `Failed to open esync
/// shared memory file; make sure no stale wineserver instances are running without WINEESYNC`)
/// or a different Wine build (`version mismatch`).
pub(crate) fn unreachable_reason(stderr: &str) -> Option<String> {
    stderr
        .lines()
        .find(|l| {
            let l = l.to_ascii_lowercase();
            l.contains("server is running with")
                || l.contains("server is not running with")
                || (l.contains("sync") && l.contains("wineserver"))
                || l.contains("version mismatch")
        })
        .map(|l| l.trim().to_string())
}

pub(crate) fn wine_binary(r: &Runner) -> Option<PathBuf> {
    match r {
        Runner::Proton { path } => {
            ["files/bin/wine", "dist/bin/wine"].iter().map(|s| path.join(s)).find(|p| p.is_file())
        }
        Runner::Wine { path } => Some(path.clone()).filter(|p| p.is_file()),
        Runner::Unknown => winprefix::which("wine"),
    }
}

// ---------------------------------------------------------------------------------------------
// Flows

pub(crate) fn launch(a: &Args, ops: &mut dyn Ops) -> anyhow::Result<String> {
    match a.kind {
        Kind::BattleNet => battlenet(a, ops),
        Kind::Ea => ea(a, ops),
        Kind::Generic => generic(a, ops),
    }
}

/// The client's main processes running in the prefix.
fn running(a: &Args, ops: &dyn Ops) -> Vec<Proc> {
    let all = ops.procs();
    procs::main_procs(&all, &a.prefix, &a.mains()).into_iter().cloned().collect()
}

/// Poll `cond` until it holds or `timeout` passes.
fn wait_for(ops: &mut dyn Ops, timeout: Duration, poll: Duration, cond: &mut dyn FnMut(&dyn Ops) -> bool) -> bool {
    let t0 = ops.now();
    loop {
        if cond(ops) {
            return true;
        }
        if ops.now().saturating_sub(t0) >= timeout {
            return false;
        }
        ops.sleep(poll);
    }
}

/// Start the client (or, `exe_is_game`, the game) cold; a client gets a record.
fn start_cold(a: &Args, ops: &mut dyn Ops, args: &[String]) -> anyhow::Result<Option<StartedClient>> {
    eprintln!("starting {} in {}", a.label(), a.prefix.display());
    if a.exe_is_game {
        ops.cold_start(a, args, None)?;
        return Ok(None);
    }
    let started_at =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let client = if a.client.is_empty() { "client" } else { a.client.as_str() };
    let tag = format!("{}-{client}-{started_at}-{}", winprefix::prefix_hash(&a.prefix), std::process::id());
    let pid = ops.cold_start(a, args, Some(&tag))?;
    let mut c = StartedClient {
        kind: a.kind.as_str().into(),
        client: client.into(),
        prefix: a.prefix.clone(),
        exe: a.exe.clone(),
        client_dirs: a.client_dirs(),
        runner: runner_arg(&a.runner),
        main: a.mains(),
        pids: vec![],
        tag,
        game_tag: std::env::var(crate::launch::track::TAG_VAR).ok(),
        started_at,
        record: PathBuf::new(),
    };
    if let Some(p) = ops.procs().iter().find(|p| p.pid == pid) {
        c.pids.push(record::RecordedPid { pid, start: p.start });
    }
    match record::write_record(&ops.records_dir(), &c) {
        Ok(path) => c.record = path,
        Err(e) => eprintln!("couldn't record the started client: {e:#}"),
    }
    Ok(Some(c))
}

/// Add the client's main processes (now that it runs) to its record.
fn refresh_record(a: &Args, ops: &dyn Ops, c: &mut Option<StartedClient>) {
    let Some(c) = c else { return };
    for p in running(a, ops) {
        if !c.pids.iter().any(|r| r.pid == p.pid) {
            c.pids.push(record::RecordedPid { pid: p.pid, start: p.start });
        }
    }
    if !c.record.as_os_str().is_empty()
        && let Err(e) = record::write_record(&ops.records_dir(), c)
    {
        eprintln!("couldn't update the started client's record: {e:#}");
    }
}

/// Sync switches to try, in order: the running client's own, then fsync, esync, neither.
fn sync_candidates(client: Vec<(String, String)>) -> Vec<Vec<(String, String)>> {
    let kv = |v: &[(&str, &str)]| -> Vec<(String, String)> {
        v.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    };
    let mut out: Vec<Vec<(String, String)>> = vec![];
    if !client.is_empty() {
        out.push(client);
    }
    for c in [
        kv(&[("WINEFSYNC", "1")]),
        kv(&[("WINEFSYNC", "0"), ("WINEESYNC", "1")]),
        kv(&[("WINEFSYNC", "0"), ("WINEESYNC", "0")]),
    ] {
        let on = |v: &[(String, String)]| -> Vec<String> {
            let mut s: Vec<String> = v.iter().filter(|(_, x)| x != "0").map(|(k, _)| k.clone()).collect();
            s.sort();
            s
        };
        if !out.iter().any(|o| on(o) == on(&c)) {
            out.push(c);
        }
    }
    out
}

/// A log set plus the test for the line that proves a delivery arrived.
type Confirm<'a> = (&'a LogSet, fn(&str) -> bool);

/// Hand `args` to the running client through its own wineserver, trying sync methods until one
/// reaches it. `confirm`: a log line that proves it arrived (else: no Wine complaint is enough).
fn deliver_any_sync(
    a: &Args,
    ops: &mut dyn Ops,
    args: &[String],
    keep: bool,
    confirm: Option<Confirm>,
) -> anyhow::Result<Delivery> {
    let client_sync = running(a, ops).first().map(Proc::sync_env).unwrap_or_default();
    let mut why = String::new();
    for sync in sync_candidates(client_sync) {
        let before = confirm.map(|(l, _)| l.mark());
        match ops.deliver(a, args, &sync, keep)? {
            Delivery::Unreachable(w) => why = w,
            d => match (confirm, &before) {
                (Some((logs, is_it)), Some(m)) => {
                    if wait_for(ops, Duration::from_secs(5), POLL, &mut |_| logs.since(m).iter().any(|l| is_it(l))) {
                        return Ok(d);
                    }
                    why = "no sign of the request in its log".into();
                }
                _ => return Ok(d),
            },
        }
    }
    anyhow::bail!(
        "couldn't reach {} (running) through its wineserver ({why}) — close it and press Play again",
        a.label()
    )
}

fn battlenet(a: &Args, ops: &mut dyn Ops) -> anyhow::Result<String> {
    let logs = LogSet::new(&a.prefix, LogKind::BattleNet);
    let mut mark = logs.mark();
    if running(a, ops).is_empty() {
        // Started bare: on a cold start Battle.net only selects the game named in --exec.
        let mut c = start_cold(a, ops, &[])?;
        // Ready = logged in with its main window up (a login or update can take minutes).
        if !wait_for(ops, READY_TIMEOUT, READY_POLL, &mut |_| logs.since(&mark).iter().any(|l| is_bnet_ready(l))) {
            eprintln!("Battle.net didn't report ready; trying anyway");
        }
        // Give the UI a moment to finish loading the game list.
        ops.sleep(Duration::from_secs(4));
        refresh_record(a, ops, &mut c);
        mark = logs.mark();
    }
    let req = vec![format!("--exec=launch {}", a.code)];
    for attempt in 1..=ATTEMPTS {
        deliver_any_sync(a, ops, &req, false, Some((&logs, |l: &str| l.contains("--exec=launch"))))?;
        if wait_for(ops, BNET_CONFIRM, POLL, &mut |_| logs.since(&mark).iter().any(|l| is_bnet_launch(l))) {
            return Ok(format!("Battle.net launched {} (attempt {attempt})", a.code));
        }
        eprintln!("Battle.net didn't launch {} yet (attempt {attempt})", a.code);
    }
    anyhow::bail!("Battle.net didn't start {} — open Battle.net and press Play", a.code)
}

/// The link the EA app takes for a game id.
pub fn ea_link(id: &str) -> String {
    format!("origin2://game/launch?offerIds={id}&autoDownload=1")
}

/// The EA app's answer to a launch request, from its log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EaOutcome {
    Launched(String),
    /// `Launch failed with error [<error>]`; `ready_after`: a ready line followed it.
    Failed {
        error: String,
        ready_after: bool,
    },
    Nothing,
}

pub(crate) fn ea_outcome(lines: &[String]) -> EaOutcome {
    for (i, l) in lines.iter().enumerate() {
        if l.contains("Successful launch") {
            return EaOutcome::Launched(l.trim().to_string());
        }
        if let Some(rest) = l.split("Launch failed with error [").nth(1) {
            return EaOutcome::Failed {
                error: rest.split(']').next().unwrap_or("").to_string(),
                ready_after: lines[i + 1..].iter().any(|l| is_ea_ready(l)),
            };
        }
    }
    EaOutcome::Nothing
}

fn ea(a: &Args, ops: &mut dyn Ops) -> anyhow::Result<String> {
    let logs = LogSet::new(&a.prefix, LogKind::Ea);
    let ids = &a.ids;
    // Content id twice, then the offer id twice (the first try may just be too early).
    let id_for = |n: usize| ids[(n / 2) % ids.len()].clone();
    let mut sent = 0usize; // requests sent so far (the cold start's pending link counts)
    let mut mark = logs.mark();
    if running(a, ops).is_empty() {
        let mut c = start_cold(a, ops, &[ea_link(&id_for(0))])?;
        sent = 1;
        eprintln!("waiting for the EA app to be ready (sign in there if it asks)");
        if !wait_for(ops, READY_TIMEOUT, READY_POLL, &mut |_| logs.since(&mark).iter().any(|l| is_ea_ready(l))) {
            eprintln!("the EA app didn't report ready; waiting for its answer anyway");
        }
        refresh_record(a, ops, &mut c);
    } else if !ea_ready_now(&logs.all()) {
        eprintln!("waiting for the EA app to be ready (sign in there if it asks)");
        if !wait_for(ops, READY_TIMEOUT, READY_POLL, &mut |_| logs.since(&mark).iter().any(|l| is_ea_ready(l))) {
            eprintln!("the EA app didn't report ready; trying anyway");
        }
    }
    let mut last: String;
    loop {
        if sent > 0 {
            let mut outcome = EaOutcome::Nothing;
            wait_for(ops, EA_CONFIRM, POLL, &mut |_| {
                outcome = ea_outcome(&logs.since(&mark));
                outcome != EaOutcome::Nothing
            });
            match outcome {
                EaOutcome::Launched(l) => return Ok(format!("the EA app launched the game: {l}")),
                EaOutcome::Failed { error, ready_after } if error == "EntitlementNotFound" => {
                    last = "it answered EntitlementNotFound (\"You don't have access\")".into();
                    if sent > ATTEMPTS {
                        break;
                    }
                    if !ready_after {
                        // Not ready: wait for the store to (re)load before asking again.
                        let m = logs.mark();
                        wait_for(ops, EA_RETRY_READY, READY_POLL, &mut |_| {
                            logs.since(&m).iter().any(|l| is_ea_ready(l))
                        });
                    }
                }
                EaOutcome::Failed { error, .. } => {
                    anyhow::bail!("the EA app couldn't start the game ({error}) — open the EA app and press Play")
                }
                EaOutcome::Nothing => {
                    last = "no answer in its log".into();
                    if sent > ATTEMPTS {
                        break;
                    }
                }
            }
        }
        mark = logs.mark();
        let id = id_for(sent);
        eprintln!("asking the EA app to launch {id} (request {})", sent + 1);
        deliver_any_sync(a, ops, &[ea_link(&id)], false, None)?;
        sent += 1;
    }
    anyhow::bail!(
        "the EA app didn't start the game after {sent} requests for {} ({last}) — open the EA app and press Play",
        ids.join(" / ")
    )
}

fn generic(a: &Args, ops: &mut dyn Ops) -> anyhow::Result<String> {
    if running(a, ops).is_empty() {
        start_cold(a, ops, &a.args)?;
        return Ok(format!("started {}", a.label()));
    }
    let d = deliver_any_sync(a, ops, &a.args, true, None)?;
    Ok(format!("handed the request to {} (running) ({d:?})", a.label()))
}

// ---------------------------------------------------------------------------------------------
// Log markers

fn is_bnet_ready(l: &str) -> bool {
    l.contains("Active product set to") || l.contains("Attempting to show main window")
}

/// `LaunchBinary: uid=osi … binaryType=game` — Battle.net's uid is the product code in lower case
/// (osi, wow, pro, fen …) or a variant (wow_classic); any game LaunchBinary counts.
fn is_bnet_launch(l: &str) -> bool {
    l.contains("LaunchBinary: uid=")
        && l.contains("binaryType=game")
        && l.split("uid=").nth(1).and_then(|r| r.split_whitespace().next()).is_some_and(|u| !u.is_empty())
}

fn is_ea_ready(l: &str) -> bool {
    l.contains("Content is now ready (entitlement store loaded)")
}

/// The EA app's current session (after its last `[STARTUP] app[EADesktop]` line) has loaded its
/// entitlements.
pub(crate) fn ea_ready_now(lines: &[String]) -> bool {
    let start = lines.iter().rposition(|l| l.contains("[STARTUP] app[EADesktop]")).map_or(0, |i| i + 1);
    lines[start..].iter().any(|l| is_ea_ready(l))
}

// ---------------------------------------------------------------------------------------------
// Client logs

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LogKind {
    /// `drive_c/users/*/AppData/Local/Battle.net/Logs/battle.net-*.log`
    BattleNet,
    /// `drive_c/ProgramData/EA Desktop/Logs/EADesktop.log`
    Ea,
}

/// A client's log files in a prefix (looked up again on every read: they may appear only once
/// the client first runs).
pub(crate) struct LogSet {
    prefix: PathBuf,
    kind: LogKind,
}

/// (file, length) of every log at a point in time; `since` returns lines written after it.
pub(crate) type Mark = Vec<(PathBuf, u64)>;

impl LogSet {
    pub(crate) fn new(prefix: &Path, kind: LogKind) -> LogSet {
        LogSet { prefix: prefix.to_path_buf(), kind }
    }

    fn files(&self) -> Vec<PathBuf> {
        let mut v = vec![];
        match self.kind {
            LogKind::BattleNet => {
                let users = self.prefix.join("drive_c/users");
                for u in std::fs::read_dir(&users).into_iter().flatten().flatten() {
                    let d = u.path().join("AppData/Local/Battle.net/Logs");
                    for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                        let n = e.file_name();
                        let n = n.to_string_lossy();
                        if n.starts_with("battle.net-") && n.ends_with(".log") {
                            v.push(e.path());
                        }
                    }
                }
            }
            LogKind::Ea => {
                let d = winprefix::win_to_unix(&self.prefix, "C:\\ProgramData\\EA Desktop\\Logs")
                    .unwrap_or_else(|| self.prefix.join("drive_c/ProgramData/EA Desktop/Logs"));
                v.extend(Some(d.join("EADesktop.log")).filter(|f| f.is_file()));
            }
        }
        v.sort();
        v
    }

    pub(crate) fn mark(&self) -> Mark {
        self.files()
            .into_iter()
            .map(|f| {
                let len = std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
                (f, len)
            })
            .collect()
    }

    /// Lines added since `mark` (new files whole, old ones from their length then, a file that
    /// got shorter — rotated — whole).
    pub(crate) fn since(&self, mark: &Mark) -> Vec<String> {
        let mut out = vec![];
        for f in self.files() {
            let from = mark.iter().find(|(p, _)| *p == f).map(|(_, l)| *l).unwrap_or(0);
            let Ok(mut file) = std::fs::File::open(&f) else {
                continue;
            };
            let len = file.metadata().map(|m| m.len()).unwrap_or(0);
            let from = if len < from { 0 } else { from };
            if len == from || file.seek(SeekFrom::Start(from)).is_err() {
                continue;
            }
            let mut bytes = vec![];
            if file.read_to_end(&mut bytes).is_ok() {
                out.extend(String::from_utf8_lossy(&bytes).lines().map(str::to_string));
            }
        }
        out
    }

    /// Every line of every log file.
    pub(crate) fn all(&self) -> Vec<String> {
        self.since(&vec![])
    }
}

#[cfg(test)]
mod tests;
