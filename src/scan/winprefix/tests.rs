//! Tests for winprefix (prefix discovery, runners, launch commands) and the Wine registry reader.
//! Fixture: tests/fixtures/wine/home (regenerate with tests/fixtures/wine/tools/gen_fixtures.py).

use super::registry::*;
use super::*;
use std::path::{Path, PathBuf};

/// Copy the fixture home into a temp dir, replacing `@HOME@` in text files with the new home.
pub(crate) fn fixture_home() -> (tempfile::TempDir, PathBuf) {
    fn copy(src: &Path, dst: &Path, home: &str) {
        std::fs::create_dir_all(dst).unwrap();
        for e in std::fs::read_dir(src).unwrap().flatten() {
            let to = dst.join(e.file_name());
            if e.file_type().unwrap().is_dir() {
                copy(&e.path(), &to, home);
                continue;
            }
            std::fs::copy(e.path(), &to).unwrap();
            if let Ok(text) = std::fs::read_to_string(e.path())
                && text.contains("@HOME@")
            {
                std::fs::write(&to, text.replace("@HOME@", home)).unwrap();
            }
        }
    }
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wine/home");
    copy(&src, &home, &home.to_string_lossy());
    (tmp, home)
}

pub(crate) fn ctx(home: &Path) -> ScanContext {
    let mut c = crate::scan::desktop::test_support::ctx(home);
    c.config.extra_prefix_dirs = vec![home.join("extra"), home.join("single")];
    c
}

fn rel(p: &Path, home: &Path) -> String {
    p.strip_prefix(home).unwrap().to_string_lossy().into_owned()
}

#[test]
fn finds_prefixes_with_runners() {
    let (_t, home) = fixture_home();
    let found = find_prefixes(&ctx(&home));
    let proton = Runner::Proton { path: home.join(".local/share/Steam/compatibilitytools.d/Fixture-Proton") };
    let got: Vec<(String, String, Runner)> =
        found.iter().map(|p| (rel(&p.path, &home), p.origin.clone(), p.runner.clone())).collect();
    let want = vec![
        ("Faugus/battlenet".into(), "Faugus: Battle.net".into(), proton.clone()),
        ("Faugus/empty".into(), "Faugus: empty".into(), proton.clone()),
        (
            "Games/ubisoft".into(),
            "Lutris: ubisoft-connect".into(),
            Runner::Wine { path: home.join(".local/share/lutris/runners/wine/wine-ge-8-26-x86_64/bin/wine") },
        ),
        (
            ".local/share/bottles/bottles/Riot".into(),
            "Bottles: Riot Games".into(),
            Runner::Wine { path: home.join(".local/share/bottles/runners/soda-9.0-1/bin/wine") },
        ),
        ("Games/Heroic/Prefixes/default/HoYo/pfx".into(), "Heroic: HoYo".into(), proton.clone()),
        (
            ".local/share/Steam/steamapps/compatdata/3000000001/pfx".into(),
            "Steam shortcut: Battle.net".into(),
            proton.clone(),
        ),
        ("Games/umu/umu-default".into(), "umu: umu-default".into(), Runner::Unknown),
        ("Games/plain".into(), "~/Games/plain".into(), Runner::Unknown),
        (".wine".into(), "~/.wine".into(), Runner::Unknown),
        ("extra/one".into(), "~/extra/one".into(), Runner::Unknown),
        ("extra/two".into(), "~/extra/two".into(), Runner::Unknown),
        ("single".into(), "~/single".into(), Runner::Unknown),
    ];
    assert_eq!(got, want);
    // Not prefixes: no system.reg; a regular Steam game's compatdata.
    assert!(!found.iter().any(|p| p.path.ends_with("notaprefix") || p.path.to_string_lossy().contains("/730/")));
}

#[test]
fn no_launchers_no_prefixes() {
    let t = tempfile::tempdir().unwrap();
    assert!(find_prefixes(&crate::scan::desktop::test_support::ctx(t.path())).is_empty());
}

#[test]
fn compat_root_and_hash() {
    let (_t, home) = fixture_home();
    let steam = home.join(".local/share/Steam/steamapps/compatdata/3000000001/pfx");
    assert_eq!(compat_root(&steam), home.join(".local/share/Steam/steamapps/compatdata/3000000001"));
    assert_eq!(compat_root(&home.join("Games/plain")), home.join("Games/plain"));
    // umu's `pfx -> .` symlink is not a separate folder.
    let umu = home.join("Games/umu/umu-default");
    std::os::unix::fs::symlink(".", umu.join("pfx")).unwrap();
    assert_eq!(compat_root(&umu.join("pfx")), umu.join("pfx"));
    assert_eq!(prefix_hash(Path::new("/a")), prefix_hash(Path::new("/a")));
    assert_ne!(prefix_hash(Path::new("/a")), prefix_hash(Path::new("/b")));
    assert_eq!(prefix_hash(Path::new("/a")).len(), 6);
}

