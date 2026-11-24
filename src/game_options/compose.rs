//! Turning effective game options into environment variables and wrappers, in the right order,
//! around a resolved launch (spec §2 per option and launch kind, §3 composition order).
//!
//! CONTRACT: `apply(ComposeInput) -> ComposeOutput`. Pure: reads only its input (caps included),
//! never touches the filesystem (not even to check that a path exists) or spawns anything, so
//! `launch::resolve` stays cheap enough to call every frame. The one exception is the injected
//! `ComposeInput::probe_runner` (the cached `caps::runner_caps`), used for a Proton dir that the
//! background probe didn't cover, and `ComposeInput::probe_wine` (`caps::wine_caps`) for a plain
//! Wine binary. With `LaunchEnv::none()` /
//! all-Default options it returns the input command as `launch.rs` always built it (env = base
//! env + the user's `VAR=` words, argv = user prefix + program + args + user suffix) and a report
//! that holds at most `Unavailable` entries. Two §3 rules apply even then, because they fix a
//! user-typed `gamescope … --` prefix: the variables gamescope strips (`ENABLE_VKBASALT`,
//! `SDL_VIDEODRIVER`, `SDL_VIDEO_DRIVER`) move behind its `--`, and `<dir>/proton run` becomes
//! `waitforexitandrun` inside it.
//!
//! The launch kind (spec §2 N P W F L C S U, [`Core`]) comes from `input.kind` for Steam / URL
//! launches and otherwise from [`classify`], which peels known wrappers off the command.
//!
//! Choices where the spec is silent:
//! * Report: every option whose effective value is not Default gets an entry; an option that
//!   is Default gets one only when it is `Unavailable` here (so the UI can grey it out), or
//!   `AppliedImplicitly` (Wayland for HDR, the HDR WSI layer). NTSync is left out for N / F.
//! * Kind notes for Steam / URL launches are only given for the Play launch (other actions are
//!   tools / documents); launcher and client notes whenever an option was wanted.
//! * Wrapper toggles (gamescope, gamemode, mangohud, obs, lsfg, power profile, keep awake, frame
//!   limit) are `Suppressed` for non-Play actions, env forms of them included.
//! * Gamescope flags with only the user's gamescope: `--mangoapp`, `--hdr-enabled`, `-r` and
//!   `--prefer-vk-device` (spec §3.2) are added whatever the action, since the options that ask
//!   for them decide for themselves whether they apply.
//! * System Wine (W): Wayland and Wine FSR need the wine's file checks (`probe_wine`, or
//!   `caps.runners` keyed by the wine root); without them they are Unavailable ("couldn't check"),
//!   because a wine without `winewayland.so` and `winex11.drv=d` would start with no display
//!   driver at all.
//! * Unrecognised wrappers before the core (`taskset -c 0-7 %command%`, a script of the user's):
//!   classify looks past them for the real core (in the user's prefix up to the base program;
//!   past known pass-through wrappers such as `nice` / `strangle` to the end) and leaves them in
//!   place as an opaque part of the chain.
//! * The user's gamescope without `--` (`gamescope -f %command%`) is split getopt-style (known
//!   valued options take the next word, a numeric word after an unknown option is its value);
//!   the rebuilt command gets a `--`.
//! * The user's gamescope flags beat Lodestar's booleans: a typed window mode (`-f` / `-b`)
//!   replaces ours, and `--framerate-limit` / `-r` there leave the Frame limit Suppressed.
//! * Caps probed and not a Wayland session (and no gamescope exposing its own): Wayland On and
//!   HDR without gamescope are Suppressed, with a note.
//! * An Off default never cancels what the user typed: a typed `mangohud` / `MANGOHUD=1`,
//!   `obs-gamecapture` / `OBS_VKCAPTURE=1` or a layer's enable variable keeps it on (Suppressed).
//! * Steam's launch-options string has `PROTON_LOG=1` without `PROTON_LOG_DIR` (Flatpak Steam
//!   can't see Lodestar's folder): Proton writes `steam-<appid>.log` to its own `$HOME`.
//! * No GPU list (caps not probed yet): GPU gates (FSR 4, DLSS) pass instead of failing.
//! * `{or_run_directly}` is added for Faugus (Lodestar can run its games with umu-run).
//! * Proton log without `LaunchEnv::log_dir`: only `PROTON_LOG=1` (Proton's own folder).
//! * Frame limit with MangoHud Off: the MangoHud limiter backends are skipped (their
//!   `DISABLE_MANGOHUD=1` would switch the limiter off too); the next backend or the note applies.
//! * Gamescope On with the user's own `gamescope … --`: our flags merge into it even when the
//!   probe found no gamescope binary.
//! * `<dir>/files/bin/wine*` counts as wine inside a Proton dir from the path alone.
//! * `steam://` URLs other than a game id (OpenUrl through Steam) get the Steam note and string.

use super::*;

/// Plasma has HDR off on every monitor (kwinoutputconfig.json).
const DESKTOP_HDR_OFF: &str =
    "HDR is off for every monitor in System Settings → Display & Monitor — turn it on there first.";
use std::path::Path;

/// How the launch is started (mirrors `launch::ResolvedKind`, kept separate so this module
/// doesn't depend on launch.rs internals).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchKind {
    /// Lodestar runs the command itself.
    Process,
    /// The Steam client starts it (`steam -applaunch`, `steam://`).
    Steam,
    /// Handed to a URL handler (`heroic://` …).
    Url,
}

/// The launch kinds of spec §2.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Core {
    /// N: a native program.
    #[default]
    Native,
    /// P: umu-run or a `<dir>/proton` script.
    Proton,
    /// W: plain wine (or a wine binary inside a Proton dir).
    Wine,
    /// F: `flatpak run <app>`.
    Flatpak,
    /// L: a launcher CLI (Lutris, Heroic, Faugus …).
    Launcher,
    /// C: Lodestar's store-client helper (`--client-launch`).
    Client,
    /// S: started by the Steam client.
    Steam,
    /// U: handed to a URL handler.
    Url,
}

