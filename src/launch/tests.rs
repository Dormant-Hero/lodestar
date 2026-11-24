use super::*;
use crate::test_support::{ev, sv, write_exe};
use std::time::Instant;

fn game(launch: Option<LaunchSpec>, ov: Option<LaunchOverride>) -> Game {
    Game { id: 7, source_id: "x".into(), launch, launch_override: ov, ..Game::blank("Test Game") }
}

fn host(steam: SteamCmd) -> Host {
    let vars = [("HOME", "/home/u"), ("LD_LIBRARY_PATH", "/lib/a"), ("EMPTY", "")]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    Host { steam, home: Some("/home/u".into()), vars }
}

fn cmd(program: &str, args: &[&str]) -> LaunchSpec {
    LaunchSpec::Command {
        program: program.into(),
        args: args.iter().map(|s| s.to_string()).collect(),
        cwd: None,
        env: vec![],
    }
}

fn opts(o: &str) -> Option<LaunchOverride> {
    Some(LaunchOverride { options: o.into(), ..Default::default() })
}

fn r(g: &Game) -> Resolved {
    resolve_with(g, &host(SteamCmd::Native)).unwrap()
}

// ----- resolve -----

#[test]
fn plain_command() {
    let mut spec = cmd("/opt/game/run", &["-a", "b c"]);
    if let LaunchSpec::Command { cwd, env, .. } = &mut spec {
        *cwd = Some("/opt/game".into());
        *env = ev(&[("A", "1")]);
    }
    let x = r(&game(Some(spec), None));
    assert_eq!(x.program, "/opt/game/run");
    assert_eq!(x.args, sv(&["-a", "b c"]));
    assert_eq!(x.env, ev(&[("A", "1")]));
    assert_eq!(x.cwd, Some("/opt/game".into()));
    assert_eq!(x.kind, ResolvedKind::Process);
    assert_eq!(x.display, "A=1 /opt/game/run -a 'b c'");
}

#[test]
fn options_with_command_placeholder() {
    let g = game(Some(cmd("/g/bin", &["--x"])), opts("MANGOHUD=1 DXVK_HUD=fps gamemoderun %command% -dx11"));
    let x = r(&g);
    assert_eq!(x.program, "gamemoderun");
    assert_eq!(x.args, sv(&["/g/bin", "--x", "-dx11"]));
    assert_eq!(x.env, ev(&[("MANGOHUD", "1"), ("DXVK_HUD", "fps")]));
    assert_eq!(x.display, "MANGOHUD=1 DXVK_HUD=fps gamemoderun /g/bin --x -dx11");
}

#[test]
fn options_without_placeholder_append() {
    let x = r(&game(Some(cmd("/g/bin", &[])), opts("-novid  -high")));
    assert_eq!((x.program.as_str(), x.args.clone()), ("/g/bin", sv(&["-novid", "-high"])));
    let x = r(&game(Some(cmd("/g/bin", &[])), opts("FOO=1 -x")));
    assert_eq!(x.env, ev(&[("FOO", "1")]));
    assert_eq!(x.args, sv(&["-x"]));
    // Only env.
    let x = r(&game(Some(cmd("/g/bin", &["a"])), opts("PROTON_LOG=1")));
    assert_eq!(x.env, ev(&[("PROTON_LOG", "1")]));
    assert_eq!(x.args, sv(&["a"]));
    // Placeholder alone / suffix only.
    let x = r(&game(Some(cmd("/g/bin", &["a"])), opts("%command% -b")));
    assert_eq!((x.program.as_str(), x.args.clone()), ("/g/bin", sv(&["a", "-b"])));
}

#[test]
fn override_env_replaces_source_env() {
    let mut spec = cmd("/g/bin", &[]);
    if let LaunchSpec::Command { env, .. } = &mut spec {
        *env = ev(&[("WINEPREFIX", "/old"), ("KEEP", "1")]);
    }
    let x = r(&game(Some(spec), opts("WINEPREFIX=/new %command%")));
    assert_eq!(x.env, ev(&[("WINEPREFIX", "/new"), ("KEEP", "1")]));
}

