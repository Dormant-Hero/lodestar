//! Fixture tests for adding games: program detection, Proton discovery, the folder-scan
//! heuristics, Wine registry parsing, prefix discovery, duplicates and merging.
//! Everything is built in temp dirs; nothing is ever launched.

use super::detect::*;
use super::import::*;
use super::*;
use crate::config::Paths;
use crate::scan::ScanContext;
use crate::test_support::{chmod_exe, write_exe, write_file};
use std::sync::atomic::AtomicBool;

fn ctx(home: &Path) -> ScanContext {
    ScanContext {
        home: home.to_path_buf(),
        xdg_data_dirs: vec![home.join("usr/share")],
        config: Default::default(),
        data_dir: Default::default(),
        addons: Default::default(),
        emulation: crate::config::EmulationConfig::detect_nothing(),
    }
}

/// A tiny ELF header (+ padding to `size`), executable.
fn elf(p: &Path, size: usize) {
    let mut b = b"\x7fELF\x02\x01\x01\0\0\0\0\0\0\0\0\0\x02\0\x3e\0".to_vec();
    b.resize(size.max(b.len()), 0);
    write_exe(p, &b);
}

fn appimage(p: &Path) {
    let mut b = b"\x7fELF\x02\x01\x01\0AI\x02\0\0\0\0\0\x02\0\x3e\0".to_vec();
    b.resize(4096, 0);
    write_exe(p, &b);
}

fn exe(p: &Path, size: usize) {
    let mut b = b"MZ\x90\0\x03\0\0\0".to_vec();
    b.resize(size.max(b.len()), 0);
    write_file(p, &b);
}

fn fake_proton(dir: &Path, display: &str, built: i64) {
    write_exe(&dir.join("proton"), b"#!/usr/bin/env python3\n");
    write_file(&dir.join("version"), format!("{built} {display}\n").as_bytes());
    let vdf = format!(
        "\"compatibilitytools\"\n{{\n  \"compat_tools\"\n  {{\n    \"{display}\" // name\n    {{\n      \"install_path\" \".\"\n      \"display_name\" \"{display}\"\n    }}\n  }}\n}}\n"
    );
    write_file(&dir.join("compatibilitytool.vdf"), vdf.as_bytes());
}

fn proton(path: &Path) -> ProtonVersion {
    ProtonVersion { name: "Proton-CachyOS Latest".into(), path: path.to_path_buf() }
}

fn paths(root: &Path) -> Paths {
    Paths {
        home: root.join("home"),
        config_dir: root.join("cfg"),
        data_dir: root.join("data"),
        cache_dir: root.join("cache"),
    }
}