impl Core {
    /// Kinds whose argv Lodestar composes (wrappers, gamescope).
    fn blocking(self) -> bool {
        matches!(self, Core::Native | Core::Proton | Core::Wine | Core::Flatpak)
    }
    /// Kinds that get environment variables at all.
    fn takes_env(self) -> bool {
        !matches!(self, Core::Steam | Core::Url)
    }
    /// Kinds that run a Proton build (its `PROTON_*` script variables apply).
    fn protonish(self) -> bool {
        matches!(self, Core::Proton | Core::Client | Core::Launcher)
    }
}

/// Looks up a plain Wine binary's caps: (the wine as typed, the env the command runs with).
pub type WineProbe = fn(&Path, &[(String, String)]) -> Arc<RunnerCaps>;

/// The pieces of a resolved launch before the user's launch options are folded in.
#[derive(Clone, Debug)]
pub struct ComposeInput<'a> {
    /// Base command (after `fold_env_command`): the source's command or the custom command.
    pub program: String,
    pub args: Vec<String>,
    /// The source's env, or the custom command's `VAR=` words.
    pub base_env: Vec<(String, String)>,
    pub custom_command: bool,
    /// From the user's launch options: `VAR=` words, words before `%command%`, words after it.
    pub user_env: Vec<(String, String)>,
    pub user_prefix: Vec<String>,
    pub user_suffix: Vec<String>,
    pub kind: LaunchKind,
    /// The Play launch (wrappers apply) vs another action (env only).
    pub play: bool,
    /// Game name / id (inhibitor reason, Proton log folder).
    pub name: &'a str,
    pub game_id: i64,
    /// The URL of a URL launch (`heroic://…`), for its note.
    pub url: Option<&'a str>,
    /// This Lodestar binary's path (a `@lodestar` program resolved by launch.rs), so its own
    /// helpers are recognised without looking at the filesystem.
    pub self_exe: Option<&'a str>,
    /// Looks up a Proton dir that isn't in `caps.runners` (spec §4.2 "other dirs are probed on
    /// first resolve"); launch.rs passes the cached `caps::runner_caps`. Called at most once per
    /// launch, only for P / C launches and wine inside a Proton dir. None (tests) = such a dir is
    /// an Unknown runner, which keeps `apply` free of filesystem access.
    pub probe_runner: Option<fn(&Path) -> Arc<RunnerCaps>>,
    /// Looks up a plain Wine binary (launch kind W outside a Proton dir): the core word as typed
    /// (`wine`, `/usr/bin/wine` …) and the env the command runs with (its `PATH` for a bare
    /// name). launch.rs passes a cached `caps::wine_caps`; None (tests, the Steam string) = the
    /// wine is unchecked, and Wayland / Wine FSR stay Unavailable.
    pub probe_wine: Option<WineProbe>,
    pub opts: &'a Effective,
    pub env: &'a LaunchEnv,
}

/// The composed command and what happened to each option.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComposeOutput {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// UI notes (one per kind of limitation; spec §2.7).
    pub notes: Vec<String>,
    /// Per option (layers carry their `layer.name`), in option-list order.
    pub report: Vec<(OptionId, Option<String>, OptionState)>,
    /// For Steam launches: the text to paste into Steam's launch options (spec §3.4).
    pub steam_string: Option<String>,
    /// Folders to create before spawning (PROTON_LOG_DIR).
    pub make_dirs: Vec<PathBuf>,
    /// The launch kind the options were applied for.
    pub core: Core,
    /// The core program of a process launch (the game binary, `umu-run`, `wine`, `flatpak` …)
    /// as it appears in `program` / `args`, under any wrappers; None for Steam / URL launches.
    pub core_program: Option<String>,
}

/// The Proton dir a command runs with: a `<dir>/proton` program; else `PROTONPATH` (absolute
/// dir, or `GE-Proton` → the newest `GE-Proton*` among `caps.runners`); else plain `umu-run` →
/// the newest `UMU-Proton*`. Pure (looks only at `caps.runners` keys). None = Unknown runner.
pub fn proton_dir_for(env: &[(String, String)], program: &str, caps: &Caps) -> Option<PathBuf> {
    let p = Path::new(program);
    if base(program) == "proton" {
        return p.parent().filter(|d| !d.as_os_str().is_empty()).map(Path::to_path_buf);
    }
    let protonpath = env.iter().rev().find(|(k, _)| k == "PROTONPATH").map(|(_, v)| v.trim());
    match protonpath {
        Some(v) if v.starts_with('/') => {
            let t = v.trim_end_matches('/');
            Some(PathBuf::from(if t.is_empty() { "/" } else { t }))
        }
        Some("GE-Proton") => newest_runner(caps, "GE-Proton"),
        Some(v) if !v.is_empty() => None,
        _ if base(program) == "umu-run" => newest_runner(caps, "UMU-Proton"),
        _ => None,
    }
}

/// The newest `caps.runners` dir whose name starts with `prefix`, by the dir's name in natural
/// version order (never the parent path). umu resolves `GE-Proton` / UMU-Proton from its own
/// `<data home>/Steam/compatibilitytools.d`, so a match there is preferred over newer builds in
/// other compat folders (`~/.steam/root/…`, `/usr/share/steam/…`); ties go by path.
fn newest_runner(caps: &Caps, prefix: &str) -> Option<PathBuf> {
    let name = |d: &PathBuf| d.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
    let umu_dir = |d: &PathBuf| {
        d.parent().is_some_and(|p| p.ends_with("Steam/compatibilitytools.d"))
            && !d.to_string_lossy().contains("/.var/app/")
    };
    let candidates: Vec<&PathBuf> = caps.runners.keys().filter(|d| name(d).starts_with(prefix)).collect();
    let pool: Vec<&PathBuf> = if candidates.iter().any(|d| umu_dir(d)) {
        candidates.into_iter().filter(|d| umu_dir(d)).collect()
    } else {
        candidates
    };
    pool.into_iter().max_by(|a, b| natural_cmp(&name(a), &name(b)).then_with(|| a.cmp(b))).cloned()
}

