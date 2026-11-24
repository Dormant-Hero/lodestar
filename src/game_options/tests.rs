//! Spec §7 cases (docs/game-options-spec.md) against a hand-built `Caps` / `LaunchEnv`
//! (never `caps::probe`). Resolve integration (Steam, URL, actions, display) is in
//! launch/tests.rs.
use super::compose::{self, ComposeInput, ComposeOutput, Core, LaunchKind};
use super::*;
use crate::test_support::{ev, sv};
use std::path::Path;

fn set(v: &[&str]) -> HashSet<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// GE-Proton 11: every fork variable, Wayland, fshack FSR, NTSync with its off switch.
pub(crate) fn ge_caps() -> RunnerCaps {
    RunnerCaps {
        family: RunnerFamily::Ge,
        version: "GE-Proton11-1".into(),
        vars: set(&[
            "PROTON_ENABLE_WAYLAND",
            "PROTON_ENABLE_HDR",
            "PROTON_FORCE_HDR",
            "PROTON_FSR4_UPGRADE",
            "PROTON_DLSS_UPGRADE",
            "PROTON_XESS_UPGRADE",
            "PROTON_PREFER_SDL",
            "PROTON_LOG",
            "PROTON_DISABLE_NVAPI",
            "PROTON_FORCE_NVAPI",
        ]),
        ntsync: true,
        wineserver_no_ntsync: true,
        winewayland: true,
        fshack_fsr: true,
        contrib_fsr4: false,
    }
}

/// The spec §7 fixture: every helper installed, gamescope 3.17 with every flag, a 2560×1440
/// monitor, one RDNA4 GPU, NTSync, a Wayland session and three runners.
pub(crate) fn fixture() -> LaunchEnv {
    let bin = |n: &str| Some(PathBuf::from(format!("/usr/bin/{n}")));
    let flags = set(&[
        "-W",
        "-H",
        "-w",
        "-h",
        "-f",
        "-b",
        "-r",
        "-F",
        "-S",
        "--sharpness",
        "--adaptive-sync",
        "--hdr-enabled",
        "--mangoapp",
        "--prefer-vk-device",
        "--expose-wayland",
        "--force-grab-cursor",
    ]);
    let valve = RunnerCaps {
        family: RunnerFamily::Valve,
        version: "UMU-Proton-10.0".into(),
        vars: set(&["PROTON_LOG", "PROTON_DISABLE_NVAPI"]),
        ntsync: true,
        wineserver_no_ntsync: true,
        ..Default::default()
    };
    let cachy = RunnerCaps {
        family: RunnerFamily::Cachyos,
        version: "proton-cachyos-10.0".into(),
        vars: set(&["PROTON_ENABLE_WAYLAND", "PROTON_FSR4_UPGRADE", "PROTON_LOG"]),
        ntsync: true,
        wineserver_no_ntsync: true,
        winewayland: true,
        fshack_fsr: true,
        ..Default::default()
    };
    let mut runners = HashMap::new();
    runners.insert(PathBuf::from("/r/ge"), Arc::new(ge_caps()));
    runners.insert(PathBuf::from("/r/valve"), Arc::new(valve));
    runners.insert(PathBuf::from("/r/cachy"), Arc::new(cachy));
    let caps = Caps {
        probed: true,
        bins: Bins {
            gamescope: bin("gamescope"),
            mangohud: bin("mangohud"),
            mangoapp: bin("mangoapp"),
            gamemoderun: bin("gamemoderun"),
            game_performance: bin("game-performance"),
            powerprofilesctl: bin("powerprofilesctl"),
            tlpctl: None,
            obs_gamecapture: bin("obs-gamecapture"),
            systemd_inhibit: bin("systemd-inhibit"),
        },
        gamescope: Some(GamescopeCaps {
            flags,
            filters: sv(&["linear", "nearest", "fsr", "nis", "pixel", "sgsr"]),
            version: Some((3, 17, 0)),
            options: vec![],
        }),
        gpus: vec![Gpu {
            card: "card1".into(),
            pci: "0000:03:00.0".into(),
            vendor: 0x1002,
            device: 0x7550,
            driver: "amdgpu".into(),
            boot_vga: true,
            vram: Some(16 << 30),
            gc_ip: Some((12, 0)),
            discrete_tag: false,
        }],
        ntsync: NtsyncState::Usable,
        wayland_session: true,
        flatpak_ext: [("org.freedesktop.Platform.VulkanLayer.MangoHud".to_string(), sv(&["24.08"]))]
            .into_iter()
            .collect(),
        runners,
        ..Default::default()
    };
    LaunchEnv {
        defaults: GameOptions::default(),
        by_source: Default::default(),
        caps: Arc::new(caps),
        monitor: Some((2560, 1440)),
        log_dir: None,
    }
}

/// One launch to compose: a command (+ env) and the user's options.
#[derive(Clone)]
struct Case {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    custom: bool,
    user_env: Vec<(String, String)>,
    prefix: Vec<String>,
    suffix: Vec<String>,
    kind: LaunchKind,
    play: bool,
    /// The runner lookup for dirs the fixture doesn't list (None = Unknown runner).
    probe: Option<fn(&Path) -> Arc<RunnerCaps>>,
    /// The plain-Wine lookup (None = an unchecked wine).
    wine: Option<compose::WineProbe>,
}

/// `umu-run /g/game.exe` with GE-Proton.
fn umu() -> Case {
    umu_with("/r/ge")
}

fn umu_with(protonpath: &str) -> Case {
    Case {
        env: ev(&[("WINEPREFIX", "/p"), ("GAMEID", "umu-default"), ("PROTONPATH", protonpath)]),
        ..cmd("umu-run", &["/g/game.exe"])
    }
}

fn cmd(program: &str, args: &[&str]) -> Case {
    Case {
        program: program.into(),
        args: sv(args),
        env: vec![],
        custom: false,
        user_env: vec![],
        prefix: vec![],
        suffix: vec![],
        kind: LaunchKind::Process,
        play: true,
        probe: None,
        wine: None,
    }
}

impl Case {
    /// Steam-style launch options (`VAR=1 wrapper %command% -arg`), split on spaces.
    fn opts(mut self, o: &str) -> Case {
        let words: Vec<&str> = o.split_whitespace().collect();
        let n = words.iter().take_while(|w| w.contains('=') && !w.starts_with('-')).count();
        self.user_env =
            words[..n].iter().map(|w| w.split_once('=').unwrap()).map(|(k, v)| (k.into(), v.into())).collect();
        let rest = &words[n..];
        match rest.iter().position(|w| *w == "%command%") {
            Some(p) => {
                self.prefix = sv(&rest[..p]);
                self.suffix = sv(&rest[p + 1..]);
            }
            None => self.suffix = sv(rest),
        }
        self
    }
    fn run(&self, g: &GameOptions) -> ComposeOutput {
        self.run_in(g, &fixture())
    }
    fn run_in(&self, g: &GameOptions, env: &LaunchEnv) -> ComposeOutput {
        let opts = effective(&env.defaults, g);
        compose::apply(ComposeInput {
            program: self.program.clone(),
            args: self.args.clone(),
            base_env: self.env.clone(),
            custom_command: self.custom,
            user_env: self.user_env.clone(),
            user_prefix: self.prefix.clone(),
            user_suffix: self.suffix.clone(),
            kind: self.kind,
            play: self.play,
            name: "Test",
            game_id: 7,
            url: None,
            self_exe: Some("/usr/bin/lodestar"),
            probe_runner: self.probe,
            probe_wine: self.wine,
            opts: &opts,
            env,
        })
    }
}

fn get<'a>(out: &'a ComposeOutput, k: &str) -> Option<&'a str> {
    out.env.iter().find(|(ek, _)| ek == k).map(|(_, v)| v.as_str())
}

fn state(out: &ComposeOutput, id: OptionId) -> Option<&OptionState> {
    out.report.iter().find(|(i, n, _)| *i == id && n.is_none()).map(|(_, _, s)| s)
}

fn is_suppressed(out: &ComposeOutput, id: OptionId) -> bool {
    matches!(state(out, id), Some(OptionState::Suppressed(_)))
}

fn is_unavailable(out: &ComposeOutput, id: OptionId) -> bool {
    matches!(state(out, id), Some(OptionState::Unavailable(_)))
}

fn has_note(out: &ComposeOutput, s: &str) -> bool {
    out.notes.iter().any(|n| n.contains(s))
}

fn on() -> Tri {
    Tri::On
}

/// Count of `w` in program + args.
fn count(out: &ComposeOutput, w: &str) -> usize {
    std::iter::once(&out.program).chain(out.args.iter()).filter(|a| *a == w).count()
}

/// The test-2 options.
fn full_stack() -> GameOptions {
    GameOptions {
        gamescope: on(),
        mangohud: on(),
        hdr: on(),
        gamemode: on(),
        performance_profile: on(),
        keep_awake: on(),
        ..Default::default()
    }
}

// ------------------------------------------------------------------------------------ cases

#[test]
fn defaults_change_nothing() {
    let c = umu().opts("A=1 %command% -x");
    let out = c.run_in(&GameOptions::default(), &LaunchEnv::none());
    assert_eq!(out.program, "umu-run");
    assert_eq!(out.args, sv(&["/g/game.exe", "-x"]));
    assert_eq!(out.env, ev(&[("WINEPREFIX", "/p"), ("GAMEID", "umu-default"), ("PROTONPATH", "/r/ge"), ("A", "1")]));
    assert!(out.notes.is_empty(), "{:?}", out.notes);
    assert!(out.report.iter().all(|(_, _, s)| matches!(s, OptionState::Unavailable(_))), "{:?}", out.report);
    // Wrappers typed by the user are kept as they are.
    let out = cmd("/opt/g/run", &[]).opts("mangohud gamemoderun %command%").run(&GameOptions::default());
    assert_eq!((out.program.as_str(), out.args.clone()), ("mangohud", sv(&["gamemoderun", "/opt/g/run"])));
}

#[test]
fn native_mangohud_gamemode() {
    let g = GameOptions { mangohud: on(), gamemode: on(), ..Default::default() };
    let out = cmd("/opt/g/run", &["-x"]).run(&g);
    assert_eq!(out.program, "gamemoderun");
    assert_eq!(out.args, sv(&["mangohud", "/opt/g/run", "-x"]));
    assert_eq!(out.env, vec![]);
    assert_eq!(out.core, Core::Native);
}

