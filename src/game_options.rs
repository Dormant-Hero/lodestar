//! Game options (full spec in docs/game-options-spec.md): toggles instead of typing
//! launch options. Global defaults live in `Config::game_options`, per-game overrides in
//! `UserEdits::game_options`; both are a [`GameOptions`] where every field's Default / None means
//! "inherit" (per game) or "Lodestar adds nothing" (global).
//!
//! Layout:
//! * this file — the data model, option metadata for the UI, the report types and the
//!   process-wide [`LaunchEnv`] (defaults + detected capabilities + monitor size) that
//!   `launch::resolve` reads;
//! * `game_options/caps.rs` (`caps`) — cheap availability detection ([`Caps`], [`RunnerCaps`]);
//! * `game_options/compose.rs` (`compose`) — turning effective options into env / wrappers in the
//!   right order around a resolved command (spec §2, §3);
//! * `game_options/tests.rs` — the spec §7 cases.
//!
//! CONTRACT (used by launch.rs and the UI):
//!   `effective(global, game) -> Effective`;
//!   `compose::apply(ComposeInput) -> ComposeOutput` (pure; see that module), `compose::classify(words,
//!   env, caps, self_exe) -> Chain` (the launch kind after peeling wrappers);
//!   `launch_env() -> Arc<LaunchEnv>`, `set_defaults(GameOptions)`, `set_caps(Caps)`,
//!   `set_monitor(Option<(u32, u32)>)`;
//!   `caps::probe() -> Caps` (blocking, a few ms + one cached `gamescope --help`; run it off the UI
//!   thread), `caps::runner_caps(dir) -> Arc<RunnerCaps>` (cached per dir; cheap after the first call),
//!   `caps::install_hint(id, distro)`;
//!   `compose::proton_dir_for(env, program, caps) -> Option<PathBuf>` (pure).

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

pub mod caps;
pub mod compose;
pub mod gamescope;
#[cfg(test)]
pub(crate) mod tests;

// ------------------------------------------------------------------------------------- values

/// Default / On / Off. Per game "Default" follows the global value; globally it means "Lodestar
/// adds nothing (the tool / runner decides)".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tri {
    #[default]
    Default,
    On,
    Off,
}