#[test]
fn env_word_rules() {
    // Quoted name: not an assignment. Value quoting is fine.
    let x = r(&game(Some(cmd("/g", &[])), opts(r#"A="x y" "B=1" %command%"#)));
    assert_eq!(x.env, ev(&[("A", "x y")]));
    assert_eq!(x.program, "B=1");
    // Env words after a wrapper are the wrapper's arguments.
    let x = r(&game(Some(cmd("/g", &[])), opts("env X=1 %command%")));
    assert!(x.env.is_empty());
    assert_eq!((x.program.as_str(), x.args.clone()), ("env", sv(&["X=1", "/g"])));
    // Not identifiers.
    let x = r(&game(Some(cmd("/g", &[])), opts("1A=2 -x=3")));
    assert!(x.env.is_empty());
    assert_eq!(x.args, sv(&["1A=2", "-x=3"]));
}

#[test]
fn variable_and_tilde_expansion() {
    let x = r(&game(
        Some(cmd("/g", &[])),
        opts("LD_LIBRARY_PATH=$LD_LIBRARY_PATH:~/lib CFG=~/cfg %command% --dir ~/x '$HOME' \"${HOME}/y\" $NOPE. ${bad"),
    ));
    assert_eq!(x.env, ev(&[("LD_LIBRARY_PATH", "/lib/a:~/lib"), ("CFG", "/home/u/cfg")]));
    assert_eq!(x.args, sv(&["--dir", "/home/u/x", "$HOME", "/home/u/y", ".", "${bad"]));
}

#[test]
fn custom_command_replaces_source() {
    let ov = LaunchOverride {
        options: "DXVK_ASYNC=1 %command% -windowed".into(),
        command: Some(r#""/opt/My Game/run.sh" --flag 'a b' c\ d"#.into()),
        cwd: Some("/opt/My Game".into()),
    };
    let x = r(&game(Some(LaunchSpec::Steam { appid: 570 }), Some(ov)));
    assert_eq!(x.program, "/opt/My Game/run.sh");
    assert_eq!(x.args, sv(&["--flag", "a b", "c d", "-windowed"]));
    assert_eq!(x.env, ev(&[("DXVK_ASYNC", "1")]));
    assert_eq!(x.cwd, Some("/opt/My Game".into()));
    assert_eq!(x.kind, ResolvedKind::Process);
    assert_eq!(x.display, "DXVK_ASYNC=1 '/opt/My Game/run.sh' --flag 'a b' 'c d' -windowed");
    // Works with no source launch at all.
    let ov = LaunchOverride { command: Some("umu-run game.exe".into()), ..Default::default() };
    let x = r(&game(None, Some(ov)));
    assert_eq!((x.program.as_str(), x.args.clone()), ("umu-run", sv(&["game.exe"])));
}

#[test]
fn override_cwd_wins_over_source_cwd() {
    let mut spec = cmd("/g", &[]);
    if let LaunchSpec::Command { cwd, .. } = &mut spec {
        *cwd = Some("/src".into());
    }
    let ov = LaunchOverride { cwd: Some("/mine".into()), ..Default::default() };
    assert_eq!(r(&game(Some(spec), Some(ov))).cwd, Some("/mine".into()));
}

#[test]
fn steam_plain() {
    let g = game(Some(LaunchSpec::Steam { appid: 570 }), None);
    let x = r(&g);
    assert_eq!((x.program.as_str(), x.args.clone()), ("steam", sv(&["steam://rungameid/570"])));
    assert_eq!(x.kind, ResolvedKind::Steam { appid: 570 });
    let x = resolve_with(&g, &host(SteamCmd::Flatpak)).unwrap();
    assert_eq!(x.program, "flatpak");
    assert_eq!(x.args, sv(&["run", "com.valvesoftware.Steam", "steam://rungameid/570"]));
    let x = resolve_with(&g, &host(SteamCmd::XdgOpen)).unwrap();
    assert_eq!((x.program.as_str(), x.args.clone()), ("xdg-open", sv(&["steam://rungameid/570"])));
    assert_eq!(x.kind, ResolvedKind::Steam { appid: 570 });
}

#[test]
fn steam_extra_args_use_applaunch() {
    let mut g = game(Some(LaunchSpec::Steam { appid: 570 }), opts("-novid -console"));
    let x = r(&g);
    assert_eq!(x.args, sv(&["-applaunch", "570", "-novid", "-console"]));
    assert!(x.notes.is_empty());
    g.source_launch_options = Some("-high".into());
    let x = r(&g);
    assert!(x.notes[0].contains("-high"), "{:?}", x.notes);
    g.launch_override = opts("%command% -novid");
    let x = resolve_with(&g, &host(SteamCmd::Flatpak)).unwrap();
    assert_eq!(x.args, sv(&["run", "com.valvesoftware.Steam", "-applaunch", "570", "-novid"]));
    let x = resolve_with(&g, &host(SteamCmd::XdgOpen)).unwrap();
    assert_eq!(x.args, sv(&["steam://run/570//-novid/"]));
    g.launch_override = opts("-a 'b c'");
    let x = resolve_with(&g, &host(SteamCmd::XdgOpen)).unwrap();
    assert_eq!(x.args, sv(&["steam://run/570//-a%20b%20c/"]));
}

#[test]
fn steam_rejects_env_and_wrappers() {
    for o in ["MANGOHUD=1 %command%", "gamemoderun %command%", "DXVK_HUD=1", "mangohud %command% -x"] {
        let g = game(Some(LaunchSpec::Steam { appid: 570 }), opts(o));
        let e = resolve_with(&g, &host(SteamCmd::Native)).unwrap_err().to_string();
        assert_eq!(e, STEAM_WRAPPER_ERR, "{o}");
    }
    // A custom command makes it possible.
    let ov = LaunchOverride {
        options: "MANGOHUD=1 %command%".into(),
        command: Some("/games/dota/dota.sh".into()),
        cwd: None,
    };
    let x = r(&game(Some(LaunchSpec::Steam { appid: 570 }), Some(ov)));
    assert_eq!(x.program, "/games/dota/dota.sh");
    assert_eq!(x.env, ev(&[("MANGOHUD", "1")]));
}

#[test]
fn steam_urls_and_shortcuts() {
    let x = r(&game(Some(LaunchSpec::Url("steam://rungameid/440".into())), None));
    assert_eq!(x.kind, ResolvedKind::Steam { appid: 440 });
    let x = r(&game(Some(LaunchSpec::Url("steam://run/440".into())), opts("-x")));
    assert_eq!(x.args, sv(&["-applaunch", "440", "-x"]));
    let gameid: u64 = (3_000_000_123u64 << 32) | 0x0200_0000;
    let g = game(Some(LaunchSpec::Url(format!("steam://rungameid/{gameid}"))), None);
    let x = r(&g);
    assert_eq!(x.kind, ResolvedKind::SteamShortcut { appid: 3_000_000_123 });
    assert_eq!(x.args, vec![format!("steam://rungameid/{gameid}")]);
    let g = game(Some(LaunchSpec::Url(format!("steam://rungameid/{gameid}"))), opts("-x"));
    assert_eq!(resolve_with(&g, &host(SteamCmd::Native)).unwrap_err().to_string(), SHORTCUT_ARGS_ERR);
    // Other steam:// URLs go through the client, untracked.
    let x = r(&game(Some(LaunchSpec::Url("steam://open/bigpicture".into())), None));
    assert_eq!((x.program.as_str(), x.kind), ("steam", ResolvedKind::OpenUrl));
}

#[test]
fn steam_commands_are_recognised() {
    let x = r(&game(Some(cmd("steam", &["steam://rungameid/10"])), None));
    assert_eq!(x.kind, ResolvedKind::Steam { appid: 10 });
    let x = r(&game(Some(cmd("/usr/bin/flatpak", &["run", "com.valvesoftware.Steam", "steam://rungameid/10"])), None));
    assert_eq!(x.kind, ResolvedKind::Steam { appid: 10 });
    assert_eq!(x.program, "steam"); // re-routed through the detected client
    let ov = LaunchOverride { command: Some("steam -applaunch 42 -x".into()), ..Default::default() };
    let x = r(&game(None, Some(ov)));
    assert_eq!(x.kind, ResolvedKind::Steam { appid: 42 });
    assert_eq!(x.args, sv(&["-applaunch", "42", "-x"]));
    let x = r(&game(Some(cmd("steam", &["-applaunch", "42"])), None));
    assert_eq!(x.kind, ResolvedKind::Steam { appid: 42 });
}

#[test]
fn urls_use_xdg_open() {
    let url = LaunchSpec::Url("heroic://launch/legendary/Fortnite".into());
    let x = r(&game(Some(url.clone()), None));
    assert_eq!((x.program.as_str(), x.args.clone()), ("xdg-open", sv(&["heroic://launch/legendary/Fortnite"])));
    assert_eq!(x.kind, ResolvedKind::OpenUrl);
    // xdg-open takes exactly one argument.
    let e = resolve_with(&game(Some(url.clone()), opts("-x")), &host(SteamCmd::Native)).unwrap_err();
    assert_eq!(e.to_string(), URL_ARGS_ERR);
    let x = r(&game(Some(url), opts("FOO=1")));
    assert_eq!(x.notes.len(), 1);
}

#[test]
fn launcher_wrappers_get_a_note() {
    let g = game(Some(cmd("/usr/bin/faugus-launcher", &["--game", "fl"])), opts("-windowed"));
    let x = r(&g);
    assert_eq!(x.args, sv(&["--game", "fl", "-windowed"]));
    assert!(x.notes[0].contains("Faugus"), "{:?}", x.notes);
    // Env and wrappers are inherited by the game, so no note for those.
    let g = game(Some(cmd("faugus-launcher", &["--game", "fl"])), opts("MANGOHUD=1 gamemoderun %command%"));
    assert!(r(&g).notes.is_empty());
    // Plain games: no note.
    assert!(r(&game(Some(cmd("/g/bin", &[])), opts("-x"))).notes.is_empty());
}

#[test]
fn command_templates() {
    let mut spec = cmd("/opt/My Game/run", &["--game", "a b"]);
    if let LaunchSpec::Command { env, .. } = &mut spec {
        *env = ev(&[("WINEPREFIX", "/p q")]);
    }
    let c = command_template(&game(Some(spec), None)).unwrap();
    assert_eq!(c, "WINEPREFIX='/p q' '/opt/My Game/run' --game 'a b'");
    // Round-trips through the custom command field (env words included).
    let ov = LaunchOverride { command: Some(c), options: "-extra".into(), cwd: None };
    let x = r(&game(None, Some(ov)));
    assert_eq!(x.program, "/opt/My Game/run");
    assert_eq!(x.args, sv(&["--game", "a b", "-extra"]));
    assert_eq!(x.env, ev(&[("WINEPREFIX", "/p q")]));
    // Options' env overrides the custom command's.
    let ov = LaunchOverride { command: Some("A=1 B=2 /g".into()), options: "A=3".into(), cwd: None };
    assert_eq!(r(&game(None, Some(ov))).env, ev(&[("A", "3"), ("B", "2")]));
    let ov = LaunchOverride { command: Some("A=1".into()), ..Default::default() };
    assert!(resolve_with(&game(None, Some(ov)), &host(SteamCmd::Native)).is_err());
    assert!(command_template(&game(Some(LaunchSpec::Steam { appid: 1 }), None)).is_none());
    assert!(command_template(&game(Some(cmd("steam", &["steam://rungameid/5"])), None)).is_none());
}

#[test]
fn errors() {
    assert!(resolve_with(&game(None, None), &host(SteamCmd::Native)).is_err());
    assert!(resolve_with(&game(Some(cmd("/g", &[])), opts("\"unterminated")), &host(SteamCmd::Native)).is_err());
    let ov = LaunchOverride { command: Some("'oops".into()), ..Default::default() };
    assert!(resolve_with(&game(Some(cmd("/g", &[])), Some(ov)), &host(SteamCmd::Native)).is_err());
    let ov = LaunchOverride { command: Some("   ".into()), ..Default::default() };
    // Blank custom command = no custom command.
    assert_eq!(r(&game(Some(cmd("/g", &[])), Some(ov))).program, "/g");
    assert!(resolve_with(&game(Some(cmd("", &[])), None), &host(SteamCmd::Native)).is_err());
}

#[test]
fn splitting_and_quoting() {
    assert_eq!(
        split_command(r#"a "b c" 'd e' f\ g "h\"i" 'j\k' "$X""#).unwrap(),
        sv(&["a", "b c", "d e", "f g", "h\"i", "j\\k", "$X"])
    );
    assert_eq!(split_command("  ").unwrap(), Vec::<String>::new());
    assert_eq!(split_command("''").unwrap(), sv(&[""]));
    assert!(split_command("'a").is_err());
    for w in ["plain", "a b", "it's", "", "$HOME", "~/x", "a\"b", "%command%"] {
        let q = shell_quote(w);
        assert_eq!(split_command(&q).unwrap(), vec![w.to_string()], "{w} -> {q}");
    }
}

#[test]
fn preview_uses_given_override() {
    let g = game(Some(cmd("/g", &[])), opts("-old"));
    let ov = LaunchOverride { options: "-new".into(), ..Default::default() };
    assert_eq!(preview(&g, &ov).unwrap().args, sv(&["-new"]));
}

#[test]
fn env_stripping_rules() {
    assert!(strip_env_var("LODESTAR_HOME"));
    assert!(strip_env_var("RUST_LOG"));
    assert!(!strip_env_var("HOME"));
    assert!(!strip_env_var("WAYLAND_DISPLAY"));
}

#[test]
fn log_names() {
    assert_eq!(log_file_name(3, "Half-Life 2: Episode One"), "3-half-life-2-episode-one.log");
    assert_eq!(log_file_name(4, "日本"), "4.log");
}

// ----- launching & tracking (real, harmless processes) -----

fn launcher(dir: &Path) -> Launcher {
    let mut l = Launcher::with_log_dir(&egui::Context::default(), dir.to_path_buf());
    l.poll_every = Duration::from_millis(150);
    l
}

fn sh(script: &str, kind: ResolvedKind) -> Resolved {
    finish("sh".into(), vec!["-c".into(), script.into()], vec![], None, kind, vec![])
}

/// Poll until `id` exits; returns all events and the time it took.
fn wait_exit(l: &mut Launcher, id: GameId, limit: Duration) -> (Vec<LaunchEvent>, Duration) {
    let t0 = Instant::now();
    let mut all = vec![];
    while t0.elapsed() < limit {
        let evs = l.poll();
        let done = evs.iter().any(|e| matches!(e, LaunchEvent::Exited { id: i, .. } if *i == id));
        all.extend(evs);
        if done {
            return (all, t0.elapsed());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    panic!("game {id} did not exit within {limit:?}; events {all:?}");
}

fn exited_secs(evs: &[LaunchEvent]) -> u64 {
    evs.iter()
        .find_map(|e| match e {
            LaunchEvent::Exited { secs, .. } => Some(*secs),
            _ => None,
        })
        .unwrap()
}

#[test]
fn tracks_simple_process() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = launcher(dir.path());
    l.launch_resolved(1, "Sleep", &sh("sleep 1", ResolvedKind::Process)).unwrap();
    assert_eq!(l.running(), Some(1));
    let (evs, took) = wait_exit(&mut l, 1, Duration::from_secs(6));
    assert_eq!(evs[0], LaunchEvent::Started { id: 1 });
    assert_eq!(exited_secs(&evs), 1);
    assert!(took >= Duration::from_millis(900), "{took:?}");
    assert!(!evs.iter().any(|e| matches!(e, LaunchEvent::Failed { .. })));
    assert_eq!(l.running(), None);
}

#[test]
fn tracks_forking_launcher_by_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = launcher(dir.path());
    let t0 = Instant::now();
    // The launcher exits at once; its background child keeps the "game" alive for 2 s.
    l.launch_resolved(2, "Fork", &sh("(sleep 2 &) ; exit 0", ResolvedKind::Process)).unwrap();
    let (evs, _) = wait_exit(&mut l, 2, Duration::from_secs(8));
    assert!(t0.elapsed() >= Duration::from_millis(1900), "{:?}", t0.elapsed());
    assert_eq!(exited_secs(&evs), 2);
}

#[test]
fn tracks_child_that_escapes_with_setsid() {
    if find_in_path("setsid").is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut l = launcher(dir.path());
    let t0 = Instant::now();
    // Child starts its own session; we must see it while its parent still lives (0.6 s).
    l.launch_resolved(3, "Escape", &sh("setsid sleep 2 & sleep 0.6", ResolvedKind::Process)).unwrap();
    let _ = wait_exit(&mut l, 3, Duration::from_secs(8));
    assert!(t0.elapsed() >= Duration::from_millis(1900), "{:?}", t0.elapsed());
}

/// The Wine/Proton pattern: the launcher detaches the game into a new session and exits at
/// once, so the game is reparented before the monitor ever sees it as a descendant. Only the
/// inherited launch tag links it to the launch.
#[test]
fn tracks_detached_child_by_launch_tag() {
    if find_in_path("setsid").is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut l = launcher(dir.path());
    let t0 = Instant::now();
    l.launch_resolved(
        5,
        "Detached",
        &sh("setsid sh -c 'sleep 3' </dev/null >/dev/null 2>&1 & exit 0", ResolvedKind::Process),
    )
    .unwrap();
    let _ = wait_exit(&mut l, 5, Duration::from_secs(10));
    assert!(t0.elapsed() >= Duration::from_millis(2800), "game was lost after {:?}", t0.elapsed());
}

#[test]
fn missing_program_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = launcher(dir.path());
    let r = finish("lodestar-no-such-program".into(), vec![], vec![], None, ResolvedKind::Process, vec![]);
    let e = l.launch_resolved(4, "Missing", &r).unwrap_err().to_string();
    assert!(e.contains("not found"), "{e}");
    assert_eq!(l.running(), None);
    let mut r2 = sh("true", ResolvedKind::Process);
    r2.cwd = Some(dir.path().join("nope"));
    assert!(l.launch_resolved(4, "Missing", &r2).is_err());
    assert!(l.poll().is_empty());
}

#[test]
fn quick_failure_is_reported_with_log_tail() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = launcher(dir.path());
    l.launch_resolved(5, "Crash", &sh("echo starting; echo 'fatal: no GPU' >&2; exit 3", ResolvedKind::Process))
        .unwrap();
    let (evs, _) = wait_exit(&mut l, 5, Duration::from_secs(5));
    let err = evs
        .iter()
        .find_map(|e| match e {
            LaunchEvent::Failed { error, .. } => Some(error.clone()),
            _ => None,
        })
        .expect("failed event");
    assert!(err.contains("code 3") && err.contains("fatal: no GPU"), "{err}");
}

#[test]
fn env_cwd_and_log_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = launcher(dir.path());
    let mut r = sh("echo \"foo=$FOO pwd=$(pwd)\"", ResolvedKind::Process);
    r.env = ev(&[("FOO", "bar")]);
    r.cwd = Some(dir.path().to_path_buf());
    l.launch_resolved(6, "Env Test", &r).unwrap();
    let _ = wait_exit(&mut l, 6, Duration::from_secs(5));
    let log = l.last_log(6).unwrap().to_path_buf();
    assert_eq!(log, dir.path().join("6-env-test.log"));
    let text = std::fs::read_to_string(&log).unwrap();
    let canon = dir.path().canonicalize().unwrap();
    assert!(text.contains(&format!("foo=bar pwd={}", canon.display())), "{text}");
}

#[test]
fn child_runs_in_its_own_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = launcher(dir.path());
    // Prints "<pid> <sid>" of the shell; they must match (setsid worked).
    l.launch_resolved(8, "Sid", &sh("echo \"$$ $(cut -d' ' -f6 /proc/$$/stat)\"", ResolvedKind::Process)).unwrap();
    let _ = wait_exit(&mut l, 8, Duration::from_secs(5));
    let text = std::fs::read_to_string(l.last_log(8).unwrap()).unwrap();
    let line = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap();
    let parts: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(parts.len(), 2, "{text}");
    assert_eq!(parts[0], parts[1], "{text}");
}

#[test]
fn stop_terminates_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = launcher(dir.path());
    l.launch_resolved(9, "Long", &sh("sleep 30 & sleep 30", ResolvedKind::Process)).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    l.stop();
    let (evs, took) = wait_exit(&mut l, 9, Duration::from_secs(5));
    assert!(took < Duration::from_secs(4));
    // Killed by us: not reported as a failure.
    assert!(!evs.iter().any(|e| matches!(e, LaunchEvent::Failed { .. })), "{evs:?}");
}

