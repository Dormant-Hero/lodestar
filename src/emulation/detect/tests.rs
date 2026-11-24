use super::*;
use crate::config::{EmulationConfig, EmulatorEntry, ProfileEntry};
use crate::test_support::{write_exe, write_file};
use std::path::Path;

/// The `filesystems=` lines of the user's real 7 Flatpaks (read from their installed metadata).
pub(crate) const USER_FLATPAKS: &[(&str, &str, &str)] = &[
    ("dev.eden_emu.eden", "eden-launcher", "xdg-run/app/com.discordapp.Discord:ro;xdg-config/kdeglobals:ro;"),
    ("io.mgba.mGBA", "mgba-qt", "xdg-config/kdeglobals:ro;host;"),
    ("net.pcsx2.PCSX2", "pcsx2-qt", "xdg-config/kdeglobals:ro;xdg-run/gamescope-0:ro;"),
    (
        "net.rpcs3.RPCS3",
        "rpcs3",
        "home:ro;/media;xdg-config/kdeglobals:ro;/run/media;xdg-run/app/com.discordapp.Discord:create;",
    ),
    (
        "org.DolphinEmu.dolphin-emu",
        "dolphin-emu-wrapper",
        "xdg-run/app/com.discordapp.Discord:create;xdg-config/kdeglobals:ro;xdg-run/app/com.discordapp.DiscordCanary:create;host:ro;xdg-run/gamescope-0:ro;",
    ),
    ("org.duckstation.DuckStation", "duckstation-qt", ""),
    ("org.ppsspp.PPSSPP", "PPSSPPSDL", "host:ro;xdg-run/gamescope-0:ro;"),
];

pub(crate) fn flatpak_metadata(install: &Path, app: &str, command: &str, fs: &str) {
    let mut text = format!(
        "[Application]\nname={app}\nruntime=org.kde.Platform/x86_64/6.11\ncommand={command}\n\n[Context]\nshared=ipc;network;\n"
    );
    if !fs.is_empty() {
        text.push_str(&format!("filesystems={fs}\n"));
    }
    text.push_str("\n[Session Bus Policy]\norg.kde.kconfig.notify=talk\n");
    write_file(&install.join("app").join(app).join("current/active/metadata"), text);
}

/// A fixture machine: `root/var/lib/flatpak` with the user's 7 Flatpaks, `root/home` as home.
pub(crate) fn machine() -> (tempfile::TempDir, ProbeRoots, crate::scan::ScanContext) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let roots = ProbeRoots::under(root, &home);
    for (app, cmd, fs) in USER_FLATPAKS {
        flatpak_metadata(&roots.system_flatpak, app, cmd, fs);
    }
    let mut ctx = crate::scan::desktop::test_support::ctx(&home);
    ctx.emulation = EmulationConfig { auto_emulators: true, auto_folders: false, ..Default::default() };
    (tmp, roots, ctx)
}

fn ids(v: &[Emulator]) -> Vec<&str> {
    v.iter().map(|e| e.id.as_str()).collect()
}

#[test]
fn finds_the_users_flatpaks() {
    let (_t, roots, ctx) = machine();
    let found = detect_with(&ctx, &roots);
    let mut got = ids(&found);
    got.sort();
    assert_eq!(got, ["dolphin", "duckstation", "eden", "mgba", "pcsx2", "ppsspp", "rpcs3"]);
    let pcsx2 = found.iter().find(|e| e.id == "pcsx2").unwrap();
    assert_eq!(pcsx2.kind, InstallKind::Flatpak { app_id: "net.pcsx2.PCSX2".into() });
    assert_eq!(pcsx2.how, "Flatpak");
    assert_eq!(pcsx2.def, "pcsx2");
    assert!(!pcsx2.user);
    // Their permissions were read on the way.
    assert_eq!(flatpak_access(&roots.home, "io.mgba.mGBA"), vec!["xdg-config/kdeglobals:ro", "host"]);
    assert!(flatpak_access(&roots.home, "org.duckstation.DuckStation").is_empty());
}

