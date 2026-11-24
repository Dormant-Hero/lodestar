//! Tests for the store-client helper: arguments, log markers, process tables (temp `/proc`-like
//! folders), the launch flows against a fake system, and closing a started client. Nothing here
//! starts or signals a real process.

use super::record::{self, CloseSys, RecordedPid, StartedClient};
use super::*;
use crate::test_support::sv;
use std::collections::HashSet;

// ---------------------------------------------------------------------------------------------
// Arguments

#[test]
fn args_roundtrip_every_kind() {
    let r = Runner::Proton { path: "/p/GE-Proton11-12".into() };
    let v = battlenet_args(
        Path::new("/home/u/Games/battlenet"),
        Path::new("/home/u/Games/battlenet/drive_c/Program Files (x86)/Battle.net/Battle.net.exe"),
        "OSI",
        &r,
    );
    assert_eq!(v[..2], sv(&["--client-launch", "battlenet"]));
    let a = parse_args(&mut v.into_iter().skip(2)).unwrap();
    assert_eq!((a.kind, a.code.as_str(), &a.runner), (Kind::BattleNet, "OSI", &r));
    assert_eq!(a.prefix, Path::new("/home/u/Games/battlenet"));
    assert_eq!(a.mains(), sv(&["Battle.net.exe"]));

    let mut ea = Args::new(Kind::Ea, Path::new("/g/ea"), Path::new("/g/ea/drive_c/EA/EALauncher.exe"), &r);
    ea.ids = sv(&["1015875", "OFB-EAST:109552408"]);
    let cli = ea.to_cli();
    assert_eq!(cli[1], "ea");
    assert_eq!(parse_kind_args(Kind::Ea, &mut cli.into_iter().skip(2)).unwrap(), ea);

    let mut g = Args::new(
        Kind::Generic,
        Path::new("/g/ubi"),
        Path::new("/g/ubi/drive_c/U/UbisoftConnect.exe"),
        &Runner::Unknown,
    );
    g.client = "ubisoft".into();
    g.args = sv(&["uplay://launch/635/0"]);
    let cli = g.to_cli();
    assert!(cli.windows(2).any(|w| w == ["--arg", "uplay://launch/635/0"]));
    assert_eq!(spec_prefix(&cli), Some(PathBuf::from("/g/ubi")));
    let back = parse_kind_args(Kind::Generic, &mut cli.into_iter().skip(2)).unwrap();
    assert_eq!(back, g);
    assert_eq!(back.mains(), sv(&["upc.exe", "UbisoftConnect.exe"]));

    let mut h =
        Args::new(Kind::Generic, Path::new("/g/h"), Path::new("/g/h/drive_c/G/GenshinImpact.exe"), &Runner::Unknown);
    h.client = "hoyoplay".into();
    h.exe_is_game = true;
    let cli = h.to_cli();
    assert_eq!(parse_kind_args(Kind::Generic, &mut cli.into_iter().skip(2)).unwrap(), h);

    // Missing / wrong options.
    assert!(parse_args(&mut sv(&["--code", "OSI"]).into_iter()).is_err());
    assert!(parse_kind_args(Kind::Ea, &mut sv(&["--prefix", "/p", "--exe", "/p/x.exe"]).into_iter()).is_err());
    assert!(parse_kind_args(Kind::Ea, &mut sv(&["--prefix", "/p", "--exe", "/e", "--code", "X"]).into_iter()).is_err());
    assert!(parse_kind_args(Kind::Generic, &mut sv(&["--prefix", "/p", "--exe"]).into_iter()).is_err());
    assert_eq!(cli_code("nope"), 2);
}

fn cli_code(kind: &str) -> i32 {
    cli(kind, &mut std::iter::empty())
}

#[test]
fn client_dirs_cover_the_clients_helpers() {
    let dir = tempfile::tempdir().unwrap();
    let pfx = dir.path();
    std::fs::create_dir_all(pfx.join("drive_c/ProgramData/Battle.net/Agent")).unwrap();
    let b = Args::new(
        Kind::BattleNet,
        pfx,
        &pfx.join("drive_c/Program Files (x86)/Battle.net/Battle.net.exe"),
        &Runner::Unknown,
    );
    assert_eq!(
        b.client_dirs(),
        vec![pfx.join("drive_c/Program Files (x86)/Battle.net"), pfx.join("drive_c/ProgramData/Battle.net")]
    );
    let ea_dir = pfx.join("drive_c/Program Files/Electronic Arts/EA Desktop");
    let e = Args::new(Kind::Ea, pfx, &ea_dir.join("EA Desktop/EALauncher.exe"), &Runner::Unknown);
    assert_eq!(e.client_dirs(), vec![ea_dir.join("EA Desktop"), ea_dir.clone()]);
}

// ---------------------------------------------------------------------------------------------
// Log markers (lines as the real clients write them)