#[test]
fn open_url_is_untracked() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = launcher(dir.path());
    l.launch_resolved(10, "Url", &sh("exit 0", ResolvedKind::OpenUrl)).unwrap();
    assert_eq!(l.running(), None);
    std::thread::sleep(Duration::from_millis(400));
    assert!(l.poll().is_empty());
}

#[test]
fn steam_kind_tracks_reaper() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = launcher(dir.path());
    let appid = 3_999_999_917u32; // not a real Steam app
    // Stand-in for the steam client command: returns at once.
    l.launch_resolved(11, "SteamGame", &sh("exit 0", ResolvedKind::Steam { appid })).unwrap();
    assert_eq!(l.running(), Some(11));
    std::thread::sleep(Duration::from_millis(400));
    assert!(l.poll().is_empty(), "not started before the reaper shows up");
    assert_eq!(l.running(), Some(11));
    // Fake reaper process: argv contains SteamLaunch AppId=<appid>. It runs until the test
    // closes its stdin (no sleep racing the monitor's polling under load).
    let mut fake = std::process::Command::new("sh")
        .args(["-c", "read _", "SteamLaunch", &format!("AppId={appid}")])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let t0 = Instant::now();
    let mut evs = vec![];
    while !evs.contains(&LaunchEvent::Started { id: 11 }) {
        assert!(t0.elapsed() < Duration::from_secs(30), "the reaper was never seen: {evs:?}");
        evs.extend(l.poll());
        std::thread::sleep(Duration::from_millis(30));
    }
    assert_eq!(evs, vec![LaunchEvent::Started { id: 11 }]);
    // Keep it up for over a second, then end it: the session counts at least that second.
    std::thread::sleep(Duration::from_millis(1100));
    let up = t0.elapsed();
    drop(fake.stdin.take());
    let (evs, took) = wait_exit(&mut l, 11, Duration::from_secs(30));
    let _ = fake.wait();
    let secs = exited_secs(&evs);
    assert!(secs >= 1 && secs <= (up + took).as_secs() + 1, "{secs} (up {up:?}, exit seen after {took:?})");
}

#[test]
fn steam_kind_gives_up_silently() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = launcher(dir.path());
    l.steam_timeout = Duration::from_millis(600);
    l.launch_resolved(12, "Never", &sh("exit 0", ResolvedKind::SteamShortcut { appid: 3_999_999_918 })).unwrap();
    let (evs, _) = wait_exit(&mut l, 12, Duration::from_secs(5));
    assert_eq!(evs, vec![LaunchEvent::Exited { id: 12, secs: 0 }]);
}

#[test]
fn relative_program_runs_from_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("run.sh");
    write_exe(&script, "#!/bin/sh\necho relative-ok\n");
    let logs = tempfile::tempdir().unwrap();
    let mut l = launcher(logs.path());
    let mut r = finish("./run.sh".into(), vec![], vec![], None, ResolvedKind::Process, vec![]);
    assert!(l.launch_resolved(13, "Rel", &r).is_err(), "not found without a cwd");
    r.cwd = Some(dir.path().to_path_buf());
    l.launch_resolved(13, "Rel", &r).unwrap();
    let _ = wait_exit(&mut l, 13, Duration::from_secs(5));
    let text = std::fs::read_to_string(l.last_log(13).unwrap()).unwrap();
    assert!(text.contains("relative-ok"), "{text}");
}

/// Real-data check: `LODESTAR_HOME=<dir with a scanned DB> cargo test real_library -- --ignored --nocapture`.
/// Resolves every game (never launches anything).
#[test]
#[ignore]
fn real_library_resolves() {
    let paths = crate::config::Paths::detect();
    let db = crate::db::Db::open(&paths.db_file()).unwrap();
    let games = db.load_all().unwrap();
    assert!(!games.is_empty(), "scan first (lodestar --scan)");
    let sample = LaunchOverride { options: "-fullscreen".into(), ..Default::default() };
    let wrap = LaunchOverride { options: "MANGOHUD=1 gamemoderun %command%".into(), ..Default::default() };
    for g in &games {
        let base = resolve(g);
        let args = preview(g, &sample);
        let wrapped = preview(g, &wrap);
        println!(
            "{:<5} {:<8} {:<40} {:?}\n      {}\n      +args: {}\n      +wrap: {}",
            g.id,
            g.source.key(),
            g.display_name(),
            base.as_ref().map(|r| r.kind).ok(),
            base.as_ref().map(|r| r.display.clone()).unwrap_or_else(|e| format!("ERR {e}")),
            args.map(|r| r.display).unwrap_or_else(|e| format!("ERR {e}")),
            wrapped.map(|r| r.display).unwrap_or_else(|e| format!("ERR {e}")),
        );
        if g.launch.is_some() {
            assert!(base.is_ok(), "{}: {base:?}", g.display_name());
        }
    }
}

#[test]
fn started_vs_running_and_cancel_pending_steam() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = launcher(dir.path());
    l.launch_resolved(14, "Pending", &sh("exit 0", ResolvedKind::Steam { appid: 3_999_999_919 })).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(l.poll().is_empty());
    assert_eq!((l.running(), l.started()), (Some(14), None));
    l.stop();
    let (evs, took) = wait_exit(&mut l, 14, Duration::from_secs(3));
    assert_eq!(evs, vec![LaunchEvent::Exited { id: 14, secs: 0 }]);
    assert!(took < Duration::from_secs(1));
    assert_eq!(l.running(), None);
    // Process kind: started right away.
    l.launch_resolved(15, "P", &sh("sleep 0.3", ResolvedKind::Process)).unwrap();
    let _ = l.poll();
    assert_eq!(l.started(), Some(15));
    let _ = wait_exit(&mut l, 15, Duration::from_secs(3));
    assert_eq!(l.started(), None);
}

#[test]
fn launch_game_applies_override_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = launcher(dir.path());
    let spec = cmd("sh", &["-c", "echo \"env=$E2E_VAR args=$*\"", "sh", "--source-arg"]);
    let mut g = game(Some(spec), opts("E2E_VAR=hello %command% --from-lodestar"));
    g.id = 16;
    l.launch(&g).unwrap();
    let (evs, _) = wait_exit(&mut l, 16, Duration::from_secs(5));
    assert_eq!(evs[0], LaunchEvent::Started { id: 16 });
    let text = std::fs::read_to_string(l.last_log(16).unwrap()).unwrap();
    assert!(text.contains("env=hello args=--source-arg --from-lodestar"), "{text}");
    // The log header shows the resolved command.
    assert!(text.contains("E2E_VAR=hello sh -c"), "{text}");
    // A custom command fully replaces the source command (and its arguments).
    g.launch_override = Some(LaunchOverride {
        options: "-x".into(),
        command: Some("E2E_VAR=custom sh -c 'echo \"mine=$E2E_VAR $*\"' sh".into()),
        cwd: None,
    });
    l.launch(&g).unwrap();
    let _ = wait_exit(&mut l, 16, Duration::from_secs(5));
    let text = std::fs::read_to_string(l.last_log(16).unwrap()).unwrap();
    assert!(text.contains("mine=custom -x") && !text.contains("--source-arg"), "{text}");
}

