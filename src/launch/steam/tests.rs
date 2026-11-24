use super::*;
use crate::client_launch::StartedClient;
use crate::launch::hygiene::{Client, Hygiene, HygieneConfig, LaunchInfo, Msg, Sys};
use std::sync::{Arc, Mutex};

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[test]
fn quiet_start_argv() {
    // Native: `steam -silent steam://rungameid/570`.
    assert_eq!(with_silent("steam", &s(&["steam://rungameid/570"])), Some(s(&["-silent", "steam://rungameid/570"])));
    assert_eq!(
        with_silent("/usr/bin/steam", &s(&["-applaunch", "570", "-novid"])),
        Some(s(&["-silent", "-applaunch", "570", "-novid"]))
    );
    // Flatpak: after `run com.valvesoftware.Steam`.
    assert_eq!(
        with_silent("flatpak", &s(&["run", "com.valvesoftware.Steam", "steam://rungameid/570"])),
        Some(s(&["run", "com.valvesoftware.Steam", "-silent", "steam://rungameid/570"]))
    );
    // Already quiet, or not the Steam client.
    assert_eq!(with_silent("steam", &s(&["-silent", "steam://rungameid/570"])), None);
    assert_eq!(with_silent("xdg-open", &s(&["steam://rungameid/570"])), None);
    assert_eq!(with_silent("flatpak", &s(&["run", "org.other.App", "x"])), None);
    assert_eq!(with_silent("gamemoderun", &s(&["steam", "steam://rungameid/1"])), None);
    assert_eq!(with_silent("/opt/steam-tools/steamcmd", &s(&["+quit"])), None);
}

#[test]
fn quiet_start_of_a_resolved_steam_game() {
    // The real resolver's command for a Steam game takes `-silent` right after the client.
    let host = crate::launch::Host { steam: crate::launch::SteamCmd::Native, home: None, vars: Default::default() };
    let g = crate::model::Game {
        source_id: "570".into(),
        launch: Some(crate::model::LaunchSpec::Steam { appid: 570 }),
        ..crate::model::Game::blank("Dota 2")
    };
    let r = crate::launch::resolve_with(&g, &host).unwrap();
    assert_eq!((r.program.as_str(), r.args.clone()), ("steam", s(&["steam://rungameid/570"])));
    assert_eq!(with_silent(&r.program, &r.args), Some(s(&["-silent", "steam://rungameid/570"])));
    let host = crate::launch::Host { steam: crate::launch::SteamCmd::Flatpak, ..host };
    let r = crate::launch::resolve_with(&g, &host).unwrap();
    assert_eq!(
        with_silent(&r.program, &r.args),
        Some(s(&["run", "com.valvesoftware.Steam", "-silent", "steam://rungameid/570"]))
    );
}

#[test]
fn shutdown_argv_native_and_flatpak() {
    assert_eq!(shutdown_argv(false), ("steam".to_string(), s(&["-shutdown"])));
    assert_eq!(shutdown_argv(true), ("flatpak".to_string(), s(&["run", "com.valvesoftware.Steam", "-shutdown"])));
}

fn p(pid: i32, cmd: &str, tag: Option<&str>) -> Proc {
    Proc {
        pid,
        ppid: 1,
        start: pid as u64 * 10,
        cmdline: cmd.split('|').map(str::to_string).collect(),
        tag: tag.map(str::to_string),
        ..Default::default()
    }
}

const STEAM: &str = "/home/u/.local/share/Steam/steamrt64/steam|-srt-logger-opened|steam://rungameid/570";
const STEAM32: &str = "/home/u/.local/share/Steam/ubuntu12_32/steam|-srt-logger-opened";
const FLATPAK_STEAM: &str = "/home/u/.var/app/com.valvesoftware.Steam/.local/share/Steam/ubuntu12_32/steam";
const WEBHELPER: &str = "./steamwebhelper|-nocrashdialog";
const REAPER: &str = "/home/u/.local/share/Steam/ubuntu12_32/reaper|SteamLaunch|AppId=570|--|/g/dota2";