#[test]
fn umu_full_stack_order() {
    let out = umu().run(&full_stack());
    assert_eq!(out.program, "systemd-inhibit");
    assert_eq!(
        out.args,
        sv(&[
            "--what=idle:sleep",
            "--who=Lodestar",
            "--why=Playing Test",
            "game-performance",
            "gamescope",
            "-W",
            "2560",
            "-H",
            "1440",
            "-w",
            "2560",
            "-h",
            "1440",
            "-f",
            "--hdr-enabled",
            "--mangoapp",
            "--",
            "gamemoderun",
            "umu-run",
            "/g/game.exe",
        ])
    );
    let mut want = umu().env;
    want.extend(ev(&[("DXVK_HDR", "1"), ("GAME_PERFORMANCE_SCREENSAVER_ON", "1")]));
    assert_eq!(out.env, want);
    assert!(get(&out, "PROTON_ENABLE_WAYLAND").is_none() && get(&out, "MANGOHUD").is_none());
    assert_eq!(state(&out, OptionId::Gamescope), Some(&OptionState::Applied));
}

#[test]
fn user_env_wins() {
    let g = GameOptions { hdr: on(), ..Default::default() };
    let out = umu().opts("DXVK_HDR=0 %command%").run(&g);
    assert_eq!(get(&out, "DXVK_HDR"), Some("0"));
    assert!(get(&out, "PROTON_ENABLE_HDR").is_none() && get(&out, "ENABLE_HDR_WSI").is_none(), "{:?}", out.env);
    assert_eq!(out.env.iter().filter(|(k, _)| k == "DXVK_HDR").count(), 1);
}

#[test]
fn user_gamescope_merged() {
    let g = GameOptions { mangohud: on(), ..Default::default() };
    let out = umu().opts("gamescope -f -W 1920 -H 1080 -- %command%").run(&g);
    assert_eq!(out.program, "gamescope");
    assert_eq!(out.args, sv(&["--mangoapp", "-f", "-W", "1920", "-H", "1080", "--", "umu-run", "/g/game.exe"]));
}

#[test]
fn toggle_and_user_gamescope_single() {
    let g = GameOptions { gamescope: on(), ..Default::default() };
    let out = umu().opts("gamescope -f -W 1920 -H 1080 -- %command%").run(&g);
    assert_eq!(count(&out, "gamescope"), 1);
    assert_eq!(out.program, "gamescope");
    let ours = out.args.iter().position(|a| a == "2560").unwrap();
    let theirs = out.args.iter().position(|a| a == "1920").unwrap();
    assert!(ours < theirs, "{:?}", out.args);
    assert_eq!(count(&out, "--"), 1);
}

#[test]
fn steam_kind_gets_notes_only() {
    let g = GameOptions { mangohud: on(), gamemode: on(), ..Default::default() };
    let c = Case { kind: LaunchKind::Steam, ..cmd("steam", &["steam://rungameid/570"]) };
    let out = c.run(&g);
    assert_eq!((out.program.as_str(), out.args.clone()), ("steam", sv(&["steam://rungameid/570"])));
    assert!(out.env.is_empty());
    assert_eq!(out.core, Core::Steam);
    assert_eq!(out.notes.len(), 1, "{:?}", out.notes);
    assert!(has_note(&out, "Steam client") && has_note(&out, "MangoHud") && has_note(&out, "GameMode"));
    let s = out.steam_string.clone().unwrap();
    assert!(s.ends_with("%command%"), "{s}");
    assert!(is_suppressed(&out, OptionId::Mangohud));
}

#[test]
fn url_note() {
    let g = GameOptions { mangohud: on(), ..Default::default() };
    let c = Case { kind: LaunchKind::Url, ..cmd("xdg-open", &["heroic://launch/legendary/x"]) };
    let opts = effective(&GameOptions::default(), &g);
    let env = fixture();
    let out = compose::apply(ComposeInput {
        program: c.program.clone(),
        args: c.args.clone(),
        base_env: vec![],
        custom_command: false,
        user_env: vec![],
        user_prefix: vec![],
        user_suffix: vec![],
        kind: LaunchKind::Url,
        play: true,
        name: "Test",
        game_id: 7,
        url: Some("heroic://launch/legendary/x"),
        self_exe: None,
        probe_runner: None,
        probe_wine: None,
        opts: &opts,
        env: &env,
    });
    assert_eq!(out.core, Core::Url);
    assert!(out.env.is_empty());
    assert!(has_note(&out, "Heroic"), "{:?}", out.notes);
}

#[test]
fn flatpak_env_and_drops() {
    let g = GameOptions { mangohud: on(), gamemode: on(), frame_limit: Some(60), ..Default::default() };
    let out = cmd("flatpak", &["run", "--branch=stable", "com.ex.Game"]).run(&g);
    assert_eq!(out.program, "flatpak");
    assert_eq!(
        out.args,
        sv(&[
            "run",
            "--env=MANGOHUD=1",
            "--env=MANGOHUD_CONFIG=read_cfg,fps_limit=60",
            "--branch=stable",
            "com.ex.Game"
        ])
    );
    assert!(out.notes.iter().any(|n| n.contains("GameMode") && n.contains("LD_PRELOAD")), "{:?}", out.notes);
    assert_eq!(out.core, Core::Flatpak);
    // No extension: MangoHud is unavailable.
    let mut e = fixture();
    Arc::make_mut(&mut e.caps).flatpak_ext.clear();
    let out = cmd("flatpak", &["run", "com.ex.Game"]).run_in(&GameOptions { mangohud: on(), ..Default::default() }, &e);
    assert!(is_unavailable(&out, OptionId::Mangohud));
    assert_eq!(out.args, sv(&["run", "com.ex.Game"]));
}

#[test]
fn flatpak_gamescope_nosocket() {
    let g = GameOptions { gamescope: on(), ..Default::default() };
    let out = cmd("flatpak", &["run", "com.ex.Game"]).run(&g);
    assert_eq!(out.program, "gamescope");
    let dd = out.args.iter().position(|a| a == "--").unwrap();
    assert_eq!(out.args[dd + 1..], sv(&["flatpak", "run", "--nosocket=wayland", "com.ex.Game"]));
    // With Expose Wayland the socket stays.
    let g = GameOptions { gamescope: on(), gs_expose_wayland: on(), ..Default::default() };
    let out = cmd("flatpak", &["run", "com.ex.Game"]).run(&g);
    assert!(!out.args.iter().any(|a| a == "--nosocket=wayland"));
}

#[test]
fn flatpak_launcher_ids_are_launchers() {
    let g = GameOptions { gamemode: on(), ..Default::default() };
    let out = cmd("flatpak", &["run", "net.lutris.Lutris", "lutris:rungameid/3"]).run(&g);
    assert_eq!(out.core, Core::Launcher);
    let out = cmd("flatpak", &["run", "--command=bottles-cli", "com.usebottles.bottles", "run", "-b", "x"]).run(&g);
    assert_eq!(out.core, Core::Launcher);
}

#[test]
fn client_helper_env_only() {
    let args = ["--client-launch", "battlenet", "--prefix", "/p", "--exe", "/p/b.exe", "--runner", "proton:/r/ge"];
    let g = GameOptions { gamescope: on(), mangohud: on(), ..Default::default() };
    let out = cmd("/usr/bin/lodestar", &args).run(&g);
    assert_eq!(out.core, Core::Client);
    assert_eq!(out.program, "/usr/bin/lodestar");
    assert_eq!(out.args, sv(&args));
    assert_eq!(get(&out, "MANGOHUD"), Some("1"));
    assert!(has_note(&out, "Game options apply only when Lodestar starts Battle.net itself"), "{:?}", out.notes);
    assert!(has_note(&out, "Gamescope"));
    assert!(is_suppressed(&out, OptionId::Gamescope));
}

#[test]
fn lutris_launcher_note() {
    let g = GameOptions { gamemode: on(), ..Default::default() };
    let out = cmd("lutris", &["lutris:rungameid/3"]).run(&g);
    assert_eq!((out.program.as_str(), out.args.clone()), ("lutris", sv(&["lutris:rungameid/3"])));
    assert_eq!(out.core, Core::Launcher);
    assert!(has_note(&out, "already open"), "{:?}", out.notes);
    assert!(has_note(&out, "GameMode aren't added"), "{:?}", out.notes);
}

#[test]
fn wayland_suppressed_in_gamescope() {
    let g = GameOptions { wayland: on(), gamescope: on(), ..Default::default() };
    let out = umu().run(&g);
    assert!(get(&out, "PROTON_ENABLE_WAYLAND").is_none());
    assert!(is_suppressed(&out, OptionId::Wayland));
    assert!(has_note(&out, "ignored inside Gamescope"));
    let g = GameOptions { gs_expose_wayland: on(), ..g };
    let out = umu().run(&g);
    assert!(out.args.iter().any(|a| a == "--expose-wayland"), "{:?}", out.args);
    assert_eq!(get(&out, "PROTON_ENABLE_WAYLAND"), Some("1"));
}

#[test]
fn native_wayland_is_per_game_and_scoped() {
    let env = LaunchEnv { defaults: GameOptions { wayland: on(), ..Default::default() }, ..fixture() };
    let out = cmd("/opt/g/run", &[]).run_in(&GameOptions::default(), &env);
    assert!(get(&out, "SDL_VIDEODRIVER").is_none());
    assert!(is_suppressed(&out, OptionId::Wayland));
    let out = cmd("/opt/g/run", &[]).run_in(&GameOptions { wayland: on(), ..Default::default() }, &env);
    assert_eq!(get(&out, "SDL_VIDEODRIVER"), Some("wayland,x11"));
    assert_eq!(get(&out, "SDL_VIDEO_DRIVER"), Some("wayland,x11"));
    // Inside gamescope with Expose Wayland: behind the `--`.
    let g = GameOptions { wayland: on(), gamescope: on(), gs_expose_wayland: on(), ..Default::default() };
    let out = cmd("/opt/g/run", &[]).run_in(&g, &env);
    assert!(get(&out, "SDL_VIDEODRIVER").is_none());
    let dd = out.args.iter().position(|a| a == "--").unwrap();
    assert_eq!(out.args[dd + 1..dd + 4], sv(&["env", "SDL_VIDEODRIVER=wayland,x11", "SDL_VIDEO_DRIVER=wayland,x11"]));
}