const EA_READY: &str = "   165\t[2026-11-21T23:59:04.647Z]\tPID:   596\tTID:   996\tINFO    \t(eax::apps::DesktopCoordinator::FsmHost::handleEntitlementCacheUpdated)\tContent is now ready (entitlement store loaded). pendingActions:[1], moreActions:[false]";
const EA_NOT_FOUND: &str = "   255\t[2026-11-21T23:59:05.183Z]\tPID:   596\tTID:   984\tWARN    \t(eax::components::launchCheck::LaunchCheckComponent::Impl::handleContentLaunchRequest)\tLaunch failed with error [EntitlementNotFound]. Request entitle and launch through SPA. IDs: offerKey.offerId=[], offerId=[], contentIds=[OFB-EAST:109552408], slug=[]";
const EA_OK: &str = "   333\t[2026-11-21T23:59:53.705Z]\tPID:   596\tTID:   984\tINFO    \t(eax::components::launchCheck::LaunchCheckComponent::Impl::handleContentLaunchRequest)\tSuccessful launch. IDs: offerKey.offerId=[OFB-EAST:109552408], offerId=[], contentIds=[], slug=[the-sims-4]";
const EA_STARTUP: &str = "     3\t[2026-11-21T22:18:19.745Z]\tPID:   632\tTID:   636\tINFO    \t(eax::apps::utils::logAppInfo)\t[STARTUP] app[EADesktop], configurable[false], buildType[opt], env[production], elevated[true], processId[632]";

#[test]
fn battle_net_log_markers() {
    assert!(is_bnet_ready("D 2026-11-21 23:30:54.773199 [GameController] {Main} Active product set to: OSI_retail"));
    assert!(is_bnet_launch(
        "I 2026-11-21 23:34:20.414335 [GameLaunchController] {Main} LaunchBinary: uid=osi selectedRegion=EU binaryType=game"
    ));
    assert!(!is_bnet_launch("D 2026-11-21 23:34:20.414213 [GameController] {Main} Selecting game family by id: OSI"));
}

#[test]
fn ea_log_markers() {
    assert!(is_ea_ready(EA_READY));
    let l = |v: &[&str]| sv(v);
    assert_eq!(ea_outcome(&l(&[EA_READY, "other"])), EaOutcome::Nothing);
    assert_eq!(
        ea_outcome(&l(&[EA_READY, EA_NOT_FOUND])),
        EaOutcome::Failed { error: "EntitlementNotFound".into(), ready_after: false }
    );
    assert_eq!(
        ea_outcome(&l(&[EA_NOT_FOUND, EA_READY])),
        EaOutcome::Failed { error: "EntitlementNotFound".into(), ready_after: true }
    );
    assert!(matches!(ea_outcome(&l(&[EA_OK])), EaOutcome::Launched(m) if m.contains("the-sims-4")));
    // Ready only counts for the current session (after the last STARTUP line).
    assert!(ea_ready_now(&l(&[EA_STARTUP, EA_READY])));
    assert!(!ea_ready_now(&l(&[EA_STARTUP, EA_READY, EA_STARTUP, "loading"])));
    assert!(!ea_ready_now(&[]));
    assert_eq!(ea_link("1015875"), "origin2://game/launch?offerIds=1015875&autoDownload=1");
}

#[test]
fn wine_complaints_about_the_wineserver() {
    for e in [
        "0024:err:fsync:fsync_init Server is running with WINEFSYNC but this process is not, please enable WINEFSYNC or restart wineserver.",
        "0024:err:esync:esync_init Server is running with WINEESYNC but this process is not, please enable WINEESYNC or restart wineserver.",
        "0024:err:esync:esync_init Failed to open esync shared memory file; make sure no stale wineserver instances are running without WINEESYNC.",
        "wine client error:0: version mismatch 790/801.\nYour wineserver binary was not upgraded correctly,",
    ] {
        assert!(unreachable_reason(e).is_some(), "{e}");
    }
    assert_eq!(unreachable_reason("0130:fixme:ntdll:NtQuerySystemInformation info_class"), None);
    assert_eq!(unreachable_reason(""), None);
}

#[test]
fn sync_candidates_start_with_the_clients_own() {
    let own = vec![("WINEESYNC".to_string(), "1".to_string())];
    let c = sync_candidates(own.clone());
    assert_eq!(c[0], own);
    // esync on is the client's own already: not tried twice.
    assert_eq!(c.len(), 3);
    assert_eq!(sync_candidates(vec![]).len(), 3);
    assert_eq!(sync_candidates(vec![]).first().unwrap(), &vec![("WINEFSYNC".to_string(), "1".to_string())]);
}

