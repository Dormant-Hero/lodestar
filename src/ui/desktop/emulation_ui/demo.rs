//! Made-up emulators and BIOS states for the `settings-emulation-demo` screenshot target and
//! the page tests (the real ones depend on the machine).

use super::catalog::{BiosData, BiosInfo, CatEmu, Catalog};
use crate::config::EmulationConfig;
use crate::emulation::bios::{BiosFile, BiosState, BiosStatus, BiosTarget, Package, PackageKind, PackageTarget, Step};
use crate::emulation::detect::{Emulator, InstallKind, Profile};
use crate::emulation::registry::Registry;
use std::collections::BTreeMap;
use std::path::PathBuf;

static PS2_USA: BiosFile = BiosFile {
    platform: "sony_playstation2",
    file: "scph39001.bin",
    md5: &[],
    required: true,
    group: Some("ps2"),
    region: Some("USA"),
    note: "",
};
static PS2_EUR: BiosFile = BiosFile {
    platform: "sony_playstation2",
    file: "scph70004.bin",
    md5: &[],
    required: false,
    group: Some("ps2"),
    region: Some("Europe"),
    note: "",
};
static PS1_USA: BiosFile = BiosFile {
    platform: "sony_playstation",
    file: "scph5501.bin",
    md5: &[],
    required: true,
    group: Some("ps1"),
    region: Some("USA"),
    note: "",
};
static SWITCH_FW: BiosFile = BiosFile {
    platform: "nintendo_switch",
    file: "firmware",
    md5: &[],
    required: true,
    group: None,
    region: None,
    note: "",
};
static SWITCH_KEYS: BiosFile = BiosFile {
    platform: "nintendo_switch",
    file: "prod.keys",
    md5: &[],
    required: true,
    group: None,
    region: None,
    note: "From your own Switch",
};

fn emu(id: &str, name: &str, app: &str) -> Emulator {
    Emulator {
        id: id.into(),
        def: id.into(),
        name: name.into(),
        kind: InstallKind::Flatpak { app_id: app.into() },
        how: "Flatpak".into(),
        user: false,
        cores: vec![],
        args: String::new(),
        auto: true,
        blocked: false,
    }
}

fn profile(id: &str, name: &str, platforms: &[&str], exts: &[&str], args: &str) -> Profile {
    Profile {
        id: id.into(),
        name: name.into(),
        platforms: platforms.iter().map(|s| s.to_string()).collect(),
        extensions: exts.iter().map(|s| s.to_string()).collect(),
        args: args.into(),
        core: None,
        folder_launch: vec![],
        custom: false,
        ..Default::default()
    }
}

