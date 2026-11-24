//! Sign-in status from emulator config fixtures written like the upstream files
//! (`tests/fixtures/ra/emulators`; every token in them is fake).

use super::*;
use crate::emulation::detect::{Emulator, InstallKind};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ra/emulators")
}

fn emu(id: &str, def: &str, name: &str, kind: InstallKind) -> Emulator {
    Emulator {
        id: id.into(),
        def: def.into(),
        name: name.into(),
        kind,
        how: String::new(),
        user: false,
        cores: vec![],
        args: String::new(),
        auto: true,
        blocked: false,
    }
}

fn flatpak(app: &str) -> InstallKind {
    InstallKind::Flatpak { app_id: app.into() }
}

fn native(bin: &str) -> InstallKind {
    InstallKind::Binary { path: PathBuf::from("/usr/bin").join(bin) }
}

fn signed(u: &str) -> SigninStatus {
    SigninStatus::SignedIn { username: u.into() }
}

/// Every fake token in the fixtures.
const TOKENS: &[&str] = &[
    "FAKEtokenRetroA1",
    "FAKEtokenKeychn1",
    "U0VBTEVEVE9LRU5",
    "RkFLRUVOQ1JZUFRFRFRPS0VOMDA",
    "FAKEtokenPCSX2a1",
    "FAKEtokenPCSX2b2",
    "FAKEtokenPPSSPP1",
    "FAKEtokenDolphn1",
    "FAKEtokenFlycst1",
];

#[test]
fn every_emulator_config_format_gives_the_right_status() {
    let f = fixtures();
    let home = f.join("home");
    let prog = |dir: &str| InstallKind::Binary { path: f.join("portable").join(dir).join("retroarch") };
    let emus = vec![
        emu("ra-native", "retroarch", "RetroArch", native("retroarch")),
        emu("ra-flatpak", "retroarch", "RetroArch", flatpak("org.libretro.RetroArch")),
        emu("ra-sealed", "retroarch", "RetroArch", prog("RetroArch-keychain")),
        emu("ra-keychain", "retroarch", "RetroArch", prog("RetroArch-plainkeychain")),
        emu("ds-flatpak", "duckstation", "DuckStation", flatpak("org.duckstation.DuckStation")),
        emu("ds-native", "duckstation", "DuckStation", native("duckstation-qt")),
        emu("p2-flatpak", "pcsx2", "PCSX2", flatpak("net.pcsx2.PCSX2")),
        emu("p2-native", "pcsx2", "PCSX2", native("pcsx2-qt")),
        emu("pp-flatpak", "ppsspp", "PPSSPP", flatpak("org.ppsspp.PPSSPP")),
        emu("pp-native", "ppsspp", "PPSSPP", native("PPSSPPSDL")),
        emu("do-flatpak", "dolphin", "Dolphin", flatpak("org.DolphinEmu.dolphin-emu")),
        emu("do-native", "dolphin", "Dolphin", native("dolphin-emu")),
        emu("fc-flatpak", "flycast", "Flycast", flatpak("org.flycast.Flycast")),
        emu("fc-native", "flycast", "Flycast", native("flycast")),
        // No RetroAchievements: not listed.
        emu("mgba", "mgba", "mGBA", native("mgba-qt")),
        emu("g64", "gopher64", "gopher64", flatpak("io.github.gopher64.gopher64")),
    ];
    let got = signins_in(&emus, &home);
    let by: std::collections::HashMap<&str, &Signin> = got.iter().map(|s| (s.instance.as_str(), s)).collect();
    let want: &[(&str, SigninStatus)] = &[
        ("ra-native", signed("NativeArcher")),
        // Achievements on, a name, but the token is empty.
        ("ra-flatpak", SigninStatus::NotSignedIn),
        // The keychain's sealed values: signed in, the name can't be read.
        ("ra-sealed", signed("")),
        // The keychain's plain values win over retroarch.cfg's.
        ("ra-keychain", signed("KeychainArcher")),
        ("ds-flatpak", signed("DuckFan")),
        // Signed in but achievements off.
        ("ds-native", SigninStatus::Disabled),
        // PCSX2 2.6: the token in secrets.ini; 2.5: in PCSX2.ini.
        ("p2-flatpak", signed("PS2Player")),
        ("p2-native", signed("OldPCSX2")),
        ("pp-flatpak", signed("PocketPlayer")),
        // A name but no token file.
        ("pp-native", SigninStatus::NotSignedIn),
        ("do-flatpak", signed("CubeFan")),
        // Dolphin set up, achievements never touched.
        ("do-native", SigninStatus::Disabled),
        ("fc-flatpak", signed("DreamFan")),
        // No settings at all.
        ("fc-native", SigninStatus::Unknown),
    ];
    assert_eq!(got.len(), want.len(), "{got:#?}");
    for (id, st) in want {
        assert_eq!(&by[id].status, st, "{id}");
    }
    assert_eq!(by["ds-flatpak"].label, "DuckStation (Flatpak)");
    assert_eq!(by["ds-flatpak"].where_to_sign_in, "Settings → Achievements");
    assert_eq!(by["pp-flatpak"].where_to_sign_in, "Settings → Tools → RetroAchievements");
    assert_eq!(by["do-flatpak"].where_to_sign_in, "Tools → Achievements → Settings");
    assert_eq!(by["fc-flatpak"].where_to_sign_in, "Settings → General → RetroAchievements");
    // No token ever reaches the result (or its Debug text, which is what a log would show).
    let dbg = format!("{got:?}");
    for t in TOKENS {
        assert!(!dbg.contains(t), "{t} in {dbg}");
    }
}

#[test]
fn ini_reading_is_per_section_and_case_exact() {
    let text =
        "[Main]\nUsername = wrong\n[Cheevos]\n; Username = commented\nEnabled = True\nUsername =  Spaced  \nToken =\n";
    let mut f = Found::default();
    read_ini(text, "Cheevos", "Enabled", "Username", "Token", &mut f);
    assert_eq!((f.enabled, f.username.as_deref(), f.token), (Some(true), Some("Spaced"), Some(false)));
    assert_eq!(finish(f, false), SigninStatus::NotSignedIn);
    // Flycast's lower-case section is its own.
    let mut f = Found::default();
    read_ini("[Achievements]\nEnabled = yes\n", "achievements", "Enabled", "UserName", "Token", &mut f);
    assert_eq!(f.enabled, None);
    for (v, b) in [("true", true), ("True", true), ("1", true), ("yes", true), ("false", false), ("no", false)] {
        assert_eq!(truthy(v), b, "{v}");
    }
}