#[test]
fn hdr_implies_wayland_ge() {
    let g = GameOptions { hdr: on(), ..Default::default() };
    let out = umu().run(&g);
    assert_eq!(get(&out, "PROTON_ENABLE_WAYLAND"), Some("1"));
    assert_eq!(get(&out, "DXVK_HDR"), Some("1"));
    assert!(matches!(state(&out, OptionId::Wayland), Some(OptionState::AppliedImplicitly(_))), "{:?}", out.report);
    // Wayland Off wins over the implication.
    let out = umu().run(&GameOptions { hdr: on(), wayland: Tri::Off, ..Default::default() });
    assert_eq!(get(&out, "PROTON_ENABLE_WAYLAND"), Some("0"));
    assert!(has_note(&out, "neither is active"));
}

#[test]
fn hdr_valve_needs_gamescope() {
    let g = GameOptions { hdr: on(), ..Default::default() };
    let out = umu_with("/r/valve").run(&g);
    assert_eq!(get(&out, "DXVK_HDR"), Some("1"));
    assert!(get(&out, "PROTON_ENABLE_WAYLAND").is_none());
    assert!(has_note(&out, "Valve Proton has no Wayland driver: turn on Gamescope for HDR."), "{:?}", out.notes);
    assert!(is_unavailable(&out, OptionId::Wayland));
}

#[test]
fn hdr_off_ge() {
    let out = umu().run(&GameOptions { hdr: Tri::Off, ..Default::default() });
    assert_eq!(get(&out, "DXVK_HDR"), Some("0"));
    assert_eq!(get(&out, "DXVK_NO_HDR"), Some("1"));
    assert_eq!(get(&out, "PROTON_ENABLE_HDR"), Some("0"));
}

#[test]
fn hdr_force_needs_wayland() {
    let g = GameOptions { hdr: on(), hdr_force: on(), ..Default::default() };
    assert_eq!(get(&umu().run(&g), "PROTON_FORCE_HDR"), Some("1"));
    let out = umu().run(&GameOptions { gamescope: on(), ..g });
    assert!(get(&out, "PROTON_FORCE_HDR").is_none());
    assert!(is_suppressed(&out, OptionId::HdrForce));
}

#[test]
fn hdr_wsi_layer_auto_on_old_nvidia() {
    let mut env = fixture();
    {
        let c = Arc::make_mut(&mut env.caps);
        c.nvidia_version = Some("580.95".into());
        c.layers.push(LayerManifest { name: "VK_LAYER_hdr_wsi".into(), ..Default::default() });
    }
    let g = GameOptions { hdr: on(), ..Default::default() };
    let out = umu().run_in(&g, &env);
    assert_eq!(get(&out, "ENABLE_HDR_WSI"), Some("1"));
    assert!(matches!(state(&out, OptionId::HdrWsiLayer), Some(OptionState::AppliedImplicitly(_))));
    let out = umu().run_in(&GameOptions { gamescope: on(), ..g }, &env);
    assert!(get(&out, "ENABLE_HDR_WSI").is_none());
    // Not on a current driver, and never without HDR.
    Arc::make_mut(&mut env.caps).nvidia_version = Some("595.10".into());
    assert!(get(&umu().run_in(&GameOptions { hdr: on(), ..Default::default() }, &env), "ENABLE_HDR_WSI").is_none());
}

#[test]
fn cachy_fsr4_off_and_audio() {
    let g = GameOptions { fsr4_upgrade: Tri::Off, audio_latency: on(), ..Default::default() };
    let out = umu_with("/r/cachy").run(&g);
    assert_eq!(get(&out, "PROTON_FSR4_UPGRADE"), Some("0"));
    assert!(get(&out, "PULSE_LATENCY_MSEC").is_none());
    assert!(is_suppressed(&out, OptionId::AudioLatency));
    // Elsewhere the fix applies.
    assert_eq!(get(&umu().run(&g), "PULSE_LATENCY_MSEC"), Some("60"));
}

#[test]
fn fsr4_needs_rdna3() {
    let mut env = fixture();
    Arc::make_mut(&mut env.caps).gpus[0].gc_ip = Some((10, 3));
    let out = umu().run_in(&GameOptions { fsr4_upgrade: on(), ..Default::default() }, &env);
    assert!(is_unavailable(&out, OptionId::Fsr4Upgrade));
    assert!(get(&out, "PROTON_FSR4_UPGRADE").is_none());
    assert_eq!(
        get(&umu().run(&GameOptions { fsr4_upgrade: on(), ..Default::default() }), "PROTON_FSR4_UPGRADE"),
        Some("1")
    );
}

#[test]
fn wine_fsr_vs_wayland() {
    let g = GameOptions { wine_fsr: on(), wayland: on(), ..Default::default() };
    let out = umu().run(&g);
    assert!(get(&out, "WINE_FULLSCREEN_FSR").is_none());
    assert!(is_suppressed(&out, OptionId::WineFsr));
    let out = umu().run(&GameOptions { wine_fsr: on(), wine_fsr_strength: Some(3), ..Default::default() });
    assert_eq!(get(&out, "WINE_FULLSCREEN_FSR"), Some("1"));
    assert_eq!(get(&out, "WINE_FULLSCREEN_FSR_STRENGTH"), Some("3"));
}

#[test]
fn ntsync_off() {
    let g = GameOptions { ntsync: Tri::Off, ..Default::default() };
    assert_eq!(get(&umu().run(&g), "PROTON_NO_NTSYNC"), Some("1"));
    let out = cmd("wine", &["x.exe"]).run(&g);
    assert!(is_unavailable(&out, OptionId::Ntsync));
    assert!(out.env.is_empty(), "{:?}", out.env);
    // Old GE (only PROTON_USE_NTSYNC) with On.
    let mut env = fixture();
    let old = RunnerCaps {
        family: RunnerFamily::Ge,
        version: "GE-Proton10-9".into(),
        vars: set(&["PROTON_USE_NTSYNC"]),
        ..Default::default()
    };
    Arc::make_mut(&mut env.caps).runners.insert(PathBuf::from("/r/ge10"), Arc::new(old));
    let out = umu_with("/r/ge10").run_in(&GameOptions { ntsync: on(), ..Default::default() }, &env);
    assert_eq!(get(&out, "PROTON_USE_NTSYNC"), Some("1"));
    // Wine inside a Proton dir: PROTON_NO_NTSYNC still applies.
    let out = cmd("/r/ge/files/bin/wine", &["x.exe"]).run(&g);
    assert_eq!(out.core, Core::Wine);
    assert_eq!(get(&out, "PROTON_NO_NTSYNC"), Some("1"));
}

#[test]
fn frame_limit_backends() {
    let fl = GameOptions { frame_limit: Some(60), ..Default::default() };
    // Gamescope: -r.
    let out = umu().run(&GameOptions { gamescope: on(), ..fl.clone() });
    let r = out.args.iter().position(|a| a == "-r").unwrap();
    assert_eq!(out.args[r + 1], "60");
    assert!(get(&out, "DXVK_CONFIG").is_none());
    // Proton without MangoHud: DXVK / VKD3D.
    let out = umu().run(&fl);
    assert_eq!(get(&out, "DXVK_CONFIG"), Some("dxvk.maxFrameRate=60;dxgi.maxFrameRate=60;d3d9.maxFrameRate=60"));
    assert_eq!(get(&out, "VKD3D_FRAME_RATE"), Some("60"));
    // Native: the mangohud wrapper as a limiter only.
    let out = cmd("/opt/g/run", &[]).run(&fl);
    assert_eq!((out.program.as_str(), out.args.clone()), ("mangohud", sv(&["/opt/g/run"])));
    assert_eq!(get(&out, "MANGOHUD_CONFIG"), Some("no_display,fps_limit=60"));
    // Merged into the user's list.
    let out = umu().opts("DXVK_CONFIG=dxgi.syncInterval=0 %command%").run(&fl);
    assert_eq!(
        get(&out, "DXVK_CONFIG"),
        Some("dxgi.syncInterval=0;dxvk.maxFrameRate=60;dxgi.maxFrameRate=60;d3d9.maxFrameRate=60")
    );
    // A key the user already set is left alone.
    let out = umu().opts("DXVK_CONFIG=dxgi.maxFrameRate=30 %command%").run(&fl);
    assert_eq!(get(&out, "DXVK_CONFIG"), Some("dxgi.maxFrameRate=30;dxvk.maxFrameRate=60;d3d9.maxFrameRate=60"));
    // Per game 0 beats a global 60.
    let env = LaunchEnv { defaults: fl.clone(), ..fixture() };
    let out = umu().run_in(&GameOptions { frame_limit: Some(0), ..Default::default() }, &env);
    assert_eq!(out.env, umu().env);
    assert_eq!(out.args, sv(&["/g/game.exe"]));
    // Never both -r and a MangoHud limit.
    let out = umu().run(&GameOptions { gamescope: on(), mangohud: on(), ..fl });
    assert!(get(&out, "MANGOHUD_CONFIG").is_none());
}

#[test]
fn dedupe_base_wrapper() {
    let g = GameOptions { gamemode: on(), ..Default::default() };
    let out = cmd("gamemoderun", &["/opt/g/run"]).run(&g);
    assert_eq!(count(&out, "gamemoderun"), 1);
    assert_eq!(state(&out, OptionId::Gamemode), Some(&OptionState::Applied));
}

#[test]
fn custom_command_env_wins() {
    let g = GameOptions { hdr: on(), ..Default::default() };
    let c = Case { env: ev(&[("DXVK_HDR", "0")]), custom: true, ..cmd("umu-run", &["x.exe"]) };
    let out = c.run(&g);
    assert_eq!(get(&out, "DXVK_HDR"), Some("0"));
    // Without a custom command the toggle overwrites the source's env.
    let c = Case { env: ev(&[("PROTON_ENABLE_WAYLAND", "1"), ("PROTONPATH", "/r/ge")]), ..cmd("umu-run", &["x.exe"]) };
    let out = c.run(&GameOptions { wayland: Tri::Off, ..Default::default() });
    assert_eq!(get(&out, "PROTON_ENABLE_WAYLAND"), Some("0"));
}

#[test]
fn stripped_user_env_scoped() {
    let g = GameOptions { gamescope: on(), ..Default::default() };
    let out = umu().opts("ENABLE_VKBASALT=1 %command%").run(&g);
    let dd = out.args.iter().position(|a| a == "--").unwrap();
    assert_eq!(out.args[dd + 1..], sv(&["env", "ENABLE_VKBASALT=1", "umu-run", "/g/game.exe"]));
    assert!(get(&out, "ENABLE_VKBASALT").is_none());
}

