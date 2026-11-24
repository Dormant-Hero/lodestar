//! Built-in emulator definitions (Linux): where each is installed from, how it starts a game,
//! which consoles it runs. Facts come from the emulators' own sources / Flathub manifests (see
//! research-emulators.md); flags that could not be verified are left out.
//!
//! Argument templates are split into words like a shell would (quotes work, nothing is run
//! through a shell), then each word is filled in:
//! * `{rom}` the file to start, `{rom_dir}` its folder, `{rom_name}` its name without the
//!   extension, `{core}` the RetroArch core's full path, `{emu_dir}` the emulator's folder.
//!   Playnite's `{ImagePath}`, `{ImageDir}`, `{ImageName}` (with extension), `{ImageNameNoExt}`
//!   and `{EmuDir}` work too.
//! * `{args}` marks where extra arguments and switches go (Settings → Emulation, the game's
//!   own, an emulator action's); without it they go right before the ROM (and its `--`).
//!   `{Rom}`, `{RomDir}`, `{RomName}`, `{Platform}` and `{Emulator}` (action-style names) work too.
//! * A word starting with `{fs}` is only used when starting fullscreen, one starting with
//!   `{win}` only when starting in a window (e.g. `{fs}-fullscreen {win}-nofullscreen`). That is
//!   how "Play in a window" drops the fullscreen words. A lone `{fullscreen}` word is dropped.
//!
//! Order matters: within the table, better / more current emulators of a console come first;
//! [`retroarch_first`] says for which consoles RetroArch is the usual choice.

/// How one emulator starts games of some platforms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileDef {
    pub id: &'static str,
    pub name: &'static str,
    /// Platform ids.
    pub platforms: &'static [&'static str],
    /// ROM extensions it opens, preferred first.
    pub extensions: &'static [&'static str],
    /// Argument template (`{rom}`, `{rom_dir}`, `{rom_name}`, `{core}`, `{fs}` …).
    pub args: &'static str,
    /// RetroArch core (file stem, e.g. "mgba_libretro").
    pub core: Option<&'static str>,
    /// Folder games: files tried inside the folder ("title.tmd", "PS3_GAME/USRDIR/EBOOT.BIN");
    /// empty = pass the folder itself. A `*` in the last part matches any name ("code/*.rpx").
    pub folder_launch: &'static [&'static str],
}

/// A built-in emulator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmulatorDef {
    pub id: &'static str,
    pub name: &'static str,
    /// Flatpak app ids (current first; old ids are kept for old installs).
    pub flatpak: &'static [&'static str],
    /// Program names looked up in PATH.
    pub binaries: &'static [&'static str],
    /// AppImage file-name globs (lower-case); a pattern with a `/` is an unpacked program
    /// ("vita3k/vita3k" = a `Vita3K` folder holding a `Vita3K` program).
    pub appimage: &'static [&'static str],
    /// Flathub app id to link to when it isn't installed.
    pub flathub: Option<&'static str>,
    pub website: &'static str,
    /// The emulator's own documentation of its command line (checked to exist and to list the
    /// flags Lodestar uses); None = no page worth linking.
    pub docs_url: Option<&'static str>,
    /// Friendly switches from its documented command line (fullscreen comes from the profiles'
    /// `{fs}` / `{win}` words, see [`crate::emulation::options`]).
    pub options: &'static [OptionDef],
    pub profiles: &'static [ProfileDef],
}

/// One switch of an emulator's command line ("Close PCSX2 when the game ends" = `-batch`).
/// On adds the `on` words (and drops the `off` ones), Off the other way round; not set leaves the
/// profile's arguments as they are. Only flags the emulator documents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OptionDef {
    /// Stable key in the config ("batch", "fastboot" …).
    pub id: &'static str,
    pub label: &'static str,
    /// One line under the label.
    pub help: &'static str,
    pub on: &'static [&'static str],
    pub off: &'static [&'static str],
    /// What it is when nobody set it (the profile's arguments already say so).
    pub default: bool,
    /// Shown before "More options" (the two or three that matter most).
    pub key: bool,
}

const NO_OPTS: &[OptionDef] = &[];

const fn opt(
    id: &'static str,
    label: &'static str,
    help: &'static str,
    on: &'static [&'static str],
    off: &'static [&'static str],
    default: bool,
    key: bool,
) -> OptionDef {
    OptionDef { id, label, help, on, off, default, key }
}

// Verified against pcsx2.net/docs/advanced/cli (Nov 2026): -batch, -fastboot / -slowboot, -bigpicture.
const PCSX2_OPTS: &[OptionDef] = &[
    opt(
        "batch",
        "Close PCSX2 when the game ends",
        "Back to Lodestar instead of PCSX2's game list",
        &["-batch"],
        &[],
        true,
        true,
    ),
    opt(
        "fastboot",
        "Skip the PS2 start-up screen",
        "Starts the game right away (some games need the slow boot)",
        &["-fastboot"],
        &["-slowboot"],
        false,
        true,
    ),
    opt(
        "bigpicture",
        "Big Picture mode",
        "PCSX2's controller-friendly full-screen menus",
        &["-bigpicture"],
        &[],
        false,
        false,
    ),
];
// Verified against DuckStation's wiki "Command Line Arguments" (Nov 2026).
const DUCKSTATION_OPTS: &[OptionDef] = &[
    opt(
        "batch",
        "Close DuckStation when the game ends",
        "Back to Lodestar instead of DuckStation's game list",
        &["-batch"],
        &[],
        true,
        true,
    ),
    opt(
        "fastboot",
        "Skip the PlayStation start-up screen",
        "Starts the game right away (some games need the slow boot)",
        &["-fastboot"],
        &["-slowboot"],
        false,
        true,
    ),
    opt(
        "bigpicture",
        "Big Picture mode",
        "DuckStation's controller-friendly full-screen menus",
        &["-bigpicture"],
        &[],
        false,
        false,
    ),
];
// Dolphin wiki, Help:Contents → Command Line Options: "-b, --batch  Exit Dolphin with emulator".
const DOLPHIN_OPTS: &[OptionDef] = &[opt(
    "batch",
    "Close Dolphin when the game ends",
    "Back to Lodestar instead of Dolphin's game list",
    &["-b"],
    &[],
    true,
    true,
)];
// ppsspp.org/docs/reference/command-line: --pause-menu-exit, --escape-exit.
const PPSSPP_OPTS: &[OptionDef] = &[
    opt(
        "pause_exit",
        "Close PPSSPP when the game ends",
        "“Exit to menu” closes PPSSPP instead of going to its menu",
        &["--pause-menu-exit"],
        &[],
        true,
        true,
    ),
    opt(
        "escape_exit",
        "Esc closes PPSSPP",
        "The pause key quits right away instead of opening PPSSPP's menu",
        &["--escape-exit"],
        &[],
        false,
        false,
    ),
];

macro_rules! prof {
    ($id:literal, $name:literal, [$($p:literal),*], $ext:expr, $args:literal) => {
        ProfileDef { id: $id, name: $name, platforms: &[$($p),*], extensions: $ext, args: $args, core: None,
            folder_launch: &[] }
    };
    ($id:literal, $name:literal, [$($p:literal),*], $ext:expr, $args:literal, folder [$($f:literal),*]) => {
        ProfileDef { id: $id, name: $name, platforms: &[$($p),*], extensions: $ext, args: $args, core: None,
            folder_launch: &[$($f),*] }
    };
}