fn cmd_env(l: &LaunchSpec, k: &str) -> Option<String> {
    match l {
        LaunchSpec::Command { env, .. } => env.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// detect_program

#[test]
fn detects_linux_elf_with_name_icon_and_folder() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("Hollow Knight");
    let bin = dir.join("hollow_knight.x86_64");
    elf(&bin, 1000);
    crate::scan::desktop::test_support::fake_png(&dir.join("hollow_knight.png"), 64, 64);
    let info = detect_program_in(&ctx(t.path()), &bin).unwrap();
    assert_eq!(info.kind, ProgramKind::Linux);
    assert_eq!(info.name, "Hollow Knight");
    assert_eq!(info.install_dir.as_deref(), Some(dir.as_path()));
    assert_eq!(info.icon.as_deref(), Some(dir.join("hollow_knight.png").as_path()));
    assert_eq!(
        info.launch,
        Some(LaunchSpec::Command {
            program: bin.to_string_lossy().into(),
            args: vec![],
            cwd: Some(dir.clone()),
            env: vec![]
        })
    );
}

#[test]
fn detects_appimages_by_magic_and_extension() {
    let t = tempfile::tempdir().unwrap();
    let a = t.path().join("Downloads/Heroic-2.15.2-x86_64.AppImage");
    appimage(&a);
    let info = detect_program_in(&ctx(t.path()), &a).unwrap();
    assert_eq!(info.kind, ProgramKind::AppImage);
    assert_eq!(info.name, "Heroic");
    // Magic only (no extension): still an AppImage.
    let b = t.path().join("Downloads/supertux");
    appimage(&b);
    assert_eq!(detect_program_in(&ctx(t.path()), &b).unwrap().kind, ProgramKind::AppImage);
    // Extension only (e.g. a type-1 AppImage without the AI marker).
    let c = t.path().join("Downloads/Some_Game-v1.0-linux.AppImage");
    elf(&c, 100);
    let info = detect_program_in(&ctx(t.path()), &c).unwrap();
    assert_eq!(info.kind, ProgramKind::AppImage);
    assert_eq!(info.name, "Some Game");
    assert_eq!(appimage_base_name("RuneLite.AppImage"), "RuneLite");
    assert_eq!(appimage_base_name("Ryujinx-1.1.1403-x86_64.AppImage"), "Ryujinx");
    assert_eq!(appimage_base_name("PCSX2-v2.2.0-linux-appimage-x64-Qt.AppImage"), "PCSX2");
}

#[test]
fn detects_scripts_with_and_without_exec_bit() {
    let t = tempfile::tempdir().unwrap();
    let s = t.path().join("Games/Cave Story/start.sh");
    write_file(&s, b"#!/bin/bash\n./doukutsu\n");
    let info = detect_program_in(&ctx(t.path()), &s).unwrap();
    assert_eq!(info.kind, ProgramKind::Script);
    assert_eq!(info.name, "Cave Story", "generic stem → folder name");
    // Not executable → runs through the shebang interpreter.
    match info.launch.unwrap() {
        LaunchSpec::Command { program, args, .. } => {
            assert_eq!(program, "/bin/bash");
            assert_eq!(args, vec![s.to_string_lossy().to_string()]);
        }
        other => panic!("{other:?}"),
    }
    chmod_exe(&s);
    match detect_program_in(&ctx(t.path()), &s).unwrap().launch.unwrap() {
        LaunchSpec::Command { program, .. } => assert_eq!(program, s.to_string_lossy()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn detects_windows_programs_and_names_them() {
    let t = tempfile::tempdir().unwrap();
    let c = ctx(t.path());
    let p = t.path().join("Games/Cool Game/game.exe");
    exe(&p, 100);
    let info = detect_program_in(&c, &p).unwrap();
    assert_eq!(info.kind, ProgramKind::Windows);
    assert_eq!(info.name, "Cool Game");
    assert!(info.launch.is_none(), "Windows programs need a Proton + prefix choice");
    let ue = t.path().join("Games/Stray Souls/Binaries/Win64/StraySouls-Win64-Shipping.exe");
    exe(&ue, 100);
    assert_eq!(detect_program_in(&c, &ue).unwrap().name, "StraySouls");
    let generic = t.path().join("Games/Stray Souls/Binaries/Win64/Game-Win64-Shipping.exe");
    exe(&generic, 100);
    assert_eq!(detect_program_in(&c, &generic).unwrap().name, "Stray Souls");
    let fl = t.path().join("pfx/drive_c/Program Files (x86)/SP Football Life 2027/FL 2027 start.exe");
    exe(&fl, 100);
    assert_eq!(detect_program_in(&c, &fl).unwrap().name, "FL 2027");
    let w3 = t.path().join("Games/The Witcher 3/bin/x64/witcher3.exe");
    exe(&w3, 100);
    assert_eq!(detect_program_in(&c, &w3).unwrap().name, "The Witcher 3", "folder has the fuller name");
    // By extension alone (.bat / .msi), and MZ magic without an extension.
    let bat = t.path().join("Games/Old/run_game.bat");
    write_file(&bat, b"@echo off\r\n");
    assert_eq!(detect_program_in(&c, &bat).unwrap().kind, ProgramKind::Windows);
    let mz = t.path().join("Games/Old/OLDGAME");
    exe(&mz, 10);
    assert_eq!(detect_program_in(&c, &mz).unwrap().kind, ProgramKind::Windows);
}

#[test]
fn detects_desktop_files_and_flatpaks() {
    let t = tempfile::tempdir().unwrap();
    let c = ctx(t.path());
    let icon = t.path().join("icons/foo.png");
    crate::scan::desktop::test_support::fake_png(&icon, 256, 256);
    let d = t.path().join("foo.desktop");
    write_file(
        &d,
        format!(
            "[Desktop Entry]\nType=Application\nName=Foo Quest\nName[de]=Foo Suche\nExec=env DXVK_HUD=1 /opt/foo/foo --fullscreen %U\nPath=/opt/foo\nIcon={}\n",
            icon.display()
        )
        .as_bytes(),
    );
    let info = detect_program_in(&c, &d).unwrap();
    assert_eq!(info.kind, ProgramKind::Desktop);
    assert_eq!(info.name, "Foo Quest");
    assert_eq!(info.icon.as_deref(), Some(icon.as_path()));
    assert_eq!(info.install_dir.as_deref(), Some(Path::new("/opt/foo")));
    assert_eq!(
        info.launch,
        Some(LaunchSpec::Command {
            program: "/opt/foo/foo".into(),
            args: vec!["--fullscreen".into()],
            cwd: Some("/opt/foo".into()),
            env: vec![("DXVK_HUD".into(), "1".into())],
        })
    );
    let f = t.path().join("org.ppsspp.PPSSPP.desktop");
    write_file(&f, b"[Desktop Entry]\nType=Application\nName=PPSSPP\nExec=/usr/bin/flatpak run --branch=stable --arch=x86_64 org.ppsspp.PPSSPP @@u %U @@\n");
    let info = detect_program_in(&c, &f).unwrap();
    assert_eq!(info.kind, ProgramKind::Flatpak);
    assert_eq!(detect::is_flatpak_launch(info.launch.as_ref().unwrap()).as_deref(), Some("org.ppsspp.PPSSPP"));
}

#[test]
fn refuses_folders_and_documents() {
    let t = tempfile::tempdir().unwrap();
    let c = ctx(t.path());
    assert!(detect_program_in(&c, t.path()).is_err());
    let txt = t.path().join("readme.txt");
    write_file(&txt, b"hello");
    assert!(detect_program_in(&c, &txt).is_err());
    assert!(detect_program_in(&c, &t.path().join("missing")).is_err());
}

#[test]
fn names_are_prettified() {
    assert_eq!(prettify("hollow_knight"), "Hollow Knight");
    assert_eq!(prettify("super-meat-boy"), "Super Meat Boy");
    assert_eq!(prettify("RuneLite"), "RuneLite");
    assert_eq!(prettify("Baldur's Gate 3"), "Baldur's Gate 3");
    assert_eq!(safe_folder_name("Baldur's Gate 3: Deluxe / Edition"), "Baldur's Gate 3 Deluxe Edition");
    assert_eq!(safe_folder_name("..."), "Game");
}

#[test]
fn flatpak_app_list() {
    let t = tempfile::tempdir().unwrap();
    let apps = t.path().join(".local/share/flatpak/exports/share/applications");
    write_file(
        &apps.join("com.example.Game.desktop"),
        b"[Desktop Entry]\nType=Application\nName=Example Game\nCategories=Game;\nExec=/usr/bin/flatpak run --branch=stable com.example.Game\n",
    );
    write_file(
        &apps.join("org.example.Editor.desktop"),
        b"[Desktop Entry]\nType=Application\nName=An Editor\nExec=/usr/bin/flatpak run org.example.Editor\n",
    );
    write_file(
        &apps.join("hidden.desktop"),
        b"[Desktop Entry]\nType=Application\nName=H\nNoDisplay=true\nExec=flatpak run x.y.Z\n",
    );
    let v: Vec<FlatpakApp> = flatpak_apps(&ctx(t.path())).into_iter().filter(|a| a.id.contains("example")).collect();
    assert_eq!(v.len(), 2);
    assert_eq!(v[0].id, "com.example.Game");
    assert!(v[0].game && !v[1].game, "games first");
    assert_eq!(
        v[0].info.launch,
        Some(LaunchSpec::Command {
            program: "flatpak".into(),
            args: vec!["run".into(), "com.example.Game".into()],
            cwd: None,
            env: vec![]
        })
    );
}

// ---------------------------------------------------------------------------------------------
// Proton

#[test]
fn proton_discovery_sorted_newest_first() {
    let t = tempfile::tempdir().unwrap();
    let h = t.path();
    let steam = h.join(".local/share/Steam");
    std::fs::create_dir_all(steam.join("steamapps")).unwrap();
    std::fs::create_dir_all(h.join(".steam")).unwrap();
    std::os::unix::fs::symlink(&steam, h.join(".steam/root")).unwrap(); // same root twice: listed once
    let tools = steam.join("compatibilitytools.d");
    fake_proton(&tools.join("Proton-CachyOS Latest"), "Proton-CachyOS Latest", 1_792_530_497);
    fake_proton(&tools.join("GE-Proton10-25"), "GE-Proton10-25", 1_793_687_165);
    std::fs::create_dir_all(tools.join("Proton EasyAntiCheat Runtime")).unwrap(); // no `proton`: skipped
    fake_proton(&steam.join("steamapps/common/Proton - Experimental"), "Proton - Experimental", 1_700_000_000);
    std::fs::create_dir_all(steam.join("steamapps/common/SomeGame")).unwrap();
    fake_proton(
        &h.join(".var/app/com.valvesoftware.Steam/data/Steam/compatibilitytools.d/UMU-Proton-9.0"),
        "UMU-Proton-9.0",
        1_600_000_000,
    );
    let v = proton_versions(&ctx(h));
    let names: Vec<&str> = v.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["GE-Proton10-25", "Proton-CachyOS Latest", "Proton - Experimental", "UMU-Proton-9.0"]);
    let flavors: Vec<&str> = v.iter().map(|p| p.flavor()).collect();
    assert_eq!(flavors, vec!["GE", "CachyOS", "Experimental", "UMU"]);
    assert!(v.iter().all(|p| p.path.join("proton").is_file()));

    // A prefix remembers its build: config_info names the build's files/ folder …
    let pfx = h.join("Faugus/fl");
    write_file(
        &pfx.join("config_info"),
        format!("CachyOS-11.0-100\n{}/files/share/fonts/\n", tools.join("Proton-CachyOS Latest").display()).as_bytes(),
    );
    assert_eq!(proton_for_prefix(&pfx, &v).unwrap().name, "Proton-CachyOS Latest");
    // … or `version` its flavour; else the newest.
    let pfx2 = h.join("Faugus/other");
    write_file(&pfx2.join("version"), b"CachyOS-11.0-100\n");
    assert_eq!(proton_for_prefix(&pfx2, &v).unwrap().flavor(), "CachyOS");
    assert_eq!(proton_for_prefix(&h.join("nothing"), &v).unwrap().name, "GE-Proton10-25");
}

#[test]
fn windows_launch_uses_umu_or_proton() {
    let p = proton(Path::new("/home/u/.local/share/Steam/compatibilitytools.d/Proton-CachyOS Latest"));
    let exe = Path::new("/home/u/Games/X/x.exe");
    let pfx = Path::new("/home/u/Games/Lodestar/X");
    let l = windows_launch_with(exe, &p, pfx, Some("umu-run".into()), Path::new("/home/u"));
    assert_eq!(
        l,
        LaunchSpec::Command {
            program: "umu-run".into(),
            args: vec!["/home/u/Games/X/x.exe".into()],
            cwd: Some("/home/u/Games/X".into()),
            env: vec![
                ("WINEPREFIX".into(), "/home/u/Games/Lodestar/X".into()),
                ("GAMEID".into(), "umu-default".into()),
                ("PROTONPATH".into(), p.path.to_string_lossy().into()),
            ],
        }
    );
    // No umu: Proton's own script.
    let l = windows_launch_with(exe, &p, pfx, None, Path::new("/home/u"));
    let LaunchSpec::Command { program, args, .. } = &l else { panic!() };
    assert!(program.ends_with("Proton-CachyOS Latest/proton"));
    assert_eq!(args, &vec!["run".to_string(), "/home/u/Games/X/x.exe".into()]);
    assert_eq!(cmd_env(&l, "STEAM_COMPAT_DATA_PATH").as_deref(), Some("/home/u/Games/Lodestar/X"));
    assert_eq!(cmd_env(&l, "STEAM_COMPAT_CLIENT_INSTALL_PATH").as_deref(), Some("/home/u/.local/share/Steam"));
    // .msi through msiexec.
    let l = windows_launch_with(Path::new("/d/setup.msi"), &p, pfx, Some("umu-run".into()), Path::new("/home/u"));
    let LaunchSpec::Command { args, .. } = &l else { panic!() };
    assert_eq!(args, &vec!["msiexec".to_string(), "/i".into(), "/d/setup.msi".into()]);
    let t = tempfile::tempdir().unwrap();
    assert_eq!(default_prefix(&paths(t.path()), "Foo: Bar"), t.path().join("home/Games/Lodestar/Foo Bar"));
}

// ---------------------------------------------------------------------------------------------
// Folder scan

fn collect(f: impl FnOnce(&mut Sink)) -> (Vec<ImportCandidate>, Vec<String>) {
    let cancel = AtomicBool::new(false);
    let mut found = vec![];
    let mut errors = vec![];
    let mut emit = |m: ImportMsg| match m {
        ImportMsg::Found(v) => found.extend(v),
        ImportMsg::Error(e) => errors.push(e),
        _ => {}
    };
    let mut sink = Sink { cancel: &cancel, emit: &mut emit, files: 0 };
    f(&mut sink);
    (found, errors)
}

#[test]
fn folder_scan_picks_one_program_per_game_and_skips_junk() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    // Native game with a crash handler and data folder.
    elf(&root.join("Hollow Knight/hollow_knight.x86_64"), 50_000);
    elf(&root.join("Hollow Knight/UnityCrashHandler64"), 90_000);
    write_file(&root.join("Hollow Knight/hollow_knight_Data/level0"), b"data");
    // Windows game with installers, redistributables and a settings tool.
    exe(&root.join("Cool Game/CoolGame.exe"), 40_000);
    exe(&root.join("Cool Game/Settings.exe"), 400_000);
    exe(&root.join("Cool Game/unins000.exe"), 900_000);
    exe(&root.join("Cool Game/vc_redist.x64.exe"), 900_000);
    exe(&root.join("Cool Game/UnityCrashHandler64.exe"), 900_000);
    exe(&root.join("Cool Game/_CommonRedist/DirectX/DXSETUP.exe"), 900_000);
    exe(&root.join("Cool Game/EasyAntiCheat/EasyAntiCheat_EOS_Setup.exe"), 900_000);
    // The main program is not named like the folder: the biggest non-tool wins.
    exe(&root.join("SP Football Life 2027/FL 2027 start.exe"), 300_000);
    exe(&root.join("SP Football Life 2027/Settings.exe"), 500_000);
    exe(&root.join("SP Football Life 2027/Uninstall FL27.exe"), 500_000);
    // AppImages are always their own game; loose installers are not games.
    appimage(&root.join("Tool-1.2-x86_64.AppImage"));
    appimage(&root.join("Other.AppImage"));
    exe(&root.join("setup.exe"), 100);
    write_file(&root.join("notes.txt"), b"x");
    // Container folders are descended into; hidden folders and folder symlinks are not.
    exe(&root.join("Games/Inner Game/Inner.exe"), 1000);
    exe(&root.join(".cache/secret.exe"), 1000);
    std::os::unix::fs::symlink(&root, root.join("loop")).unwrap();
    // A folder with only junk gives nothing.
    exe(&root.join("Redists/vcredist_x86.exe"), 100);
    exe(&root.join("JunkOnly/unins000.exe"), 100);
    // Non-executable "ELF" files and libraries are not programs.
    write_file(&root.join("Lib Game/libgame.so"), b"\x7fELF");
    let protons = vec![proton(&t.path().join("proton"))];
    let env = Env { protons: &protons, home: t.path(), umu: Some("umu-run".into()) };
    let (found, errors) = collect(|s| scan_folder(&env, &root, s));
    assert!(errors.is_empty(), "{errors:?}");
    let mut got: Vec<(String, String, ProgramKind)> = found
        .iter()
        .map(|c| (c.name.clone(), c.path.file_name().unwrap().to_string_lossy().into_owned(), c.kind))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            ("Cool Game".into(), "CoolGame.exe".into(), ProgramKind::Windows),
            ("Hollow Knight".into(), "hollow_knight.x86_64".into(), ProgramKind::Linux),
            ("Inner Game".into(), "Inner.exe".into(), ProgramKind::Windows),
            ("Other".into(), "Other.AppImage".into(), ProgramKind::AppImage),
            ("SP Football Life 2027".into(), "FL 2027 start.exe".into(), ProgramKind::Windows),
            ("Tool".into(), "Tool-1.2-x86_64.AppImage".into(), ProgramKind::AppImage),
        ]
    );
    let cool = found.iter().find(|c| c.name == "Cool Game").unwrap();
    assert_eq!(cool.install_dir.as_deref(), Some(root.join("Cool Game").as_path()));
    assert_eq!(cool.origin, "~/lib/Cool Game");
    // Windows programs outside a prefix: the checklist's Proton choice + a new prefix decide.
    assert!(cool.launch.is_none());
    let hk = found.iter().find(|c| c.name == "Hollow Knight").unwrap();
    assert!(
        matches!(hk.launch, Some(LaunchSpec::Command { ref program, .. }) if program.ends_with("hollow_knight.x86_64"))
    );
}