#[test]
fn logs_since_mark_reads_only_new_lines() {
    let dir = tempfile::tempdir().unwrap();
    let logs_dir = dir.path().join("drive_c/users/u/AppData/Local/Battle.net/Logs");
    std::fs::create_dir_all(&logs_dir).unwrap();
    std::fs::write(logs_dir.join("battle.net-1.log"), "old line\n").unwrap();
    let l = LogSet::new(dir.path(), LogKind::BattleNet);
    let m = l.mark();
    append(&logs_dir.join("battle.net-1.log"), "new A");
    std::fs::write(logs_dir.join("battle.net-2.log"), "new B\n").unwrap();
    std::fs::write(logs_dir.join("other.log"), "not a client log\n").unwrap();
    let mut got = l.since(&m);
    got.sort();
    assert_eq!(got, sv(&["new A", "new B"]));
    // A rotated (shorter) file is read whole.
    let m = l.mark();
    std::fs::write(logs_dir.join("battle.net-1.log"), "x\n").unwrap();
    assert_eq!(l.since(&m), sv(&["x"]));
    assert_eq!(l.all().len(), 2);
}

fn append(f: &Path, line: &str) {
    use std::io::Write;
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    let mut h = std::fs::OpenOptions::new().create(true).append(true).open(f).unwrap();
    writeln!(h, "{line}").unwrap();
}

// ---------------------------------------------------------------------------------------------
// Process tables

/// A process in a `/proc`-like fixture folder.
fn fake_proc(root: &Path, pid: i32, start: u64, cmd: &[&str], env: &[(&str, &str)]) {
    use crate::procfs::fake::{self, P};
    let env: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
    let env: Vec<&str> = env.iter().map(String::as_str).collect();
    fake::add(root, P::new(pid, start, cmd).env(&env));
}

#[test]
fn main_processes_of_a_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proc");
    let pfx = Path::new("/home/u/Games/bnet");
    fake_proc(
        &root,
        10,
        1,
        &["C:\\Program Files (x86)\\Battle.net\\Battle.net.exe"],
        &[("WINEPREFIX", "/home/u/Games/bnet/"), ("WINEFSYNC", "1")],
    );
    fake_proc(
        &root,
        11,
        1,
        &["C:\\Program Files (x86)\\Battle.net\\Battle.net.exe", "--type=gpu-process"],
        &[("WINEPREFIX", "/home/u/Games/bnet")],
    );
    fake_proc(
        &root,
        12,
        1,
        &["C:\\Program Files (x86)\\Battle.net\\Battle.net.exe"],
        &[("WINEPREFIX", "/home/u/Games/other")],
    );
    // umu / Proton pass the compat root; Wine loaders put the image second.
    fake_proc(
        &root,
        13,
        1,
        &["/opt/proton/files/bin/wine64", "C:\\Program Files\\Ubisoft\\upc.exe"],
        &[("WINEPREFIX", "/home/u/Games/bnet/pfx")],
    );
    fake_proc(&root, 14, 1, &["/usr/bin/bash"], &[]);
    let t = ProcTable { root: root.clone() };
    let all = t.list();
    assert_eq!(all.len(), 5);
    let ids = |v: Vec<&Proc>| v.iter().map(|p| p.pid).collect::<Vec<_>>();
    assert_eq!(ids(procs::main_procs(&all, pfx, &sv(&["battle.net.exe"]))), vec![10]);
    assert_eq!(ids(procs::main_procs(&all, &pfx.join("pfx"), &sv(&["UPC.exe"]))), vec![13]);
    let p10 = all.iter().find(|p| p.pid == 10).unwrap();
    assert_eq!(p10.sync_env(), vec![("WINEFSYNC".to_string(), "1".to_string())]);
    assert!(t.alive(10, 1));
    assert!(!t.alive(10, 2), "a reused pid isn't the same process");
    assert!(!t.alive(99, 1));
}

// ---------------------------------------------------------------------------------------------
// Flows against a fake system

type DeliverFn = Box<dyn FnMut(usize, &[String], &[(String, String)]) -> (Delivery, Vec<(u64, String)>)>;

struct Fake {
    t: Duration,
    procs: Vec<Proc>,
    log: PathBuf,
    pending: Vec<(Duration, String)>,
    /// Added when the client is started cold, with log lines (seconds after the start).
    cold_proc: Option<Proc>,
    cold_lines: Vec<(u64, String)>,
    colds: Vec<(Vec<String>, Option<String>)>,
    deliveries: Vec<DeliveryRecord>,
    on_deliver: DeliverFn,
    records: PathBuf,
}

/// One handed-over launch: (args, env, accepted).
type DeliveryRecord = (Vec<String>, Vec<(String, String)>, bool);