/// Compare strings with digit runs as numbers ("GE-Proton10-9" < "GE-Proton10-26").
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn chunks(s: &str) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = vec![];
        for c in s.chars() {
            let d = c.is_ascii_digit();
            match out.last_mut() {
                Some((ld, t)) if *ld == d => t.push(c),
                _ => out.push((d, c.to_string())),
            }
        }
        out
    }
    let (ca, cb) = (chunks(a), chunks(b));
    for (x, y) in ca.iter().zip(cb.iter()) {
        let o = match (x.0, y.0) {
            (true, true) => {
                let (xa, ya) = (x.1.trim_start_matches('0'), y.1.trim_start_matches('0'));
                xa.len().cmp(&ya.len()).then_with(|| xa.cmp(ya))
            }
            _ => x.1.cmp(&y.1),
        };
        if o != std::cmp::Ordering::Equal {
            return o;
        }
    }
    ca.len().cmp(&cb.len())
}

/// Apply the effective options to one launch (spec §2, §3).
pub fn apply(input: ComposeInput) -> ComposeOutput {
    compose(&input, None)
}

// ------------------------------------------------------------------------------ classification

mod classify;
// Public: classify, classify_with, core_program, Chain, GsSplit; the helpers are pub(super).
pub use classify::*;

// ------------------------------------------------------------------------------------ runner

/// The runner's caps, and whether they are known (Unknown = act as if everything is supported).
struct Runner {
    rc: Arc<RunnerCaps>,
    known: bool,
}

impl Runner {
    fn of(dir: Option<&Path>, caps: &Caps) -> Runner {
        Runner::with(dir.and_then(|d| caps.runners.get(d)).cloned())
    }
    fn with(rc: Option<Arc<RunnerCaps>>) -> Runner {
        match rc {
            Some(rc) => Runner { known: rc.family != RunnerFamily::Unknown, rc },
            None => Runner { rc: Arc::new(RunnerCaps::unknown()), known: false },
        }
    }
    /// A plain Wine's caps: known when its files were actually checked (`wine_caps` names the
    /// root in `version`; its family stays Unknown because `PROTON_*` don't apply to it).
    fn wine(rc: Arc<RunnerCaps>) -> Runner {
        Runner { known: rc.family != RunnerFamily::Unknown || !rc.version.trim().is_empty(), rc }
    }
    /// The chain's runner: `caps.runners`, else the injected probes (Proton launches: the runner
    /// probe; plain Wine: the wine probe, with the env the command runs with).
    fn for_chain(chain: &Chain, caps: &Caps, input: &ComposeInput, env: &[(String, String)]) -> Runner {
        if chain.kind == Core::Wine && !chain.proton_wine {
            if let Some(rc) = chain.runner.as_deref().and_then(|d| caps.runners.get(d)) {
                return Runner::wine(rc.clone());
            }
            return match (input.probe_wine, chain.words.get(chain.core_at)) {
                (Some(f), Some(bin)) => Runner::wine(f(Path::new(bin), env)),
                _ => Runner::with(None),
            };
        }
        let probe = input.probe_runner;
        let Some(dir) = chain.runner.as_deref() else { return Runner::with(None) };
        if let Some(rc) = caps.runners.get(dir) {
            return Runner::with(Some(rc.clone()));
        }
        let proton = matches!(chain.kind, Core::Proton | Core::Client) || chain.proton_wine;
        Runner::with(probe.filter(|_| proton).map(|f| f(dir)))
    }
    fn unknown() -> Runner {
        Runner::of(None, &Caps::default())
    }
    /// The script reads `var` (always true for an Unknown runner).
    fn has(&self, var: &str) -> bool {
        !self.known || self.rc.vars.contains(var)
    }
    /// A capability flag (true for an Unknown runner).
    fn flag(&self, f: fn(&RunnerCaps) -> bool) -> bool {
        !self.known || f(&self.rc)
    }
    /// A capability flag that must be known to be true (system Wine).
    fn strict(&self, f: fn(&RunnerCaps) -> bool) -> bool {
        self.known && f(&self.rc)
    }
    fn family(&self) -> RunnerFamily {
        self.rc.family
    }
    fn name(&self) -> String {
        if !self.rc.version.trim().is_empty() {
            return self.rc.version.trim().to_string();
        }
        match self.rc.family {
            RunnerFamily::Valve => "Valve Proton",
            RunnerFamily::Ge => "GE-Proton",
            RunnerFamily::Cachyos => "proton-cachyos",
            RunnerFamily::Unknown => "this runner",
        }
        .to_string()
    }
}

// ------------------------------------------------------------------------------------ composer

/// `(key, value)` pairs in order.
type EnvList = Vec<(String, String)>;

/// One environment variable a toggle wants.
struct Entry {
    id: OptionId,
    layer: String,
    key: String,
    val: String,
    /// Goes behind gamescope's `--` when gamescope runs (spec §2 "scoped").
    scoped: bool,
}

/// Gamescope flags Lodestar decided on (rendered in spec order by `gs_words`).
#[derive(Default)]
struct GsFlags {
    /// (output, internal) size.
    size: Option<((u32, u32), (u32, u32))>,
    window: Option<&'static str>,
    rate: Option<u32>,
    filter: Option<&'static str>,
    sharpness: Option<u8>,
    scaler: Option<&'static str>,
    adaptive: bool,
    hdr: bool,
    mangoapp: bool,
    prefer: Option<String>,
    expose: bool,
    grab: bool,
}