#[test]
fn lsfg_scoped() {
    let mut env = fixture();
    Arc::make_mut(&mut env.caps).lsfg =
        Some(LsfgCaps { version: 2, profiles: vec![("Genshin".into(), sv(&["GenshinImpact.exe"]))], dll: None });
    let g = GameOptions {
        lsfg: on(),
        lsfg_profile: Some("Genshin".into()),
        gamescope: on(),
        gamemode: on(),
        ..Default::default()
    };
    let out = umu().run_in(&g, &env);
    let dd = out.args.iter().position(|a| a == "--").unwrap();
    assert_eq!(out.args[dd + 1..], sv(&["env", "LSFGVK_PROFILE=Genshin", "gamemoderun", "umu-run", "/g/game.exe"]));
    assert!(get(&out, "LSFGVK_PROFILE").is_none());
    let out = umu().run_in(&GameOptions { gamescope: Tri::Default, gamemode: Tri::Default, ..g }, &env);
    assert_eq!(get(&out, "LSFGVK_PROFILE"), Some("Genshin"));
    let out = umu().run_in(&GameOptions { lsfg: Tri::Off, ..Default::default() }, &env);
    assert_eq!(get(&out, "DISABLE_LSFGVK"), Some("1"));
    // Lodestar's settings.
    let g = GameOptions { lsfg: on(), lsfg_multiplier: Some(3), lsfg_performance: on(), ..Default::default() };
    let out = umu().run_in(&g, &env);
    assert_eq!(get(&out, "LSFGVK_ENV"), Some("1"));
    assert_eq!(get(&out, "LSFGVK_MULTIPLIER"), Some("3"));
    assert_eq!(get(&out, "LSFGVK_PERFORMANCE_MODE"), Some("1"));
    assert!(has_note(&out, "Lossless.dll"));
    // Not installed.
    assert!(is_unavailable(&umu().run(&g), OptionId::Lsfg));
}

#[test]
fn internal_res_percent() {
    let g = GameOptions { gamescope: on(), gs_internal: Some(InternalRes::Percent(67)), ..Default::default() };
    let out = umu().run(&g);
    let a = out.args.join(" ");
    assert!(a.contains("-w 1715 -h 965 -f -F fsr"), "{a}");
    let mut env = fixture();
    Arc::make_mut(&mut env.caps).gamescope.as_mut().unwrap().filters.retain(|f| f != "sgsr");
    let out = umu().run_in(&GameOptions { gs_filter: Some(GsFilter::Sgsr), ..g }, &env);
    assert!(out.args.join(" ").contains("-F fsr"), "{:?}", out.args);
    assert!(has_note(&out, "SGSR"));
}

#[test]
fn gamescope_flag_order_and_check() {
    let g = GameOptions {
        gamescope: on(),
        gs_window: Some(WindowMode::Borderless),
        gs_output: Some(Res::Custom { w: 1920, h: 1080 }),
        gs_filter: Some(GsFilter::Nis),
        gs_sharpness: Some(5),
        gs_scaler: Some(GsScaler::Integer),
        gs_adaptive_sync: on(),
        gs_grab_cursor: on(),
        gs_extra_args: Some("--rt --immediate-flips".into()),
        frame_limit: Some(90),
        ..Default::default()
    };
    let out = umu().opts("gamescope --steam -- %command%").run(&g);
    assert_eq!(
        out.args[..out.args.iter().position(|a| a == "--").unwrap()],
        sv(&[
            "-W",
            "1920",
            "-H",
            "1080",
            "-w",
            "1920",
            "-h",
            "1080",
            "-b",
            "-r",
            "90",
            "-F",
            "nis",
            "--sharpness",
            "5",
            "-S",
            "integer",
            "--adaptive-sync",
            "--force-grab-cursor",
            "--rt",
            "--immediate-flips",
            "--steam",
        ])
    );
    // A flag this gamescope doesn't know is dropped with a note.
    let mut env = fixture();
    Arc::make_mut(&mut env.caps).gamescope.as_mut().unwrap().flags.remove("--adaptive-sync");
    let out = umu().run_in(&g, &env);
    assert!(!out.args.iter().any(|a| a == "--adaptive-sync"));
    assert!(has_note(&out, "--adaptive-sync"));
}

#[test]
fn no_monitor() {
    let env = LaunchEnv { monitor: None, ..fixture() };
    let out = umu()
        .run_in(&GameOptions { gamescope: on(), gs_window: Some(WindowMode::Windowed), ..Default::default() }, &env);
    for f in ["-W", "-H", "-w", "-h"] {
        assert!(!out.args.iter().any(|a| a == f), "{:?}", out.args);
    }
    assert!(out.args.iter().any(|a| a == "-f"));
    assert!(has_note(&out, "1280×720"));
    // The sysfs preferred mode is the fallback.
    let mut env = env;
    Arc::make_mut(&mut env.caps).preferred_mode = Some((3840, 2160));
    let out = umu().run_in(&GameOptions { gamescope: on(), ..Default::default() }, &env);
    assert!(out.args.join(" ").contains("-W 3840 -H 2160"));
}

#[test]
fn nested_session() {
    let mut env = fixture();
    Arc::make_mut(&mut env.caps).nested_gamescope = true;
    let out = umu().run_in(&GameOptions { gamescope: on(), ..Default::default() }, &env);
    assert_eq!(count(&out, "gamescope"), 0);
    assert!(is_suppressed(&out, OptionId::Gamescope));
}

#[test]
fn missing_tool() {
    let mut env = fixture();
    Arc::make_mut(&mut env.caps).bins.gamescope = None;
    let out = umu().run_in(&GameOptions { gamescope: on(), ..Default::default() }, &env);
    assert_eq!(count(&out, "gamescope"), 0);
    assert_eq!((out.program.as_str(), out.args.clone()), ("umu-run", sv(&["/g/game.exe"])));
    assert!(is_unavailable(&out, OptionId::Gamescope));
}

#[test]
fn non_play_action() {
    let g = GameOptions { gamescope: on(), hdr: on(), gamemode: on(), ..Default::default() };
    let out = Case { play: false, ..umu() }.run(&g);
    assert_eq!(get(&out, "DXVK_HDR"), Some("1"));
    assert_eq!((out.program.as_str(), out.args.clone()), ("umu-run", sv(&["/g/game.exe"])));
    assert!(is_suppressed(&out, OptionId::Gamescope) && is_suppressed(&out, OptionId::Gamemode));
}

#[test]
fn prefer_dgpu() {
    let mut env = fixture();
    {
        let c = Arc::make_mut(&mut env.caps);
        c.gpus = vec![
            Gpu {
                card: "card0".into(),
                pci: "0000:06:00.0".into(),
                vendor: 0x1002,
                device: 0x15bf,
                driver: "amdgpu".into(),
                boot_vga: true,
                vram: Some(512 << 20),
                ..Default::default()
            },
            Gpu {
                card: "card1".into(),
                pci: "0000:03:00.0".into(),
                vendor: 0x1002,
                device: 0x7550,
                driver: "amdgpu".into(),
                vram: Some(16 << 30),
                gc_ip: Some((12, 0)),
                ..Default::default()
            },
        ];
    }
    let g = GameOptions { prefer_dgpu: on(), ..Default::default() };
    assert_eq!(get(&umu().run_in(&g, &env), "DRI_PRIME"), Some("pci-0000_03_00_0"));
    let out = umu().run_in(&GameOptions { gamescope: on(), ..g.clone() }, &env);
    let p = out.args.iter().position(|a| a == "--prefer-vk-device").unwrap();
    assert_eq!(out.args[p + 1], "1002:7550");
    // The dGPU is already boot_vga.
    {
        let c = Arc::make_mut(&mut env.caps);
        c.gpus[0].boot_vga = false;
        c.gpus[1].boot_vga = true;
    }
    let out = umu().run_in(&g, &env);
    assert!(is_unavailable(&out, OptionId::PreferDgpu));
    assert!(get(&out, "DRI_PRIME").is_none());
    // One GPU (the fixture).
    assert!(is_unavailable(&umu().run(&g), OptionId::PreferDgpu));
}

#[test]
fn proton_run_rewrite() {
    let g = GameOptions { gamescope: on(), ..Default::default() };
    let out = cmd("/r/ge/proton", &["run", "x.exe"]).run(&g);
    assert_eq!(out.core, Core::Proton);
    let dd = out.args.iter().position(|a| a == "--").unwrap();
    assert_eq!(out.args[dd + 1..], sv(&["/r/ge/proton", "waitforexitandrun", "x.exe"]));
    // Without gamescope `run` stays.
    assert_eq!(cmd("/r/ge/proton", &["run", "x.exe"]).run(&GameOptions::default()).args, sv(&["run", "x.exe"]));
}

#[test]
fn steam_string() {
    let c = Case { kind: LaunchKind::Steam, ..cmd("steam", &["-applaunch", "570"]) };
    let out = c.run(&full_stack());
    assert_eq!(
        out.steam_string.as_deref(),
        Some(
            "DXVK_HDR=1 GAME_PERFORMANCE_SCREENSAVER_ON=1 systemd-inhibit --what=idle:sleep --who=Lodestar '--why=Playing Test' \
             game-performance gamescope -W 2560 -H 1440 -w 2560 -h 1440 -f --hdr-enabled --mangoapp -- gamemoderun %command%"
        )
    );
    // Nothing set: no string.
    assert_eq!(c.run(&GameOptions::default()).steam_string, None);
}

#[test]
fn effective_merge() {
    let global = GameOptions { mangohud: on(), wayland: on(), frame_limit: Some(60), ..Default::default() };
    assert_eq!(effective(&global, &GameOptions::default()).mangohud, Tri::On);
    let e = effective(&global, &GameOptions { mangohud: Tri::Off, frame_limit: Some(0), ..Default::default() });
    assert_eq!((e.mangohud, e.frame_limit), (Tri::Off, None));
    let env = LaunchEnv { defaults: global, ..fixture() };
    let out = cmd("/opt/g/run", &[]).run_in(&GameOptions::default(), &env);
    assert!(get(&out, "SDL_VIDEODRIVER").is_none());
    assert_eq!(out.program, "mangohud");
}