#[test]
fn finds_appimages_programs_steam_retroarch_and_cores() {
    let (t, roots, ctx) = machine();
    let home = &roots.home;
    // RetroArch as a user Flatpak with two cores.
    flatpak_metadata(&roots.user_flatpak, "org.libretro.RetroArch", "retroarch", "host;");
    let ra_cores = home.join(".var/app/org.libretro.RetroArch/config/retroarch/cores");
    write_file(&ra_cores.join("mgba_libretro.so"), b"\x7fELF");
    write_file(&ra_cores.join("snes9x_libretro.so"), b"\x7fELF");
    write_file(&ra_cores.join("readme.txt"), b"x");
    // AppImages: executable, and one downloaded without +x (ELF magic).
    write_exe(&home.join("Applications/Cemu-2.6-x86_64.AppImage"), b"\x7fELF....");
    write_file(&home.join("Downloads/rpcs3-v0.0.43-17000_linux64.AppImage"), b"\x7fELF....");
    write_file(&home.join("Downloads/not-an-emulator.AppImage"), b"\x7fELF");
    write_file(&home.join("Applications/azahar-2126.AppImage"), b"text, not runnable");
    // An unpacked program in a folder, and a program in PATH.
    write_exe(&home.join("Applications/Vita3K/Vita3K"), b"#!/bin/sh\n");
    write_exe(&t.path().join("usr/bin/duckstation-qt"), b"#!/bin/sh\n");
    write_exe(&t.path().join("usr/bin/mednafen"), b"#!/bin/sh\n");
    // Steam's RetroArch with a DLC core, in a second Steam library.
    let steam = home.join(".local/share/Steam");
    let lib2 = t.path().join("games/SteamLibrary");
    std::fs::create_dir_all(steam.join("steamapps")).unwrap();
    write_file(
        &steam.join("steamapps/libraryfolders.vdf"),
        format!(
            "\"libraryfolders\"\n{{\n \"0\" {{ \"path\" \"{}\" }}\n \"1\" {{ \"path\" \"{}\" }}\n}}\n",
            steam.display(),
            lib2.display()
        ),
    );
    write_exe(&lib2.join("steamapps/common/RetroArch/retroarch"), b"#!/bin/sh\n");
    write_file(&lib2.join("steamapps/common/RetroArch/cores/genesis_plus_gx_libretro.so"), b"\x7fELF");
    // Native RetroArch cores for a system RetroArch (no program → not used).
    write_file(&t.path().join("usr/lib/libretro/nestopia_libretro.so"), b"\x7fELF");

    let found = detect_with(&ctx, &roots);
    let by = |id: &str| found.iter().find(|e| e.id == id).unwrap_or_else(|| panic!("{id} in {:?}", ids(&found)));
    let cemu = by("cemu");
    assert_eq!(cemu.kind, InstallKind::AppImage { path: home.join("Applications/Cemu-2.6-x86_64.AppImage") });
    assert_eq!(cemu.how, "AppImage in ~/Applications");
    // The Flatpak keeps the plain id; the AppImage of the same emulator gets a suffix.
    assert_eq!(by("rpcs3").flatpak_id(), Some("net.rpcs3.RPCS3"));
    let rp = by("rpcs3-appimage");
    assert_eq!(rp.how, "AppImage in ~/Downloads — not allowed to run yet");
    assert!(rp.blocked, "downloaded without +x: listed, not allowed to run yet");
    assert!(!cemu.blocked && cemu.auto);
    assert_eq!(by("duckstation-program").kind, InstallKind::Binary { path: t.path().join("usr/bin/duckstation-qt") });
    assert_eq!(by("mednafen").how, format!("Program in PATH ({})", t.path().join("usr/bin/mednafen").display()));
    assert_eq!(by("vita3k").how, "Program in ~/Applications/Vita3K");
    assert!(!found.iter().any(|e| e.def == "azahar"), "a text file is no AppImage");
    let ra = by("retroarch");
    assert_eq!(ra.how, "Flatpak (installed for you only)");
    let names: Vec<String> = ra.cores.iter().map(|c| c.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["mgba_libretro.so", "snes9x_libretro.so"], "the Flatpak only sees its own cores");
    let steam_ra = by("retroarch-steam");
    assert_eq!(steam_ra.how, "Steam");
    assert_eq!(steam_ra.cores.len(), 1);
    // Profiles: only cores that are there.
    let profs: Vec<String> = profiles(ra, &ctx.emulation).into_iter().map(|p| p.id).collect();
    assert_eq!(profs, ["snes9x", "mgba"]);
    let p = profiles(ra, &ctx.emulation).into_iter().find(|p| p.id == "mgba").unwrap();
    assert_eq!(p.core.as_deref(), Some(ra_cores.join("mgba_libretro.so").to_str().unwrap()));
}