#[derive(Default)]
struct Wrappers {
    inhibit: Option<Vec<String>>,
    perf: Option<Vec<String>>,
    gamemode: bool,
    mangohud: bool,
    obs: bool,
}

const ONLY_PLAY: &str = "Only applies when playing (not to other actions).";
const GS_WAYLAND: &str = "Wayland native is ignored inside Gamescope (games run on Gamescope's X server).";
const GS_LAUNCHER: &str = "Gamescope can't wrap a launcher or store client: Gamescope closes when the launcher exits and would stop the game.";
const GS_NESTED: &str = "Lodestar is already running inside Gamescope.";
const MANGO_IN_GS: &str =
    "MangoHud inside Gamescope isn't supported — remove it from the launch options; the MangoHud option uses mangoapp.";
const TYPED_ON: &str = "Your launch options turn it on.";
const X11_SESSION: &str = "This is an X11 session: Wine's Wayland driver needs a Wayland session.";
const X11_HDR: &str = "This is an X11 session: HDR needs Gamescope here, so the game would show SDR.";
const VALVE_WAYLAND: &str = "Valve Proton has no Wayland switch — use GE-Proton or proton-cachyos.";

struct Composer<'a> {
    input: &'a ComposeInput<'a>,
    o: &'a Effective,
    caps: &'a Caps,
    core: Core,
    chain: Chain,
    runner: Runner,
    play: bool,
    gs_ours: bool,
    gs_active: bool,
    /// Gamescope exposes its Wayland socket (ours with the option, or the user's flag).
    gs_expose: bool,
    out_size: Option<(u32, u32)>,
    gsf: GsFlags,
    wrap: Wrappers,
    entries: Vec<Entry>,
    lists: Vec<(OptionId, &'static str, char, String)>,
    report: Vec<(OptionId, Option<String>, OptionState)>,
    notes: Vec<String>,
    /// Options that produced env (launcher note), wrappers not added (launcher / client note),
    /// LD_PRELOAD wrappers a Flatpak can't take.
    env_labels: Vec<String>,
    wrapper_labels: Vec<String>,
    preload_labels: Vec<String>,
    wayland_applied: bool,
    mangohud_applied: bool,
    fsr4_effective: bool,
    /// The user's own text turns MangoHud on (a typed `mangohud`, `MANGOHUD=1`).
    mango_user_on: bool,
    /// Composing Steam's launch-options string, not a command Lodestar runs.
    steam_text: bool,
    make_dirs: Vec<PathBuf>,
}

fn label(id: OptionId) -> &'static str {
    OPTIONS.iter().find(|m| m.id == id).map(|m| m.label).unwrap_or("Vulkan layer")
}

fn on_off(t: Tri) -> &'static str {
    if t.is_on() { "1" } else { "0" }
}

/// Whether an option's effective value differs from the built-in Default.
fn is_set(o: &Effective, id: OptionId) -> bool {
    use OptionId as I;
    match id {
        I::Wayland => o.wayland != Tri::Default,
        I::Hdr => o.hdr != Tri::Default,
        I::HdrForce => o.hdr_force != Tri::Default,
        I::Gamescope => o.gamescope != Tri::Default,
        I::GsWindow => o.gs_window != WindowMode::default(),
        I::GsOutput => o.gs_output != Res::default(),
        I::GsInternal => o.gs_internal != InternalRes::default(),
        I::GsFilter => o.gs_filter != GsFilter::default(),
        I::GsSharpness => o.gs_sharpness.is_some(),
        I::GsScaler => o.gs_scaler != GsScaler::default(),
        I::GsAdaptiveSync => o.gs_adaptive_sync != Tri::Default,
        I::GsGrabCursor => o.gs_grab_cursor != Tri::Default,
        I::GsExposeWayland => o.gs_expose_wayland != Tri::Default,
        I::GsDisableWsi => o.gs_disable_wsi != Tri::Default,
        I::GsMore => !o.gs_more.is_empty(),
        I::GsExtraArgs => o.gs_extra_args.is_some(),
        I::Gamemode => o.gamemode != Tri::Default,
        I::PerformanceProfile => o.performance_profile != Tri::Default,
        I::KeepAwake => o.keep_awake != Tri::Default,
        I::FrameLimit => o.frame_limit.is_some(),
        I::PreferDgpu => o.prefer_dgpu != Tri::Default,
        I::Ntsync => o.ntsync != Tri::Default,
        I::WineFsr => o.wine_fsr != Tri::Default,
        I::WineFsrStrength => o.wine_fsr_strength.is_some(),
        I::Fsr4Upgrade => o.fsr4_upgrade != Tri::Default,
        I::DlssUpgrade => o.dlss_upgrade != Tri::Default,
        I::XessUpgrade => o.xess_upgrade != Tri::Default,
        I::UpscalerIndicator => o.upscaler_indicator != Tri::Default,
        I::Lsfg => o.lsfg != Tri::Default,
        I::LsfgProfile => o.lsfg_profile.is_some(),
        I::LsfgMultiplier => o.lsfg_multiplier != 2,
        I::LsfgFlowScale => o.lsfg_flow_scale.is_some(),
        I::LsfgPerformance => o.lsfg_performance != Tri::Default,
        I::Mangohud => o.mangohud != Tri::Default,
        I::ObsCapture => o.obs_capture != Tri::Default,
        I::Nvapi => o.nvapi != Nvapi::Default,
        I::SdlInput => o.sdl_input != Tri::Default,
        I::AudioLatency => o.audio_latency != Tri::Default,
        I::ProtonLog => o.proton_log != Tri::Default,
        I::HdrWsiLayer => o.hdr_wsi_layer != Tri::Default,
        I::Layer => !o.layers.is_empty(),
    }
}

/// Labels of the top-level options that are set, layers by name (the kind notes' `{list}`).
fn wanted_labels(o: &Effective) -> Vec<String> {
    let mut v: Vec<String> =
        OPTIONS.iter().filter(|m| m.parent.is_none() && is_set(o, m.id)).map(|m| m.label.to_string()).collect();
    v.extend(o.layers.keys().cloned());
    v
}