// ----- audit regressions -----

const NASTY: &[&str] = &[
    "",
    " ",
    "a b",
    "it's",
    "\"double\"",
    "back\\slash",
    "trailing\\",
    "$HOME",
    "${HOME}",
    "`id`",
    "$(id)",
    "~",
    "~/x",
    "a=~/x",
    "--opt=~/x",
    "x~y",
    "%command%",
    "A=1",
    "new\nline",
    "tab\there",
    "semi;colon && pipe | amp &",
    "glob*?[x]",
    "#hash",
    "-",
    "--",
    "日本語 ゲーム",
    "nbsp\u{a0}space",
    "'",
    "''",
    "\\'",
    "!bang",
];

#[test]
fn shell_quote_round_trips_nasty_strings() {
    for w in NASTY {
        let q = shell_quote(w);
        assert_eq!(split_command(&q).unwrap(), vec![w.to_string()], "{w:?} -> {q}");
        // Also through the custom-command parser, which expands `$VAR` and `~`.
        let ov = LaunchOverride { command: Some(format!("/bin/prog {q} {}", shell_quote("z"))), ..Default::default() };
        let x = r(&game(None, Some(ov)));
        assert_eq!(x.args, sv(&[w, "z"]), "{w:?} -> {q}");
    }
    // A whole argv joined by shell_quote splits back into the same argv.
    let joined: Vec<String> = NASTY.iter().map(|w| shell_quote(w)).collect();
    assert_eq!(split_command(&joined.join(" ")).unwrap(), sv(NASTY));
}

#[test]
fn display_round_trips_through_custom_command() {
    // What the UI shows as "Will run:" can be pasted into the custom command field.
    let mut spec = cmd("B=1", &["~/x", "a b", "$HOME"]);
    if let LaunchSpec::Command { env, .. } = &mut spec {
        *env = ev(&[("WINEPREFIX", "/p q/~")]);
    }
    let x = r(&game(Some(spec.clone()), None));
    let ov = LaunchOverride { command: Some(x.display.clone()), ..Default::default() };
    let y = r(&game(None, Some(ov)));
    assert_eq!((y.program, y.args, y.env), (x.program, x.args, x.env), "{}", x.display);
    let t = command_template(&game(Some(spec), None)).unwrap();
    let ov = LaunchOverride { command: Some(t.clone()), ..Default::default() };
    assert_eq!(r(&game(None, Some(ov))).program, "B=1", "{t}");
}

#[test]
fn command_placeholder_edge_cases() {
    let g = |o: &str| game(Some(cmd("/g", &["a"])), opts(o));
    let h = host(SteamCmd::Native);
    // Twice: an error, not a literal "%command%" argument passed to the game.
    let e = resolve_with(&g("gamemoderun %command% %command%"), &h).unwrap_err();
    assert!(format!("{e:#}").contains("once"), "{e:#}");
    let e = resolve_with(&g("%command% -x '%command%'"), &h).unwrap_err();
    assert!(format!("{e:#}").contains("once"), "{e:#}");
    // Quoted placeholder still counts (Steam replaces it textually).
    let x = r(&g("mangohud \"%command%\" -x"));
    assert_eq!((x.program.as_str(), x.args.clone()), ("mangohud", sv(&["/g", "a", "-x"])));
    // Empty / whitespace options change nothing.
    for o in ["", "   ", "\n"] {
        let x = r(&g(o));
        assert_eq!((x.program.as_str(), x.args.clone(), x.env.len()), ("/g", sv(&["a"]), 0));
    }
    // Only env, before and with the placeholder.
    let x = r(&g("A=1 B=\"x y\""));
    assert_eq!((x.args.clone(), x.env.clone()), (sv(&["a"]), ev(&[("A", "1"), ("B", "x y")])));
    let x = r(&g("A=1 %command%"));
    assert_eq!((x.program.as_str(), x.env.clone()), ("/g", ev(&[("A", "1")])));
    // Env words after %command% are game arguments.
    let x = r(&g("%command% A=1"));
    assert_eq!((x.args.clone(), x.env.len()), (sv(&["a", "A=1"]), 0));
}

#[test]
fn custom_command_env_prefix_and_env_program() {
    // `env VAR=… prog` folds into the environment, so options' VAR=… override it.
    let ov = LaunchOverride {
        command: Some("WINEDEBUG=-all env WINEPREFIX=/src 'DXVK_HUD=fps' /usr/bin/env A=1 umu-run 'g a.exe' -x".into()),
        options: "WINEPREFIX=/mine gamemoderun %command% -dx11".into(),
        cwd: None,
    };
    let x = r(&game(None, Some(ov)));
    assert_eq!(x.program, "gamemoderun");
    assert_eq!(x.args, sv(&["umu-run", "g a.exe", "-x", "-dx11"]));
    assert_eq!(x.env, ev(&[("WINEDEBUG", "-all"), ("WINEPREFIX", "/mine"), ("DXVK_HUD", "fps"), ("A", "1")]));
    // env(1) with options, or without a program, is left alone.
    for c in ["env -i A=1 prog", "env -u X prog", "env A=1", "env -- A=1 prog"] {
        let ov = LaunchOverride { command: Some(c.into()), ..Default::default() };
        let x = r(&game(None, Some(ov)));
        assert_eq!(x.program, "env", "{c}");
        assert!(x.env.is_empty(), "{c}");
    }
    // Source commands of the form `env A=1 wine game.exe` (desktop entries) too.
    let x = r(&game(Some(cmd("env", &["WINEPREFIX=/p", "wine", "g.exe"])), opts("WINEPREFIX=/q")));
    assert_eq!((x.program.as_str(), x.args.clone()), ("wine", sv(&["g.exe"])));
    assert_eq!(x.env, ev(&[("WINEPREFIX", "/q")]));
    // A launcher behind env(1) is still recognised for the "arguments go to …" note.
    let x = r(&game(Some(cmd("env", &["A=1", "faugus-launcher", "--game", "x"])), opts("-x")));
    assert!(x.notes.iter().any(|n| n.contains("Faugus")), "{:?}", x.notes);
}

#[test]
fn faugus_direct_launch_line_resolves_end_to_end() {
    let dl = crate::scan::faugus::DirectLaunch {
        program: "/home/u/.local/share/faugus-launcher/umu-run".into(),
        args: vec![
            "/home/u/Faugus/fl/drive_c/Program Files (x86)/FL 2026/FL 2026 start.exe".into(),
            "-arg with \"quotes\" and $DOLLAR and `tick` and \\".into(),
        ],
        cwd: Some("/home/u/Faugus/fl/drive_c/Program Files (x86)/FL 2026".into()),
        env: vec![
            ("WINEPREFIX".into(), "/home/u/Faugus/fl".into()),
            ("GAMEID".into(), "umu-default".into()),
            ("PROTONPATH".into(), "/home/u/.local/share/Steam/compatibilitytools.d/Proton-CachyOS Latest".into()),
        ],
    };
    let ov = LaunchOverride {
        command: Some(dl.command_line()),
        options: "WINEPREFIX=/override MANGOHUD=1 gamemoderun %command% -windowed".into(),
        cwd: dl.cwd.clone(),
    };
    let x = r(&game(Some(cmd("faugus-launcher", &["--game", "fl"])), Some(ov)));
    assert_eq!(x.program, "gamemoderun", "{}", dl.command_line());
    let mut want = vec![dl.program.clone()];
    want.extend(dl.args.iter().cloned());
    want.push("-windowed".into());
    assert_eq!(x.args, want, "{}", dl.command_line());
    assert_eq!(
        x.env,
        ev(&[
            ("WINEPREFIX", "/override"),
            ("GAMEID", "umu-default"),
            ("PROTONPATH", "/home/u/.local/share/Steam/compatibilitytools.d/Proton-CachyOS Latest"),
            ("MANGOHUD", "1"),
        ])
    );
    assert_eq!(x.cwd, dl.cwd);
    assert_eq!(x.kind, ResolvedKind::Process);
    assert!(x.notes.is_empty(), "{:?}", x.notes);
    // Without options it is exactly the direct command.
    let ov = LaunchOverride { command: Some(dl.command_line()), ..Default::default() };
    let x = r(&game(None, Some(ov)));
    assert_eq!(
        (x.program.clone(), x.args.clone(), x.env.clone()),
        (dl.program.clone(), dl.args.clone(), dl.env.clone())
    );
}

#[test]
fn steam_direct_launch_line_resolves_end_to_end() {
    let dl = crate::scan::steam::direct::DirectLaunch {
        appid: 30,
        program: "/home/u/.local/share/Steam/compatibilitytools.d/GE Proton/proton".into(),
        args: vec![
            "waitforexitandrun".into(),
            "/games/steamapps/common/It's 100%/Bin/Game.exe".into(),
            "-name=~tilde".into(),
        ],
        cwd: "/games/steamapps/common/It's 100%/Bin".into(),
        env: vec![
            ("SteamAppId".into(), "30".into()),
            ("STEAM_COMPAT_DATA_PATH".into(), "/games/steamapps/compatdata/30".into()),
            ("WINEDLLOVERRIDES".into(), "dxgi=n,b;d3d11=n".into()),
        ],
        compat_tool: Some("GE Proton".into()),
        compat_tool_dir: None,
    };
    let ov = LaunchOverride {
        command: Some(dl.command_line()),
        options: "PROTON_LOG=1 WINEDLLOVERRIDES=dxgi=b %command% -dx12".into(),
        cwd: Some(dl.cwd.clone()),
    };
    // The game stays a Steam game in the library, but the custom command bypasses Steam.
    let x = r(&game(Some(LaunchSpec::Steam { appid: 30 }), Some(ov)));
    assert_eq!(x.program, dl.program, "{}", dl.command_line());
    let mut want = dl.args.clone();
    want.push("-dx12".into());
    assert_eq!(x.args, want);
    assert_eq!(
        x.env,
        ev(&[
            ("SteamAppId", "30"),
            ("STEAM_COMPAT_DATA_PATH", "/games/steamapps/compatdata/30"),
            ("WINEDLLOVERRIDES", "dxgi=b"),
            ("PROTON_LOG", "1"),
        ])
    );
    assert_eq!(x.kind, ResolvedKind::Process);
    assert_eq!(x.cwd.as_deref(), Some(dl.cwd.as_path()));
}