impl Fake {
    fn new(log: PathBuf, records: PathBuf) -> Fake {
        Fake {
            t: Duration::ZERO,
            procs: vec![],
            log,
            pending: vec![],
            cold_proc: None,
            cold_lines: vec![],
            colds: vec![],
            deliveries: vec![],
            on_deliver: Box::new(|_, _, _| (Delivery::Exited(Some(0)), vec![])),
            records,
        }
    }
    fn schedule(&mut self, lines: Vec<(u64, String)>) {
        for (dt, l) in lines {
            self.pending.push((self.t + Duration::from_secs(dt), l));
        }
        self.flush();
    }
    fn flush(&mut self) {
        let t = self.t;
        let (due, later): (Vec<_>, Vec<_>) = self.pending.drain(..).partition(|(at, _)| *at <= t);
        self.pending = later;
        for (_, l) in due {
            append(&self.log, &l);
        }
    }
}

impl Ops for Fake {
    fn procs(&self) -> Vec<Proc> {
        self.procs.clone()
    }
    fn cold_start(&mut self, _a: &Args, args: &[String], tag: Option<&str>) -> anyhow::Result<i32> {
        self.colds.push((args.to_vec(), tag.map(str::to_string)));
        if let Some(mut p) = self.cold_proc.clone() {
            if let Some(t) = tag {
                p.env.push((TAG_VAR.into(), t.into()));
            }
            self.procs.push(p);
        }
        let lines = std::mem::take(&mut self.cold_lines);
        self.schedule(lines);
        Ok(4242)
    }
    fn deliver(
        &mut self,
        _a: &Args,
        args: &[String],
        sync: &[(String, String)],
        keep: bool,
    ) -> anyhow::Result<Delivery> {
        self.deliveries.push((args.to_vec(), sync.to_vec(), keep));
        let (d, lines) = (self.on_deliver)(self.deliveries.len(), args, sync);
        self.schedule(lines);
        Ok(d)
    }
    fn sleep(&mut self, d: Duration) {
        self.t += d;
        self.flush();
    }
    fn now(&self) -> Duration {
        self.t
    }
    fn records_dir(&self) -> PathBuf {
        self.records.clone()
    }
}