/// A RetroArch core profile (`retroarch -f -L <core> <rom>`).
macro_rules! core {
    ($core:literal, $name:literal, [$($p:literal),*], $ext:expr) => {
        ProfileDef { id: $core, name: $name, platforms: &[$($p),*], extensions: $ext, args: RA_ARGS,
            core: Some(concat!($core, "_libretro")), folder_launch: &[] }
    };
}

const RA_ARGS: &str = "{fs}-f -L {core} {rom}";

// Extension lists, preferred first (playlists / cue sheets before the tracks they list).
const CD: &[&str] = &["m3u", "chd", "cue", "ccd", "toc", "mds", "iso", "img", "bin", "zip", "7z"];
const E_NES: &[&str] = &["nes", "unf", "unif", "fds", "zip", "7z"];
const E_FDS: &[&str] = &["fds", "qd", "zip", "7z"];
const E_SNES: &[&str] = &["sfc", "smc", "fig", "swc", "bs", "st", "bml", "zip", "7z"];
const E_GB: &[&str] = &["gb", "gbc", "dmg", "cgb", "sgb", "zip", "7z"];
const E_GBA: &[&str] = &["gba", "agb", "mb", "elf", "bin", "zip", "7z"];
const E_MGBA: &[&str] = &["gba", "agb", "gb", "gbc", "cgb", "sgb", "dmg", "mb", "elf", "bin", "zip", "7z"];
const E_N64: &[&str] = &["z64", "n64", "v64", "u1", "ndd", "d64", "zip", "7z"];
const E_NDS: &[&str] = &["nds", "dsi", "ids", "srl", "app", "bin", "zip", "7z"];
const E_3DS: &[&str] = &["3ds", "cci", "cxi", "3dsx", "zcci", "zcxi", "z3dsx", "app", "axf", "elf"];
const E_DOLPHIN: &[&str] =
    &["rvz", "iso", "gcm", "gcz", "ciso", "wia", "wbfs", "tgc", "wad", "dol", "elf", "dff", "m3u"];
const E_PSX: &[&str] = &[
    "m3u", "cue", "chd", "pbp", "ccd", "mds", "ecm", "iso", "img", "bin", "cbn", "toc", "exe", "psexe", "psf",
    "minipsf",
];
const E_PSX_RA: &[&str] = &["m3u", "chd", "cue", "pbp", "ccd", "toc", "iso", "img", "bin", "exe", "zip", "7z"];
/// PCSX2 opens no .cue / .m3u: a cue game starts from its .bin.
const E_PCSX2: &[&str] = &["chd", "iso", "cso", "zso", "gz", "bin", "img", "mdf", "nrg", "isz", "elf"];
const E_PS2_RA: &[&str] = &["chd", "iso", "cso", "zso", "cue", "bin", "img", "mdf", "elf", "m3u"];
/// No `.prx`: those are PSP plugins / PS4 system modules, not games.
const E_PSP: &[&str] = &["iso", "cso", "chd", "pbp", "elf", "zip", "7z"];
const E_MD: &[&str] = &["md", "gen", "smd", "bin", "68k", "sgd", "zip", "7z"];
const E_GPGX: &[&str] =
    &["md", "gen", "smd", "bin", "68k", "sgd", "sms", "gg", "sg", "m3u", "chd", "cue", "iso", "zip", "7z"];
const E_PICO: &[&str] = &["32x", "md", "gen", "smd", "bin", "sms", "m3u", "chd", "cue", "iso", "zip", "7z"];
const E_SMS: &[&str] = &["sms", "gg", "sg", "col", "bin", "rom", "zip", "7z"];
const E_DC: &[&str] = &["m3u", "chd", "gdi", "cdi", "cue", "elf", "zip", "7z"];
const E_PCE: &[&str] = &["pce", "sgx", "m3u", "chd", "cue", "ccd", "toc", "zip", "7z"];
const E_ARCADE: &[&str] = &["zip", "7z", "chd"];
const E_SWITCH: &[&str] = &["nsp", "xci", "nca", "nro", "nso"];
const E_DOS: &[&str] = &["dosz", "zip", "7z", "exe", "com", "bat", "conf", "iso", "cue", "img", "ima", "vhd", "m3u"];