#[test]
fn folder_scan_inside_a_prefix_uses_that_prefix_and_respects_limits() {
    let t = tempfile::tempdir().unwrap();
    let pfx = t.path().join("Faugus/game");
    exe(&pfx.join("drive_c/Program Files/Big Game/BigGame.exe"), 1000);
    exe(&pfx.join("drive_c/windows/system32/notepad.exe"), 1000);
    exe(&pfx.join("drive_c/Program Files/Common Files/thing/x.exe"), 1000);
    exe(&pfx.join("drive_c/Program Files (x86)/Steam/steam.exe"), 1000);
    std::fs::create_dir_all(pfx.join("dosdevices")).unwrap();
    std::os::unix::fs::symlink("/", pfx.join("dosdevices/z:")).unwrap();
    let protons = vec![proton(&t.path().join("proton"))];
    let env = Env { protons: &protons, home: t.path(), umu: Some("umu-run".into()) };
    let (found, _) = collect(|s| scan_folder(&env, &pfx, s));
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].name, "Big Game");
    assert_eq!(cmd_env(found[0].launch.as_ref().unwrap(), "WINEPREFIX").unwrap(), pfx.to_string_lossy());

    // Depth limit: programs deeper than 6 levels are not found.
    let deep = t.path().join("deep/G/1/2/3/4/5/6/7/deep.exe");
    exe(&deep, 10);
    let (found, _) = collect(|s| scan_folder(&env, &t.path().join("deep"), s));
    assert!(found.is_empty(), "{found:?}");
    // Cancelled before starting: nothing.
    let cancel = AtomicBool::new(true);
    let mut n = 0;
    let mut emit = |m: ImportMsg| {
        if matches!(m, ImportMsg::Found(_)) {
            n += 1
        }
    };
    let mut sink = Sink { cancel: &cancel, emit: &mut emit, files: 0 };
    scan_folder(&env, &pfx, &mut sink);
    assert_eq!(n, 0);
}