#[test]
fn steam_processes() {
    assert!(is_steam_main(&s(&["/home/u/.local/share/Steam/steamrt64/steam"])));
    assert!(is_steam_main(&s(&[FLATPAK_STEAM])));
    assert!(is_steam_main(&s(&["/home/u/.local/share/Steam/ubuntu12_32/steam"])));
    assert!(!is_steam_main(&s(&["./steamwebhelper"])));
    assert!(!is_steam_main(&s(&["/usr/bin/steam"])), "the launcher script isn't the client");
    assert!(!is_steam_main(&s(&["/home/u/.local/share/Steam/ubuntu12_32/reaper", "SteamLaunch"])));
    let cmds = [s(&["/usr/bin/bash"]), STEAM32.split('|').map(str::to_string).collect()];
    assert!(steam_running(cmds.iter().map(|c| c.as_slice())));
    assert!(!steam_running([s(&["/usr/bin/steam-runtime-check"])].iter().map(|c| c.as_slice())));
    // Still starting (autostart at login): its scripts count as running, so it is never ours.
    for c in [
        s(&["/bin/sh", "/usr/bin/steam", "-silent"]),
        s(&["/bin/bash", "/usr/lib/steam/steam"]),
        s(&["bash", "/home/u/.local/share/Steam/steam.sh", "-srt-logger-opened"]),
        s(&["/usr/bin/flatpak", "run", "--branch=stable", "com.valvesoftware.Steam"]),
    ] {
        assert!(steam_running([c.clone()].iter().map(|c| c.as_slice())), "{c:?}");
    }
    assert!(!steam_running(
        [s(&["/usr/bin/flatpak", "run", "org.example.App"]), s(&["vim", "steam.txt"])].iter().map(|c| c.as_slice())
    ));
    assert_eq!(other_game(&[p(1, STEAM, None), p(2, REAPER, None)]), Some(570));
    assert_eq!(other_game(&[p(1, STEAM, None), p(2, WEBHELPER, None)]), None);
}

#[test]
fn record_lifecycle_and_whose_steam_runs() {
    let dir = tempfile::tempdir().unwrap();
    let path = record_path(dir.path());
    assert_eq!(read_record(&path), None, "nothing recorded: Steam wasn't started by Lodestar");
    // Cold start: the record knows the launch tag and where the cloud log stood.
    let home = dir.path().join("home");
    let logs = home.join(".local/share/Steam/logs");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(logs.join(CLOUD_LOG), "0123456789").unwrap();
    std::os::unix::fs::symlink(home.join(".local/share/Steam"), {
        std::fs::create_dir_all(home.join(".steam")).unwrap();
        home.join(".steam/steam")
    })
    .unwrap();
    let mut rec = new_record("steam", "t1", Some(&home), 1000);
    assert!(!rec.flatpak);
    assert_eq!(rec.cloud_offset, 10);
    assert!(rec.logs.as_ref().unwrap().ends_with("logs"));
    assert!(new_record("flatpak", "t1", Some(&home), 1).flatpak);
    assert_eq!(new_record("flatpak", "t1", Some(&home), 1).logs, None, "no Flatpak Steam folder here");
    write_record(&path, &rec).unwrap();
    assert_eq!(read_record(&path).as_ref(), Some(&rec));

    // The Steam that runs carries the launch's tag: ours. One without it: the user's.
    assert!(is_ours(&rec, &[p(5, STEAM, Some("t1")), p(6, WEBHELPER, Some("t1"))]));
    assert!(!is_ours(&rec, &[p(5, STEAM, None)]), "a Steam the user opened");
    assert!(!is_ours(&rec, &[p(5, STEAM, Some("t2"))]));
    assert!(!is_ours(&rec, &[p(6, WEBHELPER, Some("t1"))]), "only the client itself counts");
    // Once the game ran, its pids are known too: the tagged client only; a Steam without the tag
    // is the user's (opened while Lodestar started its own) unless it is a Flatpak one.
    assert_eq!(
        claim_pids(&rec, &[p(5, STEAM, Some("t1")), p(7, STEAM, None), p(6, WEBHELPER, Some("t1"))]),
        vec![SteamPid { pid: 5, start: 50 }]
    );
    assert_eq!(claim_pids(&rec, &[p(5, STEAM, None)]), vec![], "native Steam without the tag: not ours");
    let fp = SteamStarted { flatpak: true, ..rec.clone() };
    assert_eq!(claim_pids(&fp, &[p(5, STEAM, Some("t9")), p(7, STEAM, None)]), vec![SteamPid { pid: 7, start: 70 }]);
    rec.pids = claim_pids(&fp, &[p(5, STEAM, None), p(6, WEBHELPER, None)]);
    assert_eq!(rec.pids, vec![SteamPid { pid: 5, start: 50 }]);
    write_record(&path, &rec).unwrap();
    let rec = read_record(&path).unwrap();
    assert!(is_ours(&rec, &[p(5, STEAM, None)]));
    let mut reused = p(5, STEAM, None);
    reused.start = 999;
    assert!(!is_ours(&rec, &[reused]), "a reused pid isn't ours");
    let mut zombie = p(5, STEAM, Some("t1"));
    zombie.zombie = true;
    assert!(!is_ours(&rec, &[zombie]));
    remove_record(&path);
    assert_eq!(read_record(&path), None);
    remove_record(&path); // gone already: fine
    // A broken file counts as no record.
    std::fs::write(&path, "{not json").unwrap();
    assert_eq!(read_record(&path), None);
}