static EMULATORS: &[EmulatorDef] = &[
    // ---- Sony ------------------------------------------------------------------------------
    EmulatorDef {
        id: "duckstation",
        name: "DuckStation",
        flatpak: &["org.duckstation.DuckStation"],
        binaries: &["duckstation-qt", "duckstation"],
        appimage: &["duckstation*.appimage"],
        flathub: Some("org.duckstation.DuckStation"),
        website: "https://www.duckstation.org",
        docs_url: Some("https://github.com/stenzek/duckstation/wiki/Command-Line-Arguments"),
        options: DUCKSTATION_OPTS,
        profiles: &[prof!(
            "default",
            "Default",
            ["sony_playstation"],
            E_PSX,
            "-batch {fs}-fullscreen {win}-nofullscreen -- {rom}"
        )],
    },
    EmulatorDef {
        id: "pcsx2",
        name: "PCSX2",
        flatpak: &["net.pcsx2.PCSX2"],
        binaries: &["pcsx2-qt", "pcsx2"],
        appimage: &["pcsx2*.appimage"],
        flathub: Some("net.pcsx2.PCSX2"),
        website: "https://pcsx2.net",
        docs_url: Some("https://pcsx2.net/docs/advanced/cli/"),
        options: PCSX2_OPTS,
        profiles: &[prof!(
            "default",
            "Default",
            ["sony_playstation2"],
            E_PCSX2,
            "-batch {fs}-fullscreen {win}-nofullscreen -- {rom}"
        )],
    },
    EmulatorDef {
        id: "rpcs3",
        name: "RPCS3",
        flatpak: &["net.rpcs3.RPCS3"],
        binaries: &["rpcs3"],
        appimage: &["rpcs3*.appimage"],
        flathub: Some("net.rpcs3.RPCS3"),
        website: "https://rpcs3.net",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!("default", "Default", ["sony_playstation3"], &["iso", "bin"],
            "--no-gui {fs}--fullscreen {rom}",
            folder ["PS3_GAME/USRDIR/EBOOT.BIN", "USRDIR/EBOOT.BIN", "EBOOT.BIN"])],
    },
    EmulatorDef {
        id: "ppsspp",
        name: "PPSSPP",
        flatpak: &["org.ppsspp.PPSSPP"],
        binaries: &["PPSSPPSDL", "PPSSPPQt", "ppsspp"],
        appimage: &["ppsspp*.appimage"],
        flathub: Some("org.ppsspp.PPSSPP"),
        website: "https://www.ppsspp.org",
        docs_url: Some("https://www.ppsspp.org/docs/reference/command-line/"),
        options: PPSSPP_OPTS,
        profiles: &[prof!(
            "default",
            "Default",
            ["sony_psp"],
            E_PSP,
            "{fs}--fullscreen {win}--windowed --pause-menu-exit {rom}"
        )],
    },
    EmulatorDef {
        id: "shadps4",
        name: "shadPS4",
        // The Flatpak's default command is the Qt launcher; games start through `--command=shadps4`.
        flatpak: &["net.shadps4.shadPS4"],
        binaries: &["shadps4"],
        appimage: &["shadps4-sdl*.appimage", "shadps4-qt*.appimage", "shadps4*.appimage", "shadps4/shadps4"],
        flathub: Some("net.shadps4.shadPS4"),
        website: "https://shadps4.net",
        docs_url: Some("https://github.com/shadps4-emu/shadPS4#usage-examples"),
        options: NO_OPTS,
        profiles: &[prof!(
            "default",
            "Default",
            ["sony_playstation4"],
            &["bin"],
            "{args} -g {rom} {fs}-f {fs}true {win}-f {win}false",
            folder["eboot.bin"]
        )],
    },
    EmulatorDef {
        id: "vita3k",
        name: "Vita3K",
        flatpak: &[],
        binaries: &["Vita3K", "vita3k"],
        appimage: &["vita3k*.appimage", "vita3k/vita3k"],
        flathub: None,
        website: "https://vita3k.org",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!("default", "Default", ["sony_vita"], &["vpk", "zip"], "{fs}-F {rom}")],
    },
    // ---- Nintendo --------------------------------------------------------------------------
    EmulatorDef {
        id: "dolphin",
        name: "Dolphin",
        flatpak: &["org.DolphinEmu.dolphin-emu"],
        binaries: &["dolphin-emu", "dolphin-emu-nogui"],
        appimage: &["dolphin_emulator*.appimage", "dolphin*.appimage"],
        flathub: Some("org.DolphinEmu.dolphin-emu"),
        website: "https://dolphin-emu.org",
        docs_url: Some("https://wiki.dolphin-emu.org/index.php?title=Help:Contents#Command_Line_Options"),
        options: DOLPHIN_OPTS,
        profiles: &[prof!(
            "default",
            "Default",
            ["nintendo_gamecube", "nintendo_wii"],
            E_DOLPHIN,
            "-b {fs}-C {fs}Dolphin.Display.Fullscreen=True {win}-C {win}Dolphin.Display.Fullscreen=False {args} -e {rom}"
        )],
    },
    EmulatorDef {
        id: "primehack",
        name: "PrimeHack",
        flatpak: &["io.github.shiiion.primehack"],
        binaries: &["primehack"],
        appimage: &["primehack*.appimage"],
        flathub: Some("io.github.shiiion.primehack"),
        website: "https://github.com/shiiion/dolphin",
        docs_url: None,
        options: DOLPHIN_OPTS,
        profiles: &[prof!(
            "default",
            "Default",
            ["nintendo_gamecube", "nintendo_wii"],
            E_DOLPHIN,
            "-b {fs}-C {fs}Dolphin.Display.Fullscreen=True {win}-C {win}Dolphin.Display.Fullscreen=False {args} -e {rom}"
        )],
    },
    EmulatorDef {
        id: "cemu",
        name: "Cemu",
        flatpak: &["info.cemu.Cemu"],
        binaries: &["Cemu", "cemu"],
        appimage: &["cemu*.appimage", "cemu/cemu"],
        flathub: Some("info.cemu.Cemu"),
        website: "https://cemu.info",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!("default", "Default", ["nintendo_wiiu"], &["wua", "wud", "wux", "iso", "rpx", "elf", "tmd"],
            "{fs}-f {args} -g {rom}", folder ["title.tmd", "code/*.rpx"])],
    },
    EmulatorDef {
        id: "eden",
        name: "Eden",
        flatpak: &["dev.eden_emu.eden"],
        binaries: &["eden"],
        appimage: &["eden*.appimage"],
        flathub: Some("dev.eden_emu.eden"),
        website: "https://eden-emu.dev",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!("default", "Default", ["nintendo_switch"], E_SWITCH, "{fs}-f {args} -g {rom}")],
    },
    EmulatorDef {
        id: "ryujinx",
        name: "Ryujinx",
        flatpak: &["io.github.ryubing.Ryujinx", "org.ryujinx.Ryujinx"],
        binaries: &["Ryujinx", "ryujinx", "Ryujinx.Ava"],
        appimage: &["*yujinx*.appimage", "publish/ryujinx"],
        flathub: Some("io.github.ryubing.Ryujinx"),
        website: "https://ryujinx.app",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!("default", "Default", ["nintendo_switch"], E_SWITCH, "{fs}--fullscreen {rom}")],
    },
    EmulatorDef {
        id: "citron",
        name: "Citron",
        flatpak: &[],
        binaries: &["citron"],
        appimage: &["citron*.appimage"],
        flathub: None,
        website: "https://citron-emu.org",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!("default", "Default", ["nintendo_switch"], E_SWITCH, "{fs}-f {args} -g {rom}")],
    },
    EmulatorDef {
        id: "sudachi",
        name: "Sudachi",
        flatpak: &[],
        binaries: &["sudachi"],
        appimage: &["sudachi*.appimage"],
        flathub: None,
        website: "https://sudachi.emuplace.app",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!("default", "Default", ["nintendo_switch"], E_SWITCH, "{fs}-f {args} -g {rom}")],
    },
    EmulatorDef {
        id: "azahar",
        name: "Azahar",
        flatpak: &["org.azahar_emu.Azahar"],
        binaries: &["azahar"],
        appimage: &["azahar*.appimage", "azaharplus*.appimage"],
        flathub: Some("org.azahar_emu.Azahar"),
        website: "https://azahar-emu.org",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!("default", "Default", ["nintendo_3ds"], E_3DS, "{fs}-f {win}-w {rom}")],
    },
    EmulatorDef {
        id: "lime3ds",
        name: "Lime3DS",
        flatpak: &["io.github.lime3ds.Lime3DS"],
        binaries: &["lime3ds", "lime3ds-gui", "lime-qt"],
        appimage: &["lime3ds*.appimage", "lime-qt*.appimage"],
        flathub: None,
        website: "https://github.com/Lime3DS/Lime3DS",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!("default", "Default", ["nintendo_3ds"], E_3DS, "{fs}-f {rom}")],
    },
    EmulatorDef {
        id: "citra",
        name: "Citra",
        flatpak: &["org.citra_emu.citra"],
        binaries: &["citra-qt", "citra"],
        appimage: &["citra-qt*.appimage", "citra*.appimage"],
        flathub: None,
        website: "https://citra-emu.org",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!("default", "Default", ["nintendo_3ds"], E_3DS, "{fs}-f {rom}")],
    },
    EmulatorDef {
        id: "melonds",
        name: "melonDS",
        flatpak: &["net.kuribo64.melonDS"],
        binaries: &["melonDS", "melonds"],
        appimage: &["melonds*.appimage"],
        flathub: Some("net.kuribo64.melonDS"),
        website: "https://melonds.kuribo64.net",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!("default", "Default", ["nintendo_ds", "nintendo_dsi"], E_NDS, "{fs}-f {rom}")],
    },
    EmulatorDef {
        id: "mgba",
        name: "mGBA",
        flatpak: &["io.mgba.mGBA"],
        binaries: &["mgba-qt", "mgba"],
        appimage: &["mgba*.appimage"],
        flathub: Some("io.mgba.mGBA"),
        website: "https://mgba.io",
        docs_url: Some("https://github.com/mgba-emu/mgba/blob/master/doc/mgba-qt.6"),
        options: NO_OPTS,
        profiles: &[prof!(
            "default",
            "Default",
            ["nintendo_gameboyadvance", "nintendo_gameboy", "nintendo_gameboycolor"],
            E_MGBA,
            "{fs}-f {rom}"
        )],
    },
    EmulatorDef {
        id: "rmg",
        name: "Rosalie's Mupen GUI",
        flatpak: &["com.github.Rosalie241.RMG"],
        binaries: &["RMG"],
        appimage: &["rmg*.appimage"],
        flathub: Some("com.github.Rosalie241.RMG"),
        website: "https://github.com/Rosalie241/RMG",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!("default", "Default", ["nintendo_64"], E_N64, "{fs}-f -q {rom}")],
    },
    EmulatorDef {
        id: "simple64",
        name: "simple64",
        flatpak: &["io.github.simple64.simple64"],
        binaries: &["simple64-gui"],
        appimage: &[],
        flathub: Some("io.github.simple64.simple64"),
        website: "https://simple64.github.io",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!("default", "Default", ["nintendo_64"], E_N64, "--nogui {rom}")],
    },
    EmulatorDef {
        id: "gopher64",
        name: "Gopher64",
        flatpak: &["io.github.gopher64.gopher64"],
        binaries: &["gopher64", "gopher64-linux-x86_64"],
        appimage: &["gopher64/gopher64-linux-x86_64"],
        flathub: Some("io.github.gopher64.gopher64"),
        website: "https://github.com/gopher64/gopher64",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!("default", "Default", ["nintendo_64"], E_N64, "{fs}-f {rom}")],
    },
    EmulatorDef {
        id: "mupen64plus",
        name: "Mupen64Plus",
        flatpak: &[],
        binaries: &["mupen64plus"],
        appimage: &[],
        flathub: None,
        website: "https://mupen64plus.org",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!("default", "Default", ["nintendo_64"], E_N64, "{fs}--fullscreen {win}--windowed {rom}")],
    },
    EmulatorDef {
        id: "bsnes",
        name: "bsnes",
        flatpak: &["dev.bsnes.bsnes"],
        binaries: &["bsnes"],
        appimage: &[],
        flathub: Some("dev.bsnes.bsnes"),
        website: "https://github.com/bsnes-emu/bsnes",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!("default", "Default", ["nintendo_super_nes"], E_SNES, "{fs}--fullscreen {rom}")],
    },
    EmulatorDef {
        id: "snes9x",
        name: "Snes9x",
        flatpak: &["com.snes9x.Snes9x"],
        binaries: &["snes9x-gtk", "snes9x"],
        appimage: &["snes9x*.appimage"],
        flathub: Some("com.snes9x.Snes9x"),
        website: "https://www.snes9x.com",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!("default", "Default", ["nintendo_super_nes"], E_SNES, "{rom}")],
    },
    EmulatorDef {
        id: "mesen",
        name: "Mesen",
        flatpak: &[],
        binaries: &["Mesen", "mesen2", "mesen"],
        appimage: &["mesen*.appimage", "mesen/mesen"],
        flathub: None,
        website: "https://www.mesen.ca",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!(
            "default",
            "Default",
            [
                "nintendo_nes",
                "nintendo_famicom_disk",
                "nintendo_super_nes",
                "nintendo_gameboy",
                "nintendo_gameboycolor",
                "nintendo_gameboyadvance",
                "nec_turbografx_16",
                "nec_supergrafx",
                "sega_mastersystem",
                "sega_gamegear",
                "coleco_vision",
                "bandai_wonderswan",
                "bandai_wonderswan_color"
            ],
            &[
                "nes", "unf", "unif", "fds", "sfc", "smc", "fig", "swc", "bs", "gb", "gbc", "gba", "pce", "sgx", "sms",
                "gg", "col", "ws", "wsc", "zip", "7z"
            ],
            "{fs}--fullscreen {rom}"
        )],
    },
    // ---- Sega / Microsoft / others -----------------------------------------------------------
    EmulatorDef {
        id: "flycast",
        name: "Flycast",
        flatpak: &["org.flycast.Flycast"],
        binaries: &["flycast"],
        appimage: &["flycast*.appimage"],
        flathub: Some("org.flycast.Flycast"),
        website: "https://github.com/flyinghead/flycast",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[
            prof!(
                "default",
                "Default",
                ["sega_dreamcast"],
                E_DC,
                "{fs}-config {fs}window:fullscreen=yes {win}-config {win}window:fullscreen=no {rom}"
            ),
            prof!(
                "arcade",
                "Naomi / Atomiswave",
                ["arcade"],
                &["zip", "7z", "dat", "lst", "bin"],
                "{fs}-config {fs}window:fullscreen=yes {win}-config {win}window:fullscreen=no {rom}"
            ),
        ],
    },
    EmulatorDef {
        id: "redream",
        name: "Redream",
        flatpak: &[],
        binaries: &["redream"],
        appimage: &["redream/redream"],
        flathub: None,
        website: "https://redream.io",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!("default", "Default", ["sega_dreamcast"], &["chd", "gdi", "cdi", "cue"], "{rom}")],
    },
    EmulatorDef {
        id: "xemu",
        name: "xemu",
        flatpak: &["app.xemu.xemu"],
        binaries: &["xemu"],
        appimage: &["xemu*.appimage"],
        flathub: Some("app.xemu.xemu"),
        website: "https://xemu.app",
        docs_url: Some("https://xemu.app/docs/cli/"),
        options: NO_OPTS,
        profiles: &[prof!("default", "Default", ["xbox"], &["iso", "xiso"], "{fs}-full-screen {args} -dvd_path {rom}")],
    },
    EmulatorDef {
        id: "supermodel",
        name: "Supermodel",
        flatpak: &["com.supermodel3.Supermodel"],
        binaries: &["supermodel"],
        appimage: &["supermodel*.appimage", "supermodel/supermodel"],
        flathub: Some("com.supermodel3.Supermodel"),
        website: "https://www.supermodel3.com",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!("default", "Sega Model 3", ["arcade"], &["zip"], "{rom} {fs}-fullscreen {win}-window")],
    },
    EmulatorDef {
        id: "mame",
        name: "MAME",
        flatpak: &["org.mamedev.MAME"],
        binaries: &["mame"],
        appimage: &["mame*.appimage"],
        flathub: Some("org.mamedev.MAME"),
        website: "https://www.mamedev.org",
        docs_url: Some("https://docs.mamedev.org/commandline/commandline-all.html"),
        options: NO_OPTS,
        // MAME starts a set by its short name, found in the ROM folder; fullscreen by default.
        profiles: &[prof!(
            "default",
            "Arcade",
            ["arcade"],
            &["zip", "7z"],
            "-rompath {rom_dir} {win}-window {args} {rom_name}"
        )],
    },
    EmulatorDef {
        id: "ares",
        name: "ares",
        flatpak: &["dev.ares.ares"],
        binaries: &["ares"],
        appimage: &["ares*.appimage"],
        flathub: Some("dev.ares.ares"),
        website: "https://ares-emu.net",
        docs_url: Some("https://github.com/ares-emulator/ares#command-line-options"),
        options: NO_OPTS,
        profiles: &[
            prof!("n64", "Nintendo 64", ["nintendo_64"], E_N64, "{fs}--fullscreen --system \"Nintendo 64\" {rom}"),
            prof!(
                "snes",
                "Super Famicom",
                ["nintendo_super_nes"],
                E_SNES,
                "{fs}--fullscreen --system \"Super Famicom\" {rom}"
            ),
            prof!("nes", "Famicom", ["nintendo_nes"], E_NES, "{fs}--fullscreen --system Famicom {rom}"),
            prof!(
                "fds",
                "Famicom Disk System",
                ["nintendo_famicom_disk"],
                E_FDS,
                "{fs}--fullscreen --system \"Famicom Disk System\" {rom}"
            ),
            prof!("gb", "Game Boy", ["nintendo_gameboy"], E_GB, "{fs}--fullscreen --system \"Game Boy\" {rom}"),
            prof!(
                "gbc",
                "Game Boy Color",
                ["nintendo_gameboycolor"],
                E_GB,
                "{fs}--fullscreen --system \"Game Boy Color\" {rom}"
            ),
            prof!(
                "gba",
                "Game Boy Advance",
                ["nintendo_gameboyadvance"],
                E_GBA,
                "{fs}--fullscreen --system \"Game Boy Advance\" {rom}"
            ),
            prof!("md", "Mega Drive", ["sega_genesis"], E_MD, "{fs}--fullscreen --system \"Mega Drive\" {rom}"),
            prof!("mcd", "Mega CD", ["sega_cd"], CD, "{fs}--fullscreen --system \"Mega CD\" {rom}"),
            prof!(
                "32x",
                "Mega 32X",
                ["sega_32x"],
                &["32x", "bin", "zip", "7z"],
                "{fs}--fullscreen --system \"Mega 32X\" {rom}"
            ),
            prof!(
                "sms",
                "Master System",
                ["sega_mastersystem"],
                &["sms", "bin", "zip", "7z"],
                "{fs}--fullscreen --system \"Master System\" {rom}"
            ),
            prof!(
                "gg",
                "Game Gear",
                ["sega_gamegear"],
                &["gg", "bin", "zip", "7z"],
                "{fs}--fullscreen --system \"Game Gear\" {rom}"
            ),
            prof!(
                "sg1000",
                "SG-1000",
                ["sega_sg1000"],
                &["sg", "bin", "zip", "7z"],
                "{fs}--fullscreen --system SG-1000 {rom}"
            ),
            prof!(
                "pce",
                "PC Engine",
                ["nec_turbografx_16"],
                &["pce", "zip", "7z"],
                "{fs}--fullscreen --system \"PC Engine\" {rom}"
            ),
            prof!(
                "pcecd",
                "PC Engine CD",
                ["nec_turbografx_cd"],
                CD,
                "{fs}--fullscreen --system \"PC Engine CD\" {rom}"
            ),
            prof!(
                "sgx",
                "SuperGrafx",
                ["nec_supergrafx"],
                &["sgx", "pce", "zip", "7z"],
                "{fs}--fullscreen --system SuperGrafx {rom}"
            ),
            prof!("psx", "PlayStation", ["sony_playstation"], E_PSX_RA, "{fs}--fullscreen --system PlayStation {rom}"),
            prof!(
                "2600",
                "Atari 2600",
                ["atari_2600"],
                &["a26", "bin", "zip", "7z"],
                "{fs}--fullscreen --system \"Atari 2600\" {rom}"
            ),
            prof!(
                "coleco",
                "ColecoVision",
                ["coleco_vision"],
                &["col", "cv", "bin", "rom", "zip", "7z"],
                "{fs}--fullscreen --system ColecoVision {rom}"
            ),
            prof!(
                "msx",
                "MSX",
                ["microsoft_msx"],
                &["rom", "mx1", "dsk", "cas", "zip", "7z"],
                "{fs}--fullscreen --system MSX {rom}"
            ),
            prof!(
                "msx2",
                "MSX2",
                ["microsoft_msx2"],
                &["rom", "mx2", "dsk", "cas", "zip", "7z"],
                "{fs}--fullscreen --system MSX2 {rom}"
            ),
            prof!(
                "ngp",
                "Neo Geo Pocket",
                ["snk_neogeopocket"],
                &["ngp", "zip", "7z"],
                "{fs}--fullscreen --system \"Neo Geo Pocket\" {rom}"
            ),
            prof!(
                "ngpc",
                "Neo Geo Pocket Color",
                ["snk_neogeopocket_color"],
                &["ngc", "ngpc", "npc", "zip", "7z"],
                "{fs}--fullscreen --system \"Neo Geo Pocket Color\" {rom}"
            ),
            prof!(
                "ws",
                "WonderSwan",
                ["bandai_wonderswan"],
                &["ws", "pc2", "zip", "7z"],
                "{fs}--fullscreen --system WonderSwan {rom}"
            ),
            prof!(
                "wsc",
                "WonderSwan Color",
                ["bandai_wonderswan_color"],
                &["wsc", "zip", "7z"],
                "{fs}--fullscreen --system \"WonderSwan Color\" {rom}"
            ),
        ],
    },
    EmulatorDef {
        id: "mednafen",
        name: "Mednafen",
        // Mednafen itself ships inside the Mednaffe Flatpak (`--command=mednafen`).
        flatpak: &["com.github.AmatCoder.mednaffe"],
        binaries: &["mednafen"],
        appimage: &[],
        flathub: Some("com.github.AmatCoder.mednaffe"),
        website: "https://mednafen.github.io",
        docs_url: Some("https://mednafen.github.io/documentation/"),
        options: NO_OPTS,
        profiles: &[prof!(
            "default",
            "Default",
            [
                "sony_playstation",
                "sega_saturn",
                "nec_turbografx_16",
                "nec_turbografx_cd",
                "nec_supergrafx",
                "nec_pcfx",
                "nintendo_nes",
                "nintendo_super_nes",
                "nintendo_gameboy",
                "nintendo_gameboycolor",
                "nintendo_gameboyadvance",
                "nintendo_virtualboy",
                "sega_genesis",
                "sega_mastersystem",
                "sega_gamegear",
                "atari_lynx",
                "snk_neogeopocket",
                "snk_neogeopocket_color",
                "bandai_wonderswan",
                "bandai_wonderswan_color",
                "apple_2"
            ],
            &[
                "m3u", "cue", "chd", "ccd", "toc", "pce", "sgx", "nes", "unf", "sfc", "smc", "gb", "gbc", "gba", "vb",
                "vboy", "md", "gen", "smd", "sms", "gg", "lnx", "ngp", "ngc", "ngpc", "ws", "wsc", "dsk", "woz", "bin",
                "zip", "7z"
            ],
            "{fs}-video.fs {fs}1 {win}-video.fs {win}0 {rom}"
        )],
    },
    EmulatorDef {
        id: "stella",
        name: "Stella",
        flatpak: &["io.github.stella_emu.Stella"],
        binaries: &["stella"],
        appimage: &[],
        flathub: Some("io.github.stella_emu.Stella"),
        website: "https://stella-emu.github.io",
        docs_url: Some("https://stella-emu.github.io/docs/index.html#CommandLine"),
        options: NO_OPTS,
        profiles: &[prof!(
            "default",
            "Default",
            ["atari_2600"],
            &["a26", "bin", "zip", "7z"],
            "{fs}-fullscreen {fs}1 {rom}"
        )],
    },
    EmulatorDef {
        id: "bigpemu",
        name: "BigPEmu",
        flatpak: &["com.richwhitehouse.BigPEmu"],
        binaries: &["bigpemu"],
        appimage: &["bigpemu/bigpemu"],
        flathub: Some("com.richwhitehouse.BigPEmu"),
        website: "https://www.richwhitehouse.com/jaguar",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!(
            "default",
            "Default",
            ["atari_jaguar"],
            &["j64", "jag", "rom", "abs", "cof", "bin", "prg", "cue", "cdi", "zip"],
            "{rom}"
        )],
    },
    EmulatorDef {
        id: "ruffle",
        name: "Ruffle",
        flatpak: &["rs.ruffle.Ruffle"],
        binaries: &["ruffle"],
        appimage: &["ruffle/ruffle"],
        flathub: Some("rs.ruffle.Ruffle"),
        website: "https://ruffle.rs",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[prof!("default", "Default", ["adobe_flash"], &["swf"], "{fs}--fullscreen {rom}")],
    },
    EmulatorDef {
        id: "dosbox-staging",
        name: "DOSBox Staging",
        flatpak: &["io.github.dosbox-staging"],
        binaries: &["dosbox-staging"],
        appimage: &["dosbox-staging*/dosbox"],
        flathub: Some("io.github.dosbox-staging"),
        website: "https://www.dosbox-staging.org",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[
            prof!("default", "Program", ["pc_dos"], &["exe", "com", "bat"], "{fs}--fullscreen -exit {rom}"),
            prof!("conf", "Config file", ["pc_dos"], &["conf"], "{fs}--fullscreen {args} -conf {rom}"),
        ],
    },
    EmulatorDef {
        id: "dosbox-x",
        name: "DOSBox-X",
        flatpak: &["com.dosbox_x.DOSBox-X"],
        binaries: &["dosbox-x"],
        appimage: &[],
        flathub: Some("com.dosbox_x.DOSBox-X"),
        website: "https://dosbox-x.com",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[
            prof!("default", "Program", ["pc_dos"], &["exe", "com", "bat"], "{fs}-fullscreen -exit {rom}"),
            prof!("conf", "Config file", ["pc_dos"], &["conf"], "{fs}-fullscreen {args} -conf {rom}"),
        ],
    },
    EmulatorDef {
        id: "dosbox",
        name: "DOSBox",
        flatpak: &["com.dosbox.DOSBox"],
        binaries: &["dosbox"],
        appimage: &[],
        flathub: Some("com.dosbox.DOSBox"),
        website: "https://www.dosbox.com",
        docs_url: None,
        options: NO_OPTS,
        profiles: &[
            prof!("default", "Program", ["pc_dos"], &["exe", "com", "bat"], "{fs}-fullscreen -exit {rom}"),
            prof!("conf", "Config file", ["pc_dos"], &["conf"], "{fs}-fullscreen {args} -conf {rom}"),
        ],
    },
    EmulatorDef {
        id: "scummvm",
        name: "ScummVM",
        flatpak: &["org.scummvm.ScummVM"],
        binaries: &["scummvm"],
        appimage: &[],
        flathub: Some("org.scummvm.ScummVM"),
        website: "https://www.scummvm.org",
        docs_url: Some("https://docs.scummvm.org/en/latest/advanced_topics/command_line.html"),
        options: NO_OPTS,
        // A `.scummvm` / `.svm` file marks the game's folder; ScummVM finds the game in it.
        profiles: &[prof!(
            "default",
            "Default",
            ["pc_dos"],
            &["scummvm", "svm"],
            "{fs}-f {args} -p {rom_dir} --auto-detect"
        )],
    },
    // ---- RetroArch (last: per console, `retroarch_first` decides whether it wins) -------------
    EmulatorDef {
        id: "retroarch",
        name: "RetroArch",
        flatpak: &["org.libretro.RetroArch"],
        binaries: &["retroarch"],
        appimage: &["retroarch-linux*.appimage", "retroarch*.appimage"],
        flathub: Some("org.libretro.RetroArch"),
        website: "https://www.retroarch.com",
        docs_url: Some("https://docs.libretro.com/guides/cli-intro/"),
        options: NO_OPTS,
        profiles: &[
            core!("mesen", "Mesen", ["nintendo_nes", "nintendo_famicom_disk"], E_NES),
            core!("nestopia", "Nestopia", ["nintendo_nes", "nintendo_famicom_disk"], E_NES),
            core!("fceumm", "FCEUmm", ["nintendo_nes", "nintendo_famicom_disk"], E_NES),
            core!("quicknes", "QuickNES", ["nintendo_nes"], E_NES),
            core!("snes9x", "Snes9x", ["nintendo_super_nes"], E_SNES),
            core!("bsnes", "bsnes", ["nintendo_super_nes"], E_SNES),
            core!("bsnes_hd_beta", "bsnes-hd beta", ["nintendo_super_nes"], E_SNES),
            core!("mesen-s", "Mesen-S", ["nintendo_super_nes"], E_SNES),
            core!("mednafen_supafaust", "Beetle Supafaust", ["nintendo_super_nes"], E_SNES),
            core!("gambatte", "Gambatte", ["nintendo_gameboy", "nintendo_gameboycolor"], E_GB),
            core!("sameboy", "SameBoy", ["nintendo_gameboy", "nintendo_gameboycolor"], E_GB),
            core!("mgba", "mGBA", ["nintendo_gameboyadvance", "nintendo_gameboy", "nintendo_gameboycolor"], E_MGBA),
            core!("gearboy", "Gearboy", ["nintendo_gameboy", "nintendo_gameboycolor"], E_GB),
            core!("tgbdual", "TGB Dual", ["nintendo_gameboy", "nintendo_gameboycolor"], E_GB),
            core!("vbam", "VBA-M", ["nintendo_gameboyadvance", "nintendo_gameboy", "nintendo_gameboycolor"], E_MGBA),
            core!("vba_next", "VBA Next", ["nintendo_gameboyadvance"], E_GBA),
            core!("gpsp", "gpSP", ["nintendo_gameboyadvance"], E_GBA),
            core!("mupen64plus_next", "Mupen64Plus-Next", ["nintendo_64"], E_N64),
            core!("parallel_n64", "ParaLLEl N64", ["nintendo_64"], E_N64),
            core!("melondsds", "melonDS DS", ["nintendo_ds", "nintendo_dsi"], E_NDS),
            core!("melonds", "melonDS", ["nintendo_ds"], E_NDS),
            core!("desmume", "DeSmuME", ["nintendo_ds"], E_NDS),
            core!("azahar", "Azahar", ["nintendo_3ds"], E_3DS),
            core!("citra", "Citra", ["nintendo_3ds"], E_3DS),
            core!("dolphin", "Dolphin", ["nintendo_gamecube", "nintendo_wii"], E_DOLPHIN),
            core!("mednafen_vb", "Beetle VB", ["nintendo_virtualboy"], &["vb", "vboy", "bin", "zip", "7z"]),
            core!("mednafen_psx_hw", "Beetle PSX HW", ["sony_playstation"], E_PSX_RA),
            core!("mednafen_psx", "Beetle PSX", ["sony_playstation"], E_PSX_RA),
            core!("pcsx_rearmed", "PCSX ReARMed", ["sony_playstation"], E_PSX_RA),
            core!("swanstation", "SwanStation", ["sony_playstation"], E_PSX_RA),
            core!("pcsx2", "LRPS2", ["sony_playstation2"], E_PS2_RA),
            core!("play", "Play!", ["sony_playstation2"], E_PS2_RA),
            core!("ppsspp", "PPSSPP", ["sony_psp"], E_PSP),
            core!(
                "genesis_plus_gx",
                "Genesis Plus GX",
                ["sega_genesis", "sega_cd", "sega_mastersystem", "sega_gamegear", "sega_sg1000"],
                E_GPGX
            ),
            core!("picodrive", "PicoDrive", ["sega_32x", "sega_genesis", "sega_cd", "sega_mastersystem"], E_PICO),
            core!("blastem", "BlastEm", ["sega_genesis"], E_MD),
            core!("gearsystem", "Gearsystem", ["sega_mastersystem", "sega_gamegear", "sega_sg1000"], E_SMS),
            core!("mednafen_saturn", "Beetle Saturn", ["sega_saturn"], CD),
            core!("kronos", "Kronos", ["sega_saturn"], CD),
            core!("yabasanshiro", "YabaSanshiro", ["sega_saturn"], CD),
            core!("yabause", "Yabause", ["sega_saturn"], CD),
            core!("flycast", "Flycast", ["sega_dreamcast"], E_DC),
            core!("mednafen_pce_fast", "Beetle PCE Fast", ["nec_turbografx_16", "nec_turbografx_cd"], E_PCE),
            core!("mednafen_pce", "Beetle PCE", ["nec_turbografx_16", "nec_turbografx_cd", "nec_supergrafx"], E_PCE),
            core!(
                "mednafen_supergrafx",
                "Beetle SuperGrafx",
                ["nec_supergrafx", "nec_turbografx_16", "nec_turbografx_cd"],
                E_PCE
            ),
            core!("mednafen_pcfx", "Beetle PC-FX", ["nec_pcfx"], CD),
            core!(
                "mednafen_ngp",
                "Beetle NeoPop",
                ["snk_neogeopocket", "snk_neogeopocket_color"],
                &["ngp", "ngc", "ngpc", "npc", "zip", "7z"]
            ),
            core!(
                "race",
                "RACE",
                ["snk_neogeopocket", "snk_neogeopocket_color"],
                &["ngp", "ngc", "ngpc", "npc", "zip", "7z"]
            ),
            core!(
                "mednafen_wswan",
                "Beetle WonderSwan",
                ["bandai_wonderswan", "bandai_wonderswan_color"],
                &["ws", "wsc", "pc2", "zip", "7z"]
            ),
            core!("stella", "Stella", ["atari_2600"], &["a26", "bin", "zip", "7z"]),
            core!("prosystem", "ProSystem", ["atari_7800"], &["a78", "bin", "zip", "7z"]),
            core!("handy", "Handy", ["atari_lynx"], &["lnx", "o", "zip", "7z"]),
            core!("mednafen_lynx", "Beetle Lynx", ["atari_lynx"], &["lnx", "o", "zip", "7z"]),
            core!(
                "virtualjaguar",
                "Virtual Jaguar",
                ["atari_jaguar"],
                &["j64", "jag", "rom", "abs", "cof", "bin", "zip", "7z"]
            ),
            core!(
                "atari800",
                "Atari800",
                ["atari_5200", "atari_8bit"],
                &["a52", "atr", "atx", "xfd", "xex", "cas", "car", "com", "bin", "rom", "dcm", "zip", "7z"]
            ),
            core!("a5200", "a5200", ["atari_5200"], &["a52", "bin", "zip", "7z"]),
            core!(
                "hatari",
                "Hatari",
                ["atari_st", "atari_falcon030"],
                &["st", "msa", "stx", "dim", "ipf", "m3u", "zip", "7z"]
            ),
            core!("opera", "Opera", ["3do"], &["chd", "cue", "iso", "bin", "zip", "7z"]),
            core!("dosbox_pure", "DOSBox Pure", ["pc_dos"], E_DOS),
            core!("dosbox_core", "DOSBox-core", ["pc_dos"], E_DOS),
            core!(
                "vice_x64sc",
                "VICE x64sc",
                ["commodore_64"],
                &["d64", "d71", "d81", "g64", "t64", "tap", "prg", "p00", "crt", "x64", "nib", "m3u", "zip", "7z"]
            ),
            core!(
                "vice_xvic",
                "VICE xvic",
                ["commodore_vci20"],
                &["20", "40", "60", "a0", "b0", "crt", "d64", "prg", "tap", "zip", "7z"]
            ),
            core!("vice_xplus4", "VICE xplus4", ["commodore_plus4"], &["d64", "prg", "tap", "crt", "t64", "zip", "7z"]),
            core!("vice_xpet", "VICE xpet", ["commodore_pet"], &["d64", "d80", "d82", "prg", "tap", "zip", "7z"]),
            core!("vice_xcbm2", "VICE xcbm2", ["commodore_cbm2"], &["d64", "d80", "d82", "prg", "crt", "zip", "7z"]),
            core!(
                "vice_xcbm5x0",
                "VICE xcbm5x0",
                ["commodore_cbm5x0"],
                &["d64", "d80", "d82", "prg", "crt", "zip", "7z"]
            ),
            core!(
                "puae",
                "PUAE",
                ["commodore_amiga", "commodore_amiga_cd32"],
                &["adf", "adz", "dms", "ipf", "lha", "hdf", "hdz", "uae", "m3u", "chd", "cue", "iso", "zip", "7z"]
            ),
            core!(
                "bluemsx",
                "blueMSX",
                ["microsoft_msx", "microsoft_msx2", "coleco_vision"],
                &["rom", "mx1", "mx2", "dsk", "cas", "ri", "col", "m3u", "zip", "7z"]
            ),
            core!(
                "fmsx",
                "fMSX",
                ["microsoft_msx", "microsoft_msx2"],
                &["rom", "mx1", "mx2", "dsk", "cas", "m3u", "zip", "7z"]
            ),
            core!("gearcoleco", "Gearcoleco", ["coleco_vision"], &["col", "cv", "bin", "rom", "zip", "7z"]),
            core!("freeintv", "FreeIntv", ["mattel_intellivision"], &["int", "bin", "rom", "zip", "7z"]),
            core!("o2em", "O2EM", ["magnavox_odyssey_2"], &["o2", "bin", "zip", "7z"]),
            core!("vecx", "vecx", ["vectrex"], &["vec", "gam", "bin", "zip", "7z"]),
            core!("potator", "Potator", ["watara_supervision"], &["sv", "bin", "zip", "7z"]),
            core!(
                "cap32",
                "Caprice32",
                ["amstrad_cpc"],
                &["dsk", "sna", "cdt", "cpr", "tap", "voc", "kcr", "m3u", "zip", "7z"]
            ),
            core!(
                "fuse",
                "Fuse",
                ["sinclair_zxspectrum", "sinclair_zxspectrum3"],
                &["tzx", "tap", "z80", "rzx", "scl", "trd", "pzx", "dsk", "zip", "7z"]
            ),
            core!("81", "EightyOne", ["sinclair_zx81"], &["p", "t81", "tzx", "zip", "7z"]),
            core!(
                "px68k",
                "PX68k",
                ["sharp_x68000"],
                &["dim", "d88", "88d", "hdm", "dup", "2hd", "xdf", "hdf", "img", "m3u", "zip", "7z"]
            ),
            core!(
                "np2kai",
                "Neko Project II Kai",
                ["nec_pc98"],
                &[
                    "d98", "98d", "fdi", "fdd", "hdi", "hdd", "nhd", "d88", "88d", "2hd", "tfd", "thd", "hdm", "xdf",
                    "zip", "7z"
                ]
            ),
            core!(
                "theodore",
                "Theodore",
                ["thomson_mo5", "thomson_to7"],
                &["fd", "k7", "m5", "m7", "rom", "sap", "zip", "7z"]
            ),
            core!("tic80", "TIC-80", ["tic_80"], &["tic", "zip", "7z"]),
            core!("uzem", "Uzem", ["uzebox"], &["uze", "zip"]),
            core!("neocd", "NeoCD", ["snk_neogeo_cd"], &["chd", "cue", "zip", "7z"]),
            core!("fbneo", "FinalBurn Neo", ["arcade", "snk_neogeo_aes"], E_ARCADE),
            core!("mame", "MAME", ["arcade"], E_ARCADE),
            core!("mame2003_plus", "MAME 2003-Plus", ["arcade"], E_ARCADE),
        ],
    },
];