fn join(v: &[String]) -> String {
    v.join(", ")
}

fn compose(input: &ComposeInput, force: Option<Core>) -> ComposeOutput {
    let o = input.opts;
    let caps = &*input.env.caps;
    let mut words = input.user_prefix.clone();
    words.push(input.program.clone());
    words.extend(input.args.iter().cloned());
    // The env the command runs with, for PROTONPATH.
    let mut look_env = input.base_env.clone();
    for (k, v) in &input.user_env {
        set_env(&mut look_env, k, v);
    }
    let mut chain = match input.kind {
        LaunchKind::Process => classify_with(words, input.user_prefix.len(), &look_env, caps, input.self_exe),
        LaunchKind::Steam => Chain { words, kind: Core::Steam, ..Default::default() },
        LaunchKind::Url => Chain { words, kind: Core::Url, ..Default::default() },
    };
    let runner = match force {
        Some(k) => {
            chain.kind = k;
            Runner::unknown()
        }
        None => Runner::for_chain(&chain, caps, input, &look_env),
    };
    let core = chain.kind;
    let mut c = Composer {
        input,
        o,
        caps,
        core,
        chain,
        runner,
        play: input.play,
        gs_ours: false,
        gs_active: false,
        gs_expose: false,
        out_size: None,
        gsf: GsFlags::default(),
        wrap: Wrappers::default(),
        entries: vec![],
        lists: vec![],
        report: vec![],
        notes: vec![],
        env_labels: vec![],
        wrapper_labels: vec![],
        preload_labels: vec![],
        wayland_applied: false,
        mangohud_applied: false,
        fsr4_effective: false,
        mango_user_on: false,
        steam_text: force.is_some(),
        make_dirs: vec![],
    };
    if !core.takes_env() {
        return c.not_applied();
    }
    c.run_options();
    c.finish()
}

// The options, by kind; `run_options` below gives their order.
mod display;
mod gpu;
mod wrappers;

