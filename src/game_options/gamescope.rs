//! "More Gamescope options": every flag the installed gamescope documents, offered as plain
//! switches and fields. Well-known flags get a friendly name, the right control and a group
//! ([`KNOWN`]); anything else `gamescope --help` lists shows up under "Other" with its own help
//! text, so new gamescope versions need no Lodestar update. Flags Lodestar sets through its own
//! options ([`MANAGED`]) and ones that would break the launch ([`HIDDEN`]) are never offered.

use super::{GameOptions, GamescopeCaps, GsHelpOption};

/// How a flag is set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlagKind {
    /// Present or not.
    Switch,
    Int {
        min: i64,
        max: i64,
        default: i64,
    },
    Decimal {
        min: f64,
        max: f64,
        default: f64,
    },
    Choice(&'static [&'static str]),
    /// A file or folder path.
    Path,
    Text,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FlagGroup {
    Picture,
    Performance,
    Input,
    Window,
    Debug,
    Other,
}

impl FlagGroup {
    pub const ALL: [FlagGroup; 6] = [
        FlagGroup::Picture,
        FlagGroup::Performance,
        FlagGroup::Input,
        FlagGroup::Window,
        FlagGroup::Debug,
        FlagGroup::Other,
    ];
    pub fn label(self) -> &'static str {
        match self {
            FlagGroup::Picture => "Picture & HDR",
            FlagGroup::Performance => "Performance",
            FlagGroup::Input => "Mouse & keyboard",
            FlagGroup::Window => "Window & display",
            FlagGroup::Debug => "Troubleshooting",
            FlagGroup::Other => "Other (from gamescope --help)",
        }
    }
}

/// A well-known flag.
#[derive(Clone, Copy, Debug)]
pub struct KnownFlag {
    pub long: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: FlagKind,
    pub group: FlagGroup,
}

const fn k(long: &'static str, label: &'static str, help: &'static str, kind: FlagKind, group: FlagGroup) -> KnownFlag {
    KnownFlag { long, label, help, kind, group }
}

use FlagGroup as G;
use FlagKind as K;