const CLOUD: &str = "\
[2026-11-22 20:42:11] [AppID 2679460] Starting sync (up,AC Exit,)
[2026-11-22 20:42:11] [AppID 2679460] Running AutoCloud on exit. Looking for new and updated files
[2026-11-22 20:42:11] [AppID 2679460] AutoCloud complete
[2026-11-22 20:42:12] [AppID 2679460] Need to upload file SEGA/METAPHOR/save0001.sav
[2026-11-22 20:42:12] [AppID 2679460] Upload batch initiated, ID: 1, ChangeNumber: 6
[2026-11-22 20:42:13] [AppID 241100] Starting sync (up,AC Exit,)
[2026-11-22 20:42:13] [U:1:1] [AppID 2358720] Init cache request complete
";

#[test]
fn cloud_syncs_still_open() {
    let mut pending = cloud_pending(CLOUD);
    pending.sort();
    assert_eq!(pending, vec![241100, 2679460]);
    let more = format!(
        "{CLOUD}[2026-11-22 20:42:13] [AppID 2679460] Upload OK for file save0001.sav\n\
         [2026-11-22 20:42:13] [AppID 2679460] Upload complete, result OK\n\
         [2026-11-22 20:42:13] [AppID 241100] Upload complete in build list\n"
    );
    assert!(cloud_pending(&more).is_empty());
    // A download sync at launch isn't waited for; garbage is ignored.
    let down = "[t] [AppID 5] Starting sync (AC Launch,down,)\n[t] [AppID x] Starting sync (up)\nnoise\n[AppID 7";
    assert!(cloud_pending(down).is_empty());
    // An app that syncs up again after it had completed is open again.
    assert_eq!(cloud_pending(&format!("{more}[t] [AppID 241100] Starting sync (up,AC Exit,)\n")), vec![241100]);
}

#[test]
fn cloud_log_read_since_the_start_and_after_rotation() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(CLOUD_LOG), "old line\nnew line\n").unwrap();
    assert_eq!(cloud_text_since(dir.path(), 9), "new line\n");
    assert_eq!(cloud_text_since(dir.path(), 0), "old line\nnew line\n");
    // Rotated (shorter than where it stood): all of it.
    assert_eq!(cloud_text_since(dir.path(), 10_000), "old line\nnew line\n");
    assert_eq!(cloud_text_since(&dir.path().join("none"), 0), "");
    let a = LogSig::read(dir.path());
    assert_eq!(a.0.len(), 2);
    assert_eq!(a.0[0].0, 18);
    assert_eq!(a.0[1], (0, None), "no content log");
}

fn sig(n: u64) -> LogSig {
    LogSig(vec![(n, None), (0, None)])
}