#[test]
fn steam_via_command_line_rejects_wrappers() {
    let h = host(SteamCmd::Native);
    // A source command that hands the game to Steam: env/wrappers would hit the Steam client.
    let g = game(Some(cmd("steam", &["-applaunch", "42"])), opts("MANGOHUD=1 %command%"));
    assert_eq!(resolve_with(&g, &h).unwrap_err().to_string(), STEAM_WRAPPER_ERR);
    let g = game(Some(cmd("steam", &["-applaunch", "42"])), opts("gamemoderun %command%"));
    assert_eq!(resolve_with(&g, &h).unwrap_err().to_string(), STEAM_WRAPPER_ERR);
    // Plain arguments still work.
    let x = r(&game(Some(cmd("steam", &["-applaunch", "42"])), opts("-x")));
    assert_eq!(x.args, sv(&["-applaunch", "42", "-x"]));
    // A custom command the user typed is run as written, with a note.
    let ov = LaunchOverride { command: Some("MANGOHUD=1 steam -applaunch 42".into()), ..Default::default() };
    let x = r(&game(None, Some(ov)));
    assert_eq!(x.kind, ResolvedKind::Steam { appid: 42 });
    assert!(x.notes.iter().any(|n| n.contains("Steam client")), "{:?}", x.notes);
}

#[test]
fn override_cwd_expands_tilde() {
    let ov = LaunchOverride { cwd: Some("~/Games/x".into()), ..Default::default() };
    assert_eq!(r(&game(Some(cmd("/g", &[])), Some(ov))).cwd, Some("/home/u/Games/x".into()));
    let ov = LaunchOverride { cwd: Some("~other/x".into()), command: Some("/g".into()), ..Default::default() };
    assert_eq!(r(&game(None, Some(ov))).cwd, Some("~other/x".into()));
}

fn fd_is_inheritable(fd: i32) -> bool {
    let fl = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    fl >= 0 && fl & libc::FD_CLOEXEC == 0
}

#[test]
fn games_do_not_inherit_lodestar_fds() {
    let dir = tempfile::tempdir().unwrap();
    // A descriptor opened without O_CLOEXEC, like gilrs' evdev handles.
    let path = std::ffi::CString::new(dir.path().join("leak").to_str().unwrap()).unwrap();
    let raw = unsafe { libc::open(path.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600) };
    assert!(raw >= 0);
    let fd = unsafe { libc::fcntl(raw, libc::F_DUPFD, 300) };
    unsafe { libc::close(raw) };
    assert!(fd >= 300 && fd_is_inheritable(fd));
    let mut l = launcher(dir.path());
    l.launch_resolved(
        17,
        "Fds",
        &sh(&format!("if [ -e /proc/self/fd/{fd} ]; then echo LEAKED; else echo CLEAN; fi"), ResolvedKind::Process),
    )
    .unwrap();
    let _ = wait_exit(&mut l, 17, Duration::from_secs(5));
    unsafe { libc::close(fd) };
    let text = std::fs::read_to_string(l.last_log(17).unwrap()).unwrap();
    assert_eq!(text.lines().last(), Some("CLEAN"), "{text}");
    // Exec failures are still reported (std's error pipe survives the marking).
    let r = finish(
        dir.path().join("leak").to_string_lossy().into_owned(),
        vec![],
        vec![],
        None,
        ResolvedKind::Process,
        vec![],
    );
    assert!(l.launch_resolved(18, "NotExec", &r).is_err());
}

#[test]
fn finished_launches_leave_no_zombies_or_threads() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = launcher(dir.path());
    let mut pids = vec![];
    for (i, kind) in [ResolvedKind::Process, ResolvedKind::OpenUrl, ResolvedKind::Process].into_iter().enumerate() {
        let id = 20 + i as GameId;
        // Prints its pid, forks a short-lived grandchild, exits.
        l.launch_resolved(id, &format!("Z{i}"), &sh("echo pid=$$; (sleep 0.2 &); exit 0", kind)).unwrap();
        if kind == ResolvedKind::Process {
            let _ = wait_exit(&mut l, id, Duration::from_secs(5));
        } else {
            std::thread::sleep(Duration::from_millis(500));
        }
        let text = std::fs::read_to_string(l.last_log(id).unwrap()).unwrap();
        let pid: i32 = text
            .lines()
            .find_map(|s| s.strip_prefix("pid="))
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or_else(|| panic!("{text}"));
        pids.push(pid);
    }
    std::thread::sleep(Duration::from_millis(300));
    let me = std::process::id() as i32;
    for pid in pids {
        // Reaped: gone from /proc entirely (a zombie would still be listed, state Z).
        if let Some(p) = track::read_procs(false).into_iter().find(|p| p.pid == pid) {
            assert!(p.ppid != me, "child {pid} not reaped: {p:?}");
        }
        let waiter = format!("reap-{pid}");
        let threads: Vec<String> = std::fs::read_dir("/proc/self/task")
            .unwrap()
            .flatten()
            .filter_map(|t| std::fs::read_to_string(t.path().join("comm")).ok())
            .map(|c| c.trim().to_string())
            .collect();
        assert!(!threads.contains(&waiter), "{waiter} still running: {threads:?}");
    }
}

#[test]
fn second_stop_kills_a_game_that_ignores_sigterm() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = launcher(dir.path());
    l.launch_resolved(19, "Stubborn", &sh("trap '' TERM; sleep 30 & wait; wait", ResolvedKind::Process)).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    l.stop();
    std::thread::sleep(Duration::from_millis(600));
    l.stop(); // too soon: ignored, still just SIGTERM'd
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_millis(2700) {
        assert!(
            !l.poll().iter().any(|e| matches!(e, LaunchEvent::Exited { .. })),
            "SIGTERM is ignored, so it must still run"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    l.stop(); // > 3 s after the first: SIGKILL
    let (evs, took) = wait_exit(&mut l, 19, Duration::from_secs(4));
    assert!(took < Duration::from_secs(2), "{took:?}");
    assert!(!evs.iter().any(|e| matches!(e, LaunchEvent::Failed { .. })), "{evs:?}");
}

#[test]
fn custom_command_with_its_own_path_finds_the_program() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let tool = bin.join("lodestar-zz-probe-tool");
    write_exe(&tool, "#!/bin/sh\nexit 0\n");
    // What the user types in the Launch Options dialog.
    let line = format!("PATH={}:/usr/bin:/bin lodestar-zz-probe-tool", bin.display());
    let g = game(Some(cmd("/bin/true", &[])), Some(LaunchOverride { command: Some(line), ..Default::default() }));
    let res = r(&g);
    assert_eq!(res.program, "lodestar-zz-probe-tool");
    let mut l = launcher(&dir.path().join("logs"));
    l.launch_resolved(7, "Probe", &res).expect("program found through the command's PATH");
    assert_eq!(l.launch_kind(7), Some(ResolvedKind::Process));
    let (evs, _) = wait_exit(&mut l, 7, Duration::from_secs(6));
    assert!(!evs.iter().any(|e| matches!(e, LaunchEvent::Failed { .. })), "{evs:?}");
    // Still answered after the exit (the app decides who counts playtime from it).
    assert_eq!(l.launch_kind(7), Some(ResolvedKind::Process));
    // Without that PATH it isn't found.
    assert!(find_in_path("lodestar-zz-probe-tool").is_none());
    assert_eq!(find_in_path_with("lodestar-zz-probe-tool", Some(bin.as_os_str())), Some(tool));
}

// ----- actions, HDR, tracking modes, scripts (Edit Game Details) -----

fn action(kind: ActionKind, path: &str, args: &str) -> GameAction {
    GameAction { name: "Act".into(), kind, path: path.into(), args: args.into(), ..Default::default() }
}

#[test]
fn play_action_replaces_the_source_launch_and_expands_variables() {
    let mut g = game(Some(cmd("/opt/game/run", &[])), None);
    g.install_dir = Some("/games/My Game".into());
    g.edits.actions = vec![
        action(ActionKind::Command, "{InstallDir}/settings", "--x"),
        GameAction {
            play: true,
            ..action(
                ActionKind::Command,
                "{InstallDir}/bin/game",
                "--name '{Name}' --id={GameId} '{InstallDir}/save dir'",
            )
        },
    ];
    let r = resolve_with(&g, &host(SteamCmd::Native)).unwrap();
    assert_eq!(r.program, "/games/My Game/bin/game");
    assert_eq!(r.args, sv(&["--name", "Test Game", "--id=7", "/games/My Game/save dir"]));
    // Starts in the program's own folder unless the action says otherwise.
    assert_eq!(r.cwd.as_deref(), Some(Path::new("/games/My Game/bin")));
    assert_eq!(r.kind, ResolvedKind::Process);
    // The other action, by index.
    let r = resolve_action_with(&g, 0, &host(SteamCmd::Native)).unwrap();
    assert_eq!((r.program.as_str(), r.args.clone()), ("/games/My Game/settings", sv(&["--x"])));
    assert!(resolve_action_with(&g, 5, &host(SteamCmd::Native)).is_err());
    // An explicit working directory, `~` expanded.
    g.edits.actions[1].cwd = Some("~/saves/{Name}".into());
    let r = resolve_with(&g, &host(SteamCmd::Native)).unwrap();
    assert_eq!(r.cwd.as_deref(), Some(Path::new("/home/u/saves/Test Game")));
}