fn wine_proc(pid: i32, prefix: &Path, image: &str, env: &[(&str, &str)]) -> Proc {
    let mut e = vec![("WINEPREFIX".to_string(), prefix.display().to_string())];
    e.extend(env.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    Proc { pid, start: 77, cmdline: vec![image.into()], env: e }
}

const EA_EXE: &str = "C:\\Program Files\\Electronic Arts\\EA Desktop\\EA Desktop\\EADesktop.exe";

struct EaSetup {
    _dir: tempfile::TempDir,
    args: Args,
    fake: Fake,
}

fn ea_setup() -> EaSetup {
    let dir = tempfile::tempdir().unwrap();
    let pfx = dir.path().join("pfx");
    let logs = pfx.join("drive_c/ProgramData/EA Desktop/Logs");
    std::fs::create_dir_all(&logs).unwrap();
    let exe = pfx.join("drive_c/Program Files/Electronic Arts/EA Desktop/EA Desktop/EALauncher.exe");
    let mut args = Args::new(Kind::Ea, &pfx, &exe, &Runner::Unknown);
    args.ids = sv(&["1015875", "OFB-EAST:109552408"]);
    let fake = Fake::new(logs.join("EADesktop.log"), dir.path().join("cache/clients"));
    EaSetup { _dir: dir, args, fake }
}

#[test]
fn ea_cold_start_retries_after_entitlement_not_found() {
    let EaSetup { _dir, args, mut fake } = ea_setup();
    append(&fake.log, EA_STARTUP); // an earlier session
    append(&fake.log, EA_READY);
    fake.cold_proc = Some(wine_proc(500, &args.prefix, EA_EXE, &[("WINEFSYNC", "1")]));
    // Cold: the pending link is answered "no access" right at the first ready line.
    fake.cold_lines =
        vec![(1, EA_STARTUP.into()), (20, EA_READY.into()), (20, EA_NOT_FOUND.into()), (25, EA_READY.into())];
    fake.on_deliver = Box::new(|_, _, _| (Delivery::Exited(Some(0)), vec![(1, EA_OK.into())]));
    let msg = launch(&args, &mut fake).unwrap();
    assert!(msg.contains("Successful launch"), "{msg}");
    // Started with the content id link, retried once with the same id through the wineserver
    // with the client's own sync switches.
    assert_eq!(fake.colds.len(), 1);
    assert_eq!(fake.colds[0].0, vec![ea_link("1015875")]);
    assert_eq!(fake.deliveries.len(), 1);
    assert_eq!(fake.deliveries[0].0, vec![ea_link("1015875")]);
    assert_eq!(fake.deliveries[0].1, vec![("WINEFSYNC".to_string(), "1".to_string())]);
    assert!(!fake.deliveries[0].2);
    // The record of the client it started.
    let recs = record::read_records(&fake.records);
    assert_eq!(recs.len(), 1, "{recs:?}");
    let r = &recs[0];
    assert_eq!((r.kind.as_str(), r.client.as_str()), ("ea", "ea"));
    assert_eq!(r.prefix, args.prefix);
    assert_eq!(r.tag, fake.colds[0].1.clone().unwrap());
    assert!(r.pids.iter().any(|p| p.pid == 500 && p.start == 77), "{:?}", r.pids);
    assert!(r.record.file_name().unwrap().to_string_lossy().ends_with("-ea.json"));
}

#[test]
fn ea_running_and_ready_delivers_at_once() {
    let EaSetup { _dir, args, mut fake } = ea_setup();
    append(&fake.log, EA_STARTUP);
    append(&fake.log, EA_READY);
    fake.procs.push(wine_proc(500, &args.prefix, EA_EXE, &[]));
    fake.on_deliver = Box::new(|_, _, _| (Delivery::Exited(Some(0)), vec![(2, EA_OK.into())]));
    launch(&args, &mut fake).unwrap();
    assert!(fake.colds.is_empty());
    assert_eq!(fake.deliveries.len(), 1);
    assert!(record::read_records(&fake.records).is_empty(), "no record for a client the user started");
    assert!(fake.t < Duration::from_secs(10), "{:?}", fake.t);
}

#[test]
fn ea_running_but_loading_waits_for_the_ready_line() {
    let EaSetup { _dir, args, mut fake } = ea_setup();
    append(&fake.log, EA_STARTUP);
    append(&fake.log, EA_READY);
    append(&fake.log, EA_STARTUP); // restarted, not ready yet
    fake.procs.push(wine_proc(500, &args.prefix, EA_EXE, &[]));
    fake.schedule(vec![(40, EA_READY.into())]);
    fake.on_deliver = Box::new(|_, _, _| (Delivery::Exited(Some(0)), vec![(1, EA_OK.into())]));
    launch(&args, &mut fake).unwrap();
    assert_eq!(fake.deliveries.len(), 1);
    assert!(fake.t >= Duration::from_secs(40), "delivered before the ready line: {:?}", fake.t);
}

#[test]
fn ea_gives_up_after_three_retries() {
    let EaSetup { _dir, args, mut fake } = ea_setup();
    append(&fake.log, EA_STARTUP);
    append(&fake.log, EA_READY);
    fake.procs.push(wine_proc(500, &args.prefix, EA_EXE, &[]));
    fake.on_deliver =
        Box::new(|_, _, _| (Delivery::Exited(Some(0)), vec![(1, EA_NOT_FOUND.into()), (3, EA_READY.into())]));
    let e = launch(&args, &mut fake).unwrap_err().to_string();
    assert!(e.contains("EntitlementNotFound") && e.contains("press Play"), "{e}");
    let sent: Vec<String> = fake.deliveries.iter().map(|d| d.0[0].clone()).collect();
    assert_eq!(
        sent,
        vec![ea_link("1015875"), ea_link("1015875"), ea_link("OFB-EAST:109552408"), ea_link("OFB-EAST:109552408")]
    );
}

#[test]
fn ea_other_errors_end_at_once() {
    let EaSetup { _dir, args, mut fake } = ea_setup();
    append(&fake.log, EA_READY);
    fake.procs.push(wine_proc(500, &args.prefix, EA_EXE, &[]));
    fake.on_deliver = Box::new(|_, _, _| {
        (Delivery::Exited(Some(0)), vec![(1, "x Launch failed with error [GameNeedsUpdate]. IDs: …".into())])
    });
    let e = launch(&args, &mut fake).unwrap_err().to_string();
    assert!(e.contains("GameNeedsUpdate"), "{e}");
    assert_eq!(fake.deliveries.len(), 1);
}

#[test]
fn unreachable_wineserver_tries_the_next_sync_method_then_fails_clearly() {
    let EaSetup { _dir, args, mut fake } = ea_setup();
    append(&fake.log, EA_READY);
    fake.procs.push(wine_proc(500, &args.prefix, EA_EXE, &[]));
    fake.on_deliver = Box::new(|_, _, sync| {
        if sync.iter().any(|(k, v)| k == "WINEESYNC" && v == "1") {
            (Delivery::Exited(Some(0)), vec![(1, EA_OK.into())])
        } else {
            (Delivery::Unreachable("Server is running with WINEESYNC but this process is not".into()), vec![])
        }
    });
    launch(&args, &mut fake).unwrap();
    assert_eq!(fake.deliveries.len(), 2, "fsync first, then esync");

    fake.on_deliver = Box::new(|_, _, _| (Delivery::Unreachable("version mismatch".into()), vec![]));
    fake.deliveries.clear();
    let e = launch(&args, &mut fake).unwrap_err().to_string();
    assert!(e.contains("couldn't reach the EA app (running)"), "{e}");
    assert_eq!(fake.deliveries.len(), 3);
}

#[test]
fn battle_net_cold_start_waits_then_delivers() {
    let dir = tempfile::tempdir().unwrap();
    let pfx = dir.path().join("bnet");
    let log = pfx.join("drive_c/users/steamuser/AppData/Local/Battle.net/Logs/battle.net-1.log");
    let exe = pfx.join("drive_c/Program Files (x86)/Battle.net/Battle.net.exe");
    let mut a = Args::new(Kind::BattleNet, &pfx, &exe, &Runner::Unknown);
    a.code = "OSI".into();
    let mut fake = Fake::new(log, dir.path().join("records"));
    fake.cold_proc = Some(wine_proc(600, &pfx, "C:\\Program Files (x86)\\Battle.net\\Battle.net.exe", &[]));
    fake.cold_lines = vec![(30, "D [GameController] {Main} Active product set to: OSI_retail".into())];
    fake.on_deliver = Box::new(|_, args, _| {
        assert_eq!(args, ["--exec=launch OSI".to_string()]);
        (
            Delivery::Exited(Some(0)),
            vec![
                (0, "I [x] {Main} Command line: --exec=launch OSI".into()),
                (3, "I [GameLaunchController] {Main} LaunchBinary: uid=osi selectedRegion=EU binaryType=game".into()),
            ],
        )
    });
    let msg = launch(&a, &mut fake).unwrap();
    assert!(msg.contains("launched OSI"), "{msg}");
    assert_eq!(fake.colds.len(), 1);
    assert!(fake.colds[0].0.is_empty(), "Battle.net starts bare");
    assert!(fake.t >= Duration::from_secs(30));
    assert_eq!(fake.deliveries.len(), 1);
    assert_eq!(record::read_records(&fake.records).len(), 1);
}

#[test]
fn battle_net_delivery_without_a_trace_tries_other_sync_methods() {
    let dir = tempfile::tempdir().unwrap();
    let pfx = dir.path().join("bnet");
    let log = pfx.join("drive_c/users/u/AppData/Local/Battle.net/Logs/battle.net-1.log");
    let mut a = Args::new(Kind::BattleNet, &pfx, &pfx.join("drive_c/B/Battle.net.exe"), &Runner::Unknown);
    a.code = "WOW".into();
    let mut fake = Fake::new(log.clone(), dir.path().join("records"));
    append(&log, "start");
    fake.procs.push(wine_proc(1, &pfx, "C:\\B\\Battle.net.exe", &[]));
    // Nothing ever logged: three sync methods, then a clear error (no cold start: it runs).
    let e = launch(&a, &mut fake).unwrap_err().to_string();
    assert!(e.contains("no sign of the request"), "{e}");
    assert_eq!(fake.deliveries.len(), 3);
    assert!(fake.colds.is_empty());
}

#[test]
fn generic_cold_start_or_delivery() {
    let dir = tempfile::tempdir().unwrap();
    let pfx = dir.path().join("ubi");
    let exe = pfx.join("drive_c/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/UbisoftConnect.exe");
    let mut a = Args::new(Kind::Generic, &pfx, &exe, &Runner::Unknown);
    a.client = "ubisoft".into();
    a.args = sv(&["uplay://launch/635/0"]);
    // Not running: cold start with the args, recorded.
    let mut fake = Fake::new(dir.path().join("unused.log"), dir.path().join("records"));
    launch(&a, &mut fake).unwrap();
    assert_eq!(fake.colds.len(), 1);
    assert_eq!(fake.colds[0].0, a.args);
    assert!(fake.colds[0].1.is_some());
    assert!(fake.deliveries.is_empty());
    let recs = record::read_records(&fake.records);
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].client, "ubisoft");
    assert_eq!(recs[0].main, sv(&["upc.exe", "UbisoftConnect.exe"]));
    // Running (upc.exe in that prefix): through its wineserver, left running if it stays.
    let mut fake = Fake::new(dir.path().join("unused.log"), dir.path().join("records2"));
    fake.procs.push(wine_proc(
        7,
        &pfx,
        "C:\\Program Files (x86)\\Ubisoft\\Ubisoft Game Launcher\\upc.exe",
        &[("WINEFSYNC", "1")],
    ));
    fake.on_deliver = Box::new(|_, _, _| (Delivery::Running, vec![]));
    launch(&a, &mut fake).unwrap();
    assert!(fake.colds.is_empty());
    assert_eq!(fake.deliveries, vec![(a.args.clone(), vec![("WINEFSYNC".to_string(), "1".to_string())], true)]);
    assert!(record::read_records(&fake.records).is_empty());
    // The client in ANOTHER prefix doesn't count.
    let mut fake = Fake::new(dir.path().join("unused.log"), dir.path().join("records3"));
    fake.procs.push(wine_proc(7, &dir.path().join("elsewhere"), "C:\\U\\upc.exe", &[]));
    launch(&a, &mut fake).unwrap();
    assert_eq!(fake.colds.len(), 1);
}