impl Composer<'_> {
    // ----------------------------------------------------------------------------- helpers

    fn rep(&mut self, id: OptionId, st: OptionState) {
        self.report.retain(|(i, n, _)| !(*i == id && n.is_none()));
        self.report.push((id, None, st));
    }
    fn applied(&mut self, id: OptionId) {
        self.rep(id, OptionState::Applied);
    }
    fn suppressed(&mut self, id: OptionId, why: &str) {
        self.rep(id, OptionState::Suppressed(why.to_string()));
    }
    fn unavailable(&mut self, id: OptionId, why: &str) {
        self.rep(id, OptionState::Unavailable(why.to_string()));
    }
    fn note(&mut self, n: impl Into<String>) {
        let n = n.into();
        if !self.notes.contains(&n) {
            self.notes.push(n);
        }
    }
    fn push(&mut self, id: OptionId, layer: &str, k: &str, v: impl Into<String>, scoped: bool) {
        let l = if id == OptionId::Layer { layer.to_string() } else { label(id).to_string() };
        if !self.env_labels.contains(&l) {
            self.env_labels.push(l);
        }
        self.entries.push(Entry { id, layer: layer.to_string(), key: k.into(), val: v.into(), scoped });
    }
    /// A process-env variable.
    fn set(&mut self, id: OptionId, k: &str, v: impl Into<String>) {
        self.push(id, "", k, v, false);
    }
    /// A variable that goes behind gamescope's `--` when gamescope runs.
    fn scoped(&mut self, id: OptionId, k: &str, v: impl Into<String>) {
        self.push(id, "", k, v, true);
    }
    /// An item merged into a list variable (`;` / `,` separated).
    fn list(&mut self, id: OptionId, var: &'static str, sep: char, item: impl Into<String>) {
        let l = label(id).to_string();
        if !self.env_labels.contains(&l) {
            self.env_labels.push(l);
        }
        self.lists.push((id, var, sep, item.into()));
    }
    /// The launch's own env (source / custom command and the user's words) has `k` (= `v`).
    fn user_env_has(&self, k: &str, v: Option<&str>) -> bool {
        self.input
            .user_env
            .iter()
            .chain(self.input.base_env.iter())
            .any(|(ek, ev)| ek == k && v.is_none_or(|v| ev == v))
    }
    /// The user typed `k` (= `v`): in the launch options, or a custom command's `VAR=` words
    /// (not a source's own env, which toggles may override).
    fn user_typed(&self, k: &str, v: Option<&str>) -> bool {
        let base: &[(String, String)] = if self.input.custom_command { &self.input.base_env } else { &[] };
        self.input.user_env.iter().chain(base.iter()).any(|(ek, ev)| ek == k && v.is_none_or(|v| ev == v))
    }
    /// Caps were probed and Lodestar doesn't run in a Wayland session (no `WAYLAND_DISPLAY`), so
    /// Wine's Wayland driver has nothing to connect to (unless gamescope exposes its own socket).
    fn no_wayland_session(&self) -> bool {
        self.caps.probed && !self.caps.wayland_session && !self.gs_expose
    }
    /// "Use discrete GPU" is On and has a GPU to switch to.
    fn prefer_dgpu_applies(&self) -> bool {
        self.o.prefer_dgpu.is_on() && self.dgpu().is_ok()
    }
    /// The words that configure the user's gamescope: the flags typed before its `--`, and (when
    /// Lodestar's gamescope options apply) the typed Extra Gamescope arguments.
    fn user_gs_words(&self) -> Vec<String> {
        let mut v = self.user_gs_flags();
        if self.gs_ours
            && let Some(extra) = &self.o.gs_extra_args
            && let Ok(w) = crate::launch::split_command(extra)
        {
            v.extend(w);
        }
        v
    }
    fn nvidia(&self) -> bool {
        self.caps.nvidia_version.is_some() || self.caps.gpus.iter().any(|g| g.vendor == 0x10de && g.driver == "nvidia")
    }
    fn flatpak_ext(&self, id: &str) -> bool {
        self.caps.flatpak_ext.contains_key(id)
    }
    /// A wrapper toggle that can't apply to this launch: report why and return true.
    fn wrapper_blocked(&mut self, id: OptionId, wanted_on: bool) -> bool {
        if !wanted_on {
            return false;
        }
        if !self.play {
            self.suppressed(id, ONLY_PLAY);
            return true;
        }
        if matches!(self.core, Core::Launcher | Core::Client) {
            let why = if self.core == Core::Client {
                "Not added for store-client games."
            } else {
                "Not added for games a launcher starts."
            };
            self.suppressed(id, why);
            let l = label(id).to_string();
            if !self.wrapper_labels.contains(&l) {
                self.wrapper_labels.push(l);
            }
            return true;
        }
        false
    }

    // ----------------------------------------------------------------------- S / U launches

    /// Steam / URL launches: nothing applies; the command is built as before, plus the notes.
    fn not_applied(self) -> ComposeOutput {
        let input = self.input;
        let mut env = input.base_env.clone();
        for (k, v) in &input.user_env {
            set_env(&mut env, k, v);
        }
        let mut argv = self.chain.words.clone();
        argv.extend(input.user_suffix.iter().cloned());
        let program = argv.remove(0);
        let list = wanted_labels(self.o);
        let mut out = ComposeOutput { program, args: argv, env, core: self.core, ..Default::default() };
        if list.is_empty() {
            return out;
        }
        let why = if self.core == Core::Steam {
            "The Steam client starts this game.".to_string()
        } else {
            format!("{} starts this game with its own settings.", url_handler(input.url))
        };
        for m in OPTIONS.iter().filter(|m| m.parent.is_none() && is_set(self.o, m.id)) {
            out.report.push((m.id, None, OptionState::Suppressed(why.clone())));
        }
        for name in self.o.layers.keys() {
            out.report.push((OptionId::Layer, Some(name.clone()), OptionState::Suppressed(why.clone())));
        }
        if self.core == Core::Steam {
            (out.steam_string, out.make_dirs) = steam_string(input);
        }
        if input.play {
            let note = if self.core == Core::Steam {
                let n = NOTE_STEAM.replace("{list}", &join(&list));
                match &out.steam_string {
                    Some(s) => n.replace("{steam_string}", s),
                    None => n.replace(" In Steam, set the game's launch options to: `{steam_string}`.", ""),
                }
            } else {
                NOTE_URL.replace("{handler}", &url_handler(input.url)).replace("{list}", &join(&list))
            };
            out.notes.push(note);
            if self.core == Core::Steam && self.o.proton_log.is_on() {
                out.notes.push(
                    "With PROTON_LOG=1 Steam's Proton writes steam-<appid>.log to your home folder \
                     (Flatpak Steam: ~/.var/app/com.valvesoftware.Steam/)."
                        .into(),
                );
            }
        }
        out
    }

    // ------------------------------------------------------------------------------ options

    fn run_options(&mut self) {
        self.setup_gamescope();
        self.opt_wayland();
        self.opt_hdr();
        self.opt_hdr_force();
        self.opt_gamescope_env();
        self.opt_gamemode();
        self.opt_performance();
        self.opt_keep_awake();
        self.opt_mangohud();
        self.opt_frame_limit();
        self.opt_prefer_dgpu();
        self.opt_ntsync();
        self.opt_wine_fsr();
        self.opt_fsr4();
        self.opt_dlss();
        self.opt_xess();
        self.opt_indicator();
        self.opt_lsfg();
        self.opt_obs();
        self.opt_nvapi();
        self.opt_sdl();
        self.opt_audio();
        self.opt_proton_log();
        self.opt_hdr_wsi();
        self.opt_layers();
    }

    fn opt_sdl(&mut self) {
        use OptionId::SdlInput as I;
        let t = self.o.sdl_input;
        if !self.core.protonish() {
            if t != Tri::Default {
                self.rep(I, OptionState::NotApplicable);
            }
            return;
        }
        if !self.runner.has("PROTON_PREFER_SDL") {
            let why = format!("Not supported by {} (GE-Proton or proton-cachyos).", self.runner.name());
            self.unavailable(I, &why);
            return;
        }
        if t != Tri::Default {
            self.set(I, "PROTON_PREFER_SDL", on_off(t));
            self.applied(I);
        }
    }

    fn opt_audio(&mut self) {
        use OptionId::AudioLatency as I;
        let t = self.o.audio_latency;
        if t == Tri::Default {
            return;
        }
        if t.is_off() {
            self.applied(I);
        } else if matches!(self.core, Core::Proton | Core::Client) && self.runner.family() == RunnerFamily::Cachyos {
            let why = "proton-cachyos plays audio through PipeWire directly; this fix has no effect.";
            self.note(why);
            self.suppressed(I, why);
        } else {
            self.set(I, "PULSE_LATENCY_MSEC", "60");
            self.applied(I);
        }
    }

    fn opt_proton_log(&mut self) {
        use OptionId::ProtonLog as I;
        let t = self.o.proton_log;
        if t == Tri::Default {
            return;
        }
        match self.core {
            c if c.protonish() => {
                if t.is_on() {
                    self.set(I, "PROTON_LOG", "1");
                    // Not in Steam's string, nor for a Flatpak launcher: their sandbox can't see
                    // Lodestar's folder (it would write the log elsewhere, or nowhere), so Proton
                    // logs to its own $HOME.
                    let flatpak_launcher = self.core == Core::Launcher
                        && self.chain.words.get(self.chain.core_at).is_some_and(|w| base(w) == "flatpak");
                    if flatpak_launcher {
                        self.note(
                            "This launcher runs as a Flatpak, so Proton writes steam-<id>.log to the app's own home \
                             (usually ~/.var/app/<app id>/), not Lodestar's log folder.",
                        );
                    }
                    if let Some(dir) = self.input.env.log_dir.as_ref().filter(|_| !self.steam_text && !flatpak_launcher)
                    {
                        let d = dir.join(self.input.game_id.to_string());
                        self.set(I, "PROTON_LOG_DIR", d.to_string_lossy().into_owned());
                        self.make_dirs.push(d);
                    }
                }
                self.applied(I);
            }
            Core::Wine => self.unavailable(I, "Proton only."),
            _ => self.rep(I, OptionState::NotApplicable),
        }
    }

    fn opt_layers(&mut self) {
        let (o, caps) = (self.o, self.caps);
        for (name, t) in &o.layers {
            let m = caps.layers.iter().find(|l| l.name == *name);
            let st = match (m, t) {
                (None, _) => OptionState::Unavailable("Not installed.".into()),
                (Some(m), Tri::On) => match &m.enable_env {
                    Some((k, v)) => {
                        self.push(OptionId::Layer, name, k, v.clone(), true);
                        OptionState::Applied
                    }
                    None => OptionState::Unavailable("This layer can only be turned off.".into()),
                },
                (Some(m), _) if m.enable_env.as_ref().is_some_and(|(k, v)| self.user_typed(k, Some(v))) => {
                    OptionState::Suppressed(TYPED_ON.into())
                }
                (Some(m), _) => match &m.disable_env {
                    Some(k) => {
                        self.push(OptionId::Layer, name, k, "1", true);
                        OptionState::Applied
                    }
                    None => OptionState::Unavailable("This layer has no off switch.".into()),
                },
            };
            self.report.push((OptionId::Layer, Some(name.clone()), st));
        }
    }

    // ------------------------------------------------------------------------------ output

    /// §3.1: the process env, the scoped env and the keys toggles produced (for flatpak --env).
    fn build_env(&mut self) -> (EnvList, EnvList, Vec<String>) {
        let input = self.input;
        let custom = input.custom_command;
        let scoped_here = self.gs_active && self.core.blocking();
        let mut env = input.base_env.clone();
        let mut scoped: Vec<(String, String)> = vec![];
        let mut produced: Vec<String> = vec![];
        let user_has = |k: &str| input.user_env.iter().any(|(uk, _)| uk == k);
        let mut entries = std::mem::take(&mut self.entries);
        entries.sort_by(|a, b| (a.id, &a.layer).cmp(&(b.id, &b.layer)));
        for e in entries {
            let in_base = input.base_env.iter().any(|(k, _)| *k == e.key);
            if custom && in_base {
                continue; // the custom command's own value wins
            }
            if e.scoped && scoped_here {
                if user_has(&e.key) {
                    continue; // the user's value wins
                }
                env.retain(|(k, _)| *k != e.key);
                set_env(&mut scoped, &e.key, &e.val);
            } else {
                set_env(&mut env, &e.key, &e.val);
            }
            if !produced.contains(&e.key) {
                produced.push(e.key);
            }
        }
        for (k, v) in &input.user_env {
            set_env(&mut env, k, v);
        }
        let lists = std::mem::take(&mut self.lists);
        let mut vars: Vec<(&'static str, char)> = vec![];
        for (_, var, sep, _) in &lists {
            if !vars.iter().any(|(v, _)| v == var) {
                vars.push((var, *sep));
            }
        }
        for (var, sep) in vars {
            let items: Vec<&str> = lists.iter().filter(|l| l.1 == var).map(|l| l.3.as_str()).collect();
            let cur = env.iter().find(|(k, _)| k == var).map(|(_, v)| v.as_str());
            let merged = merge_list(cur, &items, sep);
            set_env(&mut env, var, &merged);
            if !produced.iter().any(|p| p == var) {
                produced.push(var.to_string());
            }
        }
        if scoped_here {
            for k in ["ENABLE_VKBASALT", "SDL_VIDEODRIVER", "SDL_VIDEO_DRIVER"] {
                if let Some(i) = env.iter().position(|(ek, _)| ek == k) {
                    let (k, v) = env.remove(i);
                    set_env(&mut scoped, &k, &v);
                }
            }
        }
        (env, scoped, produced)
    }

    fn finish(mut self) -> ComposeOutput {
        let input = self.input;
        let core = self.core;
        let (env, scoped, produced) = self.build_env();
        let mut argv: Vec<String>;
        if core.blocking() {
            let mut words = self.chain.words.clone();
            if self.gs_active
                && let Some(i) = self.chain.proton_run
            {
                words[i] = "waitforexitandrun".into();
            }
            if let Some(run) = self.chain.flatpak_run {
                let mut ins = vec![];
                if self.gs_active && !self.gs_expose {
                    ins.push("--nosocket=wayland".to_string());
                }
                for k in &produced {
                    let v = scoped.iter().chain(env.iter()).find(|(ek, _)| ek == k).map(|(_, v)| v.clone());
                    if let Some(v) = v {
                        ins.push(format!("--env={k}={v}"));
                    }
                }
                words.splice(run + 1..run + 1, ins);
            }
            let (pre, gs_word, user_flags, post) = match self.chain.gs {
                Some(g) => (
                    words[..g.at].to_vec(),
                    Some(words[g.at].clone()),
                    words[g.flags()].to_vec(),
                    words[g.cmd.min(words.len())..].to_vec(),
                ),
                None => (vec![], None, vec![], words),
            };
            argv = vec![];
            if let Some(w) = self.wrap.inhibit.take() {
                argv.extend(w);
            }
            if let Some(w) = self.wrap.perf.take() {
                argv.extend(w);
            }
            argv.extend(pre);
            if self.gs_active {
                argv.push(gs_word.unwrap_or_else(|| "gamescope".into()));
                argv.extend(self.gs_words());
                argv.extend(user_flags);
                argv.push("--".into());
                if !scoped.is_empty() {
                    argv.push("env".into());
                    argv.extend(scoped.iter().map(|(k, v)| format!("{k}={v}")));
                }
            }
            if self.wrap.gamemode {
                argv.push("gamemoderun".into());
            }
            if self.wrap.mangohud && !self.gs_active {
                argv.push("mangohud".into());
            }
            if self.wrap.obs {
                argv.push("obs-gamecapture".into());
            }
            argv.extend(post);
        } else {
            argv = self.chain.words.clone();
        }
        argv.extend(input.user_suffix.iter().cloned());
        self.kind_notes();
        let program = argv.remove(0);
        let core_program =
            self.chain.words.get(self.chain.core_at).cloned().filter(|_| input.kind == LaunchKind::Process);
        let mut report = std::mem::take(&mut self.report);
        report.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        ComposeOutput {
            program,
            args: argv,
            env,
            notes: self.notes,
            report,
            steam_string: None,
            make_dirs: self.make_dirs,
            core,
            core_program,
        }
    }

    /// The launcher / client / Flatpak notes (spec §2.7).
    fn kind_notes(&mut self) {
        if !self.preload_labels.is_empty() {
            let n = NOTE_FLATPAK_PRELOAD.replace("{list}", &join(&self.preload_labels));
            self.note(n);
        }
        let name = self.chain.launcher.clone().unwrap_or_default();
        let (envs, wraps) = (join(&self.env_labels), join(&self.wrapper_labels));
        match self.core {
            Core::Launcher if !envs.is_empty() || !wraps.is_empty() => {
                let mut n = NOTE_LAUNCHER.to_string();
                if envs.is_empty() {
                    n = n.replace("{env_list} are passed to it but may not reach the game; ", "");
                }
                if wraps.is_empty() {
                    n = n.replace("; {wrapper_list} aren't added", "");
                }
                let direct = if name == "Faugus" { " or \"Run directly\"" } else { "" };
                n = n
                    .replace("{launcher}", &name)
                    .replace("{env_list}", &envs)
                    .replace("{wrapper_list}", &wraps)
                    .replace("{or_run_directly}", direct);
                if name == "Lutris" {
                    n.push_str(NOTE_LUTRIS_OPEN);
                }
                self.note(n);
            }
            Core::Client if !envs.is_empty() || !wraps.is_empty() => {
                let mut n = NOTE_CLIENT.to_string();
                if wraps.is_empty() {
                    n = n.replace(" Not added for store-client games: {wrapper_list}.", "");
                }
                let n = n.replace("{client}", &name).replace("{wrapper_list}", &wraps);
                self.note(n);
            }
            _ => {}
        }
    }
}