impl Tri {
    /// `self` unless it is Default, then `fallback`.
    pub fn or(self, fallback: Tri) -> Tri {
        if self == Tri::Default { fallback } else { self }
    }
    pub fn is_on(self) -> bool {
        self == Tri::On
    }
    pub fn is_off(self) -> bool {
        self == Tri::Off
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowMode {
    #[default]
    Fullscreen,
    Borderless,
    Windowed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Res {
    /// The monitor Lodestar's window is on.
    #[default]
    Auto,
    Custom {
        w: u32,
        h: u32,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InternalRes {
    #[default]
    Native,
    /// Percent of the output size (77 / 67 / 58 / 50 are the FSR quality presets).
    Percent(u8),
    Custom {
        w: u32,
        h: u32,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GsFilter {
    /// FSR when the game renders below the output size, else gamescope's default.
    #[default]
    Auto,
    Linear,
    Nearest,
    Fsr,
    Nis,
    Sgsr,
    Pixel,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GsScaler {
    #[default]
    Auto,
    Fit,
    Integer,
    Fill,
    Stretch,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Nvapi {
    /// On with NVIDIA drivers (Proton's own choice).
    #[default]
    Default,
    Off,
    Force,
}

/// One extra gamescope flag ("More Gamescope options"): switched on, given a value, or — per
/// game / library — switched off again when a broader level turned it on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlagSetting {
    On,
    Off,
    Value(String),
}

/// Global defaults (Config) and per-game overrides (UserEdits) alike. Every field's Default /
/// None = inherit (per game) or built-in default (global). Old configs / rows load (serde default).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameOptions {
    // Display
    pub wayland: Tri,
    pub hdr: Tri,
    pub hdr_force: Tri,
    pub gamescope: Tri,
    pub gs_window: Option<WindowMode>,
    pub gs_output: Option<Res>,
    pub gs_internal: Option<InternalRes>,
    pub gs_filter: Option<GsFilter>,
    pub gs_sharpness: Option<u8>,
    pub gs_scaler: Option<GsScaler>,
    pub gs_adaptive_sync: Tri,
    pub gs_grab_cursor: Tri,
    pub gs_expose_wayland: Tri,
    pub gs_disable_wsi: Tri,
    /// Extra gamescope words (shell-like); the game's value replaces the global one.
    pub gs_extra_args: Option<String>,
    /// More Gamescope options by long flag (`--hdr-itm-enabled`, `--rt` …); merged per flag.
    pub gs_more: BTreeMap<String, FlagSetting>,
    // Performance
    pub gamemode: Tri,
    pub performance_profile: Tri,
    pub keep_awake: Tri,
    /// FPS cap; per game `Some(0)` = no limit even when the global default sets one.
    pub frame_limit: Option<u32>,
    pub prefer_dgpu: Tri,
    pub ntsync: Tri,
    // Upscaling & frame generation
    pub wine_fsr: Tri,
    pub wine_fsr_strength: Option<u8>,
    pub fsr4_upgrade: Tri,
    pub dlss_upgrade: Tri,
    pub xess_upgrade: Tri,
    pub upscaler_indicator: Tri,
    pub lsfg: Tri,
    /// A profile name from lsfg-vk's conf.toml; `Some("")` = "Lodestar settings" (multiplier …).
    pub lsfg_profile: Option<String>,
    pub lsfg_multiplier: Option<u8>,
    pub lsfg_flow_scale: Option<f32>,
    pub lsfg_performance: Tri,
    // Overlay & capture
    pub mangohud: Tri,
    pub obs_capture: Tri,
    // Compatibility
    pub nvapi: Nvapi,
    pub sdl_input: Tri,
    pub audio_latency: Tri,
    // Advanced
    pub proton_log: Tri,
    pub hdr_wsi_layer: Tri,
    /// Other implicit Vulkan layers by `layer.name` (vkBasalt …).
    pub layers: BTreeMap<String, Tri>,
}

impl GameOptions {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// `upper`'s non-default fields over `self` (a library's options under a game's own):
    /// the layer a game's "Default" follows. Layers merge per layer name.
    pub fn overlay(&self, upper: &GameOptions) -> GameOptions {
        if upper.is_default() {
            return self.clone();
        }
        if self.is_default() {
            return upper.clone();
        }
        let (
            Ok(serde_json::Value::Object(mut lower)),
            Ok(serde_json::Value::Object(up)),
            Ok(serde_json::Value::Object(def)),
        ) = (serde_json::to_value(self), serde_json::to_value(upper), serde_json::to_value(GameOptions::default()))
        else {
            return upper.clone();
        };
        for (k, v) in up {
            if k == "layers" || k == "gs_more" || def.get(&k) == Some(&v) {
                continue;
            }
            lower.insert(k, v);
        }
        let mut out: GameOptions =
            serde_json::from_value(serde_json::Value::Object(lower)).unwrap_or_else(|_| upper.clone());
        for (name, v) in &upper.layers {
            if *v != Tri::Default {
                out.layers.insert(name.clone(), *v);
            }
        }
        for (flag, v) in &upper.gs_more {
            out.gs_more.insert(flag.clone(), v.clone());
        }
        out
    }
}

// ------------------------------------------------------------------------ lenient loading

/// Deserialize any JSON-shaped `value` as `T`, keeping every top-level field that is valid on
/// its own and dropping only the invalid ones (an unknown enum variant from a newer build, a
/// hand-edited `300` in a `u8` …). Map-valued fields (`gs_more`, `layers`) are filtered per
/// entry and list fields (`UserEdits::actions`) per item, so one bad flag or action drops only
/// itself. Returns the value and what was dropped, as
/// `path = value` strings (`path` is `prefix` + the dotted key). A `value` that isn't an object
/// at all gives `T::default()` and one entry. `T` must ignore unknown fields (no
/// `deny_unknown_fields`) and default missing ones (`#[serde(default)]`).
pub fn lenient_from_value<T>(value: serde_json::Value, prefix: &str) -> (T, Vec<String>)
where
    T: serde::de::DeserializeOwned + Default,
{
    use serde_json::{Map, Value};
    if let Ok(t) = T::deserialize(&value) {
        return (t, vec![]);
    }
    let key = |k: &str| if prefix.is_empty() { k.to_string() } else { format!("{prefix}.{k}") };
    let Value::Object(fields) = value else {
        return (T::default(), vec![format!("{prefix} = {value} (not a table)")]);
    };
    let parses =
        |k: &str, v: &Value| T::deserialize(&Value::Object(Map::from_iter([(k.to_string(), v.clone())]))).is_ok();
    let mut dropped = vec![];
    let mut good = Map::new();
    for (k, v) in fields {
        if parses(&k, &v) {
            good.insert(k, v);
            continue;
        }
        // A map field: keep the entries that are valid on their own.
        if let Value::Object(entries) = &v {
            let (ok, bad): (Map<String, Value>, Map<String, Value>) = entries
                .iter()
                .map(|(ek, ev)| (ek.clone(), ev.clone()))
                .partition(|(ek, ev)| parses(&k, &Value::Object(Map::from_iter([(ek.clone(), ev.clone())]))));
            let ok = Value::Object(ok);
            if !bad.is_empty() && ok.as_object().is_some_and(|m| !m.is_empty()) && parses(&k, &ok) {
                dropped.extend(bad.iter().map(|(ek, ev)| format!("{}.{ek} = {ev}", key(&k))));
                good.insert(k, ok);
                continue;
            }
        }
        // A list field (`actions`, `links` …): keep the items that are valid on their own.
        if let Value::Array(items) = &v {
            let valid: Vec<bool> = items.iter().map(|it| parses(&k, &Value::Array(vec![it.clone()]))).collect();
            let bad: Vec<(usize, &Value)> = items.iter().enumerate().filter(|(i, _)| !valid[*i]).collect();
            let ok = Value::Array(items.iter().zip(&valid).filter(|(_, v)| **v).map(|(it, _)| it.clone()).collect());
            if !bad.is_empty() && ok.as_array().is_some_and(|a| !a.is_empty()) && parses(&k, &ok) {
                dropped.extend(bad.iter().map(|(i, it)| format!("{}[{i}] = {it}", key(&k))));
                good.insert(k, ok);
                continue;
            }
        }
        dropped.push(format!("{} = {v}", key(&k)));
    }
    let t = T::deserialize(&Value::Object(good)).unwrap_or_default();
    (t, dropped)
}

/// `#[serde(deserialize_with)]` for a [`GameOptions`] field (`UserEdits::game_options`,
/// `Config::game_options`): reads any value and keeps every valid option ([`lenient_from_value`]),
/// logging what it dropped instead of failing the whole row / config. Works for JSON and TOML.
pub fn deserialize_lenient<'de, D: serde::Deserializer<'de>>(d: D) -> Result<GameOptions, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    let (opts, dropped) = lenient_from_value(v, "game_options");
    if !dropped.is_empty() {
        log::warn!("game options: ignored invalid {}", dropped.join(", "));
    }
    Ok(opts)
}

/// [`deserialize_lenient`] for a map of options (`Config::game_options_by_source`): each value
/// on its own, so one library's bad option never drops another library's.
pub fn deserialize_lenient_map<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, GameOptions>, D::Error> {
    let m = BTreeMap::<String, serde_json::Value>::deserialize(d)?;
    Ok(m.into_iter()
        .map(|(k, v)| {
            let (opts, dropped) = lenient_from_value(v, &k);
            if !dropped.is_empty() {
                log::warn!("game options for {k}: ignored invalid {}", dropped.join(", "));
            }
            (k, opts)
        })
        .collect())
}

/// The options a game's "Default" follows: the library's (Settings → Games → Applies to) over
/// the global ones.
pub fn defaults_for(env: &LaunchEnv, source_key: &str) -> GameOptions {
    match env.by_source.get(source_key) {
        Some(lib) => env.defaults.overlay(lib),
        None => env.defaults.clone(),
    }
}

/// Fully merged values: per game, else global, else built-in.
#[derive(Clone, Debug, PartialEq)]
pub struct Effective {
    pub wayland: Tri,
    /// The game's own wayland value (native games honour only a per-game setting).
    pub wayland_per_game: Tri,
    pub hdr: Tri,
    /// The game's own HDR value (the "native games switch HDR on themselves" note is only shown
    /// for a per-game On).
    pub hdr_per_game: Tri,
    pub hdr_force: Tri,
    pub gamescope: Tri,
    pub gs_window: WindowMode,
    pub gs_output: Res,
    pub gs_internal: InternalRes,
    pub gs_filter: GsFilter,
    pub gs_sharpness: Option<u8>,
    pub gs_scaler: GsScaler,
    pub gs_adaptive_sync: Tri,
    pub gs_grab_cursor: Tri,
    pub gs_expose_wayland: Tri,
    pub gs_disable_wsi: Tri,
    pub gs_extra_args: Option<String>,
    /// More Gamescope options in effect: (flag, value) in flag order.
    pub gs_more: Vec<(String, Option<String>)>,
    pub gamemode: Tri,
    pub performance_profile: Tri,
    pub keep_awake: Tri,
    /// None or Some(0) = no limit.
    pub frame_limit: Option<u32>,
    pub prefer_dgpu: Tri,
    pub ntsync: Tri,
    pub wine_fsr: Tri,
    pub wine_fsr_strength: Option<u8>,
    pub fsr4_upgrade: Tri,
    pub dlss_upgrade: Tri,
    pub xess_upgrade: Tri,
    pub upscaler_indicator: Tri,
    pub lsfg: Tri,
    pub lsfg_profile: Option<String>,
    pub lsfg_multiplier: u8,
    pub lsfg_flow_scale: Option<f32>,
    pub lsfg_performance: Tri,
    pub mangohud: Tri,
    pub obs_capture: Tri,
    pub nvapi: Nvapi,
    pub sdl_input: Tri,
    pub audio_latency: Tri,
    pub proton_log: Tri,
    pub hdr_wsi_layer: Tri,
    pub layers: BTreeMap<String, Tri>,
}

/// Merge a game's overrides over the global defaults (spec §5).
pub fn effective(global: &GameOptions, game: &GameOptions) -> Effective {
    let mut layers = global.layers.clone();
    for (k, v) in &game.layers {
        if *v != Tri::Default {
            layers.insert(k.clone(), *v);
        }
    }
    layers.retain(|_, v| *v != Tri::Default);
    let mut more = global.gs_more.clone();
    more.extend(game.gs_more.iter().map(|(k, v)| (k.clone(), v.clone())));
    let gs_more = more
        .into_iter()
        .filter_map(|(flag, v)| match v {
            FlagSetting::On => Some((flag, None)),
            FlagSetting::Value(x) if !x.trim().is_empty() => Some((flag, Some(x.trim().to_string()))),
            _ => None,
        })
        .collect();
    Effective {
        wayland: game.wayland.or(global.wayland),
        wayland_per_game: game.wayland,
        hdr: game.hdr.or(global.hdr),
        hdr_per_game: game.hdr,
        hdr_force: game.hdr_force.or(global.hdr_force),
        gamescope: game.gamescope.or(global.gamescope),
        gs_window: game.gs_window.or(global.gs_window).unwrap_or_default(),
        gs_output: game.gs_output.or(global.gs_output).unwrap_or_default(),
        gs_internal: game.gs_internal.or(global.gs_internal).unwrap_or_default(),
        gs_filter: game.gs_filter.or(global.gs_filter).unwrap_or_default(),
        gs_sharpness: game.gs_sharpness.or(global.gs_sharpness),
        gs_scaler: game.gs_scaler.or(global.gs_scaler).unwrap_or_default(),
        gs_adaptive_sync: game.gs_adaptive_sync.or(global.gs_adaptive_sync),
        gs_grab_cursor: game.gs_grab_cursor.or(global.gs_grab_cursor),
        gs_expose_wayland: game.gs_expose_wayland.or(global.gs_expose_wayland),
        gs_disable_wsi: game.gs_disable_wsi.or(global.gs_disable_wsi),
        gs_extra_args: game
            .gs_extra_args
            .clone()
            .or_else(|| global.gs_extra_args.clone())
            .filter(|s| !s.trim().is_empty()),
        gs_more,
        gamemode: game.gamemode.or(global.gamemode),
        performance_profile: game.performance_profile.or(global.performance_profile),
        keep_awake: game.keep_awake.or(global.keep_awake),
        frame_limit: game.frame_limit.or(global.frame_limit).filter(|n| *n > 0),
        prefer_dgpu: game.prefer_dgpu.or(global.prefer_dgpu),
        ntsync: game.ntsync.or(global.ntsync),
        wine_fsr: game.wine_fsr.or(global.wine_fsr),
        wine_fsr_strength: game.wine_fsr_strength.or(global.wine_fsr_strength),
        fsr4_upgrade: game.fsr4_upgrade.or(global.fsr4_upgrade),
        dlss_upgrade: game.dlss_upgrade.or(global.dlss_upgrade),
        xess_upgrade: game.xess_upgrade.or(global.xess_upgrade),
        upscaler_indicator: game.upscaler_indicator.or(global.upscaler_indicator),
        lsfg: game.lsfg.or(global.lsfg),
        lsfg_profile: game.lsfg_profile.clone().or_else(|| global.lsfg_profile.clone()),
        lsfg_multiplier: game.lsfg_multiplier.or(global.lsfg_multiplier).unwrap_or(2).clamp(2, 4),
        lsfg_flow_scale: game.lsfg_flow_scale.or(global.lsfg_flow_scale),
        lsfg_performance: game.lsfg_performance.or(global.lsfg_performance),
        mangohud: game.mangohud.or(global.mangohud),
        obs_capture: game.obs_capture.or(global.obs_capture),
        nvapi: if game.nvapi == Nvapi::Default { global.nvapi } else { game.nvapi },
        sdl_input: game.sdl_input.or(global.sdl_input),
        audio_latency: game.audio_latency.or(global.audio_latency),
        proton_log: game.proton_log.or(global.proton_log),
        hdr_wsi_layer: game.hdr_wsi_layer.or(global.hdr_wsi_layer),
        layers,
    }
}

// ------------------------------------------------------------------------------ option metadata

/// Every option row the UI shows (sub-settings of gamescope / lsfg / wine FSR included).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OptionId {
    Wayland,
    Hdr,
    HdrForce,
    Gamescope,
    GsWindow,
    GsOutput,
    GsInternal,
    GsFilter,
    GsSharpness,
    GsScaler,
    GsAdaptiveSync,
    GsGrabCursor,
    GsExposeWayland,
    GsDisableWsi,
    GsMore,
    GsExtraArgs,
    Gamemode,
    PerformanceProfile,
    KeepAwake,
    FrameLimit,
    PreferDgpu,
    Ntsync,
    WineFsr,
    WineFsrStrength,
    Fsr4Upgrade,
    DlssUpgrade,
    XessUpgrade,
    UpscalerIndicator,
    Lsfg,
    LsfgProfile,
    LsfgMultiplier,
    LsfgFlowScale,
    LsfgPerformance,
    Mangohud,
    ObsCapture,
    Nvapi,
    SdlInput,
    AudioLatency,
    ProtonLog,
    HdrWsiLayer,
    /// One detected Vulkan layer (by `layer.name`).
    Layer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Group {
    Display,
    Performance,
    Upscaling,
    Overlay,
    Compatibility,
    Advanced,
}

impl Group {
    pub const ALL: [Group; 6] =
        [Group::Display, Group::Performance, Group::Upscaling, Group::Overlay, Group::Compatibility, Group::Advanced];
    pub fn label(self) -> &'static str {
        match self {
            Group::Display => "Display",
            Group::Performance => "Performance",
            Group::Upscaling => "Upscaling & frame generation",
            Group::Overlay => "Overlay & capture",
            Group::Compatibility => "Compatibility",
            Group::Advanced => "Advanced",
        }
    }
}

/// How an option row is edited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Tri,
    WindowMode,
    Res,
    InternalRes,
    GsFilter,
    GsScaler,
    Nvapi,
    /// Number with an inclusive range.
    Number {
        min: u32,
        max: u32,
    },
    /// Fraction (lsfg flow scale).
    Fraction,
    Text,
    LsfgProfile,
    /// The "More Gamescope options" picker.
    GsMore,
}

#[derive(Clone, Copy, Debug)]
pub struct OptionMeta {
    pub id: OptionId,
    pub group: Group,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: Kind,
    /// Shown under the "Advanced" fold of its group.
    pub advanced: bool,
    /// Shown only while this option is effectively On.
    pub parent: Option<OptionId>,
}

const fn m(
    id: OptionId,
    group: Group,
    label: &'static str,
    help: &'static str,
    kind: Kind,
    advanced: bool,
    parent: Option<OptionId>,
) -> OptionMeta {
    OptionMeta { id, group, label, help, kind, advanced, parent }
}

use Group as G;
use OptionId as O;

/// Every row, in display (and env emission) order. Layers are listed separately (detected).
pub const OPTIONS: &[OptionMeta] = &[
    m(
        O::Wayland,
        G::Display,
        "Wayland native",
        "Run Windows games on Wine's Wayland driver instead of XWayland (GE-Proton or proton-cachyos). Native games: SDL games only, and only when set for that game.",
        Kind::Tri,
        false,
        None,
    ),
    m(
        O::Hdr,
        G::Display,
        "HDR",
        "Let the game output HDR. Needs HDR on in System Settings → Display, and Wayland native or Gamescope (Lodestar picks one).",
        Kind::Tri,
        false,
        None,
    ),
    m(
        O::HdrForce,
        G::Display,
        "Force HDR (GE-Proton)",
        "For KDE setups where the game doesn't detect HDR. HDR monitors only.",
        Kind::Tri,
        true,
        Some(O::Hdr),
    ),
    m(
        O::Gamescope,
        G::Display,
        "Gamescope",
        "Run the game inside a Gamescope window: upscaling, frame limiting, HDR and a fixed resolution.",
        Kind::Tri,
        false,
        None,
    ),
    m(
        O::GsWindow,
        G::Display,
        "Window mode",
        "Fullscreen, borderless window or a normal window.",
        Kind::WindowMode,
        false,
        Some(O::Gamescope),
    ),
    m(
        O::GsOutput,
        G::Display,
        "Output resolution",
        "Auto = the monitor Lodestar is on.",
        Kind::Res,
        false,
        Some(O::Gamescope),
    ),
    m(
        O::GsInternal,
        G::Display,
        "Game resolution",
        "What the game renders at; Gamescope upscales it to the output resolution.",
        Kind::InternalRes,
        false,
        Some(O::Gamescope),
    ),
    m(
        O::GsFilter,
        G::Display,
        "Upscaler",
        "Auto = FSR when the game renders below the output resolution.",
        Kind::GsFilter,
        false,
        Some(O::Gamescope),
    ),
    m(
        O::GsSharpness,
        G::Display,
        "Sharpness",
        "0 = sharpest, 20 = softest (Gamescope's default is 2). FSR, NIS and SGSR only.",
        Kind::Number { min: 0, max: 20 },
        true,
        Some(O::Gamescope),
    ),
    m(O::GsScaler, G::Display, "Scaling", "How the picture fits the screen.", Kind::GsScaler, true, Some(O::Gamescope)),
    m(
        O::GsAdaptiveSync,
        G::Display,
        "Adaptive sync (VRR)",
        "Also set Adaptive sync to Automatic in System Settings → Display.",
        Kind::Tri,
        false,
        Some(O::Gamescope),
    ),
    m(
        O::GsGrabCursor,
        G::Display,
        "Lock mouse to the game",
        "For games whose camera spins or whose cursor escapes the window.",
        Kind::Tri,
        true,
        Some(O::Gamescope),
    ),
    m(
        O::GsExposeWayland,
        G::Display,
        "Expose Wayland",
        "Let Wayland-native games connect to Gamescope (HDR usually works better without).",
        Kind::Tri,
        true,
        Some(O::Gamescope),
    ),
    m(
        O::GsDisableWsi,
        G::Display,
        "Disable Gamescope WSI layer",
        "NVIDIA workaround for crashes with upscalers. Turns HDR off.",
        Kind::Tri,
        true,
        Some(O::Gamescope),
    ),
    m(
        O::GsMore,
        G::Display,
        "More Gamescope options",
        "Everything else your Gamescope can do: SDR → HDR, SDR brightness, realtime scheduling, background FPS, mouse sensitivity, ReShade …",
        Kind::GsMore,
        false,
        Some(O::Gamescope),
    ),
    m(
        O::GsExtraArgs,
        G::Display,
        "Extra Gamescope arguments",
        "Added after Lodestar's flags, so these win.",
        Kind::Text,
        true,
        Some(O::Gamescope),
    ),
    m(
        O::Gamemode,
        G::Performance,
        "GameMode",
        "Feral GameMode: CPU governor, I/O priority and screensaver inhibit while playing.",
        Kind::Tri,
        false,
        None,
    ),
    m(
        O::PerformanceProfile,
        G::Performance,
        "Performance power profile",
        "Switch the power profile to Performance while playing (CachyOS game-performance or powerprofilesctl).",
        Kind::Tri,
        false,
        None,
    ),
    m(
        O::KeepAwake,
        G::Performance,
        "Keep screen awake",
        "Stop the screen dimming, locking or the PC suspending while playing (handy with a controller).",
        Kind::Tri,
        false,
        None,
    ),
    m(
        O::FrameLimit,
        G::Performance,
        "Frame limit",
        "FPS cap through Gamescope, MangoHud or DXVK, whichever applies. 0 = no limit.",
        Kind::Number { min: 0, max: 1000 },
        false,
        None,
    ),
    m(
        O::PreferDgpu,
        G::Performance,
        "Use discrete GPU",
        "Render on the discrete GPU on laptops / hybrid systems.",
        Kind::Tri,
        false,
        None,
    ),
    m(
        O::Ntsync,
        G::Performance,
        "NTSync",
        "Fast Windows sync primitives. Current Proton uses it automatically when the kernel has it; Off disables it for this game.",
        Kind::Tri,
        true,
        None,
    ),
    m(
        O::WineFsr,
        G::Upscaling,
        "Wine fullscreen FSR",
        "Pick a fullscreen resolution below your monitor's in the game; Wine upscales it with FSR 1 (GE-Proton / cachyos, X11 only).",
        Kind::Tri,
        false,
        None,
    ),
    m(
        O::WineFsrStrength,
        G::Upscaling,
        "FSR sharpness",
        "0 = sharpest … 5.",
        Kind::Number { min: 0, max: 5 },
        true,
        Some(O::WineFsr),
    ),
    m(
        O::Fsr4Upgrade,
        G::Upscaling,
        "FSR 4 upgrade",
        "Use FSR 4 in games with FSR 3.1 (RDNA4; experimental on RDNA3).",
        Kind::Tri,
        false,
        None,
    ),
    m(O::DlssUpgrade, G::Upscaling, "DLSS upgrade", "Use the newest DLSS (NVIDIA RTX).", Kind::Tri, false, None),
    m(O::XessUpgrade, G::Upscaling, "XeSS upgrade", "Use the newest XeSS.", Kind::Tri, true, None),
    m(
        O::UpscalerIndicator,
        G::Upscaling,
        "Upscaler indicator",
        "On-screen FSR 4 / DLSS indicator, to check that an upgrade worked.",
        Kind::Tri,
        true,
        None,
    ),
    m(
        O::Lsfg,
        G::Upscaling,
        "Frame generation (lsfg-vk)",
        "Lossless Scaling frame generation. Needs lsfg-vk and Lossless Scaling (Steam).",
        Kind::Tri,
        false,
        None,
    ),
    m(
        O::LsfgProfile,
        G::Upscaling,
        "lsfg-vk profile",
        "A profile from lsfg-vk's own config, or Lodestar's settings below.",
        Kind::LsfgProfile,
        false,
        Some(O::Lsfg),
    ),
    m(
        O::LsfgMultiplier,
        G::Upscaling,
        "Multiplier",
        "2×, 3× or 4× frames.",
        Kind::Number { min: 2, max: 4 },
        false,
        Some(O::Lsfg),
    ),
    m(O::LsfgFlowScale, G::Upscaling, "Flow scale", "0.25–1.0; lower is faster.", Kind::Fraction, true, Some(O::Lsfg)),
    m(
        O::LsfgPerformance,
        G::Upscaling,
        "Performance mode",
        "Lighter frame generation model.",
        Kind::Tri,
        true,
        Some(O::Lsfg),
    ),
    m(
        O::Mangohud,
        G::Overlay,
        "MangoHud",
        "Performance overlay (FPS, frame times, temperatures). Inside Gamescope it uses mangoapp.",
        Kind::Tri,
        false,
        None,
    ),
    m(
        O::ObsCapture,
        G::Overlay,
        "OBS game capture",
        "Lets OBS's Game Capture source (obs-vkcapture) see the game.",
        Kind::Tri,
        false,
        None,
    ),
    m(O::Nvapi, G::Compatibility, "NVAPI (DLSS, Reflex)", "Auto = on with NVIDIA drivers.", Kind::Nvapi, false, None),
    m(
        O::SdlInput,
        G::Compatibility,
        "Use SDL for controllers",
        "Fixes some controller problems (GE-Proton / cachyos).",
        Kind::Tri,
        true,
        None,
    ),
    m(
        O::AudioLatency,
        G::Compatibility,
        "Audio crackling fix",
        "Bigger audio buffer (60 ms) for Wine / PulseAudio games.",
        Kind::Tri,
        false,
        None,
    ),
    m(
        O::ProtonLog,
        G::Advanced,
        "Proton log",
        "Write a Proton log for this game into Lodestar's log folder.",
        Kind::Tri,
        false,
        None,
    ),
    m(
        O::HdrWsiLayer,
        G::Advanced,
        "HDR WSI layer (legacy)",
        "Only for NVIDIA drivers older than 595 without Gamescope; Default picks it when needed.",
        Kind::Tri,
        false,
        None,
    ),
];

#[cfg(test)]
pub fn meta(id: OptionId) -> &'static OptionMeta {
    OPTIONS.iter().find(|o| o.id == id).unwrap_or(&OPTIONS[0])
}

// ---------------------------------------------------------------------------------- the report

/// What happened to one option in one resolved launch (chips on the options page; tests).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptionState {
    Applied,
    /// Applied because another option needs it (HDR turning on Wayland …).
    AppliedImplicitly(&'static str),
    /// Wanted but not applied here, with the reason.
    Suppressed(String),
    /// Means nothing for this kind of launch (NTSync for a native game).
    NotApplicable,
    /// The tool / runner / hardware isn't there, with the reason (and install hint).
    Unavailable(String),
}

// ------------------------------------------------------------------------------------ notes

// Kind notes (spec §2.7). `{…}` are filled in by `compose`; a clause whose list is empty is left
// out. `{list}` names only options whose effective value differs from the built-in Default.

/// Steam / Steam shortcut launches.
pub const NOTE_STEAM: &str = "Not applied — the Steam client starts this game: {list}. In Steam, set the game's launch options to: `{steam_string}`.";
/// URL launches (`heroic://` …); `{handler}` is "Heroic" for `heroic://`, else the URL scheme.
pub const NOTE_URL: &str = "Not applied — {handler} starts this game with its own settings: {list}.";
/// Launcher CLIs (Lutris, Heroic, Faugus …).
pub const NOTE_LAUNCHER: &str = "{launcher} starts this game: {env_list} are passed to it but may not reach the game; {wrapper_list} aren't added. Use {launcher}'s own settings{or_run_directly}.";
/// Added to [`NOTE_LAUNCHER`] for Lutris (it forwards to a running instance without our env).
pub const NOTE_LUTRIS_OPEN: &str = " Nothing reaches the game while Lutris is already open.";
/// Store-client helper launches (`@lodestar --client-launch …`).
pub const NOTE_CLIENT: &str = "Game options apply only when Lodestar starts {client} itself — close {client} first. Not added for store-client games: {wrapper_list}.";
/// Flatpak apps and LD_PRELOAD wrappers (GameMode).
pub const NOTE_FLATPAK_PRELOAD: &str = "{list} can't reach a Flatpak app (Flatpak drops LD_PRELOAD).";

// ------------------------------------------------------------------------------ capabilities

/// What is installed (spec §4). Built by `caps::probe`; tests build it by hand.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Caps {
    /// A real probe filled this in (false: nothing detected yet — don't conclude "missing").
    pub probed: bool,
    pub bins: Bins,
    pub gamescope: Option<GamescopeCaps>,
    /// Implicit Vulkan layers found on the system.
    pub layers: Vec<LayerManifest>,
    pub lsfg: Option<LsfgCaps>,
    pub gpus: Vec<Gpu>,
    /// NVIDIA proprietary driver version ("595.12"), from /sys/module/nvidia/version.
    pub nvidia_version: Option<String>,
    pub ntsync: NtsyncState,
    pub wayland_session: bool,
    /// Lodestar itself runs inside gamescope (Steam Deck game mode, a gamescope session).
    pub nested_gamescope: bool,
    /// KDE output settings when readable (None = unknown).
    pub kde_hdr_enabled: Option<bool>,
    pub kde_vrr_never: Option<bool>,
    /// Connected outputs whose EDID advertises HDR with PQ, by connector name ("DP-2"; spec
    /// §4.1). For notes only, never a disable.
    pub hdr_outputs: Vec<String>,
    /// Flatpak extension id → installed branches (`org.freedesktop.Platform.VulkanLayer.MangoHud`).
    pub flatpak_ext: HashMap<String, Vec<String>>,
    pub distro: Distro,
    /// Preferred mode of the primary connected output, when the app didn't report the monitor.
    pub preferred_mode: Option<(u32, u32)>,
    /// Proton / runner dirs probed in the background pass.
    pub runners: HashMap<PathBuf, Arc<RunnerCaps>>,
}

/// Absolute paths of the helper programs (None = not installed). Hits inside Lodestar's own
/// AppImage mount ($APPDIR) are ignored.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bins {
    pub gamescope: Option<PathBuf>,
    pub mangohud: Option<PathBuf>,
    pub mangoapp: Option<PathBuf>,
    pub gamemoderun: Option<PathBuf>,
    pub game_performance: Option<PathBuf>,
    pub powerprofilesctl: Option<PathBuf>,
    pub tlpctl: Option<PathBuf>,
    pub obs_gamecapture: Option<PathBuf>,
    pub systemd_inhibit: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GamescopeCaps {
    /// Long options its `--help` lists (`--mangoapp`, `--hdr-enabled` …) plus short ones as `-W`.
    pub flags: HashSet<String>,
    /// Filters `-F` accepts (`linear`, `nearest`, `fsr`, `nis`, `pixel`, `sgsr` …).
    pub filters: Vec<String>,
    pub version: Option<(u32, u32, u32)>,
    /// Every option its `--help` documents, in help order (the "More Gamescope options" list).
    pub options: Vec<GsHelpOption>,
}

/// One option from `gamescope --help`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GsHelpOption {
    /// `--nested-unfocused-refresh`.
    pub long: String,
    /// `-o`.
    pub short: Option<String>,
    /// Its help text (continuation lines joined).
    pub help: String,
    /// The help section it's listed under ("Options", "Nested mode options", "Debug options" …).
    pub section: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayerManifest {
    /// `layer.name`.
    pub name: String,
    pub description: String,
    /// `enable_environment` (key, value), when the layer has one.
    pub enable_env: Option<(String, String)>,
    /// `disable_environment` key.
    pub disable_env: Option<String>,
    pub file: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LsfgCaps {
    /// 1 or 2.
    pub version: u8,
    /// Profiles from conf.toml: (name, active_in executables).
    pub profiles: Vec<(String, Vec<String>)>,
    /// Lossless.dll, when found in a Steam library.
    pub dll: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gpu {
    pub card: String,
    /// "0000:03:00.0".
    pub pci: String,
    pub vendor: u16,
    pub device: u16,
    /// Kernel driver ("amdgpu", "nvidia", "i915", "xe", "nouveau").
    pub driver: String,
    pub boot_vga: bool,
    pub vram: Option<u64>,
    /// AMD GC IP version (12 = RDNA4, 11 = RDNA3).
    pub gc_ip: Option<(u8, u8)>,
    /// udev `switcheroo-discrete-gpu` tag.
    pub discrete_tag: bool,
}

impl Caps {
    /// The GPU games render on without help: the boot (primary) GPU, else the first one.
    pub fn default_gpu(&self) -> Option<&Gpu> {
        self.gpus.iter().find(|g| g.boot_vga).or(self.gpus.first())
    }

    /// The strongest GPU, the one "Use discrete GPU" picks. One ranking for the UI and the
    /// launch alike: the udev `switcheroo-discrete-gpu` tag, then NVIDIA (proprietary or
    /// nouveau/NVK), then the most dedicated VRAM, then not the boot GPU (ties go to the other
    /// card, which on hybrid laptops is the discrete one).
    pub fn best_gpu(&self) -> Option<&Gpu> {
        self.gpus.iter().max_by_key(|g| {
            (
                g.discrete_tag,
                g.vendor == 0x10de,
                !g.likely_integrated(),
                g.vram.is_some(),
                g.vram.unwrap_or(0),
                !g.boot_vga,
            )
        })
    }

    /// The GPU a game renders on, for GPU-gated options (FSR 4, DLSS): with "Use discrete GPU"
    /// the best one; otherwise also the best one for Windows games, because DXVK / VKD3D pick
    /// the discrete GPU first on their own (so an Optimus laptop still gets DLSS).
    pub fn rendering_gpu(&self, prefer_dgpu: bool) -> Option<&Gpu> {
        let _ = prefer_dgpu;
        self.best_gpu().or(self.default_gpu())
    }
}

impl Gpu {
    /// An integrated GPU by its look: Intel graphics on the root bus (00:02.0), or an AMD APU
    /// reporting at most 2 GiB (its carved-out system memory) without the discrete tag.
    pub fn likely_integrated(&self) -> bool {
        if self.discrete_tag {
            return false;
        }
        (self.vendor == 0x8086 && self.pci.starts_with("0000:00:"))
            || (self.vendor == 0x1002 && self.vram.is_some_and(|v| v <= 2 << 30))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NtsyncState {
    Usable,
    NotLoaded,
    #[default]
    Missing,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Distro {
    Arch,
    Fedora,
    Debian,
    #[default]
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RunnerFamily {
    Valve,
    Ge,
    Cachyos,
    #[default]
    Unknown,
}

/// What one Proton build supports (spec §4.2 RunnerCaps), from strings in its files.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunnerCaps {
    pub family: RunnerFamily,
    pub version: String,
    /// `PROTON_*` / `DXVK_*` variable names its `proton` script and protonfixes read.
    pub vars: HashSet<String>,
    pub ntsync: bool,
    pub wineserver_no_ntsync: bool,
    pub winewayland: bool,
    pub fshack_fsr: bool,
    pub contrib_fsr4: bool,
}

impl RunnerCaps {
    /// Unknown runner: act as if every `PROTON_*` variable were supported (grey nothing out).
    pub fn unknown() -> RunnerCaps {
        RunnerCaps::default()
    }
    #[cfg(test)]
    pub fn has(&self, var: &str) -> bool {
        self.family == RunnerFamily::Unknown || self.vars.contains(var)
    }
}

// ------------------------------------------------------------------------- process-wide state

/// What `launch::resolve` needs besides the game: global defaults, detected capabilities and the
/// monitor size. Swapped atomically; readers clone the Arc.
#[derive(Clone, Debug, Default)]
pub struct LaunchEnv {
    pub defaults: GameOptions,
    /// Per library (`Source::key()`): its options over the global ones, under a game's own.
    pub by_source: BTreeMap<String, GameOptions>,
    pub caps: Arc<Caps>,
    pub monitor: Option<(u32, u32)>,
    /// Folder for per-game Proton logs (`<data dir>/proton-logs`).
    pub log_dir: Option<PathBuf>,
}

impl LaunchEnv {
    /// No toggles at all (tests / `resolve_with`): launches resolve exactly as before 4d.
    pub fn none() -> LaunchEnv {
        LaunchEnv::default()
    }
}

fn state() -> &'static RwLock<Arc<LaunchEnv>> {
    static S: std::sync::OnceLock<RwLock<Arc<LaunchEnv>>> = std::sync::OnceLock::new();
    S.get_or_init(|| RwLock::new(Arc::new(LaunchEnv::none())))
}

pub fn launch_env() -> Arc<LaunchEnv> {
    state().read().map(|s| s.clone()).unwrap_or_default()
}

fn update(f: impl FnOnce(&mut LaunchEnv)) {
    if let Ok(mut s) = state().write() {
        let mut next = (**s).clone();
        f(&mut next);
        *s = Arc::new(next);
    }
}

pub fn set_defaults(defaults: GameOptions) {
    update(|e| e.defaults = defaults);
}

pub fn set_source_defaults(by_source: BTreeMap<String, GameOptions>) {
    update(|e| e.by_source = by_source);
}

pub fn set_caps(caps: Caps) {
    update(|e| e.caps = Arc::new(caps));
}

pub fn set_monitor(monitor: Option<(u32, u32)>) {
    let cur = launch_env().monitor;
    if cur != monitor {
        update(|e| e.monitor = monitor);
    }
}

pub fn set_log_dir(dir: PathBuf) {
    update(|e| e.log_dir = Some(dir));
}