#[test]
fn a_game_started_directly_is_not_a_client_record() {
    let dir = tempfile::tempdir().unwrap();
    let pfx = dir.path().join("hoyo");
    let mut a = Args::new(
        Kind::Generic,
        &pfx,
        &pfx.join("drive_c/Program Files/HoYoPlay/games/GenshinImpact.exe"),
        &Runner::Unknown,
    );
    a.client = "hoyoplay".into();
    a.exe_is_game = true;
    let mut fake = Fake::new(dir.path().join("unused.log"), dir.path().join("records"));
    launch(&a, &mut fake).unwrap();
    assert_eq!(fake.colds, vec![(vec![], None)]);
    assert!(record::read_records(&fake.records).is_empty());
}

// ---------------------------------------------------------------------------------------------
// Started clients: records and closing

const HOME_PFX: &str = "/home/u/Games/battlenet";

fn bnet_record(dir: &Path, tag: &str, pids: Vec<RecordedPid>) -> StartedClient {
    record_as(dir, "battlenet", tag, pids)
}

fn record_as(dir: &Path, client: &str, tag: &str, pids: Vec<RecordedPid>) -> StartedClient {
    let pfx = PathBuf::from(HOME_PFX);
    let mut c = StartedClient {
        kind: "battlenet".into(),
        client: client.into(),
        prefix: pfx.clone(),
        exe: pfx.join("drive_c/Program Files (x86)/Battle.net/Battle.net.exe"),
        client_dirs: vec![
            pfx.join("drive_c/Program Files (x86)/Battle.net"),
            pfx.join("drive_c/ProgramData/Battle.net"),
        ],
        runner: "unknown".into(),
        main: sv(&["Battle.net.exe"]),
        pids,
        tag: tag.into(),
        game_tag: None,
        started_at: 1,
        record: PathBuf::new(),
    };
    c.record = record::write_record(dir, &c).unwrap();
    c
}