#[test]
fn resolves_unknown_runners() {
    let (_t, home) = fixture_home();
    let c = ctx(&home);
    let mut ps = find_prefixes(&c);
    resolve_runners(&c, &mut ps);
    let umu = ps.iter().find(|p| p.origin == "umu: umu-default").unwrap();
    // Proton-made prefix (has `version`) → a Proton build, never left Unknown when one exists.
    assert!(matches!(umu.runner, Runner::Proton { .. }), "{:?}", umu.runner);
    for p in &ps {
        assert_ne!(p.runner, Runner::Unknown, "{}", p.origin);
    }
}

fn env_of(l: &LaunchSpec, k: &str) -> Option<String> {
    let LaunchSpec::Command { env, .. } = l else {
        return None;
    };
    env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
}

#[test]
fn run_in_prefix_commands() {
    let exe = Path::new("/p/drive_c/Program Files/Game/game.exe");
    let args = vec!["--exec=launch Pro".to_string()];
    let home = Path::new("/home/u");
    let proton_dir = PathBuf::from("/home/u/.local/share/Steam/compatibilitytools.d/GE-Proton10-1");
    // Proton with umu: WINEPREFIX is the compat root of a Steam-style pfx.
    let steam = Prefix {
        path: "/nonexistent/compatdata/3000000001/pfx".into(),
        origin: "x".into(),
        runner: Runner::Proton { path: proton_dir.clone() },
    };
    let l = run_in_prefix_with(&steam, exe, &args, Some("umu-run".into()), None, home);
    assert_eq!(
        l,
        LaunchSpec::Command {
            program: "umu-run".into(),
            args: vec![exe.to_string_lossy().into(), "--exec=launch Pro".into()],
            cwd: Some("/p/drive_c/Program Files/Game".into()),
            env: vec![
                // pfx doesn't exist here, so it isn't treated as a real folder: kept as is.
                ("WINEPREFIX".into(), "/nonexistent/compatdata/3000000001/pfx".into()),
                ("GAMEID".into(), "umu-default".into()),
                ("PROTONPATH".into(), proton_dir.to_string_lossy().into()),
            ],
        }
    );
    // Proton without umu: the build's own script.
    let l = run_in_prefix_with(&steam, exe, &args, None, None, home);
    let LaunchSpec::Command { program, args: a, .. } = &l else { panic!() };
    assert_eq!(program, "/home/u/.local/share/Steam/compatibilitytools.d/GE-Proton10-1/proton");
    assert_eq!(a[0], "run");
    assert_eq!(a.last().unwrap(), "--exec=launch Pro");
    assert_eq!(env_of(&l, "STEAM_COMPAT_CLIENT_INSTALL_PATH").as_deref(), Some("/home/u/.local/share/Steam"));
    assert!(env_of(&l, "STEAM_COMPAT_DATA_PATH").is_some());
    // Wine runner.
    let wine =
        Prefix { path: "/pfx".into(), origin: "x".into(), runner: Runner::Wine { path: "/opt/wine/bin/wine".into() } };
    let l = run_in_prefix_with(&wine, exe, &args, Some("umu-run".into()), None, home);
    let LaunchSpec::Command { program, args: a, env, .. } = &l else { panic!() };
    assert_eq!(program, "/opt/wine/bin/wine");
    assert_eq!(a.len(), 2);
    assert_eq!(env, &vec![("WINEPREFIX".to_string(), "/pfx".to_string())]);
    // Unknown: wine on PATH first, then umu with its default Proton, then plain `wine`.
    let unk = Prefix { path: "/pfx".into(), origin: "x".into(), runner: Runner::Unknown };
    let l = run_in_prefix_with(&unk, exe, &[], Some("umu-run".into()), Some("/usr/bin/wine".into()), home);
    assert!(matches!(&l, LaunchSpec::Command { program, .. } if program == "/usr/bin/wine"));
    let l = run_in_prefix_with(&unk, exe, &[], Some("umu-run".into()), None, home);
    assert!(matches!(&l, LaunchSpec::Command { program, .. } if program == "umu-run"));
    assert_eq!(env_of(&l, "PROTONPATH"), None);
    let l = run_in_prefix_with(&unk, exe, &[], None, None, home);
    assert!(matches!(&l, LaunchSpec::Command { program, .. } if program == "wine"));
    // .msi through msiexec.
    let l = run_in_prefix_with(&wine, Path::new("/d/setup.msi"), &[], None, None, home);
    let LaunchSpec::Command { args: a, .. } = &l else { panic!() };
    assert_eq!(a, &vec!["msiexec".to_string(), "/i".into(), "/d/setup.msi".into()]);
}