/// Flags with a friendly name and the right control.
pub const KNOWN: &[KnownFlag] = &[
    k(
        "--hdr-itm-enabled",
        "SDR → HDR (inverse tone mapping)",
        "Make SDR games look HDR on an HDR screen. Needs HDR on.",
        K::Switch,
        G::Picture,
    ),
    k(
        "--hdr-itm-sdr-nits",
        "SDR → HDR input brightness",
        "Brightness of the SDR picture fed into the SDR → HDR mapping (nits; default 100).",
        K::Int { min: 1, max: 1000, default: 100 },
        G::Picture,
    ),
    k(
        "--hdr-itm-target-nits",
        "SDR → HDR peak brightness",
        "How bright the mapped highlights get (nits; default 1000). Set it to your screen's peak.",
        K::Int { min: 1, max: 10000, default: 1000 },
        G::Picture,
    ),
    k(
        "--hdr-sdr-content-nits",
        "SDR brightness in HDR",
        "How bright SDR content (menus, SDR games) looks while HDR is on (nits; default 400).",
        K::Int { min: 1, max: 1000, default: 400 },
        G::Picture,
    ),
    k(
        "--sdr-gamut-wideness",
        "SDR colour wideness",
        "0 = accurate sRGB colours, 1 = stretched to the screen's wide gamut (more vivid).",
        K::Decimal { min: 0.0, max: 1.0, default: 0.5 },
        G::Picture,
    ),
    k(
        "--reshade-effect",
        "ReShade effect",
        "A ReShade shader file name from /usr/share/gamescope/reshade/Shaders or ~/.local/share/gamescope/reshade/Shaders.",
        K::Text,
        G::Picture,
    ),
    k(
        "--reshade-technique-idx",
        "ReShade technique",
        "Which technique of the ReShade effect to use (0 = the first).",
        K::Int { min: 0, max: 64, default: 0 },
        G::Picture,
    ),
    k(
        "--max-scale",
        "Maximum upscale factor",
        "Never scale the game up by more than this (e.g. 2 keeps pixel art crisp).",
        K::Int { min: 1, max: 16, default: 2 },
        G::Picture,
    ),
    k(
        "--disable-color-management",
        "Turn off colour management",
        "Show colours exactly as the game outputs them.",
        K::Switch,
        G::Picture,
    ),
    k(
        "--rt",
        "Realtime scheduling",
        "Run Gamescope with realtime priority: smoother frame pacing when the CPU is busy (needs permission; harmless without).",
        K::Switch,
        G::Performance,
    ),
    k(
        "--nested-unfocused-refresh",
        "FPS when in the background",
        "Frame cap while the Gamescope window isn't focused, to save power (e.g. 10).",
        K::Int { min: 1, max: 1000, default: 10 },
        G::Performance,
    ),
    k(
        "--immediate-flips",
        "Allow tearing (lowest latency)",
        "Show frames immediately instead of waiting for vsync. Mainly for Gamescope sessions (Steam Deck style); may do nothing in a window.",
        K::Switch,
        G::Performance,
    ),
    k(
        "--framerate-limit",
        "Refresh-rate divisor limit",
        "Gamescope's simple limiter: a divisor of the refresh rate (60 on 120 Hz → 60 fps). Prefer Performance → Frame limit.",
        K::Int { min: 0, max: 1000, default: 60 },
        G::Performance,
    ),
    k(
        "--disable-layers",
        "Turn off hardware planes",
        "Composite everything on the GPU (libliftoff off). Can fix flicker; costs a little performance.",
        K::Switch,
        G::Performance,
    ),
    k(
        "--force-composition",
        "Always composite",
        "Never scan the game out directly. Can fix flicker or overlay issues; costs a little latency.",
        K::Switch,
        G::Performance,
    ),
    k(
        "--mouse-sensitivity",
        "Mouse sensitivity",
        "Multiply mouse movement (e.g. 0.5 = half speed, 2 = double).",
        K::Decimal { min: 0.05, max: 10.0, default: 1.0 },
        G::Input,
    ),
    k(
        "--grab",
        "Grab the keyboard",
        "Send every key to the game, including system shortcuts (Alt+Tab, Super …).",
        K::Switch,
        G::Input,
    ),
    k(
        "--hide-cursor-delay",
        "Hide the cursor after",
        "Hide an idle mouse cursor after this many milliseconds.",
        K::Int { min: 0, max: 60000, default: 3000 },
        G::Input,
    ),
    k("--cursor", "Cursor image", "A picture to use as the mouse cursor.", K::Path, G::Input),
    k(
        "--cursor-scale-height",
        "Cursor scaling base height",
        "Scale the cursor with the output height against this base height (e.g. 720).",
        K::Int { min: 100, max: 8640, default: 720 },
        G::Input,
    ),
    k(
        "--force-windows-fullscreen",
        "Force game windows to fill the screen",
        "Make every game window the size of Gamescope's display. Fixes games that open small or off-centre.",
        K::Switch,
        G::Window,
    ),
    k(
        "--display-index",
        "Monitor",
        "Open Gamescope on this monitor (0 = the first).",
        K::Int { min: 0, max: 16, default: 0 },
        G::Window,
    ),
    k(
        "--backend",
        "Backend",
        "How Gamescope draws: auto, sdl, wayland … (sdl loses HDR and VRR; only change it to work around problems).",
        K::Choice(&["auto", "sdl", "wayland", "drm", "headless", "openvr"]),
        G::Window,
    ),
    k(
        "--force-orientation",
        "Rotate the display",
        "For rotated (portrait) screens.",
        K::Choice(&["normal", "left", "right", "upsidedown"]),
        G::Window,
    ),
    k(
        "--xwayland-count",
        "XWayland servers",
        "Number of X servers inside Gamescope (2 helps some launchers that open a second window).",
        K::Int { min: 1, max: 4, default: 2 },
        G::Window,
    ),
    k(
        "--hdr-debug-force-output",
        "Force HDR output",
        "Output HDR10 even when Gamescope thinks the display can't (KDE setups where HDR isn't detected). Looks wrong on SDR screens.",
        K::Switch,
        G::Debug,
    ),
    k(
        "--hdr-debug-force-support",
        "Force HDR support",
        "Pretend the display supports HDR (HDR games still show SDR if it doesn't).",
        K::Switch,
        G::Debug,
    ),
    k(
        "--hdr-debug-heatmap",
        "HDR brightness heatmap",
        "Show a heatmap of brightness in nits instead of the picture.",
        K::Switch,
        G::Debug,
    ),
    k("--debug-hud", "Debug HUD", "Paint Gamescope's debug information over the game.", K::Switch, G::Debug),
    k(
        "--composite-debug",
        "Show compositing markers",
        "Draw markers in the corners when Gamescope composites.",
        K::Switch,
        G::Debug,
    ),
];