/// "Heroic" for `heroic://`, else the URL's scheme.
fn url_handler(url: Option<&str>) -> String {
    let u = url.unwrap_or("").trim();
    if u.to_ascii_lowercase().starts_with("heroic://") {
        return "Heroic".into();
    }
    match u.split_once(':') {
        Some((s, _)) if !s.is_empty() => s.to_string(),
        _ => "The URL handler".into(),
    }
}

/// `words` set gamescope flag `name` (`-f`, `--framerate-limit`, `--framerate-limit=60`).
fn has_gs_flag(words: &[String], names: &[&str]) -> bool {
    words.iter().any(|w| {
        names.iter().any(|n| w == n || (n.starts_with("--") && w.strip_prefix(n).is_some_and(|r| r.starts_with('='))))
    })
}

/// A GPU vendor's name for notes.
fn vendor_name(v: u16) -> &'static str {
    match v {
        0x1002 => "AMD",
        0x10de => "NVIDIA",
        0x8086 => "Intel",
        _ => "other",
    }
}

/// `p` percent of `n`, rounded (spec §2.1 internal size), at least 1.
fn percent(n: u32, p: u8) -> u32 {
    ((u64::from(n) * u64::from(p) + 50) / 100).clamp(1, u64::from(u32::MAX)) as u32
}