/// A client session, the game it launched, and processes that aren't the client's.
fn bnet_session(root: &Path) {
    let pfx = ("WINEPREFIX", HOME_PFX);
    let tag = (TAG_VAR, "T1");
    fake_proc(root, 100, 5, &["umu-run", "C:\\Program Files (x86)\\Battle.net\\Battle.net.exe"], &[pfx, tag]);
    fake_proc(root, 101, 6, &["C:\\Program Files (x86)\\Battle.net\\Battle.net.exe"], &[pfx, tag]);
    fake_proc(root, 102, 6, &["C:\\Program Files (x86)\\Battle.net\\Battle.net.exe", "--type=renderer"], &[pfx, tag]);
    fake_proc(root, 103, 6, &["C:\\ProgramData\\Battle.net\\Agent\\Agent.9000\\Agent.exe"], &[pfx]);
    fake_proc(root, 104, 6, &["C:\\windows\\system32\\services.exe"], &[pfx, tag]);
    fake_proc(root, 105, 6, &["/opt/proton/files/bin/wineserver"], &[pfx, tag]);
    // The game Battle.net started (same session, carries the tag) — never the client's.
    fake_proc(root, 106, 7, &["C:\\Games\\Overwatch\\Overwatch.exe"], &[pfx, tag]);
    // Battle.net of another prefix, and an unrelated program.
    fake_proc(
        root,
        200,
        6,
        &["C:\\Program Files (x86)\\Battle.net\\Battle.net.exe"],
        &[("WINEPREFIX", "/home/u/Games/other")],
    );
    fake_proc(root, 201, 6, &["/usr/bin/firefox"], &[]);
}

#[test]
fn started_clients_keeps_live_records_and_drops_finished_ones() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proc");
    let recs = dir.path().join("clients");
    bnet_session(&root);
    let t = ProcTable { root };
    let live = bnet_record(&recs, "T1", vec![]);
    let gone = record_as(&recs, "ea", "T-old", vec![RecordedPid { pid: 101, start: 1 }]); // pid reused
    let got = record::started_clients_in(&recs, &t);
    assert_eq!(got.iter().map(|c| c.tag.as_str()).collect::<Vec<_>>(), vec!["T1"]);
    assert!(live.record.is_file());
    assert!(!gone.record.exists(), "the finished client's record is removed");
    // Recorded pid with the right start time keeps a record alive even without the tag.
    let by_pid = record_as(&dir.path().join("other"), "riot", "", vec![RecordedPid { pid: 103, start: 6 }]);
    assert!(record::is_alive(&by_pid, &t, &t.list()));
}