pub fn all() -> &'static [EmulatorDef] {
    EMULATORS
}

pub fn by_id(id: &str) -> Option<&'static EmulatorDef> {
    all().iter().find(|d| d.id == id)
}

/// The program to run inside a definition's Flatpak when it isn't the app's default command
/// (`flatpak run --command=<this> <app>`).
pub fn flatpak_command(def: &str) -> Option<&'static str> {
    match def {
        "shadps4" => Some("shadps4"),
        "mednafen" => Some("mednafen"),
        _ => None,
    }
}

/// Consoles where RetroArch is the usual choice: cartridge-era consoles and old computers, where
/// its cores are the best-maintained emulators. Everywhere else (PlayStation, GameCube / Wii,
/// Wii U, Switch, 3DS, DS, PSP, Game Boy family with mGBA, Dreamcast, N64 …) a standalone
/// emulator is preferred when both are installed.
pub fn retroarch_first(platform: &str) -> bool {
    !matches!(
        platform,
        "sony_playstation"
            | "sony_playstation2"
            | "sony_playstation3"
            | "sony_playstation4"
            | "sony_psp"
            | "sony_vita"
            | "nintendo_gamecube"
            | "nintendo_wii"
            | "nintendo_wiiu"
            | "nintendo_switch"
            | "nintendo_3ds"
            | "nintendo_ds"
            | "nintendo_dsi"
            | "nintendo_gameboyadvance"
            | "nintendo_gameboy"
            | "nintendo_gameboycolor"
            | "nintendo_64"
            | "sega_dreamcast"
            | "xbox"
            | "atari_jaguar"
            | "adobe_flash"
            | "pc_dos"
    )
}