#[test]
fn user_emulators_override_hide_and_add() {
    let (_t, roots, mut ctx) = machine();
    let cfg = &mut ctx.emulation;
    cfg.hidden_emulators = vec!["mgba".into()];
    cfg.emulators.insert(
        "pcsx2".into(),
        EmulatorEntry { path: "/opt/pcsx2/pcsx2-qt".into(), args: "-fullscreen {rom}".into(), ..Default::default() },
    );
    cfg.emulators.insert(
        "my-ps1".into(),
        EmulatorEntry {
            name: "My DuckStation".into(),
            follows: "duckstation".into(),
            path: "/x/DuckStation.AppImage".into(),
            ..Default::default()
        },
    );
    cfg.emulators.insert(
        "custom".into(),
        EmulatorEntry {
            name: "Something".into(),
            kind: "flatpak".into(),
            app_id: "org.example.Emu".into(),
            ..Default::default()
        },
    );
    cfg.emulators.insert("broken".into(), EmulatorEntry::default());
    let found = detect_with(&ctx, &roots);
    assert!(!ids(&found).contains(&"mgba"));
    assert!(!ids(&found).contains(&"broken"));
    let p = found.iter().find(|e| e.id == "pcsx2").unwrap();
    assert_eq!(p.kind, InstallKind::Binary { path: "/opt/pcsx2/pcsx2-qt".into() });
    assert_eq!(
        (p.user, p.how.as_str(), p.args.as_str(), p.def.as_str()),
        (true, "Changed by you", "-fullscreen {rom}", "pcsx2")
    );
    let m = found.iter().find(|e| e.id == "my-ps1").unwrap();
    assert_eq!(m.kind, InstallKind::AppImage { path: "/x/DuckStation.AppImage".into() });
    assert_eq!((m.def.as_str(), m.name.as_str(), m.how.as_str()), ("duckstation", "My DuckStation", "Added by you"));
    let c = found.iter().find(|e| e.id == "custom").unwrap();
    assert_eq!(c.def, "");
    assert_eq!(c.flatpak_id(), Some("org.example.Emu"));

    // auto_emulators = false: only the user's.
    ctx.emulation.auto_emulators = false;
    let mut only = ids(&detect_with(&ctx, &roots)).into_iter().map(str::to_string).collect::<Vec<_>>();
    only.sort();
    // (the pcsx2 entry has its own program, so it stays as one of the user's)
    assert_eq!(only, ["custom", "my-ps1", "pcsx2"]);
}

#[test]
fn user_profiles() {
    let emu = Emulator {
        id: "custom".into(),
        def: String::new(),
        name: "Custom".into(),
        kind: InstallKind::Binary { path: "/usr/bin/x".into() },
        how: String::new(),
        user: true,
        cores: vec![],
        args: String::new(),
        auto: false,
        blocked: false,
    };
    let mut cfg = EmulationConfig::default();
    cfg.profiles.insert(
        "custom/ps2".into(),
        ProfileEntry {
            name: "PS2".into(),
            platforms: vec!["sony_playstation2".into()],
            extensions: vec![".ISO".into()],
            ..Default::default()
        },
    );
    cfg.profiles.insert("other/x".into(), ProfileEntry::default());
    let p = profiles(&emu, &cfg);
    assert_eq!(p.len(), 1);
    assert_eq!(
        (p[0].id.as_str(), p[0].args.as_str(), p[0].extensions.clone(), p[0].custom),
        ("ps2", "{rom}", vec!["iso".to_string()], true)
    );
    // Overriding a built-in profile.
    let duck = Emulator { id: "duckstation".into(), def: "duckstation".into(), ..emu.clone() };
    cfg.profiles
        .insert("duckstation/default".into(), ProfileEntry { args: "-fullscreen {rom}".into(), ..Default::default() });
    let p = profiles(&duck, &cfg);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].args, "-fullscreen {rom}");
    assert!(p[0].custom);
    assert_eq!(p[0].extensions[0], "m3u");
}