#[test]
fn proton_dir_for_rules() {
    let e = fixture();
    let mut caps = (*e.caps).clone();
    for d in ["/c/GE-Proton10-9", "/c/GE-Proton10-26", "/c/UMU-Proton-9.0", "/c/UMU-Proton-10.0"] {
        caps.runners.insert(PathBuf::from(d), Arc::new(ge_caps()));
    }
    let pp = |v: &str| ev(&[("PROTONPATH", v)]);
    assert_eq!(compose::proton_dir_for(&pp("/r/ge/"), "umu-run", &caps), Some(PathBuf::from("/r/ge")));
    assert_eq!(compose::proton_dir_for(&pp("GE-Proton"), "umu-run", &caps), Some(PathBuf::from("/c/GE-Proton10-26")));
    assert_eq!(compose::proton_dir_for(&[], "umu-run", &caps), Some(PathBuf::from("/c/UMU-Proton-10.0")));
    assert_eq!(compose::proton_dir_for(&pp("Something"), "umu-run", &caps), None);
    assert_eq!(compose::proton_dir_for(&[], "/x/GE/proton", &caps), Some(PathBuf::from("/x/GE")));
    assert_eq!(compose::proton_dir_for(&[], "/opt/game", &caps), None);
}

#[test]
fn unknown_runner_greys_nothing_out() {
    let g = GameOptions { wayland: on(), sdl_input: on(), ..Default::default() };
    let out = umu_with("/somewhere/else").run(&g);
    assert_eq!(get(&out, "PROTON_ENABLE_WAYLAND"), Some("1"));
    assert_eq!(get(&out, "PROTON_PREFER_SDL"), Some("1"));
    assert!(!out.report.iter().any(
        |(i, _, s)| matches!(i, OptionId::Wayland | OptionId::SdlInput) && matches!(s, OptionState::Unavailable(_))
    ));
}

#[test]
fn obs_and_layers() {
    let mut env = fixture();
    Arc::make_mut(&mut env.caps).layers.push(LayerManifest {
        name: "VK_LAYER_vkBasalt".into(),
        enable_env: Some(("ENABLE_VKBASALT".into(), "1".into())),
        disable_env: Some("DISABLE_VKBASALT".into()),
        ..Default::default()
    });
    let mut g = GameOptions { obs_capture: on(), ..Default::default() };
    g.layers.insert("VK_LAYER_vkBasalt".into(), Tri::On);
    g.layers.insert("VK_LAYER_missing".into(), Tri::Off);
    // Native: the obs-gamecapture wrapper (GL too); layer env at the top without gamescope.
    let out = cmd("/opt/g/run", &[]).run_in(&g, &env);
    assert_eq!((out.program.as_str(), out.args.clone()), ("obs-gamecapture", sv(&["/opt/g/run"])));
    assert_eq!(get(&out, "ENABLE_VKBASALT"), Some("1"));
    assert!(out.report.iter().any(|(i, n, s)| *i == OptionId::Layer
        && n.as_deref() == Some("VK_LAYER_missing")
        && matches!(s, OptionState::Unavailable(_))));
    // Proton under gamescope: both behind the `--`.
    g.gamescope = on();
    let out = umu().run_in(&g, &env);
    let dd = out.args.iter().position(|a| a == "--").unwrap();
    assert_eq!(out.args[dd + 1..dd + 4], sv(&["env", "OBS_VKCAPTURE=1", "ENABLE_VKBASALT=1"]));
}

#[test]
fn mangohud_inside_gamescope_warns() {
    let g = GameOptions { gamescope: on(), ..Default::default() };
    let out = umu().opts("mangohud %command%").run(&g);
    assert!(has_note(&out, "MangoHud inside Gamescope"), "{:?}", out.notes);
}

#[test]
fn proton_log_dir() {
    let env = LaunchEnv { log_dir: Some(PathBuf::from("/data/proton-logs")), ..fixture() };
    let out = umu().run_in(&GameOptions { proton_log: on(), ..Default::default() }, &env);
    assert_eq!(get(&out, "PROTON_LOG"), Some("1"));
    assert_eq!(get(&out, "PROTON_LOG_DIR"), Some("/data/proton-logs/7"));
    assert_eq!(out.make_dirs, vec![PathBuf::from("/data/proton-logs/7")]);
    assert!(is_unavailable(
        &cmd("wine", &["x.exe"]).run(&GameOptions { proton_log: on(), ..Default::default() }),
        OptionId::ProtonLog
    ));
}