/// Built-in emulators for a console, best first (the order `choose` uses).
pub fn for_platform(platform: &str) -> Vec<&'static EmulatorDef> {
    let mut v: Vec<&'static EmulatorDef> =
        all().iter().filter(|d| d.profiles.iter().any(|p| p.platforms.contains(&platform))).collect();
    if retroarch_first(platform) {
        v.sort_by_key(|d| d.id != "retroarch");
    }
    v
}

/// Is this menu entry an emulator itself (not a game)? `desktop_id` is the entry's file name
/// without `.desktop`, `argv` its parsed `Exec`. True when it starts a known emulator — by
/// Flatpak id, program / AppImage name, entry id or name — with no game file after it (an entry
/// that starts an emulator *with* a game is a game shortcut and stays).
pub fn emulator_app(desktop_id: &str, argv: &[String], name: &str) -> Option<&'static EmulatorDef> {
    let mut it = argv.iter().map(String::as_str);
    let mut prog = it.next()?;
    let base = |s: &str| s.rsplit('/').next().unwrap_or(s).to_string();
    if base(prog) == "env" {
        prog = it.by_ref().find(|a| !a.contains('='))?;
    }
    let rest: Vec<&str> = it.collect();
    let only_options = |v: &[&str]| v.iter().all(|a| a.starts_with('-'));
    if base(prog) == "flatpak" {
        // flatpak run [--opts] <app id> [args]
        let mut r = rest.iter().copied();
        if r.next() != Some("run") {
            return None;
        }
        let tail: Vec<&str> = r.skip_while(|a| a.starts_with('-')).collect();
        let (app, args) = tail.split_first()?;
        return all()
            .iter()
            .find(|d| d.flatpak.iter().any(|f| f.eq_ignore_ascii_case(app)))
            .filter(|_| only_options(args));
    }
    if !only_options(&rest) {
        return None;
    }
    let pname = base(prog);
    all().iter().find(|d| {
        d.binaries.iter().any(|b| b.eq_ignore_ascii_case(&pname))
            || d.appimage.iter().any(|pat| !pat.contains('/') && super::detect::glob_match(pat, &pname))
            || d.flatpak.iter().any(|f| f.eq_ignore_ascii_case(desktop_id))
            || d.name.eq_ignore_ascii_case(name.trim())
    })
}