#[test]
fn launch_options_wrap_command_actions_and_unknown_variables_are_noted() {
    let mut g = game(
        Some(cmd("/opt/game/run", &[])),
        Some(LaunchOverride {
            options: "MANGOHUD=1 gamemoderun %command% -dx11".into(),
            command: Some("/other/thing".into()),
            cwd: None,
        }),
    );
    g.edits.actions =
        vec![GameAction { play: true, ..action(ActionKind::Command, "umu-run", "'{Prefix}/drive_c/x.exe'") }];
    let r = resolve_with(&g, &host(SteamCmd::Native)).unwrap();
    assert_eq!(r.program, "gamemoderun");
    assert_eq!(r.args, sv(&["umu-run", "/drive_c/x.exe", "-dx11"]));
    assert_eq!(r.env, ev(&[("MANGOHUD", "1")]));
    assert!(r.notes.iter().any(|n| n.contains("instead of the custom command")), "{:?}", r.notes);
    assert!(r.notes.iter().any(|n| n.contains("{Prefix}")), "{:?}", r.notes);
    // URL actions don't take launch options (and say so about arguments).
    g.edits.actions[0] = GameAction { play: true, ..action(ActionKind::Url, "heroic://launch/{GameId}", "--x") };
    let r = resolve_with(&g, &host(SteamCmd::Native)).unwrap();
    assert_eq!((r.program.as_str(), r.args.clone()), ("xdg-open", sv(&["heroic://launch/7"])));
    assert_eq!(r.kind, ResolvedKind::OpenUrl);
    assert!(r.notes.iter().any(|n| n.contains("ignored")));
    // steam:// URLs go to the Steam client.
    g.edits.actions[0].path = "steam://rungameid/570".into();
    g.edits.actions[0].args.clear();
    assert_eq!(resolve_with(&g, &host(SteamCmd::Native)).unwrap().kind, ResolvedKind::Steam { appid: 570 });
    // Files open with the default application.
    g.edits.actions[0] = GameAction { play: true, ..action(ActionKind::File, "~/manual.pdf", "") };
    let r = resolve_with(&g, &host(SteamCmd::Native)).unwrap();
    assert_eq!(
        (r.program.as_str(), r.args.clone(), r.kind),
        ("xdg-open", sv(&["/home/u/manual.pdf"]), ResolvedKind::OpenUrl)
    );
    // Nothing to run.
    g.edits.actions[0].path = "  ".into();
    assert!(resolve_with(&g, &host(SteamCmd::Native)).unwrap_err().to_string().contains("nothing to run"));
}

#[test]
fn prefix_and_proton_variables() {
    let mut g = game(
        Some(LaunchSpec::Command {
            program: "umu-run".into(),
            args: sv(&["/g/x.exe"]),
            cwd: None,
            env: ev(&[("WINEPREFIX", "/pfx/a"), ("PROTONPATH", "/proton/GE")]),
        }),
        None,
    );
    let v = ActionVars::of(&g);
    assert_eq!((v.prefix.as_deref(), v.proton.as_deref()), (Some("/pfx/a"), Some("/proton/GE")));
    // Steam: the compatdata prefix next to the install folder's library.
    g.launch = Some(LaunchSpec::Steam { appid: 620 });
    g.source = Source::Steam;
    g.source_id = "620".into();
    g.install_dir = Some("/lib/steamapps/common/Portal 2".into());
    let v = ActionVars::of(&g);
    assert_eq!(v.prefix.as_deref(), Some("/lib/steamapps/compatdata/620/pfx"));
    assert_eq!(v.proton, None);
    let mut missing = vec![];
    assert_eq!(v.expand("{Proton}/proton run", &mut missing), "/proton run");
    assert_eq!(missing, vec!["{Proton}"]);
}

// ----- game options (spec §5 plumbing; the option rules are in game_options/tests.rs) -----

/// The game-options fixture (fake caps: every tool, GE-Proton at /r/ge, a 2560×1440 monitor).
fn opt_env() -> LaunchEnv {
    crate::game_options::tests::fixture()
}

fn rf(g: &Game, env: &LaunchEnv) -> Resolved {
    resolve_full(g, &host(SteamCmd::Native), env).unwrap()
}

fn umu_game(go: crate::game_options::GameOptions, ov: Option<LaunchOverride>) -> Game {
    let mut g = game(
        Some(LaunchSpec::Command {
            program: "umu-run".into(),
            args: sv(&["/g/game.exe"]),
            cwd: None,
            env: ev(&[("WINEPREFIX", "/p"), ("GAMEID", "umu-default"), ("PROTONPATH", "/r/ge")]),
        }),
        ov,
    );
    g.name = "Test".into();
    g.edits.game_options = go;
    g
}

fn on() -> crate::game_options::Tri {
    crate::game_options::Tri::On
}

#[test]
fn hdr_option_applies_to_proton_launches() {
    use crate::game_options::GameOptions;
    let g = umu_game(GameOptions { hdr: on(), ..Default::default() }, opts("DXVK_HDR=0"));
    let r = rf(&g, &opt_env());
    // The user's own value wins; HDR turns on Wine's Wayland driver (no gamescope).
    assert_eq!(
        r.env,
        ev(&[
            ("WINEPREFIX", "/p"),
            ("GAMEID", "umu-default"),
            ("PROTONPATH", "/r/ge"),
            ("PROTON_ENABLE_WAYLAND", "1"),
            ("DXVK_HDR", "0"),
        ])
    );
    assert!(r.display.contains("PROTON_ENABLE_WAYLAND=1"), "{}", r.display);
    assert!(!r.env.iter().any(|(k, _)| k == "ENABLE_HDR_WSI" || k == "PROTON_ENABLE_HDR"));
    assert!(r.options.iter().any(|(id, _, _)| *id == OptionId::Hdr));
    // resolve_with: the game's own options still apply (no caps, unknown runner).
    let r = resolve_with(&g, &host(SteamCmd::Native)).unwrap();
    assert!(r.env.iter().any(|(k, v)| k == "PROTON_ENABLE_WAYLAND" && v == "1"), "{:?}", r.env);
    // Native programs get no HDR variables.
    let mut n = game(Some(cmd("/opt/game/run", &[])), None);
    n.edits.game_options.hdr = on();
    assert!(rf(&n, &opt_env()).env.is_empty());
}

#[test]
fn steam_global_defaults_no_error() {
    use crate::game_options::GameOptions;
    let mut env = opt_env();
    env.defaults = GameOptions { mangohud: on(), gamemode: on(), ..Default::default() };
    let s = game(Some(LaunchSpec::Steam { appid: 570 }), None);
    let r = rf(&s, &env);
    assert_eq!(r.kind, ResolvedKind::Steam { appid: 570 });
    assert_eq!((r.program.as_str(), r.args.clone()), ("steam", sv(&["steam://rungameid/570"])));
    assert!(r.env.is_empty());
    let steam_notes: Vec<_> = r.notes.iter().filter(|n| n.contains("Steam client")).collect();
    assert_eq!(steam_notes.len(), 1, "{:?}", r.notes);
    assert!(r.steam_string.as_deref().is_some_and(|s| s.ends_with("%command%")), "{:?}", r.steam_string);
    // Extra args still work; typed wrappers are still an error (unchanged rule).
    let s = game(Some(LaunchSpec::Steam { appid: 570 }), opts("-novid"));
    assert_eq!(rf(&s, &env).args, sv(&["-applaunch", "570", "-novid"]));
    let s = game(Some(LaunchSpec::Steam { appid: 570 }), opts("mangohud %command%"));
    assert!(resolve_full(&s, &host(SteamCmd::Native), &env).is_err());
    // Shortcuts too.
    let sc = game(Some(LaunchSpec::Url("steam://rungameid/12345678901234".into())), None);
    let r = rf(&sc, &env);
    assert!(matches!(r.kind, ResolvedKind::SteamShortcut { .. }));
    assert!(r.notes.iter().any(|n| n.contains("Steam client")));
}

#[test]
fn steam_url_gets_the_steam_note() {
    use crate::game_options::GameOptions;
    let mut env = opt_env();
    env.defaults = GameOptions { mangohud: on(), ..Default::default() };
    let g = game(Some(LaunchSpec::Url("steam://open/games".into())), None);
    for steam in [SteamCmd::Native, SteamCmd::Flatpak] {
        let r = resolve_full(&g, &host(steam), &env).unwrap();
        assert_eq!(r.kind, ResolvedKind::OpenUrl);
        assert!(r.env.is_empty());
        assert!(r.notes.iter().any(|n| n.contains("Steam client")), "{:?}", r.notes);
        assert!(r.steam_string.as_deref().is_some_and(|s| s.ends_with("%command%")), "{:?}", r.steam_string);
    }
}

#[test]
fn url_launch_gets_a_note() {
    use crate::game_options::GameOptions;
    let mut g = game(Some(LaunchSpec::Url("heroic://launch/legendary/abc".into())), None);
    g.edits.game_options = GameOptions { mangohud: on(), ..Default::default() };
    let r = rf(&g, &opt_env());
    assert_eq!(r.kind, ResolvedKind::OpenUrl);
    assert!(r.env.is_empty());
    assert!(r.notes.iter().any(|n| n.contains("Heroic")), "{:?}", r.notes);
}

#[test]
fn non_play_action_gets_env_only() {
    use crate::game_options::GameOptions;
    let mut g = umu_game(GameOptions { gamescope: on(), hdr: on(), ..Default::default() }, None);
    g.edits.actions = vec![action(ActionKind::Command, "umu-run", "/g/settings.exe")];
    let r = resolve_action_full(&g, 0, &host(SteamCmd::Native), &opt_env()).unwrap();
    assert_eq!(r.program, "umu-run");
    assert!(r.env.iter().any(|(k, v)| k == "DXVK_HDR" && v == "1"), "{:?}", r.env);
    assert!(!r.display.contains("gamescope"), "{}", r.display);
    // The Play action gets the wrappers.
    g.edits.actions[0].play = true;
    let r = rf(&g, &opt_env());
    assert_eq!(r.program, "gamescope");
}

#[test]
fn display_matches_composed_command() {
    use crate::game_options::GameOptions;
    let g = umu_game(
        GameOptions {
            gamescope: on(),
            mangohud: on(),
            hdr: on(),
            gamemode: on(),
            performance_profile: on(),
            keep_awake: on(),
            ..Default::default()
        },
        None,
    );
    let r = rf(&g, &opt_env());
    assert_eq!(r.program, "systemd-inhibit");
    let mut words: Vec<String> = r.env.iter().map(|(k, v)| format!("{k}={}", shell_quote(v))).collect();
    words.push(r.program.clone());
    words.extend(r.args.iter().map(|a| shell_quote(a)));
    assert_eq!(r.display, words.join(" "));
    assert!(r.display.contains("'--why=Playing Test'"), "{}", r.display);
}