#[test]
fn idle_watch_with_a_fake_clock() {
    let t = SteamTiming {
        poll: Duration::from_secs(1),
        min_wait: Duration::from_secs(3),
        quiet: Duration::from_secs(5),
        max_wait: Duration::from_secs(120),
    };
    let t0 = Instant::now();
    let at = |s: u64| t0 + Duration::from_secs(s);
    // Quiet logs, nothing open: idle once both the minimum wait and the quiet time passed.
    let mut w = IdleWatch::new(t0);
    assert_eq!(w.poll(at(0), &t, sig(1), &[]), Idle::Wait);
    assert_eq!(w.poll(at(4), &t, sig(1), &[]), Idle::Wait);
    assert_eq!(w.poll(at(5), &t, sig(1), &[]), Idle::Ready);
    // The logs keep changing (sync running): wait until they've been quiet.
    let mut w = IdleWatch::new(t0);
    for s in 0..10 {
        assert_eq!(w.poll(at(s), &t, sig(s), &[]), Idle::Wait, "second {s}");
    }
    assert_eq!(w.poll(at(13), &t, sig(9), &[]), Idle::Wait);
    assert_eq!(w.poll(at(14), &t, sig(9), &[]), Idle::Ready);
    // An upload still open: wait, however quiet; at most two minutes.
    let mut w = IdleWatch::new(t0);
    assert_eq!(w.poll(at(60), &t, sig(1), &[570]), Idle::Wait);
    assert_eq!(w.poll(at(119), &t, sig(1), &[570]), Idle::Wait);
    assert_eq!(w.poll(at(120), &t, sig(1), &[570]), Idle::TimedOut);
    // Logs that never stop changing time out too.
    let mut w = IdleWatch::new(t0);
    let mut last = Idle::Wait;
    for s in 0..=120 {
        last = w.poll(at(s), &t, sig(s), &[]);
        if last != Idle::Wait {
            assert_eq!(s, 120);
        }
    }
    assert_eq!(last, Idle::TimedOut);
}

// ---------------------------------------------------------------------------------------------
// Launcher hygiene closing Steam (a fake system).

#[derive(Default)]
struct World {
    procs: Vec<Proc>,
    rec: Option<SteamStarted>,
    sig: u64,
    cloud: String,
    shutdowns: Vec<bool>,
    iconified: usize,
    signals: usize,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<World>>);

impl Sys for Fake {
    fn procs(&mut self) -> Vec<Proc> {
        self.0.lock().unwrap().procs.clone()
    }
    fn signal(&mut self, _: i32, _: u64, _: i32) {
        self.0.lock().unwrap().signals += 1;
    }
    fn iconify(&mut self, _: &[u32]) -> usize {
        self.0.lock().unwrap().iconified += 1;
        1
    }
    fn started_clients(&mut self) -> Vec<StartedClient> {
        vec![]
    }
    fn close_client(&mut self, _: StartedClient) {
        panic!("no store client here");
    }
    fn steam_record(&mut self) -> Option<SteamStarted> {
        self.0.lock().unwrap().rec.clone()
    }
    fn save_steam_record(&mut self, r: &SteamStarted) {
        self.0.lock().unwrap().rec = Some(r.clone());
    }
    fn remove_steam_record(&mut self) {
        self.0.lock().unwrap().rec = None;
    }
    fn steam_logs(&mut self, _: &SteamStarted) -> (LogSig, String) {
        let w = self.0.lock().unwrap();
        (sig(w.sig), w.cloud.clone())
    }
    fn steam_shutdown(&mut self, flatpak: bool) {
        self.0.lock().unwrap().shutdowns.push(flatpak);
    }
}

fn cfg(close_steam: bool, keep_steam: bool) -> HygieneConfig {
    HygieneConfig { hide_steam: true, close_steam, keep_steam, ..Default::default() }
}

fn steam_hygiene(w: &Fake, c: HygieneConfig) -> Hygiene<Fake> {
    Hygiene::new(w.clone(), c)
        .with_timing(vec![Duration::ZERO], Duration::from_secs(2), Duration::from_secs(3))
        .with_steam_timing(SteamTiming {
            poll: Duration::from_secs(1),
            min_wait: Duration::from_secs(3),
            quiet: Duration::from_secs(5),
            max_wait: Duration::from_secs(120),
        })
}

fn steam_launch(id: crate::model::GameId, tag: &str) -> LaunchInfo {
    LaunchInfo { id, tag: tag.into(), folder: None, prefix: None, client: Client::Steam, heroic_was_running: false }
}