#[test]
fn steam_prefix_runs_in_compat_root() {
    let (_t, home) = fixture_home();
    let ps = find_prefixes(&ctx(&home));
    let s = ps.iter().find(|p| p.origin.starts_with("Steam shortcut")).unwrap();
    let l = run_in_prefix_with(s, Path::new("/x/a.exe"), &[], Some("umu-run".into()), None, &home);
    assert_eq!(
        env_of(&l, "WINEPREFIX").as_deref(),
        Some(home.join(".local/share/Steam/steamapps/compatdata/3000000001").to_str().unwrap())
    );
}

#[test]
fn windows_paths() {
    let (_t, home) = fixture_home();
    let pfx = home.join("Games/ubisoft");
    // Case-insensitive match against what exists, both separators, trailing separator.
    assert_eq!(
        win_to_unix(&pfx, "c:\\program files (x86)\\UBISOFT\\ubisoft game launcher\\"),
        Some(pfx.join("drive_c/Program Files (x86)/Ubisoft/Ubisoft Game Launcher"))
    );
    assert_eq!(
        win_to_unix(&pfx, "C:/Program Files/EA Games/Battlefield 1/bf1.exe"),
        Some(pfx.join("drive_c/Program Files/EA Games/Battlefield 1/bf1.exe"))
    );
    // Missing parts are kept as written.
    assert_eq!(win_to_unix(&pfx, "C:\\Nope\\x"), Some(pfx.join("drive_c/Nope/x")));
    // Other drives through dosdevices.
    std::fs::create_dir_all(pfx.join("dosdevices")).unwrap();
    std::os::unix::fs::symlink(&home, pfx.join("dosdevices/d:")).unwrap();
    assert_eq!(win_to_unix(&pfx, "D:\\Games\\plain"), Some(home.join("Games/plain")));
    assert_eq!(win_to_unix(&pfx, "E:\\x"), None);
    assert_eq!(win_to_unix(&pfx, "relative\\x"), None);
    assert_eq!(win_to_unix(&pfx, "::"), None);
    assert_eq!(unix_to_win(&pfx, &pfx.join("drive_c/Program Files/X")), "C:\\Program Files\\X");
    assert_eq!(unix_to_win(&pfx, Path::new("/home/u/g")), "Z:\\home\\u\\g");
}

// ---------------------------------------------------------------------------------------------
// Registry

#[test]
fn registry_parses_wine_format() {
    let (_t, home) = fixture_home();
    let text = std::fs::read_to_string(home.join("Games/ubisoft/system.reg")).unwrap();
    let reg = Registry::parse(&text);
    let k = reg.key("HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion").unwrap();
    assert_eq!(k.get_str("productname"), Some("Microsoft Windows 10"));
    assert_eq!(k.get("PathExpand"), Some(&RegValue::Str("%SystemRoot%".into())));
    assert_eq!(k.get("Blob"), Some(&RegValue::Bytes(vec![0xde, 0xad, 0xbe, 0xef])));
    assert_eq!(k.get("Multi"), Some(&RegValue::Multi(vec!["one".into(), "two".into()])));
    assert_eq!(k.get("Expand"), Some(&RegValue::Str("%SystemRoot%\\system32".into())));
    assert_eq!(k.get(""), Some(&RegValue::Str("default".into())));
    assert_eq!(k.get("Uni"), Some(&RegValue::Str("café 中".into())));
    assert_eq!(k.get("Quote"), Some(&RegValue::Str("say \"hi\"".into())));
    assert_eq!(k.name(), "CurrentVersion");
    assert_eq!(reg.value("Software/Wow6432Node/Ubisoft/Launcher", "Version"), Some(&RegValue::Dword(0x96)));
    assert_eq!(
        reg.get_str("software\\wow6432node\\ubisoft\\launcher", "installdir"),
        Some("C:\\Program Files (x86)\\Ubisoft\\Ubisoft Game Launcher\\")
    );
    let installs: Vec<&str> =
        reg.subkeys("Software\\Wow6432Node\\Ubisoft\\Launcher\\Installs").iter().map(|k| k.name()).collect();
    assert_eq!(installs, vec!["5595", "635", "999"]);
    // Parents without a section of their own.
    assert_eq!(
        reg.child_paths("Software\\Wow6432Node\\Rockstar Games").len(),
        5,
        "{:?}",
        reg.child_paths("Software\\Wow6432Node\\Rockstar Games")
    );
    assert_eq!(
        reg.child_paths("software"),
        vec!["Software\\Electronic Arts", "Software\\Microsoft", "Software\\Wine", "Software\\Wow6432Node"]
    );
    assert!(reg.key("Software\\Nope").is_none());
}