#[test]
fn custom_command_env_wins_over_toggles() {
    use crate::game_options::{GameOptions, Tri};
    let g = umu_game(
        GameOptions { hdr: on(), ..Default::default() },
        Some(LaunchOverride {
            command: Some("DXVK_HDR=0 PROTONPATH=/r/ge umu-run x.exe".into()),
            ..Default::default()
        }),
    );
    let r = rf(&g, &opt_env());
    assert!(r.env.iter().any(|(k, v)| k == "DXVK_HDR" && v == "0"), "{:?}", r.env);
    assert!(!r.env.iter().any(|(k, v)| k == "DXVK_HDR" && v == "1"));
    let mut g = umu_game(GameOptions { wayland: Tri::Off, ..Default::default() }, None);
    if let Some(LaunchSpec::Command { env, .. }) = &mut g.launch {
        env.push(("PROTON_ENABLE_WAYLAND".into(), "1".into()));
    }
    let r = rf(&g, &opt_env());
    assert!(r.env.iter().any(|(k, v)| k == "PROTON_ENABLE_WAYLAND" && v == "0"), "{:?}", r.env);
}

#[test]
fn proton_log_dir_is_created_before_spawning() {
    use crate::game_options::GameOptions;
    let dir = tempfile::tempdir().unwrap();
    let mut env = opt_env();
    env.log_dir = Some(dir.path().join("proton-logs"));
    // A harmless command classified as Proton: `<dir>/proton` is a fake script.
    let fake = dir.path().join("proton");
    write_exe(&fake, "#!/bin/sh\nexit 0\n");
    let mut g = game(Some(cmd(&fake.display().to_string(), &["run", "x.exe"])), None);
    g.edits.game_options = GameOptions { proton_log: on(), ..Default::default() };
    let r = rf(&g, &env);
    let want = dir.path().join("proton-logs/7");
    assert_eq!(r.make_dirs, vec![want.clone()]);
    assert!(!want.exists());
    let mut l = Launcher::with_log_dir(&egui::Context::default(), dir.path().join("logs"));
    l.launch_resolved(7, "Test Game", &r).unwrap();
    assert!(want.is_dir());
    let t = Instant::now();
    while l.running().is_some() && t.elapsed() < Duration::from_secs(10) {
        l.poll();
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn plan(tracking: TrackingMode, dir: Option<&Path>) -> LaunchPlan {
    LaunchPlan { tracking, install_dir: dir.map(Path::to_path_buf), via_client: false, scripts: Default::default() }
}

#[test]
fn tracking_mode_process_follows_only_the_started_process() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = launcher(dir.path());
    // Leaves a child running for 2 s and exits at once.
    let r = sh("(sleep 2 &); exit 0", ResolvedKind::Process);
    l.launch_planned(30, "Proc", &r, &plan(TrackingMode::Process, None)).unwrap();
    let (evs, took) = wait_exit(&mut l, 30, Duration::from_secs(5));
    assert_eq!(evs[0], LaunchEvent::Started { id: 30 });
    assert!(took < Duration::from_millis(1500), "{took:?}");
    // Default: the whole session.
    l.launch_resolved(31, "Sess", &r).unwrap();
    let (_, took) = wait_exit(&mut l, 31, Duration::from_secs(6));
    assert!(took >= Duration::from_millis(1800), "{took:?}");
}

#[test]
fn tracking_mode_folder_follows_a_handed_off_game() {
    let dir = tempfile::tempdir().unwrap();
    let game_dir = dir.path().join("My Game");
    std::fs::create_dir_all(&game_dir).unwrap();
    let mut l = launcher(dir.path());
    // A launcher that hands the game to an unrelated process (own session, no launch tag) in the
    // game's folder, then exits.
    let script = format!(
        "cd '{}' && env -u {} setsid -f sh -c 'sleep 2' >/dev/null 2>&1; exit 0",
        game_dir.display(),
        track::TAG_VAR
    );
    let r = sh(&script, ResolvedKind::Process);
    l.launch_planned(32, "Folder", &r, &plan(TrackingMode::Folder, Some(&game_dir))).unwrap();
    let (evs, took) = wait_exit(&mut l, 32, Duration::from_secs(10));
    assert!(took >= Duration::from_millis(1800), "stopped tracking early: {took:?} {evs:?}");
    assert!(exited_secs(&evs) >= 2, "{evs:?}");
    assert!(!evs.iter().any(|e| matches!(e, LaunchEvent::Failed { .. })), "{evs:?}");
    // The same launch tracked by its session ends when the launcher does.
    l.launch_resolved(33, "Default", &r).unwrap();
    let (_, took) = wait_exit(&mut l, 33, Duration::from_secs(5));
    assert!(took < Duration::from_millis(1500), "{took:?}");
    // Folder tracking without a folder falls back to the usual tracking.
    assert_eq!(track_mode(ResolvedKind::Process, TrackingMode::Folder, None), track::Mode::Session);
    assert_eq!(track_mode(ResolvedKind::Process, TrackingMode::Folder, Some(Path::new("/"))), track::Mode::Session);
    assert_eq!(
        track_mode(ResolvedKind::Steam { appid: 1 }, TrackingMode::Process, None),
        track::Mode::Steam { appid: 1 }
    );
    assert_eq!(track_mode(ResolvedKind::Process, TrackingMode::None, None), track::Mode::Untracked);
    assert_eq!(track_mode(ResolvedKind::OpenUrl, TrackingMode::Folder, Some(&game_dir)), track::Mode::Folder);
}

#[test]
fn folder_tracking_waits_for_a_url_launched_game_to_show_up() {
    let dir = tempfile::tempdir().unwrap();
    let game_dir = dir.path().join("g");
    std::fs::create_dir_all(&game_dir).unwrap();
    let mut l = launcher(dir.path());
    // Like `xdg-open heroic://…`: the handler returns at once, the game appears a moment later.
    let script = format!(
        "env -u {} setsid -f sh -c 'sleep 0.5; cd \"{}\" && sleep 1.5' >/dev/null 2>&1; exit 0",
        track::TAG_VAR,
        game_dir.display()
    );
    l.launch_planned(34, "Url", &sh(&script, ResolvedKind::OpenUrl), &plan(TrackingMode::Folder, Some(&game_dir)))
        .unwrap();
    assert_eq!(l.running(), Some(34));
    assert_eq!(l.started(), None);
    let (evs, _) = wait_exit(&mut l, 34, Duration::from_secs(10));
    assert_eq!(evs.iter().filter(|e| matches!(e, LaunchEvent::Started { .. })).count(), 1, "{evs:?}");
    let secs = exited_secs(&evs);
    assert!((1..=3).contains(&secs), "{secs} {evs:?}");
}

#[test]
fn tracking_none_runs_but_follows_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = launcher(dir.path());
    let marker = dir.path().join("ran");
    let r = sh(&format!("touch '{}'", marker.display()), ResolvedKind::Process);
    l.launch_planned(35, "None", &r, &plan(TrackingMode::None, None)).unwrap();
    assert_eq!(l.running(), None);
    std::thread::sleep(Duration::from_millis(400));
    assert!(l.poll().is_empty());
    assert!(marker.exists());
}

#[test]
fn folder_path_helpers() {
    assert!(track::is_inside(Path::new("/g/a/bin/x"), Path::new("/g/a")));
    assert!(!track::is_inside(Path::new("/g/ab/x"), Path::new("/g/a")));
    assert!(!track::is_inside(Path::new("/g/a/x"), Path::new("/")));
    assert_eq!(track::unix_path_of("Z:\\g\\a\\game.exe"), Some(PathBuf::from("/g/a/game.exe")));
    assert_eq!(track::unix_path_of("/g/a/game"), Some(PathBuf::from("/g/a/game")));
    assert_eq!(track::unix_path_of("C:\\windows\\x.exe"), None);
    assert_eq!(track::unix_path_of("game"), None);
}

#[test]
fn spawn_errors_after_before_scripts_arrive_as_events() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = launcher(dir.path());
    let gone = dir.path().join("gone");
    std::fs::create_dir_all(&gone).unwrap();
    let mut r = sh("exit 0", ResolvedKind::Process);
    r.cwd = Some(gone.clone());
    let mut p = plan(TrackingMode::Default, None);
    // The script removes the working directory before the game starts.
    p.scripts.before_start =
        vec![scripts::Script { label: "Before-start script".into(), text: format!("rmdir '{}'", gone.display()) }];
    l.launch_planned(36, "Gone", &r, &p).unwrap();
    let (evs, _) = wait_exit(&mut l, 36, Duration::from_secs(5));
    assert!(evs.iter().any(|e| matches!(e, LaunchEvent::Failed { .. })), "{evs:?}");
    assert!(
        !l.scripts_busy() || {
            std::thread::sleep(Duration::from_millis(200));
            !l.scripts_busy()
        }
    );
}

#[test]
fn windows_program_actions_run_with_the_games_proton_and_prefix() {
    let mut g = game(
        Some(LaunchSpec::Command {
            program: "umu-run".into(),
            args: sv(&["/pfx/drive_c/Game/game.exe"]),
            cwd: None,
            env: ev(&[("WINEPREFIX", "/pfx"), ("PROTONPATH", "/protons/GE-Proton10")]),
        }),
        None,
    );
    g.install_dir = Some("/pfx/drive_c/Game".into());
    g.edits.actions = vec![action(ActionKind::Command, "{InstallDir}/Settings.EXE", "-lang en")];
    let r = resolve_action_with(&g, 0, &host(SteamCmd::Native)).unwrap();
    // umu-run when it's installed, else Proton's own `proton run`.
    let exe_at = if r.program == "umu-run" {
        0
    } else {
        assert_eq!(r.program, "/protons/GE-Proton10/proton");
        assert_eq!(r.args[0], "run");
        1
    };
    assert_eq!(r.args[exe_at..], sv(&["/pfx/drive_c/Game/Settings.EXE", "-lang", "en"]));
    assert!(r.env.contains(&("WINEPREFIX".into(), "/pfx".into())), "{:?}", r.env);
    assert!(r.env.contains(&("PROTONPATH".into(), "/protons/GE-Proton10".into())), "{:?}", r.env);
    assert_eq!(r.cwd.as_deref(), Some(Path::new("/pfx/drive_c/Game")));
    assert!(r.notes.iter().any(|n| n.contains("GE-Proton10")), "{:?}", r.notes);

    // No prefix known (a native game, the .exe outside any prefix): runs as written, with a note.
    let mut g = game(Some(cmd("/opt/game/run", &[])), None);
    g.edits.actions = vec![action(ActionKind::Command, "/opt/tools/tool.exe", "")];
    let r = resolve_action_with(&g, 0, &host(SteamCmd::Native)).unwrap();
    assert_eq!(r.program, "/opt/tools/tool.exe");
    assert!(r.notes.iter().any(|n| n.contains("no Wine prefix")), "{:?}", r.notes);
}