#[test]
fn exclusion_list() {
    for junk in [
        "unins000",
        "Uninstall FL26",
        "setup",
        "vc_redist.x64",
        "DXSETUP",
        "UnityCrashHandler64",
        "CrashReportClient",
        "UnrealCEFSubProcess",
        "EpicWebHelper",
        "QtWebEngineProcess",
        "dotNetFx40_Full_x86_x64",
        "Updater",
        "EasyAntiCheat_EOS_Setup",
        "start_protected_game",
        "python",
        "steam",
    ] {
        assert!(is_excluded_program(junk), "{junk}");
    }
    for game in ["CoolGame", "witcher3", "FL 2027 start", "hollow_knight", "Settings", "Launcher", "SteamWorld Dig"] {
        assert!(!is_excluded_program(game), "{game}");
    }
}

// ---------------------------------------------------------------------------------------------
// Prefixes and the registry

const REG: &str = r#"WINE REGISTRY Version 2
;; All keys relative to \\Machine

#arch=win64

[Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\WIC] 1793404016
#time=1dd68c8f29b201e
"NoRemove"=dword:00000001

[Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{15640F65-1142-441F-8FD7-120F77A80B65}_is1] 1793545952
#time=1dd6a136ad1335c
"DisplayIcon"="C:\\Program Files (x86)\\SP Football Life 2027\\FL 2027 start.exe"
"DisplayName"="SP Football Life 2027"
"EstimatedSize"=dword:01f3ba3e
"InstallLocation"="C:\\PROGRAM FILES (X86)\\sp football life 2027\\"
"Publisher"="SmokePatch"
"UninstallString"="\"C:\\Program Files (x86)\\SP Football Life 2027\\unins000.exe\""

[Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{16735AF7-1D8D-3681-94A5-C578A61EC832}] 1793404075
"DisplayName"="Microsoft .NET Framework 4.8"
"InstallLocation"=""

[Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Football Life 2026] 1793403079
"DisplayName"="Football Life 2026"
"UninstallString"="C:\\Program Files (x86)\\SP Football Life 2026\\Uninstall FL26.exe"

[Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{33F1EAF0}\\variables] 1793544951
"DisplayName"="Not an app"

[Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Caf\x00e9] 1
"DisplayName"=str(2):"Caf\x00e9 \"Deluxe\""
"SystemComponent"=dword:00000001

[Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Divinity] 1
"DisplayName"="Divinity: Original Sin"
"InstallLocation"="C:\\GOG Games\\Divinity Original Sin"
"#;

#[test]
fn parses_uninstall_entries() {
    let v = parse_uninstall_entries(REG);
    let names: Vec<&str> = v.iter().map(|a| a.display_name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "SP Football Life 2027",
            "Microsoft .NET Framework 4.8",
            "Football Life 2026",
            "Café \"Deluxe\"",
            "Divinity: Original Sin"
        ]
    );
    let fl = &v[0];
    assert_eq!(fl.install_location.as_deref(), Some("C:\\PROGRAM FILES (X86)\\sp football life 2027\\"));
    assert_eq!(fl.display_icon.as_deref(), Some("C:\\Program Files (x86)\\SP Football Life 2027\\FL 2027 start.exe"));
    assert_eq!(
        fl.uninstall_string.as_deref(),
        Some("\"C:\\Program Files (x86)\\SP Football Life 2027\\unins000.exe\"")
    );
    assert_eq!(fl.publisher.as_deref(), Some("SmokePatch"));
    assert!(v[3].system_component);
    let system: Vec<bool> = v.iter().map(is_system_app).collect();
    assert_eq!(system, vec![false, true, false, true, false]);
}

#[test]
fn scans_a_prefix_from_registry_and_program_files() {
    let t = tempfile::tempdir().unwrap();
    let pfx = t.path().join("Faugus/football-life");
    let pf86 = pfx.join("drive_c/Program Files (x86)");
    exe(&pf86.join("SP Football Life 2027/FL 2027 start.exe"), 300_000);
    exe(&pf86.join("SP Football Life 2027/Settings.exe"), 600_000);
    exe(&pf86.join("SP Football Life 2027/unins000.exe"), 600_000);
    exe(&pf86.join("SP Football Life 2026/FL 2026 start.exe"), 300_000);
    exe(&pf86.join("SP Football Life 2026/Settings.exe"), 300_000);
    exe(&pf86.join("SP Football Life 2026/Uninstall FL26.exe"), 600_000);
    // The real game binary is bigger than its launcher; the installer's shortcut says which to run.
    exe(&pf86.join("SP Football Life 2026/FL_2026.exe"), 5_000_000);
    let sc = pfx.join("drive_c/proton_shortcuts");
    write_file(
        &sc.join("Football Life 2026.desktop"),
        format!(
            "[Desktop Entry]\nName=Football Life 2026\nExec=\"C:\\\\users\\\\Public\\\\Desktop\\\\Football Life 2026.lnk\"\nType=Application\nPath={}\nIcon=8C6A_FL 2026 start.0\nStartupWMClass=fl 2026 start.exe\n",
            pf86.join("SP Football Life 2026").display()
        )
        .as_bytes(),
    );
    write_file(
        &sc.join("Uninstall.desktop"),
        format!(
            "[Desktop Entry]\nName=Uninstall FL26\nPath={}\nStartupWMClass=uninstall fl26.exe\n",
            pf86.join("SP Football Life 2026").display()
        )
        .as_bytes(),
    );
    crate::scan::desktop::test_support::fake_png(&sc.join("icons/256x256/apps/8C6A_FL 2026 start.0.png"), 256, 256);
    exe(&pf86.join("Steam/steam.exe"), 1000);
    exe(&pf86.join("Common Files/x/y.exe"), 1000);
    exe(&pfx.join("drive_c/Program Files/Loose Game/LooseGame.exe"), 1000);
    exe(&pfx.join("drive_c/Program Files/dotnet/dotnet.exe"), 1000);
    write_file(&pfx.join("system.reg"), REG.as_bytes());
    write_file(&pfx.join("version"), b"CachyOS-11.0-100\n");
    let protons = vec![
        ProtonVersion { name: "GE-Proton10-25".into(), path: t.path().join("ge") },
        ProtonVersion { name: "Proton-CachyOS Latest".into(), path: t.path().join("cachy") },
    ];
    let env = Env { protons: &protons, home: t.path(), umu: Some("umu-run".into()) };
    let prefix = Prefix { path: pfx.clone(), origin: "Faugus prefix ~/Faugus/football-life".into() };
    let (found, _) = collect(|s| {
        let v = scan_prefix(&env, &prefix, s);
        (s.emit)(ImportMsg::Found(v));
    });
    let got: Vec<(&str, String)> =
        found.iter().map(|c| (c.name.as_str(), c.path.file_name().unwrap().to_string_lossy().into_owned())).collect();
    assert_eq!(
        got,
        vec![
            ("SP Football Life 2027", "FL 2027 start.exe".to_string()),
            ("Football Life 2026", "FL 2026 start.exe".to_string()),
            ("Loose Game", "LooseGame.exe".to_string()),
        ]
    );
    assert_eq!(found[1].icon.as_deref(), Some(sc.join("icons/256x256/apps/8C6A_FL 2026 start.0.png").as_path()));
    let shortcuts = prefix_shortcuts(&pfx);
    assert_eq!(shortcuts.len(), 1, "the uninstaller's shortcut is not one");
    let fl27 = &found[0];
    assert_eq!(fl27.install_dir.as_deref(), Some(pf86.join("SP Football Life 2027").as_path()));
    assert_eq!(fl27.origin, "Faugus prefix ~/Faugus/football-life");
    let l = fl27.launch.as_ref().unwrap();
    assert_eq!(cmd_env(l, "WINEPREFIX").unwrap(), pfx.to_string_lossy());
    assert_eq!(cmd_env(l, "PROTONPATH").unwrap(), t.path().join("cachy").to_string_lossy(), "the prefix's own build");
    assert_eq!(cmd_env(l, "GAMEID").as_deref(), Some("umu-default"));

    assert_eq!(
        win_to_unix(&pfx, "\"C:\\program files (x86)\\SP FOOTBALL LIFE 2027\\FL 2027 start.exe\",0").unwrap(),
        pf86.join("SP Football Life 2027/FL 2027 start.exe")
    );
}