#[test]
fn registry_filter_and_values_api() {
    let (_t, home) = fixture_home();
    let pfx = home.join("Games/ubisoft");
    let reg = Registry::load(&pfx, Hive::System, |k| k.contains("rockstar"));
    assert!(reg.key("Software\\Wine").is_none());
    assert!(reg.key("Software\\Wow6432Node\\Rockstar Games\\Launcher").is_some());
    let v = registry_values(&pfx, Hive::System, "HKLM\\Software\\Wow6432Node\\Ubisoft\\Launcher");
    assert_eq!(
        v,
        vec![
            ("InstallDir".to_string(), "C:\\Program Files (x86)\\Ubisoft\\Ubisoft Game Launcher\\".to_string()),
            ("Version".to_string(), "150".to_string()),
        ]
    );
    let hoyo = home.join("Games/Heroic/Prefixes/default/HoYo/pfx");
    let v = registry_values(&hoyo, Hive::User, "HKCU\\Software\\Cognosphere\\HYP\\1_0\\hk4e_global");
    assert_eq!(v.len(), 1);
    assert!(registry_values(&home.join("nope"), Hive::System, "Software").is_empty());
}

#[test]
fn registry_survives_garbage() {
    let junk =
        "[unterminated\n\"a\"=\"b\"\n[Ok] 1\n\"x\"=dword:zz\n\"y\"=hex:0g,01\n\"z\"=\"unterminated\n\"w\"=hex:01,\\\n";
    let reg = Registry::parse(junk);
    let k = reg.key("Ok").unwrap();
    assert_eq!(k.get("x"), None);
    assert_eq!(k.get("y"), Some(&RegValue::Bytes(vec![1])));
    assert_eq!(k.get("z"), Some(&RegValue::Str("unterminated".into())));
    assert_eq!(k.get("w"), Some(&RegValue::Bytes(vec![1])));
    assert!(Registry::parse("").is_empty());
    assert_eq!(unescape("a\\\\b\\\"c\\x41\\n\\0"), "a\\b\"cA\n\0");
    assert_eq!(unescape("\\xd83d\\xde00"), "😀");
    assert_eq!(parse_value_line("\"n\" = \"v\""), Some(("n".into(), RegValue::Str("v".into()))));
}

/// Read-only look at this machine's prefixes and launcher games (`cargo test real_machine -- --ignored --nocapture`).
#[test]
#[ignore]
fn real_machine_readonly() {
    let c = ScanContext::from_config(&crate::config::Config::default());
    let t = std::time::Instant::now();
    let mut ps = find_prefixes(&c);
    let found = t.elapsed();
    resolve_runners(&c, &mut ps);
    for p in &ps {
        println!("{:<45} {}  {:?}", p.origin, p.path.display(), p.runner);
        let db = p.path.join("drive_c/ProgramData/Battle.net/Agent/product.db");
        if let Ok(b) = std::fs::read(&db) {
            println!("    product.db: {:?}", crate::scan::wine_launchers::battlenet::decode_product_db(&b));
        }
        println!("    launchers: {:?}", crate::scan::wine_launchers::launchers_in(&p.path));
    }
    let t = std::time::Instant::now();
    let games = crate::scan::wine_launchers::scan(&c).unwrap();
    println!("find_prefixes: {found:?}, launcher scan: {:?}, {} games", t.elapsed(), games.len());
    for g in &games {
        println!("  {:?} {} {}", g.source, g.source_id, g.name);
    }
    // What Install / open-client would run (a LaunchSpec only; nothing is started).
    println!(
        "launcher_command(BattleNet, OSI): {:?}",
        crate::scan::wine_launchers::launcher_command(&c, crate::model::Source::BattleNet, "OSI")
    );
}

/// Timing of the registry read on a real prefix (read-only; set LODESTAR_REG_PREFIX).
#[test]
#[ignore]
fn real_registry_timing() {
    let Ok(p) = std::env::var("LODESTAR_REG_PREFIX") else {
        return;
    };
    let p = PathBuf::from(p);
    let t = std::time::Instant::now();
    let text = crate::scan::util::read_text(p.join("system.reg")).unwrap();
    let read = t.elapsed();
    let t = std::time::Instant::now();
    let n = text.lines().filter(|l| l.starts_with('[')).count();
    let lines = t.elapsed();
    let t = std::time::Instant::now();
    let r = Registry::parse_filtered(&text, |k| k.starts_with("software\\wow6432node\\electronic arts"));
    let parse = t.elapsed();
    let t = std::time::Instant::now();
    let d = crate::scan::wine_launchers::scan_prefix(&p);
    println!(
        "read {read:?}, {n} keys counted in {lines:?}, parse {parse:?} (empty: {}), scan_prefix {:?} ({} found)",
        r.is_empty(),
        t.elapsed(),
        d.0.len()
    );
}