#[test]
fn store_links_with_an_install_folder_are_followed_by_folder() {
    let dir = Path::new("/home/someone/Games/Heroic/Rayman Forever");
    assert_eq!(track_mode(ResolvedKind::OpenUrl, TrackingMode::Default, Some(dir)), track::Mode::Folder);
    assert_eq!(track_mode(ResolvedKind::OpenUrl, TrackingMode::Default, None), track::Mode::Untracked);
    // Folders too broad to stand for one game are never followed.
    for broad in
        ["/", "/home", "/mnt/drive", "/home/someone/Games", "/home/someone/Games/Heroic", "/mnt/x/SteamLibrary"]
    {
        assert!(!is_game_folder(Path::new(broad)), "{broad}");
        assert_eq!(
            track_mode(ResolvedKind::OpenUrl, TrackingMode::Default, Some(Path::new(broad))),
            track::Mode::Untracked,
            "{broad}"
        );
    }
    if let Some(h) = dirs::home_dir() {
        assert!(!is_game_folder(&h));
    }
}

/// A game started through a store client that stays open (Ubisoft Connect, Battle.net …): the game
/// is over when its own folder is quiet, even though the client keeps running, and the client is
/// not closed. Found testing Brawlhalla through Ubisoft Connect on a real machine.
#[test]
fn store_client_games_end_with_the_game_not_the_client() {
    let dir = tempfile::tempdir().unwrap();
    let game_dir = dir.path().join("Ubisoft/games/Some Game");
    std::fs::create_dir_all(&game_dir).unwrap();
    let marker = dir.path().join("client-alive");
    let mut l = launcher(dir.path());
    // The "client" runs 6 s in the launch session; after 0.5 s it starts the "game" (2 s) from
    // the game folder, detached like Wine does.
    let script = format!(
        "(sleep 0.5; cd '{g}' && setsid -f sh -c 'sleep 2' >/dev/null 2>&1) & sleep 6; touch '{m}'",
        g = game_dir.display(),
        m = marker.display()
    );
    let r = sh(&script, ResolvedKind::Process);
    let mut p = plan(TrackingMode::Default, Some(&game_dir));
    p.via_client = true;
    l.launch_planned(40, "Client game", &r, &p).unwrap();
    let (evs, took) = wait_exit(&mut l, 40, Duration::from_secs(12));
    assert!(evs.contains(&LaunchEvent::Started { id: 40 }), "{evs:?}");
    // Ended with the game (≈2.5 s + grace), not with the client (6 s).
    assert!(took < Duration::from_millis(5500), "followed the client: {took:?} {evs:?}");
    assert!(exited_secs(&evs) >= 1, "{evs:?}");
    // The client was left alone and finishes its own work.
    std::thread::sleep(Duration::from_secs(4));
    assert!(marker.exists(), "the store client was closed");
}

/// One game at a time may stop a store-client launch still waiting for its client (login …):
/// the launch's own helper ends too, so it can't start that game later; a "client" it started in
/// a process group of its own stays.
#[test]
fn stopping_a_client_launch_that_waits_ends_its_helper_only() {
    let dir = tempfile::tempdir().unwrap();
    let game_dir = dir.path().join("Game");
    std::fs::create_dir_all(&game_dir).unwrap();
    let pidfile = dir.path().join("client.pid");
    let mut l = launcher(dir.path());
    // The "helper": starts a "client" in its own process group, then waits (never reaches the game).
    let helper_file = dir.path().join("helper.pid");
    let script = format!(
        "setsid sleep 30 & echo $! > '{}'; echo $$ > '{}'; exec sleep 30",
        pidfile.display(),
        helper_file.display()
    );
    let plan = LaunchPlan { install_dir: Some(game_dir), via_client: true, ..Default::default() };
    l.launch_planned(80, "Waiting", &sh(&script, ResolvedKind::Process), &plan).unwrap();
    let t = Instant::now();
    while !helper_file.exists() && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(30));
    }
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(l.started(), None);
    l.stop_game(80);
    let (evs, took) = wait_exit(&mut l, 80, Duration::from_secs(5));
    assert!(took < Duration::from_secs(3), "{took:?} {evs:?}");
    let client: i32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
    let alive = |pid: i32| {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|s| track::parse_stat(pid, &s))
            .is_some_and(|p| !p.zombie)
    };
    let client_alive = alive(client);
    unsafe { libc::kill(client, libc::SIGKILL) };
    assert!(client_alive, "the client was stopped with the helper");
    // The helper (our child, `sleep 30` via exec) is gone: its waiter thread reaped it.
    let helper: i32 = std::fs::read_to_string(&helper_file).unwrap().trim().parse().unwrap();
    let t = Instant::now();
    let helper_gone = loop {
        let gone = !alive(helper);
        if gone || t.elapsed() > Duration::from_secs(3) {
            break gone;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(helper_gone, "the waiting helper still runs");
}

#[test]
fn missing_game_under_a_wrapper_is_caught() {
    // The wrapper exists, the game binary inside it doesn't: caught before spawning.
    let r =
        finish("env".into(), sv(&["A=1", "/nonexistent/lodestar-game"]), vec![], None, ResolvedKind::Process, vec![]);
    let e = check_runnable(&r).unwrap_err().to_string();
    assert!(e.contains("program not found: /nonexistent/lodestar-game"), "{e}");
    // A present game under the same wrapper is fine, relative to the working directory too.
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("run.sh");
    write_exe(&script, "#!/bin/sh\n");
    let mut r = finish("env".into(), sv(&["A=1", "./run.sh"]), vec![], None, ResolvedKind::Process, vec![]);
    assert!(check_runnable(&r).is_err());
    r.cwd = Some(dir.path().to_path_buf());
    assert_eq!(check_runnable(&r).unwrap(), "env");
    // `sh -c '…'`: the script isn't a program.
    assert!(check_runnable(&sh("exit 0", ResolvedKind::Process)).is_ok());
    // Steam / URL launches aren't looked into.
    let r =
        finish("env".into(), sv(&["A=1", "/nonexistent/x"]), vec![], None, ResolvedKind::Steam { appid: 1 }, vec![]);
    assert!(check_runnable(&r).is_ok());
    // A composed launch with a wrapper toggle names the game in its error.
    let mut g = game(Some(cmd("/nonexistent/lodestar-run", &[])), None);
    g.edits.game_options.keep_awake = on();
    let r = rf(&g, &opt_env());
    assert_eq!(r.program, "systemd-inhibit");
    assert!(check_runnable(&r).unwrap_err().to_string().contains("not found"));
}

#[test]
fn core_under_env_path_is_looked_up_in_that_path() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("lodestar-test-game");
    write_exe(&game, "#!/bin/sh\n");
    let path = format!("PATH={}", dir.path().display());
    let r = finish("env".into(), vec![path, "lodestar-test-game".into()], vec![], None, ResolvedKind::Process, vec![]);
    assert_eq!(check_runnable(&r).unwrap(), "env");
    let r = finish("env".into(), sv(&["A=1", "lodestar-test-game"]), vec![], None, ResolvedKind::Process, vec![]);
    assert!(check_runnable(&r).unwrap_err().to_string().contains("program not found: lodestar-test-game"));
}

// ----- emulated games (core hooks; the emulation registry builds the real commands) -----

fn emu_game(launch: Option<LaunchSpec>) -> Game {
    let mut g = game(launch, None);
    g.source = Source::Emulator;
    g.platform = Some("Sony PlayStation 2".into());
    g.emu = Some(EmuRef {
        platform: "sony_playstation2".into(),
        folder: "f1".into(),
        rom: "/roms/ps2/Game (USA).iso".into(),
        ..Default::default()
    });
    g
}

#[test]
fn emulated_games_use_their_scanned_launch_without_a_registry() {
    let g = emu_game(Some(cmd("flatpak", &["run", "net.pcsx2.PCSX2", "-batch", "--", "/roms/ps2/Game (USA).iso"])));
    let x = r(&g);
    assert_eq!(x.program, "flatpak");
    assert_eq!(
        command_template(&g).as_deref(),
        Some("flatpak run net.pcsx2.PCSX2 -batch -- '/roms/ps2/Game (USA).iso'")
    );
    // Nothing to start: the reason says what to do, in plain words.
    let e = resolve_with(&emu_game(None), &host(SteamCmd::Native)).unwrap_err().to_string();
    assert!(e.contains("PlayStation 2") && e.contains("Settings → Emulation"), "{e}");
    // A custom command still works.
    let mut g = emu_game(None);
    g.launch_override = Some(LaunchOverride { command: Some("/bin/emu '/x.iso'".into()), ..Default::default() });
    assert_eq!(r(&g).program, "/bin/emu");
}

#[test]
fn flatpak_arguments_reach_the_app_unless_it_is_a_launcher() {
    // An emulator (or any Flatpak game): extra arguments reach it, no note.
    let g = game(
        Some(cmd("flatpak", &["run", "--filesystem=/mnt/g", "org.DolphinEmu.dolphin-emu", "-e", "/g/x.rvz"])),
        opts("-b"),
    );
    let x = r(&g);
    assert_eq!(x.args.last().map(String::as_str), Some("-b"));
    assert!(x.notes.iter().all(|n| !n.contains("Extra arguments")), "{:?}", x.notes);
    // A launcher app: they go to the launcher.
    let g = game(Some(cmd("flatpak", &["run", "net.lutris.Lutris", "lutris:rungameid/3"])), opts("-x"));
    assert!(r(&g).notes.iter().any(|n| n.contains("Extra arguments go to Lutris")), "{:?}", r(&g).notes);
}

#[test]
fn actions_know_the_rom() {
    let mut g = emu_game(None);
    g.edits.actions = vec![GameAction {
        play: true,
        ..action(ActionKind::Command, "/bin/other-emu", "--system={Platform} --dir '{RomDir}' '{Rom}'")
    }];
    let x = r(&g);
    assert_eq!(x.program, "/bin/other-emu");
    assert_eq!(x.args, sv(&["--system=sony_playstation2", "--dir", "/roms/ps2", "/roms/ps2/Game (USA).iso"]));
    // Not an emulated game: left empty, noted.
    let mut g = game(None, None);
    g.edits.actions = vec![GameAction { play: true, ..action(ActionKind::Command, "/bin/x", "'{Rom}'") }];
    assert!(r(&g).notes.iter().any(|n| n.contains("{Rom}")));
}