/// Lodestar started Steam for launch t1 (it carries the tag); the game runs under the reaper.
fn started_world(w: &Fake) {
    let mut x = w.0.lock().unwrap();
    x.procs = vec![p(50, STEAM, Some("t1")), p(51, WEBHELPER, Some("t1")), p(60, REAPER, Some("t1"))];
    x.rec = Some(SteamStarted { game_tag: "t1".into(), ..Default::default() });
}

fn game_exits(w: &Fake) {
    w.0.lock().unwrap().procs.retain(|p| p.pid != 60);
}

/// Run every job up to `secs` after `t0`, a second at a time.
fn run_until(h: &mut Hygiene<Fake>, t0: Instant, from: u64, secs: u64) {
    for s in from..=secs {
        h.run_due(t0 + Duration::from_secs(s));
    }
}

#[test]
fn steam_lodestar_started_is_closed_once_idle() {
    let w = Fake::default();
    started_world(&w);
    let mut h = steam_hygiene(&w, cfg(true, false));
    let t0 = Instant::now();
    h.handle(Msg::Launch(steam_launch(1, "t1")), t0);
    h.handle(Msg::Started(1), t0);
    h.run_due(t0);
    assert_eq!(w.0.lock().unwrap().iconified, 1, "still minimised");
    assert_eq!(w.0.lock().unwrap().rec.as_ref().unwrap().pids, vec![SteamPid { pid: 50, start: 500 }], "claimed");
    game_exits(&w);
    w.0.lock().unwrap().cloud = "[t] [AppID 570] Starting sync (up,AC Exit,)\n".into();
    h.handle(Msg::Exited(1), t0 + Duration::from_secs(10));
    run_until(&mut h, t0, 10, 40);
    assert!(w.0.lock().unwrap().shutdowns.is_empty(), "the cloud upload is still open");
    {
        let mut x = w.0.lock().unwrap();
        x.cloud.push_str("[t] [AppID 570] Upload complete, result OK\n");
        x.sig = 2;
    }
    run_until(&mut h, t0, 41, 45);
    assert!(w.0.lock().unwrap().shutdowns.is_empty(), "the log just changed");
    run_until(&mut h, t0, 46, 47);
    let x = w.0.lock().unwrap();
    assert_eq!(x.shutdowns, vec![false], "steam -shutdown once quiet");
    assert_eq!(x.rec, None, "record gone");
    assert_eq!(x.signals, 0, "never a signal");
    drop(x);
    assert!(h.next_due().is_none());
}

#[test]
fn steam_still_syncing_is_asked_to_quit_after_two_minutes() {
    let w = Fake::default();
    started_world(&w);
    let mut h = steam_hygiene(&w, cfg(true, false));
    let t0 = Instant::now();
    h.handle(Msg::Launch(steam_launch(1, "t1")), t0);
    h.handle(Msg::Started(1), t0);
    game_exits(&w);
    w.0.lock().unwrap().cloud = "[t] [AppID 570] Starting sync (up,AC Exit,)\n".into();
    h.handle(Msg::Exited(1), t0);
    run_until(&mut h, t0, 0, 120);
    assert!(w.0.lock().unwrap().shutdowns.is_empty());
    run_until(&mut h, t0, 121, 123);
    assert_eq!(w.0.lock().unwrap().shutdowns, vec![false]);
}

#[test]
fn flatpak_steam_is_closed_through_flatpak() {
    let w = Fake::default();
    started_world(&w);
    {
        let mut x = w.0.lock().unwrap();
        x.procs[0] = p(50, FLATPAK_STEAM, None); // the tag may not reach into the sandbox
        x.rec.as_mut().unwrap().flatpak = true;
    }
    let mut h = steam_hygiene(&w, cfg(true, false));
    let t0 = Instant::now();
    h.handle(Msg::Launch(steam_launch(1, "t1")), t0);
    h.handle(Msg::Started(1), t0);
    game_exits(&w);
    h.handle(Msg::Exited(1), t0);
    run_until(&mut h, t0, 0, 20);
    assert_eq!(w.0.lock().unwrap().shutdowns, vec![true], "known by its pid from when the game ran");
}