#[test]
fn finds_prefixes() {
    let t = tempfile::tempdir().unwrap();
    let h = t.path();
    let mk = |p: &Path| std::fs::create_dir_all(p.join("drive_c")).unwrap();
    mk(&h.join("Faugus/football-life"));
    mk(&h.join("Faugus/default"));
    std::fs::create_dir_all(h.join("Faugus/not-a-prefix")).unwrap();
    write_file(&h.join("Faugus/fl.log"), b"log");
    mk(&h.join("Elsewhere/custom"));
    write_file(
        &h.join(".local/share/faugus-launcher/games.json"),
        format!(
            r#"[{{"gameid":"a","title":"A","prefix":"{0}/Faugus/football-life"}},{{"gameid":"b","title":"B","prefix":"{0}/Elsewhere/custom"}}]"#,
            h.display()
        )
        .as_bytes(),
    );
    mk(&h.join("Games/umu/umu-default"));
    mk(&h.join("Games/Heroic/Prefixes/default/Some Game"));
    mk(&h.join(".local/share/bottles/bottles/Gaming"));
    write_file(
        &h.join(".config/lutris/games/foo-123.yml"),
        format!("game:\n  exe: x.exe\n  prefix: {}/Games/lutris-foo\n", h.display()).as_bytes(),
    );
    mk(&h.join("Games/lutris-foo"));
    mk(&h.join(".wine"));
    let v = find_prefixes(&ctx(h));
    let got: Vec<(String, String)> = v
        .iter()
        .map(|p| {
            (p.path.strip_prefix(h).unwrap().display().to_string(), p.origin.split(" ~").next().unwrap().to_string())
        })
        .collect();
    let want = [
        ("Faugus/football-life", "Faugus prefix"),
        ("Elsewhere/custom", "Faugus prefix"),
        ("Faugus/default", "Faugus prefix"),
        ("Games/lutris-foo", "Lutris prefix"),
        (".local/share/bottles/bottles/Gaming", "Bottles"),
        ("Games/Heroic/Prefixes/default/Some Game", "Heroic prefix"),
        ("Games/umu/umu-default", "Wine prefix"),
        (".wine", "Wine prefix"),
    ];
    assert_eq!(got, want.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>());
}

#[test]
fn background_scan_streams_results_and_finishes() {
    let t = tempfile::tempdir().unwrap();
    elf(&t.path().join("lib/A Game/agame"), 1000);
    elf(&t.path().join("lib/B Game/bgame"), 1000);
    let cancel = Arc::new(AtomicBool::new(false));
    let rx = import::spawn(
        ImportSource::Folder,
        Some(t.path().join("lib")),
        ctx(t.path()),
        cancel,
        egui::Context::default(),
    );
    let mut names = vec![];
    loop {
        match rx.recv_timeout(std::time::Duration::from_secs(20)).expect("scan never finished") {
            ImportMsg::Found(v) => names.extend(v.into_iter().map(|c| c.name)),
            ImportMsg::Done => break,
            _ => {}
        }
    }
    names.sort();
    assert_eq!(names, vec!["A Game", "B Game"]);
}

// ---------------------------------------------------------------------------------------------
// Duplicates and merging

fn sg(source: Source, id: &str, name: &str) -> ScannedGame {
    ScannedGame::new(source, id, name)
}

fn library(db: &mut Db, games: Vec<ScannedGame>) -> Library {
    let mut sources: Vec<Source> = games.iter().map(|g| g.source).collect();
    sources.sort();
    sources.dedup();
    db.merge_scan(&games, &sources).unwrap();
    Library::new(db.load_all().unwrap())
}

fn id(l: &Library, name: &str) -> GameId {
    l.games.iter().find(|g| g.display_name() == name).unwrap().id
}