#[test]
fn permissions_and_overrides() {
    let (_t, roots, _ctx) = machine();
    let home = roots.home.clone();
    let covers = |app: &str, p: &str| fs_covers(&flatpak_filesystems(&roots, app), &home, Path::new(p));
    assert!(covers("io.mgba.mGBA", "/mnt/space1/games/gba/x.gba"));
    assert!(covers("org.DolphinEmu.dolphin-emu", "/mnt/space1/games/gc/x.rvz"), "host:ro is enough to read");
    assert!(covers("org.ppsspp.PPSSPP", "/mnt/space1/games/psp"));
    assert!(!covers("net.rpcs3.RPCS3", "/mnt/space1/games/ps3"), "home:ro + /media + /run/media don't reach /mnt");
    assert!(covers("net.rpcs3.RPCS3", "/run/media/me/disk/ps3"));
    assert!(covers("net.rpcs3.RPCS3", home.join("ROMs/ps3").to_str().unwrap()));
    assert!(!covers("net.rpcs3.RPCS3", "/run/mediafoo"));
    assert!(!covers("org.duckstation.DuckStation", home.join("ROMs").to_str().unwrap()));
    assert!(!covers("dev.eden_emu.eden", "/mnt/space1/games/switch"));
    assert!(!covers("net.pcsx2.PCSX2", "/mnt/space1/games/ps2"));
    // A user override adds /mnt/space1; a global one takes host away from mGBA.
    write_file(&roots.user_flatpak.join("overrides/net.pcsx2.PCSX2"), "[Context]\nfilesystems=/mnt/space1:ro;\n");
    write_file(&roots.user_flatpak.join("overrides/global"), "[Context]\nfilesystems=!host;\n");
    assert!(covers("net.pcsx2.PCSX2", "/mnt/space1/games/ps2"));
    assert!(!covers("net.pcsx2.PCSX2", "/mnt/space2"));
    assert!(!covers("io.mgba.mGBA", "/mnt/space1/games/gba"));
    assert!(flatpak_filesystems(&roots, "org.nope.Nope").is_empty());
    // Other forms.
    let fs = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert!(fs_covers(&fs(&["~/Games:ro"]), &home, &home.join("Games/x")));
    assert!(fs_covers(&fs(&["xdg-download/roms"]), &home, &home.join("Downloads/roms/a.iso")));
    assert!(!fs_covers(&fs(&["xdg-download/roms"]), &home, &home.join("Downloads/other")));
    assert!(!fs_covers(&fs(&["host"]), &home, Path::new("/usr/share/games/x")));
    assert!(fs_covers(&fs(&["host-os"]), &home, Path::new("/usr/share/games/x")));
    assert!(!fs_covers(&fs(&["home"]), &home, Path::new("/mnt/x")));
}

#[test]
fn globs() {
    assert!(glob_match("pcsx2*.appimage", "pcsx2-v2.2.0-linux-appimage-x64-Qt.AppImage"));
    assert!(glob_match("*yujinx*.appimage", "ryujinx-1.3.3-x64.AppImage"));
    assert!(!glob_match("eden*.appimage", "Eden.AppImage.zsync"));
    assert!(glob_match("e?en.appimage", "Eden.AppImage"));
    assert!(!glob_match("cemu*.appimage", "notcemu.AppImage"));
}

#[test]
fn cache_notices_new_installs_and_invalidate() {
    let (_t, roots, ctx) = machine();
    let a = detect_with(&ctx, &roots);
    assert_eq!(detect_with(&ctx, &roots), a);
    write_exe(&roots.home.join("Applications/xemu-v0.8.AppImage"), b"\x7fELF");
    let b = detect_with(&ctx, &roots);
    assert!(ids(&b).contains(&"xemu"), "{:?}", ids(&b));
    invalidate();
    assert_eq!(detect_with(&ctx, &roots), b);
}

#[test]
fn cores_by_any_name() {
    let cores = vec![PathBuf::from("/c/mgba_libretro.so"), PathBuf::from("/c/mesen-s_libretro.so")];
    assert_eq!(find_core(&cores, "mgba"), Some(&cores[0]));
    assert_eq!(find_core(&cores, "mgba_libretro"), Some(&cores[0]));
    assert_eq!(find_core(&cores, ".\\cores\\mesen-s_libretro.dll".rsplit('\\').next().unwrap()), Some(&cores[1]));
    assert_eq!(find_core(&cores, "snes9x"), None);
}