#[test]
fn client_processes_are_only_the_clients_own_in_its_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proc");
    bnet_session(&root);
    let t = ProcTable { root };
    let c = bnet_record(&dir.path().join("clients"), "T1", vec![]);
    let procs = t.list();
    let mut got: Vec<i32> = record::client_processes(&c, &t, &procs).iter().map(|p| p.pid).collect();
    got.sort();
    // The umu-run wrapper (100) shows the client's path as an argument, not as its image.
    assert_eq!(got, vec![101, 102, 103]);
    // Not alive (no tag, no recorded pid running): nothing is the client's.
    let stale = bnet_record(&dir.path().join("clients2"), "T-none", vec![]);
    assert!(record::client_processes(&stale, &t, &procs).is_empty());
}

#[test]
fn games_folder_inside_a_client_folder_is_spared() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proc");
    let pfx = "/g/ubi";
    let env = [("WINEPREFIX", pfx), (TAG_VAR, "U")];
    fake_proc(&root, 1, 1, &["C:\\Program Files (x86)\\Ubisoft\\Ubisoft Game Launcher\\upc.exe"], &env);
    fake_proc(
        &root,
        2,
        1,
        &["C:\\Program Files (x86)\\Ubisoft\\Ubisoft Game Launcher\\games\\Brawlhalla\\Brawlhalla.exe"],
        &env,
    );
    let t = ProcTable { root };
    let mut c = bnet_record(&dir.path().join("clients"), "U", vec![]);
    c.prefix = pfx.into();
    c.client_dirs = vec![PathBuf::from(pfx).join("drive_c/Program Files (x86)/Ubisoft/Ubisoft Game Launcher")];
    let procs = t.list();
    let got: Vec<i32> = record::client_processes(&c, &t, &procs).iter().map(|p| p.pid).collect();
    assert_eq!(got, vec![1]);
}

/// Signals by deleting fixture processes; 102 ignores SIGTERM.
struct FakeSys {
    table: ProcTable,
    polite_closes: HashSet<i32>,
    ignore_term: HashSet<i32>,
    signals: Vec<(i32, i32)>,
    polite_calls: usize,
}

impl CloseSys for FakeSys {
    fn table(&self) -> &ProcTable {
        &self.table
    }
    fn polite(&mut self, _c: &StartedClient) {
        self.polite_calls += 1;
        for pid in &self.polite_closes {
            let _ = std::fs::remove_dir_all(self.table.root.join(pid.to_string()));
        }
    }
    fn signal(&mut self, p: &Proc, sig: i32) {
        self.signals.push((p.pid, sig));
        if sig == libc::SIGKILL || !self.ignore_term.contains(&p.pid) {
            let _ = std::fs::remove_dir_all(self.table.root.join(p.pid.to_string()));
        }
    }
    fn sleep(&mut self, _d: Duration) {}
}

#[test]
fn close_client_polite_then_term_then_kill_and_never_others() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proc");
    bnet_session(&root);
    let c = bnet_record(&dir.path().join("clients"), "T1", vec![RecordedPid { pid: 100, start: 5 }]);
    let mut sys = FakeSys {
        table: ProcTable { root: root.clone() },
        polite_closes: [103].into(),
        ignore_term: [102].into(),
        signals: vec![],
        polite_calls: 0,
    };
    let r = record::close_with(&c, &mut sys).unwrap();
    assert_eq!(r.found, 3);
    assert!(!r.closed_politely);
    assert_eq!(sys.polite_calls, 1);
    assert_eq!(r.terminated, vec![101, 102]);
    assert_eq!(r.killed, vec![102]);
    let touched: HashSet<i32> = sys.signals.iter().map(|(p, _)| *p).collect();
    assert_eq!(touched, [101, 102].into());
    for keep in [100, 104, 105, 106, 200, 201] {
        assert!(root.join(keep.to_string()).is_dir(), "{keep} must not be touched");
    }
    assert!(!c.record.exists());
}

#[test]
fn close_client_that_closed_politely_or_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proc");
    bnet_session(&root);
    let c = bnet_record(&dir.path().join("clients"), "T1", vec![]);
    let mut sys = FakeSys {
        table: ProcTable { root: root.clone() },
        polite_closes: [101, 102, 103].into(),
        ignore_term: HashSet::new(),
        signals: vec![],
        polite_calls: 0,
    };
    let r = record::close_with(&c, &mut sys).unwrap();
    assert!(r.closed_politely);
    assert!(sys.signals.is_empty());
    assert!(!c.record.exists());
    // A record whose client is gone: nothing is signalled, the record goes.
    let stale = bnet_record(&dir.path().join("clients"), "T-gone", vec![]);
    let r = record::close_with(&stale, &mut sys).unwrap();
    assert_eq!(r.found, 0);
    assert_eq!(sys.polite_calls, 1, "no polite close for a client that isn't there");
    assert!(!stale.record.exists());
}