/// The emulator to suggest installing for a console: the best one that is on Flathub.
pub fn suggest(platform: &str) -> Option<&'static EmulatorDef> {
    let v = for_platform(platform);
    v.iter().find(|d| d.flathub.is_some()).or(v.first()).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defs_are_consistent() {
        let mut ids = std::collections::HashSet::new();
        for d in all() {
            assert!(ids.insert(d.id), "duplicate {}", d.id);
            assert!(!d.profiles.is_empty(), "{}", d.id);
            let mut pids = std::collections::HashSet::new();
            for p in d.profiles {
                assert!(pids.insert(p.id), "{}: duplicate profile {}", d.id, p.id);
                for plat in p.platforms {
                    assert!(super::super::platforms::by_id(plat).is_some(), "{}/{}: {plat}", d.id, p.id);
                }
                assert!(p.args.contains("{rom") || p.args.contains("{Image"), "{}/{}", d.id, p.id);
                assert!(crate::launch::split_command(p.args).is_ok(), "{}/{}", d.id, p.id);
                for e in p.extensions {
                    assert_eq!(*e, e.to_ascii_lowercase(), "{}/{}", d.id, p.id);
                }
                assert_eq!(p.core.is_some(), d.id == "retroarch", "{}/{}", d.id, p.id);
            }
            for g in d.appimage {
                assert_eq!(*g, g.to_lowercase(), "{}", d.id);
            }
        }
        for id in [
            "retroarch",
            "duckstation",
            "pcsx2",
            "ppsspp",
            "dolphin",
            "cemu",
            "rpcs3",
            "eden",
            "ryujinx",
            "citron",
            "sudachi",
            "shadps4",
            "azahar",
            "lime3ds",
            "citra",
            "melonds",
            "xemu",
            "flycast",
            "mgba",
            "mednafen",
            "mame",
            "scummvm",
            "vita3k",
            "stella",
            "snes9x",
            "bsnes",
            "mesen",
            "simple64",
            "rmg",
            "mupen64plus",
            "redream",
            "ares",
            "bigpemu",
            "supermodel",
            "dosbox-staging",
            "dosbox-x",
            "ruffle",
            "primehack",
        ] {
            assert!(by_id(id).is_some(), "{id}");
        }
    }

    #[test]
    fn pcsx2_opens_no_cue_and_duckstation_prefers_playlists() {
        let p = by_id("pcsx2").unwrap().profiles[0];
        assert!(!p.extensions.contains(&"cue") && !p.extensions.contains(&"m3u"));
        assert_eq!(p.extensions[0], "chd");
        let d = by_id("duckstation").unwrap().profiles[0];
        assert_eq!(&d.extensions[..3], &["m3u", "cue", "chd"]);
        assert!(d.args.contains("-batch") && d.args.contains("{fs}-fullscreen"));
        let c = by_id("cemu").unwrap().profiles[0];
        assert_eq!(c.folder_launch[0], "title.tmd");
    }

    #[test]
    fn preference_order() {
        assert_eq!(for_platform("sony_playstation")[0].id, "duckstation");
        assert_eq!(for_platform("nintendo_super_nes")[0].id, "retroarch");
        assert_eq!(for_platform("nintendo_switch")[0].id, "eden");
        assert_eq!(suggest("nintendo_switch").unwrap().flathub, Some("dev.eden_emu.eden"));
        assert_eq!(suggest("nintendo_nes").unwrap().id, "retroarch");
        assert_eq!(suggest("sony_playstation2").unwrap().id, "pcsx2");
        assert!(for_platform("xbox_one").is_empty());
        assert_eq!(flatpak_command("shadps4"), Some("shadps4"));
    }
}