#[test]
fn migration() {
    let mut e: crate::model::UserEdits = serde_json::from_str(r#"{"hdr":true}"#).unwrap();
    e.migrate();
    assert_eq!(e.game_options.hdr, Tri::On);
    assert!(!e.hdr);
    let json = serde_json::to_string(&e).unwrap();
    assert!(!json.contains("\"hdr\":true"), "{json}");
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(v.get("hdr").is_none(), "{json}");
    // An Off the game already had is kept.
    let mut e: crate::model::UserEdits = serde_json::from_str(r#"{"hdr":true,"game_options":{"hdr":"off"}}"#).unwrap();
    e.migrate();
    assert_eq!(e.game_options.hdr, Tri::Off);
    // Old rows and configs load as Default; default edits serialise without game_options.
    let e: crate::model::UserEdits = serde_json::from_str("{}").unwrap();
    assert!(e.game_options.is_default());
    assert!(!serde_json::to_string(&e).unwrap().contains("game_options"));
    let (c, problems) = crate::config::Config::parse_lenient("ui_scale = 1.5\n");
    assert!(problems.is_empty() && c.game_options.is_default());
}

#[test]
fn config_round_trips_game_options() {
    let game_options = GameOptions {
        mangohud: on(),
        gs_output: Some(Res::Custom { w: 1920, h: 1080 }),
        gs_internal: Some(InternalRes::Percent(67)),
        frame_limit: Some(60),
        lsfg_flow_scale: Some(0.5),
        ..Default::default()
    };
    let mut c = crate::config::Config { game_options, ..Default::default() };
    c.game_options.layers.insert("VK_LAYER_vkBasalt".into(), Tri::Off);
    let text = toml::to_string(&c).unwrap();
    let (back, problems) = crate::config::Config::parse_lenient(&text);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(back.game_options, c.game_options);
}

#[test]
fn sync_vars() {
    assert!(crate::client_launch::procs::SYNC_VARS.contains(&"PROTON_NO_NTSYNC"));
}

// ----------------------------------------------------------------------- review regressions

#[test]
fn frame_limit_skips_mangohud_when_mangohud_is_off() {
    let g = GameOptions { frame_limit: Some(60), mangohud: Tri::Off, ..Default::default() };
    // Native: DISABLE_MANGOHUD=1 would switch a MangoHud limiter off, so there is none.
    let out = cmd("/opt/g/run", &[]).run(&g);
    assert_eq!((out.program.as_str(), out.args.clone()), ("/opt/g/run", vec![]));
    assert!(get(&out, "MANGOHUD_CONFIG").is_none());
    assert!(is_unavailable(&out, OptionId::FrameLimit));
    // Launcher: no MANGOHUD=1 next to DISABLE_MANGOHUD=1.
    let out = cmd("lutris", &["lutris:rungameid/3"]).run(&g);
    assert!(get(&out, "MANGOHUD").is_none());
    assert_eq!(get(&out, "DISABLE_MANGOHUD"), Some("1"));
    // Proton still uses DXVK's limiter.
    assert_eq!(get(&umu().run(&g), "VKD3D_FRAME_RATE"), Some("60"));
}

#[test]
fn toggle_merges_into_user_gamescope_without_detected_binary() {
    let mut env = fixture();
    Arc::make_mut(&mut env.caps).bins.gamescope = None;
    let g = GameOptions { gamescope: on(), ..Default::default() };
    let out = umu().opts("gamescope -f -- %command%").run_in(&g, &env);
    assert_eq!(out.program, "gamescope");
    assert_eq!(count(&out, "gamescope"), 1);
    assert_eq!(out.args[..4], sv(&["-W", "2560", "-H", "1440"]));
    assert_eq!(state(&out, OptionId::Gamescope), Some(&OptionState::Applied));
}

#[test]
fn proton_wine_recognised_without_probe() {
    let g = GameOptions { ntsync: Tri::Off, wayland: on(), ..Default::default() };
    let out = cmd("/x/GE-Proton9-1/files/bin/wine64", &["x.exe"]).run(&g);
    assert_eq!(out.core, Core::Wine);
    assert_eq!(get(&out, "PROTON_NO_NTSYNC"), Some("1"));
    // Unknown wine caps: Wayland stays unavailable (no winex11.drv=d without winewayland.so).
    assert!(is_unavailable(&out, OptionId::Wayland));
    assert!(get(&out, "WINEDLLOVERRIDES").is_none());
}

#[test]
fn unprobed_proton_dir_uses_the_runner_probe() {
    fn valve(_: &Path) -> Arc<RunnerCaps> {
        Arc::new(RunnerCaps { family: RunnerFamily::Valve, version: "Proton 10.0".into(), ..Default::default() })
    }
    let g = GameOptions { wayland: on(), ..Default::default() };
    // Without a probe the dir is an Unknown runner: everything is offered.
    let out = umu_with("/opt/proton-x").run(&g);
    assert_eq!(get(&out, "PROTON_ENABLE_WAYLAND"), Some("1"));
    // With it, its real caps apply.
    let out = Case { probe: Some(valve), ..umu_with("/opt/proton-x") }.run(&g);
    assert!(is_unavailable(&out, OptionId::Wayland));
    assert!(get(&out, "PROTON_ENABLE_WAYLAND").is_none());
    // Runners the fixture lists don't go through the probe.
    let out = Case { probe: Some(valve), ..umu() }.run(&g);
    assert_eq!(get(&out, "PROTON_ENABLE_WAYLAND"), Some("1"));
}

#[test]
fn steam_string_has_no_log_dir() {
    // Flatpak Steam can't see Lodestar's folder: only PROTON_LOG=1 (Proton's own $HOME), and no
    // empty folder is created for it.
    let env = LaunchEnv { log_dir: Some(PathBuf::from("/l")), ..fixture() };
    let c = Case { kind: LaunchKind::Steam, ..cmd("steam", &["-applaunch", "570"]) };
    let out = c.run_in(&GameOptions { proton_log: on(), ..Default::default() }, &env);
    let s = out.steam_string.clone().unwrap();
    assert!(s.contains("PROTON_LOG=1") && !s.contains("PROTON_LOG_DIR"), "{s}");
    assert!(out.make_dirs.is_empty());
    assert!(out.env.is_empty());
    assert!(has_note(&out, "steam-<appid>.log"), "{:?}", out.notes);
    // Lodestar's own launches still log into its folder.
    let out = umu().run_in(&GameOptions { proton_log: on(), ..Default::default() }, &env);
    assert_eq!(get(&out, "PROTON_LOG_DIR"), Some("/l/7"));
}

#[test]
fn lsfg_flow_scale_nan_is_left_out() {
    let mut env = fixture();
    Arc::make_mut(&mut env.caps).lsfg = Some(LsfgCaps { version: 2, ..Default::default() });
    let g = GameOptions {
        lsfg: on(),
        lsfg_profile: Some(String::new()),
        lsfg_flow_scale: Some(f32::NAN),
        ..Default::default()
    };
    let out = umu().run_in(&g, &env);
    assert_eq!(get(&out, "LSFGVK_ENV"), Some("1"));
    assert!(get(&out, "LSFGVK_FLOW_SCALE").is_none());
}

#[test]
fn hdr_wsi_note_with_user_gamescope() {
    let g = GameOptions { hdr: on(), gs_disable_wsi: on(), ..Default::default() };
    let out = umu().opts("gamescope -f -- %command%").run(&g);
    assert_eq!(get(&out, "ENABLE_GAMESCOPE_WSI"), Some("0"));
    assert!(has_note(&out, "needs the Gamescope WSI layer"), "{:?}", out.notes);
}

#[test]
fn library_layer_sits_between_global_and_game() {
    let global = GameOptions { mangohud: Tri::On, gamemode: Tri::On, frame_limit: Some(60), ..Default::default() };
    let heroic = GameOptions { mangohud: Tri::Off, gamescope: Tri::On, ..Default::default() };
    let mut env = LaunchEnv::none();
    env.defaults = global.clone();
    env.by_source.insert("epic".into(), heroic);
    let d = defaults_for(&env, "epic");
    assert_eq!((d.mangohud, d.gamemode, d.gamescope, d.frame_limit), (Tri::Off, Tri::On, Tri::On, Some(60)));
    // Another library only sees the global options.
    assert_eq!(defaults_for(&env, "steam"), global);
    // The game's own value still wins.
    let game = GameOptions { mangohud: Tri::On, ..Default::default() };
    assert_eq!(effective(&d, &game).mangohud, Tri::On);
    assert_eq!(effective(&d, &GameOptions::default()).gamescope, Tri::On);
}

#[test]
fn more_gamescope_options_go_after_lodestars_flags() {
    let env = fixture();
    let mut game = GameOptions { gamescope: Tri::On, ..Default::default() };
    game.gs_more.insert("--rt".into(), FlagSetting::On);
    game.gs_more.insert("--hdr-sdr-content-nits".into(), FlagSetting::Value("300".into()));
    game.gs_more.insert("--fullscreen".into(), FlagSetting::On); // managed: ignored
    game.gs_more.insert("--steam".into(), FlagSetting::On); // hidden: ignored
    let global = GameOptions { gs_more: [("--grab".to_string(), FlagSetting::On)].into(), ..Default::default() };
    // The game switches the global --grab off again.
    game.gs_more.insert("--grab".into(), FlagSetting::Off);
    let eff = effective(&global, &game);
    assert_eq!(eff.gs_more.len(), 4);
    let mut caps = (*env.caps).clone();
    if let Some(g) = &mut caps.gamescope {
        for f in ["--rt", "--hdr-sdr-content-nits", "--grab"] {
            g.flags.insert(f.into());
        }
    }
    let env = LaunchEnv { caps: std::sync::Arc::new(caps), ..env };
    let out = compose::apply(compose::ComposeInput {
        program: "/opt/g/run".into(),
        args: vec![],
        base_env: vec![],
        custom_command: false,
        user_env: vec![],
        user_prefix: vec![],
        user_suffix: vec![],
        kind: compose::LaunchKind::Process,
        play: true,
        name: "Test",
        game_id: 1,
        url: None,
        self_exe: None,
        opts: &eff,
        env: &env,
        probe_runner: None,
        probe_wine: None,
    });
    let argv: Vec<&str> = std::iter::once(out.program.as_str()).chain(out.args.iter().map(String::as_str)).collect();
    let dd = argv.iter().position(|w| *w == "--").unwrap();
    let gs = &argv[..dd];
    let rt = gs.iter().position(|w| *w == "--rt").unwrap();
    assert!(gs.iter().position(|w| *w == "-f").unwrap() < rt, "{argv:?}");
    assert_eq!(gs[gs.iter().position(|w| *w == "--hdr-sdr-content-nits").unwrap() + 1], "300");
    assert!(!gs.contains(&"--grab") && !gs.contains(&"--steam"), "{argv:?}");
    assert_eq!(gs.iter().filter(|w| **w == "--fullscreen" || **w == "-f").count(), 1, "{argv:?}");
}

// ------------------------------------------------------------------ review regressions (2)

#[test]
fn user_gamescope_window_flag_beats_ours() {
    let g = GameOptions { gamescope: on(), ..Default::default() };
    let out = umu().opts("gamescope -b -- %command%").run(&g);
    assert_eq!(count(&out, "-b"), 1, "{:?}", out.args);
    assert_eq!(count(&out, "-f"), 0, "{:?}", out.args);
    // Typed in Extra Gamescope arguments too; our booleans aren't repeated.
    let g = GameOptions { gs_extra_args: Some("--borderless --adaptive-sync".into()), gs_adaptive_sync: on(), ..g };
    let out = umu().run(&g);
    assert_eq!(count(&out, "-f"), 0, "{:?}", out.args);
    assert_eq!(count(&out, "--adaptive-sync"), 1, "{:?}", out.args);
    // Without a typed window mode ours stays.
    assert_eq!(count(&umu().run(&GameOptions { gamescope: on(), ..Default::default() }), "-f"), 1);
}

#[test]
fn user_gamescope_without_dashdash_is_split() {
    let g = GameOptions { gamescope: on(), hdr: on(), ..Default::default() };
    let out = umu().opts("gamescope -f %command%").run(&g);
    assert_eq!(out.core, Core::Proton);
    assert_eq!(count(&out, "gamescope"), 1, "{:?}", out.args);
    let dd = out.args.iter().position(|a| a == "--").unwrap();
    assert_eq!(out.args[dd + 1..], sv(&["umu-run", "/g/game.exe"]));
    assert!(out.args[..dd].iter().any(|a| a == "--hdr-enabled"), "{:?}", out.args);
    assert_eq!(get(&out, "DXVK_HDR"), Some("1"));
    // Valued options take their value (-W 1920), an unknown option its number.
    let out = umu().opts("gamescope -W 1920 -H 1080 --some-new-opt 3 -b %command%").run(&GameOptions::default());
    assert_eq!(out.core, Core::Proton);
    assert_eq!(out.program, "gamescope");
    assert_eq!(
        out.args,
        sv(&["-W", "1920", "-H", "1080", "--some-new-opt", "3", "-b", "--", "umu-run", "/g/game.exe"])
    );
}

#[test]
fn unrecognised_prefix_wrappers_keep_the_proton_kind() {
    let g = GameOptions { hdr: on(), ..Default::default() };
    for o in [
        "taskset -c 0-7 %command%",
        "nice -n -5 %command%",
        "strangle 60 %command%",
        "env -u FOO %command%",
        "powerprofilesctl launch -p performance %command%",
        "/home/u/bin/my-wrapper.sh --fast %command%",
    ] {
        let out = umu().opts(o).run(&g);
        assert_eq!(out.core, Core::Proton, "{o}");
        assert_eq!(get(&out, "DXVK_HDR"), Some("1"), "{o}");
        assert_eq!(out.core_program.as_deref(), Some("umu-run"), "{o}");
        // The wrapper stays where the user put it.
        assert_eq!(out.program, o.split_whitespace().next().unwrap(), "{o}");
    }
    // Past an unknown wrapper, a known one is still recognised (no second gamemoderun).
    let out =
        umu().opts("taskset -c 0 gamemoderun %command%").run(&GameOptions { gamemode: on(), ..Default::default() });
    assert_eq!(count(&out, "gamemoderun"), 1, "{:?}", out.args);
    // A native game's own arguments are never searched.
    let out = cmd("/opt/g/run", &["proton", "umu-run"]).opts("taskset -c 0-7 %command%").run(&g);
    assert_eq!(out.core, Core::Native);
    let out = cmd("/opt/g/run", &["umu-run"]).run(&g);
    assert_eq!(out.core, Core::Native);
}

fn wine_with_wayland(_: &Path, _: &[(String, String)]) -> Arc<RunnerCaps> {
    Arc::new(RunnerCaps { version: "usr".into(), winewayland: true, fshack_fsr: true, ..Default::default() })
}

fn wine_without_wayland(_: &Path, _: &[(String, String)]) -> Arc<RunnerCaps> {
    Arc::new(RunnerCaps { version: "usr".into(), ..Default::default() })
}

fn wine_unchecked(_: &Path, _: &[(String, String)]) -> Arc<RunnerCaps> {
    Arc::new(RunnerCaps::unknown())
}

#[test]
fn plain_wine_is_probed() {
    let w = |f: compose::WineProbe| Case { wine: Some(f), ..cmd("/usr/bin/wine", &["x.exe"]) };
    let out = w(wine_with_wayland).run(&GameOptions { wayland: on(), ..Default::default() });
    assert_eq!(out.core, Core::Wine);
    assert_eq!(get(&out, "WINEDLLOVERRIDES"), Some("winex11.drv=d"));
    assert_eq!(state(&out, OptionId::Wayland), Some(&OptionState::Applied));
    let out = w(wine_with_wayland).run(&GameOptions { wine_fsr: on(), ..Default::default() });
    assert_eq!(get(&out, "WINE_FULLSCREEN_FSR"), Some("1"));
    // HDR gets the Wayland driver.
    let out = w(wine_with_wayland).run(&GameOptions { hdr: on(), ..Default::default() });
    assert_eq!(get(&out, "WINEDLLOVERRIDES"), Some("winex11.drv=d"));
    assert!(!has_note(&out, "neither is active"), "{:?}", out.notes);
    // A wine without the driver, and one that couldn't be checked.
    let out = w(wine_without_wayland).run(&GameOptions { wayland: on(), ..Default::default() });
    assert!(
        matches!(state(&out, OptionId::Wayland), Some(OptionState::Unavailable(m)) if m.contains("has no Wayland driver"))
    );
    assert!(get(&out, "WINEDLLOVERRIDES").is_none());
    for c in [w(wine_unchecked), cmd("wine", &["x.exe"])] {
        let out = c.run(&GameOptions { wayland: on(), ..Default::default() });
        assert!(
            matches!(state(&out, OptionId::Wayland), Some(OptionState::Unavailable(m)) if m.contains("Couldn't check")),
            "{:?}",
            out.report
        );
        assert!(get(&out, "WINEDLLOVERRIDES").is_none());
    }
    // PROTON_* stay out for plain Wine even when its caps are known.
    let out = w(wine_with_wayland).run(&GameOptions { hdr: Tri::Off, ..Default::default() });
    assert!(get(&out, "PROTON_ENABLE_HDR").is_none());
}

#[test]
fn mangohud_frame_limit_keeps_the_config_file() {
    let g = GameOptions { mangohud: on(), frame_limit: Some(60), ..Default::default() };
    let out = cmd("/opt/g/run", &[]).run(&g);
    assert_eq!(out.program, "mangohud");
    assert_eq!(get(&out, "MANGOHUD_CONFIG"), Some("read_cfg,fps_limit=60"));
    // The user's own MANGOHUD_CONFIG already decided about the file.
    let out = cmd("/opt/g/run", &[]).opts("MANGOHUD_CONFIG=position=top-left %command%").run(&g);
    assert_eq!(get(&out, "MANGOHUD_CONFIG"), Some("position=top-left,fps_limit=60"));
    // The user's MANGOHUD=1 isn't hidden by the limiter-only form.
    let fl = GameOptions { frame_limit: Some(60), ..Default::default() };
    let out = cmd("/opt/g/run", &[]).opts("MANGOHUD=1 %command%").run(&fl);
    assert_eq!(out.program, "mangohud");
    assert_eq!(get(&out, "MANGOHUD_CONFIG"), Some("read_cfg,fps_limit=60"));
    let out = cmd("lutris", &["lutris:rungameid/3"]).opts("MANGOHUD=1 %command%").run(&fl);
    assert_eq!(get(&out, "MANGOHUD_CONFIG"), Some("read_cfg,fps_limit=60"));
    // Without it the limiter stays hidden.
    assert_eq!(get(&cmd("/opt/g/run", &[]).run(&fl), "MANGOHUD_CONFIG"), Some("no_display,fps_limit=60"));
}

#[test]
fn off_default_never_cancels_typed_text() {
    let defaults = GameOptions { mangohud: Tri::Off, obs_capture: Tri::Off, ..Default::default() };
    let env = LaunchEnv { defaults, ..fixture() };
    for o in ["mangohud %command%", "MANGOHUD=1 %command%"] {
        let out = umu().opts(o).run_in(&GameOptions::default(), &env);
        assert!(get(&out, "DISABLE_MANGOHUD").is_none(), "{o}: {:?}", out.env);
        assert!(is_suppressed(&out, OptionId::Mangohud), "{o}");
    }
    // Without typed text the Off still applies.
    assert_eq!(get(&umu().run_in(&GameOptions::default(), &env), "DISABLE_MANGOHUD"), Some("1"));
    for o in ["obs-gamecapture %command%", "OBS_VKCAPTURE=1 %command%"] {
        let out = umu().opts(o).run_in(&GameOptions::default(), &env);
        assert!(get(&out, "DISABLE_OBS_VKCAPTURE").is_none(), "{o}");
        assert!(is_suppressed(&out, OptionId::ObsCapture), "{o}");
    }
    // A layer's enable variable.
    let mut env = fixture();
    Arc::make_mut(&mut env.caps).layers.push(LayerManifest {
        name: "VK_LAYER_vkBasalt".into(),
        enable_env: Some(("ENABLE_VKBASALT".into(), "1".into())),
        disable_env: Some("DISABLE_VKBASALT".into()),
        ..Default::default()
    });
    let mut g = GameOptions::default();
    g.layers.insert("VK_LAYER_vkBasalt".into(), Tri::Off);
    let out = umu().opts("ENABLE_VKBASALT=1 %command%").run_in(&g, &env);
    assert!(get(&out, "DISABLE_VKBASALT").is_none(), "{:?}", out.env);
    assert!(out.report.iter().any(|(i, n, s)| *i == OptionId::Layer
        && n.as_deref() == Some("VK_LAYER_vkBasalt")
        && matches!(s, OptionState::Suppressed(_))));
    assert_eq!(get(&umu().run_in(&g, &env), "DISABLE_VKBASALT"), Some("1"));
}

#[test]
fn user_gamescope_frame_rate_flags_win() {
    let fl = GameOptions { frame_limit: Some(60), ..Default::default() };
    for o in [
        "gamescope --framerate-limit 60 -- %command%",
        "gamescope --framerate-limit=60 -- %command%",
        "gamescope -r 30 -- %command%",
    ] {
        let out = umu().opts(o).run(&fl);
        let dd = out.args.iter().position(|a| a == "--").unwrap();
        let want = usize::from(o.contains("-r "));
        assert_eq!(out.args[..dd].iter().filter(|a| *a == "-r").count(), want, "{o}: {:?}", out.args);
        assert!(is_suppressed(&out, OptionId::FrameLimit), "{o}");
        assert!(get(&out, "DXVK_CONFIG").is_none() && get(&out, "MANGOHUD_CONFIG").is_none(), "{o}");
    }
    // Lodestar's More Gamescope options count as well.
    let mut g = GameOptions { gamescope: on(), ..fl.clone() };
    g.gs_more.insert("--framerate-limit".into(), FlagSetting::Value("60".into()));
    let mut env = fixture();
    Arc::make_mut(&mut env.caps).gamescope.as_mut().unwrap().flags.insert("--framerate-limit".into());
    let out = umu().run_in(&g, &env);
    assert_eq!(count(&out, "-r"), 0, "{:?}", out.args);
}

#[test]
fn typed_sdl_video_driver_leaves_both_names() {
    let g = GameOptions { wayland: on(), ..Default::default() };
    for o in ["SDL_VIDEODRIVER=x11 %command%", "SDL_VIDEO_DRIVER=x11 %command%"] {
        let out = cmd("/opt/g/run", &[]).opts(o).run(&g);
        assert!(!out.env.iter().any(|(_, v)| v.contains("wayland")), "{o}: {:?}", out.env);
        assert!(is_suppressed(&out, OptionId::Wayland), "{o}");
        let g = GameOptions { gamescope: on(), gs_expose_wayland: on(), ..g.clone() };
        let out = cmd("/opt/g/run", &[]).opts(o).run(&g);
        assert!(!out.args.iter().any(|a| a.contains("wayland,x11")), "{o}: {:?}", out.args);
    }
}

#[test]
fn newest_runner_compares_names_and_prefers_umus_dir() {
    let mut caps = (*fixture().caps).clone();
    for d in [
        "/home/u/.local/share/Steam/compatibilitytools.d/GE-Proton10-26",
        "/home/u/.steam/root/compatibilitytools.d/GE-Proton9-20",
        "/usr/share/steam/compatibilitytools.d/GE-Proton10-30",
        "/a/UMU-Proton-10.0-4",
        "/z/UMU-Proton-9.0-3",
    ] {
        caps.runners.insert(PathBuf::from(d), Arc::new(ge_caps()));
    }
    let pp = ev(&[("PROTONPATH", "GE-Proton")]);
    // umu's own folder wins over newer builds elsewhere; the parent path never decides.
    assert_eq!(
        compose::proton_dir_for(&pp, "umu-run", &caps),
        Some(PathBuf::from("/home/u/.local/share/Steam/compatibilitytools.d/GE-Proton10-26"))
    );
    assert_eq!(compose::proton_dir_for(&[], "umu-run", &caps), Some(PathBuf::from("/a/UMU-Proton-10.0-4")));
    caps.runners.remove(Path::new("/home/u/.local/share/Steam/compatibilitytools.d/GE-Proton10-26"));
    assert_eq!(
        compose::proton_dir_for(&pp, "umu-run", &caps),
        Some(PathBuf::from("/usr/share/steam/compatibilitytools.d/GE-Proton10-30"))
    );
}

/// A Ryzen laptop: RDNA3 iGPU (boot) and an NVIDIA dGPU.
fn hybrid() -> LaunchEnv {
    let mut env = fixture();
    Arc::make_mut(&mut env.caps).gpus = vec![
        Gpu {
            card: "card0".into(),
            pci: "0000:65:00.0".into(),
            vendor: 0x1002,
            device: 0x15bf,
            driver: "amdgpu".into(),
            boot_vga: true,
            vram: Some(512 << 20),
            gc_ip: Some((11, 0)),
            ..Default::default()
        },
        Gpu {
            card: "card1".into(),
            pci: "0000:01:00.0".into(),
            vendor: 0x10de,
            device: 0x28a0,
            driver: "nvidia".into(),
            ..Default::default()
        },
    ];
    env
}

#[test]
fn gpu_gates_use_the_rendering_gpu() {
    let env = hybrid();
    // Use discrete GPU picks the NVIDIA card (Caps::best_gpu).
    let g = GameOptions { prefer_dgpu: on(), ..Default::default() };
    assert_eq!(get(&umu().run_in(&g, &env), "__NV_PRIME_RENDER_OFFLOAD"), Some("1"));
    // Windows games render on the NVIDIA card here anyway (DXVK picks the discrete GPU first):
    // FSR 4 isn't available (the RDNA3 chip is only the iGPU), DLSS is — with or without
    // Use discrete GPU.
    let fsr = GameOptions { fsr4_upgrade: on(), ..Default::default() };
    let out = umu().run_in(&fsr, &env);
    assert!(matches!(state(&out, OptionId::Fsr4Upgrade), Some(OptionState::Unavailable(_))), "{:?}", out.report);
    assert!(get(&out, "PROTON_FSR4_UPGRADE").is_none());
    let dlss = GameOptions { dlss_upgrade: on(), ..Default::default() };
    assert_eq!(get(&umu().run_in(&dlss, &env), "PROTON_DLSS_UPGRADE"), Some("1"));
    let out = umu().run_in(&GameOptions { prefer_dgpu: on(), ..dlss.clone() }, &env);
    assert_eq!(get(&out, "PROTON_DLSS_UPGRADE"), Some("1"));
    // No GPU list yet: the gates pass.
    let mut env = fixture();
    Arc::make_mut(&mut env.caps).gpus.clear();
    assert_eq!(get(&umu().run_in(&dlss, &env), "PROTON_DLSS_UPGRADE"), Some("1"));
}

#[test]
fn ntsync_on_checks_runner_and_kernel() {
    let g = GameOptions { ntsync: on(), ..Default::default() };
    assert_eq!(state(&umu().run(&g), OptionId::Ntsync), Some(&OptionState::Applied));
    // A runner without NTSync.
    let mut env = fixture();
    let none = RunnerCaps { family: RunnerFamily::Valve, version: "UMU-Proton-10.0-4".into(), ..Default::default() };
    Arc::make_mut(&mut env.caps).runners.insert(PathBuf::from("/r/umu"), Arc::new(none));
    let out = umu_with("/r/umu").run_in(&g, &env);
    assert!(
        matches!(state(&out, OptionId::Ntsync), Some(OptionState::Unavailable(m)) if m.contains("no NTSync")),
        "{:?}",
        out.report
    );
    // The kernel module.
    for (st, why) in [(NtsyncState::NotLoaded, "isn't loaded"), (NtsyncState::Missing, "no NTSync")] {
        let mut env = fixture();
        Arc::make_mut(&mut env.caps).ntsync = st;
        let out = umu().run_in(&g, &env);
        assert!(
            matches!(state(&out, OptionId::Ntsync), Some(OptionState::Unavailable(m)) if m.contains(why)),
            "{:?}",
            out.report
        );
    }
    // Not probed yet: no conclusion about the kernel.
    let mut env = fixture();
    {
        let c = Arc::make_mut(&mut env.caps);
        c.probed = false;
        c.ntsync = NtsyncState::Missing;
    }
    assert_eq!(state(&umu().run_in(&g, &env), OptionId::Ntsync), Some(&OptionState::Applied));
}

#[test]
fn x11_session_suppresses_wayland_and_hdr() {
    let mut env = fixture();
    Arc::make_mut(&mut env.caps).wayland_session = false;
    let out = umu().run_in(&GameOptions { hdr: on(), ..Default::default() }, &env);
    assert!(get(&out, "PROTON_ENABLE_WAYLAND").is_none() && get(&out, "DXVK_HDR").is_none(), "{:?}", out.env);
    assert!(is_suppressed(&out, OptionId::Hdr));
    assert!(has_note(&out, "HDR needs Gamescope"), "{:?}", out.notes);
    let out = umu().run_in(&GameOptions { wayland: on(), ..Default::default() }, &env);
    assert!(get(&out, "PROTON_ENABLE_WAYLAND").is_none());
    assert!(is_suppressed(&out, OptionId::Wayland));
    let out = cmd("/opt/g/run", &[]).run_in(&GameOptions { wayland: on(), ..Default::default() }, &env);
    assert!(get(&out, "SDL_VIDEODRIVER").is_none() && is_suppressed(&out, OptionId::Wayland));
    // Gamescope does HDR there; with Expose Wayland it gives Wine a Wayland display.
    let out = umu().run_in(&GameOptions { hdr: on(), gamescope: on(), ..Default::default() }, &env);
    assert!(out.args.iter().any(|a| a == "--hdr-enabled"));
    assert_eq!(state(&out, OptionId::Hdr), Some(&OptionState::Applied));
    let g = GameOptions { wayland: on(), gamescope: on(), gs_expose_wayland: on(), ..Default::default() };
    assert_eq!(get(&umu().run_in(&g, &env), "PROTON_ENABLE_WAYLAND"), Some("1"));
    // Before the first probe nothing is concluded.
    Arc::make_mut(&mut env.caps).probed = false;
    let out = umu().run_in(&GameOptions { hdr: on(), ..Default::default() }, &env);
    assert_eq!(get(&out, "PROTON_ENABLE_WAYLAND"), Some("1"));
}

#[test]
fn core_program_under_wrappers() {
    let g = GameOptions { keep_awake: on(), gamescope: on(), gamemode: on(), mangohud: on(), ..Default::default() };
    let cases = [
        (cmd("/opt/g/run", &["-x"]), "/opt/g/run"),
        (umu(), "umu-run"),
        (cmd("/usr/bin/wine", &["x.exe"]), "/usr/bin/wine"),
    ];
    for (c, core) in cases {
        let out = c.run(&g);
        assert_ne!(out.program, core);
        assert_eq!(out.core_program.as_deref(), Some(core));
        // Found again from the built command (what launch.rs checks before spawning).
        let mut argv = vec![out.program.clone()];
        argv.extend(out.args.iter().cloned());
        assert_eq!(compose::core_program(&argv, None).as_deref(), Some(core), "{argv:?}");
    }
    let out = cmd("/opt/g/run", &[])
        .opts("taskset -c 0 %command%")
        .run(&GameOptions { keep_awake: on(), ..Default::default() });
    let mut argv = vec![out.program.clone()];
    argv.extend(out.args.iter().cloned());
    assert_eq!(compose::core_program(&argv, None).as_deref(), Some("taskset"));
    // Steam launches have none.
    let c = Case { kind: LaunchKind::Steam, ..cmd("steam", &["-applaunch", "570"]) };
    assert_eq!(c.run(&g).core_program, None);
}

#[test]
fn gamescope_split_keeps_a_following_dashdash() {
    // A typed boolean right before `--` isn't taken for an option with a value.
    let g = GameOptions { gamescope: on(), ..Default::default() };
    let out = umu().opts("gamescope -f -- %command%").run(&g);
    assert_eq!(count(&out, "--"), 1, "{:?}", out.args);
    assert_eq!(count(&out, "-f"), 1, "{:?}", out.args);
    // An option this list doesn't know, with a text value, before the user's `--`.
    let out =
        umu().opts("gamescope --future-opt xyz -- %command%").run(&GameOptions { hdr: on(), ..Default::default() });
    assert_eq!(out.core, Core::Proton);
    assert_eq!(out.args, sv(&["--hdr-enabled", "--future-opt", "xyz", "--", "umu-run", "/g/game.exe"]));
    // Without `--` before the core, a `--` among the game's own arguments isn't gamescope's.
    let c = Case { args: sv(&["/g/game.exe", "--", "x"]), ..umu() };
    let out = c.opts("gamescope -f %command%").run(&GameOptions::default());
    assert_eq!(out.core, Core::Proton);
    assert_eq!(out.args, sv(&["-f", "--", "umu-run", "/g/game.exe", "--", "x"]));
    // Lodestar's own gamescope with such an option: the built command still names the game.
    let mut g = GameOptions { gamescope: on(), ..Default::default() };
    g.gs_more.insert("--future-opt".into(), FlagSetting::Value("abcxyz".into()));
    let mut env = fixture();
    Arc::make_mut(&mut env.caps).gamescope.as_mut().unwrap().flags.insert("--future-opt".into());
    let out = cmd("/opt/g/run", &[]).run_in(&g, &env);
    assert!(out.args.iter().any(|a| a == "abcxyz"), "{:?}", out.args);
    let mut argv = vec![out.program.clone()];
    argv.extend(out.args.iter().cloned());
    assert_eq!(compose::core_program(&argv, None).as_deref(), Some("/opt/g/run"), "{argv:?}");
}

#[test]
fn unsupported_more_gamescope_framerate_limit_doesnt_suppress_frame_limit() {
    // This gamescope doesn't document --framerate-limit, so gs_words leaves it out: -r stays.
    let mut g = GameOptions { gamescope: on(), frame_limit: Some(60), ..Default::default() };
    g.gs_more.insert("--framerate-limit".into(), FlagSetting::Value("60".into()));
    let out = umu().run(&g);
    assert!(!out.args.iter().any(|a| a == "--framerate-limit"), "{:?}", out.args);
    assert_eq!(count(&out, "-r"), 1, "{:?}", out.args);
    assert_eq!(state(&out, OptionId::FrameLimit), Some(&OptionState::Applied));
}

#[test]
fn flatpak_launcher_proton_log_has_no_log_dir() {
    let env = LaunchEnv { log_dir: Some(PathBuf::from("/l")), ..fixture() };
    let g = GameOptions { proton_log: on(), ..Default::default() };
    let out = cmd("flatpak", &["run", "com.heroicgameslauncher.hgl", "--no-gui"]).run_in(&g, &env);
    assert_eq!(out.core, Core::Launcher);
    assert_eq!(get(&out, "PROTON_LOG"), Some("1"));
    assert!(get(&out, "PROTON_LOG_DIR").is_none() && out.make_dirs.is_empty(), "{:?}", out.env);
    assert!(has_note(&out, "~/.var/app/"), "{:?}", out.notes);
    // A host launcher still logs into Lodestar's folder.
    let out = cmd("lutris", &["lutris:rungameid/3"]).run_in(&g, &env);
    assert_eq!(get(&out, "PROTON_LOG_DIR"), Some("/l/7"));
}

#[test]
fn gpu_ranking_prefers_the_discrete_card() {
    let g = |card: &str, pci: &str, vendor: u16, vram: Option<u64>, boot: bool| Gpu {
        card: card.into(),
        pci: pci.into(),
        vendor,
        vram,
        boot_vga: boot,
        driver: if vendor == 0x10de { "nvidia".into() } else { "amdgpu".into() },
        ..Default::default()
    };
    // This machine: RDNA2 iGPU (2 GiB carve-out) + RDNA4 dGPU as boot GPU.
    let caps = Caps {
        gpus: vec![
            g("card0", "0000:75:00.0", 0x1002, Some(2 << 30), false),
            g("card1", "0000:03:00.0", 0x1002, Some(16 << 30), true),
        ],
        ..Default::default()
    };
    assert_eq!(caps.best_gpu().unwrap().card, "card1");
    // Optimus laptop: Intel iGPU boots, NVIDIA renders DXVK games (DLSS stays available).
    let caps = Caps {
        gpus: vec![g("card0", "0000:00:02.0", 0x8086, None, true), g("card1", "0000:01:00.0", 0x10de, None, false)],
        ..Default::default()
    };
    assert_eq!(caps.rendering_gpu(false).unwrap().card, "card1");
    // Intel iGPU + Arc dGPU with unknown VRAM: the Arc wins.
    let caps = Caps {
        gpus: vec![g("card0", "0000:00:02.0", 0x8086, None, true), g("card1", "0000:03:00.0", 0x8086, None, false)],
        ..Default::default()
    };
    assert_eq!(caps.best_gpu().unwrap().card, "card1");
}

#[test]
fn hdr_does_nothing_while_desktop_hdr_is_off() {
    let mut env = fixture();
    Arc::make_mut(&mut env.caps).kde_hdr_enabled = Some(false);
    let g = GameOptions { hdr: on(), ..Default::default() };
    let out = umu().run_in(&g, &env);
    assert!(get(&out, "PROTON_ENABLE_WAYLAND").is_none() && get(&out, "DXVK_HDR").is_none(), "{:?}", out.env);
    assert!(matches!(state(&out, OptionId::Hdr), Some(OptionState::Suppressed(_))));
    let gs = GameOptions { hdr: on(), gamescope: on(), ..Default::default() };
    let out = umu().run_in(&gs, &env);
    assert!(!out.args.iter().any(|a| a == "--hdr-enabled"), "{:?}", out.args);
}