#[test]
fn never_closes_a_steam_the_user_opened() {
    let w = Fake::default();
    started_world(&w);
    w.0.lock().unwrap().rec = None; // Steam was running before the launch: no record
    let mut h = steam_hygiene(&w, cfg(true, false));
    let t0 = Instant::now();
    h.handle(Msg::Launch(steam_launch(1, "t1")), t0);
    h.handle(Msg::Started(1), t0);
    game_exits(&w);
    h.handle(Msg::Exited(1), t0);
    run_until(&mut h, t0, 0, 200);
    assert!(w.0.lock().unwrap().shutdowns.is_empty());

    // A record of an earlier launch (Steam stayed open then): not this launch's, stays open too.
    let w = Fake::default();
    started_world(&w);
    w.0.lock().unwrap().rec.as_mut().unwrap().game_tag = "t0".into();
    let mut h = steam_hygiene(&w, cfg(true, false));
    h.handle(Msg::Launch(steam_launch(1, "t1")), t0);
    h.handle(Msg::Started(1), t0);
    game_exits(&w);
    h.handle(Msg::Exited(1), t0);
    run_until(&mut h, t0, 0, 200);
    assert!(w.0.lock().unwrap().shutdowns.is_empty());

    // The user quit Steam meanwhile and opened their own: the record is stale, nothing closed.
    let w = Fake::default();
    started_world(&w);
    let mut h = steam_hygiene(&w, cfg(true, false));
    h.handle(Msg::Launch(steam_launch(1, "t1")), t0);
    h.handle(Msg::Started(1), t0);
    w.0.lock().unwrap().procs = vec![p(90, STEAM, None)];
    h.handle(Msg::Exited(1), t0);
    run_until(&mut h, t0, 0, 200);
    let x = w.0.lock().unwrap();
    assert!(x.shutdowns.is_empty());
    assert_eq!(x.rec, None, "stale record dropped");
}

#[test]
fn never_claims_a_steam_without_the_launch_tag() {
    // Lodestar started Steam, but the Steam that runs when the game does is the user's (it was
    // still starting at login): nothing claimed, nothing closed.
    let w = Fake::default();
    started_world(&w);
    w.0.lock().unwrap().procs[0] = p(50, STEAM, None);
    let mut h = steam_hygiene(&w, cfg(true, false));
    let t0 = Instant::now();
    h.handle(Msg::Launch(steam_launch(1, "t1")), t0);
    h.handle(Msg::Started(1), t0);
    h.run_due(t0);
    assert!(w.0.lock().unwrap().rec.as_ref().unwrap().pids.is_empty());
    game_exits(&w);
    h.handle(Msg::Exited(1), t0);
    run_until(&mut h, t0, 0, 200);
    let x = w.0.lock().unwrap();
    assert!(x.shutdowns.is_empty());
    assert_eq!(x.rec, None, "the record matched no running Steam: dropped");
}

#[test]
fn never_closes_while_another_steam_game_runs() {
    let w = Fake::default();
    started_world(&w);
    let mut h = steam_hygiene(&w, cfg(true, false));
    let t0 = Instant::now();
    h.handle(Msg::Launch(steam_launch(1, "t1")), t0);
    h.handle(Msg::Started(1), t0);
    {
        // Game 1 ends; the user started another game from Steam itself.
        let mut x = w.0.lock().unwrap();
        x.procs.retain(|p| p.pid != 60);
        x.procs.push(p(70, "/s/reaper|SteamLaunch|AppId=440|--|/g/tf2", Some("t1")));
    }
    h.handle(Msg::Exited(1), t0);
    run_until(&mut h, t0, 0, 200);
    assert!(w.0.lock().unwrap().shutdowns.is_empty());

    // Two Steam games from Lodestar at once: Steam stays for the other, closes after it.
    let w = Fake::default();
    started_world(&w);
    let mut h = steam_hygiene(&w, cfg(true, false));
    h.handle(Msg::Launch(steam_launch(1, "t1")), t0);
    h.handle(Msg::Started(1), t0);
    h.handle(Msg::Launch(steam_launch(2, "t2")), t0);
    h.handle(Msg::Started(2), t0);
    h.handle(Msg::Exited(1), t0);
    run_until(&mut h, t0, 0, 60);
    assert!(w.0.lock().unwrap().shutdowns.is_empty(), "game 2 still runs");
    game_exits(&w);
    h.handle(Msg::Exited(2), t0 + Duration::from_secs(60));
    run_until(&mut h, t0, 60, 80);
    assert_eq!(w.0.lock().unwrap().shutdowns, vec![false]);
}