#[test]
fn emudeck_settings_move_the_app_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    write_file(
        &home.join(".config/EmuDeck/settings.sh"),
        "emusFolder=\"$HOME/Emus\"\ntoolsPath=/run/media/sd/Emulation/tools\n",
    );
    let r = ProbeRoots::under(tmp.path(), &home);
    assert!(r.app_dirs.contains(&home.join("Emus")));
    assert!(r.app_dirs.contains(&PathBuf::from("/run/media/sd/Emulation/tools")));
}

#[test]
fn appimages_in_named_folders_menu_entries_and_not_runnable_ones() {
    // The user's layout: ~/Documents/programs/appimages with an executable DuckStation (which
    // registered a menu entry) and an RPCS3 AppImage without +x; DuckStation + RPCS3 Flatpaks too.
    let (t, roots, ctx) = machine();
    let home = &roots.home;
    let dir = home.join("Documents/programs/appimages");
    write_exe(&dir.join("DuckStation-x64.AppImage"), b"\x7fELF....");
    write_file(&dir.join("rpcs3-v0.0.43-20644-3bfac607_linux64.AppImage"), b"\x7fELF....");
    // An Eden AppImage only known through its menu entry, somewhere else.
    let elsewhere = t.path().join("opt-like/eden/Eden-Linux-x86_64.AppImage");
    write_exe(&elsewhere, b"\x7fELF....");
    write_file(
        &home.join(".local/share/applications/org.duckstation.DuckStation.desktop"),
        format!(
            "[Desktop Entry]\nType=Application\nName=DuckStation\nExec=\"{}\" %f\n",
            dir.join("DuckStation-x64.AppImage").display()
        ),
    );
    write_file(
        &home.join(".local/share/applications/eden.desktop"),
        format!(
            "[Desktop Entry]\nType=Application\nName=Eden\nExec={} %f\nCategories=Game;Emulator;\n",
            elsewhere.display()
        ),
    );
    // A game's menu entry is not an emulator.
    write_file(
        &home.join(".local/share/applications/some-game.desktop"),
        "[Desktop Entry]\nType=Application\nName=Game\nExec=/usr/bin/true\n",
    );
    let found = detect_with(&ctx, &roots);
    let by = |id: &str| found.iter().find(|e| e.id == id).unwrap_or_else(|| panic!("{id} in {:?}", ids(&found)));
    // Same emulator twice: the Flatpak keeps the plain id, the AppImage gets its own.
    assert_eq!(by("duckstation").flatpak_id(), Some("org.duckstation.DuckStation"));
    let duck = by("duckstation-appimage");
    assert_eq!(duck.kind, InstallKind::AppImage { path: dir.join("DuckStation-x64.AppImage") });
    assert_eq!(duck.how, "AppImage in ~/Documents/programs/appimages (from its menu entry)");
    assert!(!duck.blocked);
    let rp = by("rpcs3-appimage");
    assert!(rp.blocked);
    assert_eq!(rp.how, "AppImage in ~/Documents/programs/appimages — not allowed to run yet");
    let eden = by("eden-appimage");
    assert_eq!(eden.kind, InstallKind::AppImage { path: elsewhere.clone() });
    assert!(eden.how.ends_with("(from its menu entry)"), "{}", eden.how);
    // Never chosen while not runnable: an RPCS3 AppImage alone runs no PS3 game.
    let reg = crate::emulation::registry::Registry {
        emulators: found.iter().filter(|e| e.def == "rpcs3" && e.blocked).cloned().collect(),
        config: ctx.emulation.clone(),
        home: home.clone(),
    };
    assert!(
        crate::emulation::registry::choose(&reg, "sony_playstation3", Path::new("/g/x"), None, None, None).is_none()
    );
    assert!(
        crate::emulation::registry::choose(
            &reg,
            "sony_playstation3",
            Path::new("/g/x"),
            Some("rpcs3-appimage"),
            None,
            None
        )
        .is_none()
    );
    // "Allow it to run" adds u+x; detection then sees it as runnable.
    allow_to_run(&dir.join("rpcs3-v0.0.43-20644-3bfac607_linux64.AppImage")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mode =
        std::fs::metadata(dir.join("rpcs3-v0.0.43-20644-3bfac607_linux64.AppImage")).unwrap().permissions().mode();
    assert_eq!(mode & 0o100, 0o100);
    invalidate();
    let again = detect_with(&ctx, &roots);
    assert!(!again.iter().find(|e| e.id == "rpcs3-appimage").unwrap().blocked);
}

#[test]
fn removing_a_changed_found_emulator_sticks() {
    let (_t, roots, mut ctx) = machine();
    ctx.emulation
        .emulators
        .insert("pcsx2".into(), EmulatorEntry { args: "-fullscreen {rom}".into(), ..Default::default() });
    let found = detect_with(&ctx, &roots);
    let p = found.iter().find(|e| e.id == "pcsx2").unwrap();
    assert!(p.user && p.auto, "an override of a found emulator");
}

#[test]
fn emulator_kind_labels() {
    let mk = |id: &str, kind: InstallKind, user: bool, auto: bool, how: &str| Emulator {
        id: id.into(),
        def: "duckstation".into(),
        name: "DuckStation".into(),
        kind,
        how: how.into(),
        user,
        cores: vec![],
        args: String::new(),
        auto,
        blocked: false,
    };
    let fp = mk(
        "duckstation",
        InstallKind::Flatpak { app_id: "org.duckstation.DuckStation".into() },
        false,
        true,
        "Flatpak",
    );
    let ai = mk(
        "duckstation-appimage",
        InstallKind::AppImage { path: "/x/DuckStation.AppImage".into() },
        false,
        true,
        "AppImage in ~/x",
    );
    let bin = mk(
        "duckstation-path",
        InstallKind::Binary { path: "/usr/bin/duckstation".into() },
        false,
        true,
        "Program in PATH",
    );
    let mine = mk("my-duck", InstallKind::Binary { path: "/opt/duck".into() }, true, false, "Added by you");
    let changed = mk("duckstation", InstallKind::AppImage { path: "/y.AppImage".into() }, true, true, "Changed by you");
    assert_eq!(fp.label(), "DuckStation — Flatpak");
    assert_eq!(ai.label(), "DuckStation — AppImage");
    assert_eq!(bin.label(), "DuckStation — installed program");
    // Added by hand: its kind all the same ("Added by you" is where it was found).
    assert_eq!(mine.label(), "DuckStation — installed program");
    let portable = mk(
        "my-duck-2",
        InstallKind::AppImage { path: "/e/DuckStation/DuckStation.AppImage".into() },
        true,
        false,
        "Added by you · portable",
    );
    assert_eq!(portable.kind_label(), "portable");
    assert_eq!(changed.kind_label(), "AppImage");
    let both = [fp.clone(), ai.clone()];
    assert_eq!(fp.name_among(&both), "DuckStation — Flatpak");
    assert_eq!(ai.name_among(&both), "DuckStation — AppImage");
    assert_eq!(fp.name_among([&fp]), "DuckStation");
    // Two AppImages of it: their folders tell them apart.
    let old = mk(
        "duckstation-3",
        InstallKind::AppImage { path: "/e/Duckstation.backup/DuckStation.AppImage".into() },
        false,
        true,
        "AppImage in /e",
    );
    assert_eq!(old.name_among([&ai, &old]), "DuckStation — Duckstation.backup");
}

#[test]
fn programs_are_known_by_their_names() {
    let ai = |p: &str| InstallKind::AppImage { path: p.into() };
    let id = |k: &InstallKind| identify(k).map(|d| d.id);
    assert_eq!(id(&ai("/e/Ryujinx/Ryujinx.AppImage")), Some("ryujinx"));
    assert_eq!(id(&ai("/e/Ryujinx-1.3.2-x64.AppImage")), Some("ryujinx"));
    assert_eq!(id(&ai("/e/rpcs3-v0.0.43_linux64.AppImage")), Some("rpcs3"));
    assert_eq!(id(&InstallKind::Binary { path: "/e/RPCS3_old/rpcs3".into() }), Some("rpcs3"));
    assert_eq!(id(&InstallKind::Binary { path: "/e/DuckStation/Duckstation".into() }), Some("duckstation"));
    assert_eq!(id(&InstallKind::Binary { path: "/e/dosbox-staging/dosbox-staging".into() }), Some("dosbox-staging"));
    assert_eq!(id(&InstallKind::Flatpak { app_id: "io.github.ryubing.Ryujinx".into() }), Some("ryujinx"));
    assert_eq!(id(&ai("/e/MySwitchEmu.AppImage")), None);
    assert_eq!(id(&ai("/e/citron.AppImage")), Some("citron"));
    assert_eq!(name_from_program(&ai("/opt/pcsx2-v2.2.0-linux.AppImage")), "pcsx2");
}

#[test]
fn emulators_added_by_hand_keep_the_name_they_were_given() {
    let ry = InstallKind::AppImage { path: "/e/Ryujinx/Ryujinx.AppImage".into() };
    // Older versions saved the name of the emulator it was based on: the program's own wins.
    assert_eq!(user_name("Eden", "eden", &ry, "eden"), "Ryujinx");
    assert_eq!(user_def("Eden", "eden", &ry), "ryujinx", "starts like Ryujinx, not like Eden");
    // A name the user typed stays, and so does what they based it on.
    assert_eq!(user_name("My Switch", "eden", &ry, "eden"), "My Switch");
    assert_eq!(user_def("My Switch", "eden", &ry), "eden");
    // Unknown program: the saved name stays.
    let other = InstallKind::AppImage { path: "/e/thing.AppImage".into() };
    assert_eq!(user_name("Eden", "eden", &other, "eden"), "Eden");
    assert_eq!(user_def("Eden", "eden", &other), "eden");
    // No name: the program's, else the built-in one's, else the key.
    assert_eq!(user_name("", "eden", &ry, "k"), "Ryujinx");
    assert_eq!(user_name("", "eden", &other, "k"), "Eden");
    assert_eq!(user_name("", "", &other, "k"), "k");
    // Another console's emulator is never taken for it (PCSX2 doesn't run Switch games).
    let p2 = InstallKind::AppImage { path: "/e/pcsx2.AppImage".into() };
    assert_eq!(user_def("Eden", "eden", &p2), "eden");
}

#[test]
fn an_emulator_added_by_hand_replaces_the_same_one_found() {
    let (_t, roots, mut ctx) = machine();
    let dir = roots.home.join("Applications");
    write_exe(&dir.join("Ryujinx-1.3.AppImage"), b"\x7fELF");
    let found = detect_with(&ctx, &roots);
    assert!(found.iter().any(|e| e.def == "ryujinx" && e.auto));
    // A tester's setup: the same program added by hand "based on" Eden, saved as "Eden".
    ctx.emulation.emulators.insert(
        "eden-2".into(),
        EmulatorEntry {
            name: "Eden".into(),
            follows: "eden".into(),
            kind: "appimage".into(),
            path: dir.join("Ryujinx-1.3.AppImage"),
            ..Default::default()
        },
    );
    let found = detect_with(&ctx, &roots);
    let same: Vec<&Emulator> =
        found.iter().filter(|e| e.program_path() == Some(dir.join("Ryujinx-1.3.AppImage").as_path())).collect();
    assert_eq!(same.len(), 1, "listed once: {found:?}");
    assert_eq!((same[0].id.as_str(), same[0].name.as_str(), same[0].def.as_str()), ("eden-2", "Ryujinx", "ryujinx"));
    assert!(same[0].how.starts_with("Added by you"));
}

/// "Kept aside" is a whole word of the file or folder name, never part of one: "latest",
/// "Gold", "contest" and "preview" builds are current builds and keep the automatic choice.
#[test]
fn only_whole_words_mark_a_copy_as_kept_aside() {
    for old in [
        "RPCS3_old",
        "Duckstation.backup",
        "rpcs3-test",
        "Copy of PCSX2",
        "pcsx2.bak",
        "RPCS3Old",
        "ppsspp (previous)",
        "Cemu-OLD",
    ] {
        assert!(name_says_kept_aside(old), "{old}");
    }
    for current in [
        "rpcs3-latest",
        "PPSSPPGold",
        "Cemu_latest.AppImage",
        "contest",
        "preview-build",
        "DuckStation",
        "Golden",
        "bakery",
        "Bold",
    ] {
        assert!(!name_says_kept_aside(current), "{current}");
    }
}

#[test]
fn folders_put_aside_are_recognised_by_whole_words() {
    for aside in [
        "Backups - Modding - Resources",
        "unused BIOS",
        "PCSX2_old",
        "Archive",
        "bios.bak",
        "OldBios",
        "previous dumps",
    ] {
        assert!(name_says_set_aside(aside), "{aside}");
    }
    for kept in ["bios", "Gold", "Emulation", "Bold Games", "Holdings", "PS2 Resources", "used", "test"] {
        assert!(!name_says_set_aside(kept), "{kept}");
    }
}