/// Flags Lodestar sets through its own options (window mode, sizes, upscaler, HDR, MangoHud …).
pub const MANAGED: &[&str] = &[
    "--output-width",
    "--output-height",
    "--nested-width",
    "--nested-height",
    "--nested-refresh",
    "--scaler",
    "--filter",
    "--sharpness",
    "--fsr-sharpness",
    "--expose-wayland",
    "--prefer-vk-device",
    "--hdr-enabled",
    "--mangoapp",
    "--adaptive-sync",
    "--borderless",
    "--fullscreen",
    "--force-grab-cursor",
];

/// Flags never offered: they'd break the launch or Lodestar's tracking, or only make sense to
/// Steam / VR / a whole Gamescope session.
pub const HIDDEN: &[&str] = &["--help", "--ready-fd", "--steam", "--keep-alive", "--stats-path", "--mura-map"];

/// One row of the picker.
#[derive(Clone, Debug, PartialEq)]
pub struct FlagRow {
    pub long: String,
    pub label: String,
    pub help: String,
    pub kind: FlagKind,
    pub group: FlagGroup,
}

/// The rows to offer: known flags this gamescope documents (all known flags when its help
/// couldn't be read), then every other documented flag that isn't managed or hidden.
/// VR-only flags are left out (Lodestar runs games, not VR overlays).
pub fn rows(caps: Option<&GamescopeCaps>) -> Vec<FlagRow> {
    let documented: &[GsHelpOption] = caps.map(|c| c.options.as_slice()).unwrap_or_default();
    let has = |long: &str| documented.is_empty() || documented.iter().any(|o| o.long == long);
    let mut out: Vec<FlagRow> = KNOWN
        .iter()
        .filter(|f| has(f.long))
        .map(|f| FlagRow {
            long: f.long.into(),
            label: f.label.into(),
            help: f.help.into(),
            kind: f.kind,
            group: f.group,
        })
        .collect();
    for o in documented {
        let known = KNOWN.iter().any(|f| f.long == o.long);
        let vr = o.long.starts_with("--vr-") || o.section.to_ascii_lowercase().starts_with("vr");
        if known || vr || MANAGED.contains(&o.long.as_str()) || HIDDEN.contains(&o.long.as_str()) {
            continue;
        }
        out.push(FlagRow {
            long: o.long.clone(),
            label: o.long.trim_start_matches('-').replace('-', " "),
            help: format!("{} ({})", o.help, o.section),
            kind: K::Text,
            group: G::Other,
        });
    }
    out
}

/// How many extra flags a set of options turns on (for the row summary).
pub fn count_set(o: &GameOptions) -> usize {
    o.gs_more.values().filter(|v| !matches!(v, super::FlagSetting::Off)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_flags_are_unique_and_not_managed() {
        for (i, f) in KNOWN.iter().enumerate() {
            assert!(f.long.starts_with("--"), "{}", f.long);
            assert!(!MANAGED.contains(&f.long) && !HIDDEN.contains(&f.long), "{}", f.long);
            assert!(KNOWN[i + 1..].iter().all(|g| g.long != f.long), "duplicate {}", f.long);
        }
    }

    #[test]
    fn rows_follow_the_installed_help() {
        let help = "usage: gamescope [options...] -- [command...]\n\nOptions:\n  --help    show help\n  -W, --output-width             output width\n  --rt                           Use realtime scheduling\n  --brand-new-thing              something new\n                                 continued\nVR mode options:\n  --vr-overlay-key     key\nKeyboard shortcuts:\n  Super + F                      toggle fullscreen\n";
        let caps = crate::game_options::caps::parse_gamescope_help(help);
        let longs: Vec<&str> = caps.options.iter().map(|o| o.long.as_str()).collect();
        assert_eq!(longs, ["--output-width", "--rt", "--brand-new-thing", "--vr-overlay-key"]);
        assert_eq!(caps.options[0].short.as_deref(), Some("-W"));
        assert_eq!(caps.options[2].help, "something new continued");
        let r = rows(Some(&caps));
        let names: Vec<&str> = r.iter().map(|r| r.long.as_str()).collect();
        // Known + documented, then unknown documented ones; managed, hidden and VR left out.
        assert_eq!(names, ["--rt", "--brand-new-thing"]);
        assert_eq!(r[1].group, FlagGroup::Other);
        // Help unreadable: every known flag is offered.
        assert_eq!(rows(None).len(), KNOWN.len());
    }
}