/// A catalog with six emulators (one an AppImage that isn't allowed to run yet) and BIOS states
/// for PS1 (OK), PS2 (found, copyable) and Switch (keys found, firmware found, not installed).
pub(super) fn demo() -> (Catalog, BiosData) {
    let home = dirs::home_dir().unwrap_or_default();
    let emus = vec![
        CatEmu {
            emu: emu("pcsx2", "PCSX2", "net.pcsx2.PCSX2"),
            profiles: vec![profile(
                "default",
                "Default",
                &["sony_playstation2"],
                &["chd", "iso", "bin"],
                "-batch -fullscreen -- {rom}",
            )],
        },
        CatEmu {
            emu: emu("duckstation", "DuckStation", "org.duckstation.DuckStation"),
            profiles: vec![profile(
                "default",
                "Default",
                &["sony_playstation"],
                &["m3u", "cue", "chd"],
                "-batch -fullscreen -- {rom}",
            )],
        },
        CatEmu {
            emu: emu("dolphin", "Dolphin", "org.DolphinEmu.dolphin-emu"),
            profiles: vec![profile("default", "Default", &["nintendo_gamecube"], &["rvz", "iso"], "-b -e {rom}")],
        },
        CatEmu {
            emu: emu("mgba", "mGBA", "io.mgba.mGBA"),
            profiles: vec![profile("default", "Default", &["nintendo_gameboyadvance"], &["gba"], "-f {rom}")],
        },
        CatEmu {
            emu: emu("eden", "Eden", "dev.eden_emu.eden"),
            profiles: vec![profile("default", "Default", &["nintendo_switch"], &["nsp", "xci"], "-f -g {rom}")],
        },
        CatEmu {
            emu: Emulator {
                kind: InstallKind::AppImage { path: home.join("Applications/rpcs3-v0.0.43_linux64.AppImage") },
                how: "AppImage in ~/Applications — not allowed to run yet".into(),
                blocked: true,
                ..emu("rpcs3", "RPCS3", "")
            },
            profiles: vec![profile("default", "Default", &["sony_playstation3"], &["bin"], "--no-gui {rom}")],
        },
    ];
    let auto: BTreeMap<String, (String, String)> = [
        ("sony_playstation2", "pcsx2"),
        ("sony_playstation", "duckstation"),
        ("nintendo_gamecube", "dolphin"),
        ("nintendo_gameboyadvance", "mgba"),
        ("nintendo_switch", "eden"),
    ]
    .into_iter()
    .map(|(p, e)| (p.to_string(), (e.to_string(), "default".to_string())))
    .collect();
    let reg = Registry {
        emulators: emus.iter().map(|c| c.emu.clone()).collect(),
        config: EmulationConfig::default(),
        home: home.clone(),
    };
    let roms = PathBuf::from("/mnt/games");
    let mut by_platform = BTreeMap::new();
    by_platform.insert(
        "sony_playstation2".to_string(),
        BiosInfo {
            status: BiosStatus {
                platform: "sony_playstation2".into(),
                state: BiosState::Ok,
                files: vec![
                    (&PS2_USA, BiosState::Ok, Some(roms.join("ps2/bios/scph39001.bin"))),
                    (&PS2_EUR, BiosState::Missing, None),
                ],
                targets: vec![BiosTarget {
                    emulator: "pcsx2".into(),
                    dir: home.join(".var/app/net.pcsx2.PCSX2/config/PCSX2/bios"),
                    missing: vec![roms.join("ps2/bios/scph39001.bin")],
                    ready: false,
                    firmware: None,
                }],
                note: String::new(),
                packages: vec![],
            },
            conflicts: BTreeMap::new(),
            plan: vec![Step::Copy {
                emulator: "pcsx2".into(),
                dir: home.join(".var/app/net.pcsx2.PCSX2/config/PCSX2/bios"),
                files: vec![(
                    roms.join("ps2/bios/scph39001.bin"),
                    home.join(".var/app/net.pcsx2.PCSX2/config/PCSX2/bios/scph39001.bin"),
                )],
            }],
        },
    );
    by_platform.insert(
        "sony_playstation".to_string(),
        BiosInfo {
            status: BiosStatus {
                platform: "sony_playstation".into(),
                state: BiosState::Ok,
                files: vec![(
                    &PS1_USA,
                    BiosState::Ok,
                    Some(home.join(".var/app/org.duckstation.DuckStation/config/duckstation/bios/scph5501.bin")),
                )],
                targets: vec![BiosTarget {
                    emulator: "duckstation".into(),
                    dir: home.join(".var/app/org.duckstation.DuckStation/config/duckstation/bios"),
                    missing: vec![],
                    ready: true,
                    firmware: None,
                }],
                note: String::new(),
                packages: vec![],
            },
            conflicts: BTreeMap::new(),
            plan: vec![],
        },
    );
    by_platform.insert(
        "nintendo_switch".to_string(),
        BiosInfo {
            status: BiosStatus {
                platform: "nintendo_switch".into(),
                state: BiosState::Missing,
                files: vec![
                    (&SWITCH_KEYS, BiosState::Ok, Some(home.join("Documents/bios/switch/prod.keys"))),
                    (&SWITCH_FW, BiosState::Missing, None),
                ],
                targets: vec![BiosTarget {
                    emulator: "eden".into(),
                    dir: home.join(".var/app/dev.eden_emu.eden/data/eden/keys"),
                    missing: vec![home.join("Documents/bios/switch/prod.keys")],
                    ready: false,
                    firmware: None,
                }],
                note: String::new(),
                packages: vec![Package {
                    kind: PackageKind::SwitchFirmware,
                    path: home.join("Documents/bios/switch/Firmware.23.5.0"),
                    version: Some("23.5.0".into()),
                    files: 238,
                    targets: vec![PackageTarget {
                        emulator: "eden".into(),
                        dir: home.join(".var/app/dev.eden_emu.eden/data/eden/nand/system/Contents/registered"),
                        installed: None,
                    }],
                }],
            },
            conflicts: BTreeMap::new(),
            plan: vec![
                Step::Copy {
                    emulator: "eden".into(),
                    dir: home.join(".var/app/dev.eden_emu.eden/data/eden/keys"),
                    files: vec![(
                        home.join("Documents/bios/switch/prod.keys"),
                        home.join(".var/app/dev.eden_emu.eden/data/eden/keys/prod.keys"),
                    )],
                },
                Step::SwitchFirmware {
                    emulator: "eden".into(),
                    dir: home.join(".var/app/dev.eden_emu.eden/data/eden/nand/system/Contents/registered"),
                    from: home.join("Documents/bios/switch/Firmware.23.5.0"),
                    label: "Firmware 23.5.0".into(),
                    files: 238,
                },
            ],
        },
    );
    let ds_bios = home.join(".var/app/org.duckstation.DuckStation/config/duckstation/bios");
    let data = BiosData {
        by_platform,
        search_dirs: vec![ds_bios.clone(), roms.join("ps2/bios")],
        own: [(ds_bios, "DuckStation".to_string())].into(),
    };
    (Catalog::from_parts(reg, emus, auto), data)
}