#[test]
fn duplicates_by_program_folder_name_and_hidden_copy() {
    let mut db = Db::open_memory().unwrap();
    let fl_dir = PathBuf::from("/home/u/Faugus/fl/drive_c/Program Files (x86)/SP Football Life 2027");
    let mut fl = sg(Source::Faugus, "abc", "Football Life 2027");
    fl.install_dir = Some(fl_dir.clone());
    fl.launch = Some(LaunchSpec::Command {
        program: "faugus-launcher".into(),
        args: vec!["--game".into(), "abc".into()],
        cwd: None,
        env: vec![],
    });
    let mut foo = sg(Source::Desktop, "foo.desktop", "Foo Quest");
    foo.launch = Some(LaunchSpec::Command { program: "/opt/foo/foo".into(), args: vec![], cwd: None, env: vec![] });
    let mut fp = sg(Source::Desktop, "com.x.Y.desktop", "Flat Game");
    fp.launch = Some(LaunchSpec::Command {
        program: "/usr/bin/flatpak".into(),
        args: vec!["run".into(), "--branch=stable".into(), "com.x.Y".into()],
        cwd: None,
        env: vec![],
    });
    let mut dl = sg(Source::Desktop, "dl.desktop", "Downloaded");
    dl.install_dir = Some("/home/u/Downloads".into());
    let mut lib = library(&mut db, vec![fl, foo, fp, dl]);
    let old = db
        .add_manual(
            "Old Game",
            LaunchSpec::Command { program: "/g/old".into(), args: vec![], cwd: None, env: vec![] },
            Art::default(),
        )
        .unwrap();
    db.set_hidden(old, true).unwrap();
    lib.replace(db.load_all().unwrap());

    let d = find_duplicates(&lib, "Something", None, Some(&fl_dir.join("FL 2027 start.exe")));
    assert_eq!(d, vec![DupMatch { id: id(&lib, "Football Life 2027"), reason: DupReason::SameProgram }]);
    let launch = LaunchSpec::Command {
        program: "flatpak".into(),
        args: vec!["run".into(), "com.x.Y".into()],
        cwd: None,
        env: vec![],
    };
    assert_eq!(find_duplicates(&lib, "x", Some(&launch), None)[0].reason, DupReason::SameProgram);
    let launch =
        LaunchSpec::Command { program: "/opt/foo/foo".into(), args: vec!["--x".into()], cwd: None, env: vec![] };
    assert_eq!(find_duplicates(&lib, "Other", Some(&launch), None)[0].id, id(&lib, "Foo Quest"));
    assert_eq!(
        find_duplicates(&lib, "FOO QUEST™", None, None),
        vec![DupMatch { id: id(&lib, "Foo Quest"), reason: DupReason::SameName }]
    );
    assert_eq!(find_duplicates(&lib, "old game", None, None)[0].reason, DupReason::HiddenCopy);
    // A program in a broad folder (Downloads) is not "the same game" as another one there.
    assert!(find_duplicates(&lib, "New", None, Some(Path::new("/home/u/Downloads/new.AppImage"))).is_empty());
    // umu-run commands compare by the .exe they run.
    let exe = PathBuf::from("/home/u/Games/X/x.exe");
    let umu = windows_launch_with(
        &exe,
        &proton(Path::new("/p")),
        Path::new("/pfx"),
        Some("umu-run".into()),
        Path::new("/home/u"),
    );
    assert_eq!(program_key(&umu), Some(ProgKey::Exe(exe)));

    // Candidates get `existing`.
    let mut cands = vec![ImportCandidate {
        name: "SP Football Life 2027".into(),
        path: fl_dir.join("FL 2027 start.exe"),
        kind: ProgramKind::Windows,
        launch: None,
        icon: None,
        install_dir: Some(fl_dir.clone()),
        existing: None,
        origin: String::new(),
    }];
    mark_existing(&lib.games, &mut cands);
    assert_eq!(cands[0].existing, Some(id(&lib, "Football Life 2027")));
}

#[test]
fn merge_suggestions_and_merge_moves_user_data() {
    let mut db = Db::open_memory().unwrap();
    let mut hades = sg(Source::Steam, "1145360", "Hades");
    hades.playtime_secs = Some(600);
    hades.install_dir = Some("/s/steamapps/common/Hades".into());
    let other = sg(Source::Steam, "10", "Unrelated");
    let lib = library(&mut db, vec![hades, other]);
    let steam_id = id(&lib, "Hades");
    let manual = db
        .add_manual(
            "HADES",
            LaunchSpec::Command {
                program: "/s/steamapps/common/Hades/Hades.exe".into(),
                args: vec![],
                cwd: None,
                env: vec![],
            },
            Art { cover: Some("/art/cover.png".into()), icon: Some("/art/icon.png".into()), ..Default::default() },
        )
        .unwrap();
    db.set_user_art(steam_id, &Art { icon: Some("/art/steam-icon.png".into()), ..Default::default() }).unwrap();
    db.set_favorite(manual, true).unwrap();
    db.set_tags(manual, &["Roguelike".into(), "rpg".into()]).unwrap();
    db.set_tags(steam_id, &["RPG".into()]).unwrap();
    db.set_rating(manual, Some(5)).unwrap();
    db.set_play_stats(manual, 3600, 3, Some(2_000_000_000)).unwrap();
    db.set_added(manual, 1_000).unwrap();
    db.set_controller_override(manual, Some(ControllerSupport::Full)).unwrap();
    db.set_launch_override(
        manual,
        Some(&LaunchOverride { options: "MANGOHUD=1 %command%".into(), ..Default::default() }),
    )
    .unwrap();
    let edits = UserEdits {
        notes: "great".into(),
        completion: Some(CompletionStatus::Beaten),
        install_dir: Some("/elsewhere".into()),
        links: vec![GameLink { name: "Wiki".into(), url: "https://x".into() }],
        ..Default::default()
    };
    db.set_edits(manual, &edits).unwrap();
    let lib = Library::new(db.load_all().unwrap());

    let s = compute_merge_suggestions(&lib.games, &HashSet::new());
    assert_eq!(s, vec![MergeSuggestion { manual_id: manual, scanned_id: steam_id }]);
    let dismissed: HashSet<(GameId, GameId)> = [(manual, steam_id)].into_iter().collect();
    assert!(compute_merge_suggestions(&lib.games, &dismissed).is_empty());

    merge_in_db(&db, lib.get(manual).unwrap(), lib.get(steam_id).unwrap()).unwrap();
    let lib = Library::new(db.load_all().unwrap());
    assert!(lib.get(manual).is_none(), "the hand-added copy is gone");
    let g = lib.get(steam_id).unwrap();
    assert!(g.favorite);
    assert_eq!(g.tags, vec!["RPG", "Roguelike"]);
    assert_eq!(g.user_rating, Some(5));
    assert_eq!(g.playtime_secs, 4200);
    assert_eq!(g.play_count, 3);
    assert_eq!(g.last_played, Some(2_000_000_000));
    assert_eq!(g.added, 1_000);
    assert_eq!(g.controller_override, Some(ControllerSupport::Full));
    assert_eq!(g.launch_override.as_ref().unwrap().options, "MANGOHUD=1 %command%");
    assert_eq!(g.edits.notes, "great");
    assert_eq!(g.completion(), Some(CompletionStatus::Beaten));
    assert_eq!(g.edits.links.len(), 1);
    assert_eq!(g.edits.install_dir, None, "the scanned game keeps its own folder");
    assert_eq!(g.install_dir.as_deref(), Some(Path::new("/s/steamapps/common/Hades")));
    let art = db.user_art(steam_id).unwrap();
    assert_eq!(art.icon.as_deref(), Some(Path::new("/art/steam-icon.png")), "scanned game's own choice kept");
    assert_eq!(art.cover.as_deref(), Some(Path::new("/art/cover.png")), "filled from the hand-added game");
    assert!(compute_merge_suggestions(&lib.games, &HashSet::new()).is_empty());
}