/// The Steam launch-options string (spec §3.4): the composer for a Proton game with an Unknown
/// runner around `%command%`. None when nothing would be added; also the folders it needs.
fn steam_string(input: &ComposeInput) -> (Option<String>, Vec<PathBuf>) {
    let probe = ComposeInput {
        program: "%command%".into(),
        args: vec![],
        base_env: vec![],
        custom_command: false,
        user_env: vec![],
        user_prefix: vec![],
        user_suffix: vec![],
        kind: LaunchKind::Process,
        play: true,
        url: None,
        self_exe: None,
        probe_runner: None,
        probe_wine: None,
        ..input.clone()
    };
    let out = compose(&probe, Some(Core::Proton));
    let mut words: Vec<String> =
        out.env.iter().map(|(k, v)| format!("{k}={}", crate::launch::shell_quote(v))).collect();
    words.push(crate::launch::shell_quote(&out.program));
    words.extend(out.args.iter().map(|a| crate::launch::shell_quote(a)));
    let s = words.join(" ");
    ((s != "%command%").then_some(s), out.make_dirs)
}

fn set_env(env: &mut Vec<(String, String)>, k: &str, v: &str) {
    match env.iter_mut().find(|(ek, _)| ek == k) {
        Some(e) => e.1 = v.to_string(),
        None => env.push((k.to_string(), v.to_string())),
    }
}

/// Append the `items` whose key (text before `=`) isn't already in the list `cur`.
fn merge_list(cur: Option<&str>, items: &[&str], sep: char) -> String {
    let key = |s: &str| s.split('=').next().unwrap_or("").trim().to_string();
    let mut parts: Vec<String> =
        cur.unwrap_or("").split(sep).filter(|p| !p.trim().is_empty()).map(str::to_string).collect();
    for it in items {
        if !parts.iter().any(|p| key(p) == key(it)) {
            parts.push(it.to_string());
        }
    }
    parts.join(&sep.to_string())
}