#[test]
fn a_new_steam_launch_while_waiting_keeps_steam_for_that_game() {
    let w = Fake::default();
    started_world(&w);
    let mut h = steam_hygiene(&w, cfg(true, false));
    let t0 = Instant::now();
    h.handle(Msg::Launch(steam_launch(1, "t1")), t0);
    h.handle(Msg::Started(1), t0);
    game_exits(&w);
    w.0.lock().unwrap().sig = 1;
    h.handle(Msg::Exited(1), t0);
    run_until(&mut h, t0, 0, 3); // waiting now
    h.handle(Msg::Launch(steam_launch(2, "t2")), t0 + Duration::from_secs(3));
    run_until(&mut h, t0, 3, 60);
    assert!(w.0.lock().unwrap().shutdowns.is_empty(), "taken over by game 2");
    h.handle(Msg::Started(2), t0 + Duration::from_secs(60));
    h.handle(Msg::Exited(2), t0 + Duration::from_secs(70));
    run_until(&mut h, t0, 70, 90);
    assert_eq!(w.0.lock().unwrap().shutdowns, vec![false], "closed after game 2");
}

#[test]
fn never_closes_when_kept_for_controllers_or_closing_is_off() {
    for (close, keep) in [(false, false), (true, true), (false, true)] {
        let w = Fake::default();
        started_world(&w);
        let mut h = steam_hygiene(&w, cfg(close, keep));
        let t0 = Instant::now();
        h.handle(Msg::Launch(steam_launch(1, "t1")), t0);
        h.handle(Msg::Started(1), t0);
        h.run_due(t0);
        game_exits(&w);
        h.handle(Msg::Exited(1), t0);
        run_until(&mut h, t0, 0, 200);
        let x = w.0.lock().unwrap();
        assert!(x.shutdowns.is_empty(), "close {close} keep {keep}");
        assert_eq!(x.iconified, 1, "still minimised");
    }
    // Turned off while waiting: stays.
    let w = Fake::default();
    started_world(&w);
    let mut h = steam_hygiene(&w, cfg(true, false));
    let t0 = Instant::now();
    h.handle(Msg::Launch(steam_launch(1, "t1")), t0);
    h.handle(Msg::Started(1), t0);
    game_exits(&w);
    h.handle(Msg::Exited(1), t0);
    run_until(&mut h, t0, 0, 3);
    h.handle(Msg::Config(cfg(true, true)), t0 + Duration::from_secs(3));
    run_until(&mut h, t0, 3, 200);
    assert!(w.0.lock().unwrap().shutdowns.is_empty());
}

#[test]
fn a_game_that_never_ran_leaves_steam_alone() {
    let w = Fake::default();
    started_world(&w);
    let mut h = steam_hygiene(&w, cfg(true, false));
    let t0 = Instant::now();
    h.handle(Msg::Launch(steam_launch(1, "t1")), t0);
    // No Started (sign-in screen, update, cancelled): Steam stays.
    h.handle(Msg::Exited(1), t0);
    run_until(&mut h, t0, 0, 200);
    assert!(w.0.lock().unwrap().shutdowns.is_empty());
}

#[test]
fn settings_map_to_hygiene() {
    let mut l = crate::config::LaunchersConfig::default();
    assert!(l.hide_steam && !l.close_steam_after && !l.keep_steam_for_controller, "defaults");
    l.set_close_steam(true);
    assert!(HygieneConfig::from_settings(&l).closes_steam());
    l.set_keep_steam(true);
    assert!(!l.close_steam_after, "keeping Steam turns closing off");
    let c = HygieneConfig::from_settings(&l);
    assert!(c.keep_steam && !c.closes_steam());
    l.set_close_steam(true);
    assert!(!l.keep_steam_for_controller, "and the other way round");
    // Closing alone is enough to follow Steam launches.
    let c = HygieneConfig { close_steam: true, ..Default::default() };
    assert!(c.active());
}