#[test]
fn merge_suggestions_by_folder_and_skip_hidden() {
    let mut db = Db::open_memory().unwrap();
    let mut fl = sg(Source::Faugus, "abc", "Football Life 2027");
    fl.install_dir = Some("/home/u/Faugus/fl/drive_c/Program Files (x86)/SP Football Life 2027".into());
    let hidden = sg(Source::Steam, "5", "Secret");
    let lib = library(&mut db, vec![fl, hidden]);
    db.set_hidden(id(&lib, "Secret"), true).unwrap();
    let m1 = db
        .add_manual(
            "FL27",
            LaunchSpec::Command {
                program: "umu-run".into(),
                args: vec![
                    "/home/u/Faugus/fl/drive_c/Program Files (x86)/SP Football Life 2027/FL 2027 start.exe".into(),
                ],
                cwd: None,
                env: vec![("WINEPREFIX".into(), "/home/u/Faugus/fl".into())],
            },
            Art::default(),
        )
        .unwrap();
    db.add_manual("Secret", LaunchSpec::Url("x".into()), Art::default()).unwrap();
    let lib = Library::new(db.load_all().unwrap());
    let s = compute_merge_suggestions(&lib.games, &HashSet::new());
    assert_eq!(s, vec![MergeSuggestion { manual_id: m1, scanned_id: id(&lib, "Football Life 2027") }]);
}

#[test]
fn merge_edits_fills_only_defaults() {
    let into = UserEdits { notes: "mine".into(), ..Default::default() };
    let from = UserEdits {
        notes: "theirs".into(),
        series: Some("S".into()),
        installed: Some(false),
        scripts: GameScripts { before_start: "echo".into(), ..Default::default() },
        ..Default::default()
    };
    let m = merge_edits(&into, &from);
    assert_eq!(m.notes, "mine");
    assert_eq!(m.series.as_deref(), Some("S"));
    assert_eq!(m.installed, None);
    assert_eq!(m.scripts.before_start, "echo");
    assert!(m.scripts.use_global);
}

#[test]
fn dismissed_merges_are_stored() {
    let db = Db::open_memory().unwrap();
    assert!(dismissed(&db).is_empty());
    db.kv_set(DISMISSED_KEY, "[[1,2],[3,4]]").unwrap();
    assert_eq!(dismissed(&db), [(1, 2), (3, 4)].into_iter().collect());
    db.kv_set(DISMISSED_KEY, "garbage").unwrap();
    assert!(dismissed(&db).is_empty());
}

// ---------------------------------------------------------------------------------------------
// The real machine (read-only): `cargo test real_installed_scan -- --ignored --nocapture`

#[test]
#[ignore]
fn real_installed_scan() {
    let ctx = ScanContext::from_config(&crate::config::Config::default());
    let t = tempfile::tempdir().unwrap();
    let protons = proton_versions(&ctx);
    println!("Proton builds (newest first):");
    for pv in &protons {
        println!("  {:<28} {:<12} {}", pv.name, pv.flavor(), pv.path.display());
    }
    println!("umu: {:?}", find_umu(&ctx.home));
    println!("Prefixes:");
    for pf in find_prefixes(&ctx) {
        println!("  {}", pf.origin);
    }
    let env = Env { protons: &protons, home: &ctx.home, umu: find_umu(&ctx.home) };
    let t0 = std::time::Instant::now();
    let (mut found, errors) = collect(|s| scan_installed(&env, &ctx, s));
    println!("Detect installed: {} candidates in {:?} (errors {errors:?})", found.len(), t0.elapsed());
    // Mark against a *copy* of the real library (the original is never opened).
    let real = ctx.home.join(".local/share/lodestar");
    let copy = t.path().join("lib");
    std::fs::create_dir_all(&copy).unwrap();
    for f in ["library.db", "library.db-wal", "library.db-shm"] {
        let _ = std::fs::copy(real.join(f), copy.join(f));
    }
    let games = Db::open(&copy.join("library.db")).and_then(|d| d.load_all()).unwrap_or_default();
    mark_existing(&games, &mut found);
    let name_of =
        |id: GameId| games.iter().find(|g| g.id == id).map(|g| g.display_name().to_string()).unwrap_or_default();
    for c in &found {
        let ex = c.existing.map(|i| format!("  (in library: {})", name_of(i))).unwrap_or_default();
        println!("  [{:<8}] {:<40} {:<45} {}{ex}", c.kind.label(), c.name, c.origin, c.path.display());
        if let Some(l) = &c.launch
            && c.kind == ProgramKind::Windows
        {
            println!(
                "             PROTONPATH={:?} WINEPREFIX={:?}",
                cmd_env(l, "PROTONPATH"),
                cmd_env(l, "WINEPREFIX")
            );
        }
    }
}

/// `cargo test real_folder_scan -- --ignored --nocapture` (read-only; folders from
/// `LODESTAR_SCAN_DIRS`, colon-separated, default ~/Faugus:~/Games:~/.local/bin).
#[test]
#[ignore]
fn real_folder_scan() {
    let ctx = ScanContext::from_config(&crate::config::Config::default());
    let protons = proton_versions(&ctx);
    let env = Env { protons: &protons, home: &ctx.home, umu: find_umu(&ctx.home) };
    let dirs = std::env::var("LODESTAR_SCAN_DIRS").unwrap_or_else(|_| {
        ["Faugus", "Games", ".local/bin"].map(|d| ctx.home.join(d).display().to_string()).join(":")
    });
    for d in dirs.split(':') {
        let t0 = std::time::Instant::now();
        let cancel = AtomicBool::new(false);
        let mut found = vec![];
        let mut emit = |m: ImportMsg| {
            if let ImportMsg::Found(v) = m {
                found.extend(v)
            }
        };
        let mut sink = Sink { cancel: &cancel, emit: &mut emit, files: 0 };
        scan_folder(&env, Path::new(d), &mut sink);
        let files = sink.files;
        println!("{d}: {} candidates, {files} files, {:?}", found.len(), t0.elapsed());
        for c in &found {
            println!("  [{:<8}] {:<36} {}", c.kind.label(), c.name, c.path.display());
        }
    }
}
