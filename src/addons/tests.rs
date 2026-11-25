//! Add-on tests. Every add-on run here is a tiny sh script that only prints, sleeps or touches a
//! file inside its temp folder.

use super::*;
use crate::config::AddonsConfig;
use crate::model::{LaunchSpec, Source};
use crate::test_support::write_exe;
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/addons")
}
fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/addons")
}

/// Write an add-on folder `<parent>/<id>` with `main.sh` = `script`.
fn make_addon(parent: &Path, id: &str, kind: &str, timeout: u64, script: &str) -> PathBuf {
    let dir = parent.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("addon.toml"),
        format!("id = \"{id}\"\nname = \"{id} name\"\nauthor = \"tests\"\nkind = \"{kind}\"\nexec = \"main.sh\"\ntimeout_secs = {timeout}\n"),
    )
    .unwrap();
    write_exe(&dir.join("main.sh"), format!("#!/bin/sh\n{script}\n"));
    dir
}

fn enabled(ids: &[&str]) -> AddonsConfig {
    AddonsConfig { enabled: ids.iter().map(|s| s.to_string()).collect(), ..Default::default() }
}

fn manager(tmp: &Path) -> AddonManager {
    let mut m = AddonManager::new(tmp.join("data/addons"));
    m.refresh();
    m
}

fn installed(tmp: &Path, id: &str, kind: &str, timeout: u64, script: &str) -> AddonManager {
    let src = tmp.join("src");
    let dir = make_addon(&src, id, kind, timeout, script);
    let mut m = manager(tmp);
    assert_eq!(m.install_from(&dir).unwrap(), id);
    m
}

// ----- manifest -----

#[test]
fn manifest_validation() {
    let ok = manifest::parse("id = \"my-addon.v2_x\"\nexec = \"bin/run\"\n").unwrap();
    assert_eq!(ok.name, "my-addon.v2_x", "name falls back to the id");
    assert_eq!(ok.timeout_secs, manifest::DEFAULT_TIMEOUT_SECS);
    assert_eq!(ok.kind, AddonKind::Library);
    let capped = manifest::parse("id = \"a\"\nexec = \"x\"\ntimeout_secs = 99999\nkind = \"metadata\"\n").unwrap();
    assert_eq!(capped.timeout_secs, manifest::MAX_TIMEOUT_SECS);
    assert_eq!(capped.kind, AddonKind::Metadata);

    for (bad, why) in [
        ("exec = \"x\"\n", "missing id"),
        ("id = \"My Addon\"\nexec = \"x\"\n", "uppercase / space"),
        ("id = \"../evil\"\nexec = \"x\"\n", "path in id"),
        ("id = \".hidden\"\nexec = \"x\"\n", "hidden id"),
        ("id = \"a\"\n", "missing exec"),
        ("id = \"a\"\nexec = \"/bin/sh\"\n", "absolute exec"),
        ("id = \"a\"\nexec = \"../x\"\n", "parent exec"),
        ("id = \"a\"\nexec = \"bin/../../x\"\n", "parent inside exec"),
        ("id = \"a\"\nexec = \"x\"\nkind = \"theme\"\n", "unknown kind"),
        ("id = \"a\"\nexec = \"x\"\n[[settings]]\nkey = \"k\"\n[[settings]]\nkey = \"k\"\n", "duplicate key"),
        ("id = \"a\"\nexec = \"x\"\n[[settings]]\nkey = \"bad key\"\n", "bad key"),
        (
            "id = \"a\"\nexec = \"x\"\n[[settings]]\nkey = \"t\"\nkind = \"toggle\"\ndefault = \"yes\"\n",
            "toggle default",
        ),
        ("id = [1]\n", "not toml for us"),
    ] {
        assert!(manifest::parse(bad).is_err(), "{why} should be refused");
    }
    assert!(manifest::valid_id("a-b_c.d9"));
    assert!(!manifest::valid_id(&"x".repeat(65)));
}

#[test]
fn exec_must_exist_be_executable_and_stay_inside() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = make_addon(tmp.path(), "a", "library", 5, "echo {}");
    assert!(manifest::resolve_exec(&dir, "main.sh").is_ok());
    assert!(manifest::resolve_exec(&dir, "nope.sh").is_err());
    // Not executable.
    std::fs::write(dir.join("plain.sh"), "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(dir.join("plain.sh"), std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(manifest::resolve_exec(&dir, "plain.sh").unwrap_err().to_string().contains("chmod"));
    // Symlink escaping the folder.
    std::os::unix::fs::symlink("/bin/sh", dir.join("escape")).unwrap();
    assert!(manifest::resolve_exec(&dir, "escape").unwrap_err().to_string().contains("outside"));
    // Symlink inside the folder is fine.
    std::os::unix::fs::symlink("main.sh", dir.join("alias")).unwrap();
    assert!(manifest::resolve_exec(&dir, "alias").is_ok());
}

#[test]
fn refresh_reports_broken_addons() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("data/addons");
    make_addon(&root, "good", "library", 5, "echo {}");
    let wrong = make_addon(&root, "wrong-folder", "library", 5, "echo {}");
    std::fs::write(wrong.join("addon.toml"), "id = \"other\"\nexec = \"main.sh\"\n").unwrap();
    std::fs::create_dir_all(root.join("garbage")).unwrap();
    std::fs::write(root.join("garbage/addon.toml"), "this is = = not toml").unwrap();
    std::fs::create_dir_all(root.join(".staging-x-1")).unwrap();
    std::fs::write(root.join("stray-file"), "x").unwrap();
    let m = manager(tmp.path());
    let ids: Vec<&str> = m.list().iter().map(|a| a.manifest.id.as_str()).collect();
    assert_eq!(ids.len(), 3, "{ids:?}");
    assert!(m.get("good").unwrap().is_usable());
    assert!(m.get("wrong-folder").unwrap().error.as_deref().unwrap().contains("doesn't match"));
    assert!(m.get("garbage").unwrap().error.is_some());
    // A broken add-on is never run.
    let err = m.call("garbage", "list", serde_json::json!({}), &enabled(&["garbage"])).unwrap_err();
    assert!(err.to_string().contains("addon.toml"), "{err}");
}

// ----- install / remove -----

#[test]
fn install_from_folder_copies_keeps_exec_bit_and_never_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join("ran");
    let script = format!("touch '{}'\necho '{{}}'", marker.display());
    let mut m = installed(tmp.path(), "inst", "library", 5, &script);
    assert!(!marker.exists(), "install must never run the add-on");
    let info = m.get("inst").unwrap().clone();
    assert_eq!(info.dir, tmp.path().join("data/addons/inst"));
    let mode = std::fs::metadata(info.dir.join("main.sh")).unwrap().permissions().mode();
    assert_ne!(mode & 0o111, 0, "executable bit kept");
    assert!(info.is_usable());

    // Updating the same add-on works; a different add-on with the same id is refused.
    let src = tmp.path().join("src/inst");
    std::fs::write(src.join("extra.txt"), "v2").unwrap();
    m.install_from(&src).unwrap();
    assert!(tmp.path().join("data/addons/inst/extra.txt").exists());
    let other = tmp.path().join("other");
    make_addon(&other, "inst", "library", 5, "echo {}");
    std::fs::write(other.join("inst/addon.toml"), "id = \"inst\"\nname = \"Impostor\"\nexec = \"main.sh\"\n").unwrap();
    let err = m.install_from(&other.join("inst")).unwrap_err().to_string();
    assert!(err.contains("remove it first"), "{err}");
    // Installing the installed folder itself is refused.
    assert!(m.install_from(&info.dir).is_err());
    // A folder wrapping exactly one add-on folder is accepted (like an unpacked zip).
    let wrapped = tmp.path().join("wrapped");
    make_addon(&wrapped, "wrapped-one", "metadata", 5, "echo {}");
    assert_eq!(m.install_from(&wrapped).unwrap(), "wrapped-one");
    assert!(!marker.exists());
    assert_eq!(m.list().len(), 2);
    // No work folders left behind.
    let hidden = std::fs::read_dir(m.addons_dir())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
        .count();
    assert_eq!(hidden, 0);

    m.remove("inst").unwrap();
    assert!(m.get("inst").is_none());
    assert!(!tmp.path().join("data/addons/inst").exists());
    assert!(m.remove("inst").is_err());
    assert!(m.remove("../src").is_err(), "ids with paths are refused");
    assert!(src.exists());
}

#[test]
fn install_refuses_symlinks_out_of_the_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = make_addon(&tmp.path().join("src"), "linky", "library", 5, "echo {}");
    std::fs::write(tmp.path().join("secret"), "s").unwrap();
    std::os::unix::fs::symlink(tmp.path().join("secret"), dir.join("leak")).unwrap();
    let mut m = manager(tmp.path());
    let err = m.install_from(&dir).unwrap_err().to_string();
    assert!(err.contains("outside"), "{err}");
    assert!(!tmp.path().join("data/addons/linky").exists());
    // A link to a file inside is copied as a file.
    std::fs::remove_file(dir.join("leak")).unwrap();
    std::fs::write(dir.join("readme"), "r").unwrap();
    std::os::unix::fs::symlink("readme", dir.join("readme-link")).unwrap();
    m.install_from(&dir).unwrap();
    let copied = tmp.path().join("data/addons/linky/readme-link");
    assert!(!std::fs::symlink_metadata(&copied).unwrap().file_type().is_symlink());
}

#[test]
fn install_fixture_with_nested_exec() {
    let tmp = tempfile::tempdir().unwrap();
    let mut m = manager(tmp.path());
    assert_eq!(m.install_from(&fixtures().join("fixture-library")).unwrap(), "fixture-library");
    let info = m.get("fixture-library").unwrap();
    assert!(info.is_usable(), "{:?}", info.error);
    assert!(info.exec_path.as_ref().unwrap().ends_with("bin/run.sh"));
    assert_eq!(info.manifest.permissions, vec!["reads its own folder".to_string()]);
}

/// Minimal zip writer (stored entries) so the test needs no `zip` program.
fn write_zip(path: &Path, entries: &[(&str, &[u8], u32)]) {
    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { (c >> 1) ^ 0xedb8_8320 } else { c >> 1 };
            }
        }
        !c
    }
    let mut out = vec![];
    let mut central = vec![];
    for (name, data, mode) in entries {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let n = name.len() as u16;
        let len = data.len() as u32;
        out.extend_from_slice(b"PK\x03\x04");
        for v in [20u16, 0, 0, 0, 0] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(b"PK\x01\x02");
        central.extend_from_slice(&(0x0300u16 | 20).to_le_bytes()); // made by: Unix
        for v in [20u16, 0, 0, 0, 0] {
            central.extend_from_slice(&v.to_le_bytes());
        }
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&len.to_le_bytes());
        central.extend_from_slice(&len.to_le_bytes());
        central.extend_from_slice(&n.to_le_bytes());
        for v in [0u16, 0, 0, 0] {
            central.extend_from_slice(&v.to_le_bytes());
        }
        central.extend_from_slice(&(mode << 16).to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd_off = out.len() as u32;
    let count = entries.len() as u16;
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06");
    for v in [0u16, 0, count, count] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    std::fs::write(path, out).unwrap();
}

#[test]
fn install_from_zip() {
    if install::which("unzip").is_none() {
        eprintln!("unzip not installed; skipping");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let zip = tmp.path().join("z.zip");
    write_zip(
        &zip,
        &[
            ("zipped/addon.toml", b"id = \"zipped\"\nname = \"Zipped\"\nexec = \"main.sh\"\n", 0o100644),
            ("zipped/main.sh", b"#!/bin/sh\necho '{\"games\":[]}'\n", 0o100755),
        ],
    );
    let mut m = manager(tmp.path());
    assert_eq!(m.install_from(&zip).unwrap(), "zipped");
    let info = m.get("zipped").unwrap();
    assert!(info.is_usable(), "{:?}", info.error);
    let reply = m.call("zipped", "list", serde_json::json!({}), &enabled(&["zipped"])).unwrap();
    assert_eq!(reply["games"], serde_json::json!([]));

    let evil = tmp.path().join("evil.zip");
    write_zip(&evil, &[("../escape/addon.toml", b"id = \"e\"\nexec = \"x\"\n", 0o100644)]);
    assert!(m.install_from(&evil).unwrap_err().to_string().contains("unsafe"));
    assert!(!tmp.path().join("data/escape").exists());
    let hidden = std::fs::read_dir(m.addons_dir())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
        .count();
    assert_eq!(hidden, 0);
    // Not a zip, not a folder.
    std::fs::write(tmp.path().join("x.txt"), "x").unwrap();
    assert!(m.install_from(&tmp.path().join("x.txt")).is_err());
}

// ----- calls -----

#[test]
fn call_passes_env_settings_request_and_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    let mut m = manager(tmp.path());
    m.install_from(&fixtures().join("fixture-library")).unwrap();
    let mut cfg = enabled(&["fixture-library"]);
    cfg.settings.entry("fixture-library".into()).or_default().insert("region".into(), "us".into());
    let r = m.call("fixture-library", "env", serde_json::json!({"hello": 1}), &cfg).unwrap();
    let dir = m.get("fixture-library").unwrap().dir.clone();
    assert_eq!(r["id"], "fixture-library");
    assert_eq!(r["cmd"], "env");
    assert_eq!(r["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(r["dir"], dir.to_string_lossy().as_ref());
    assert_eq!(r["pwd"], std::fs::canonicalize(&dir).unwrap().to_string_lossy().as_ref());
    assert_eq!(r["settings"], serde_json::json!({"token": "default-token", "region": "us"}));
    assert_eq!(r["request"], serde_json::json!({"hello": 1}));
    let data = PathBuf::from(r["data"].as_str().unwrap());
    assert_eq!(data, tmp.path().join("data/addon-data/fixture-library"));
    assert!(data.is_dir(), "data folder is created");
    assert!(m.last_error("fixture-library").is_none());

    // Unknown command → the add-on's {"error"} with exit 2.
    let err = m.call("fixture-library", "bogus", serde_json::json!({}), &cfg).unwrap_err().to_string();
    assert!(err.contains("unknown command bogus"), "{err}");
    assert!(m.last_error("fixture-library").unwrap().contains("unknown command"));
}

#[test]
fn disabled_and_missing_addons_are_not_run() {
    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join("ran");
    let m = installed(tmp.path(), "off", "library", 5, &format!("touch '{}'\necho '{{}}'", marker.display()));
    assert!(
        m.call("off", "list", serde_json::json!({}), &AddonsConfig::default())
            .unwrap_err()
            .to_string()
            .contains("disabled")
    );
    assert!(m.call("nope", "list", serde_json::json!({}), &enabled(&["nope"])).is_err());
    assert!(m.call("off", "../x", serde_json::json!({}), &enabled(&["off"])).is_err());
    assert!(!marker.exists());
    // Enabled: runs.
    m.call("off", "list", serde_json::json!({}), &enabled(&["off"])).unwrap();
    assert!(marker.exists());
}

#[test]
fn no_launch_refuses_launching_commands() {
    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join("ran");
    let mut m =
        installed(tmp.path(), "nl", "library", 5, &format!("touch '{}'\necho '{{\"ok\":true}}'", marker.display()));
    m.set_no_launch(true);
    let cfg = enabled(&["nl"]);
    for c in ["install", "uninstall", "launch"] {
        assert!(m.call("nl", c, serde_json::json!({"id": "g"}), &cfg).is_err(), "{c}");
    }
    assert!(m.game_command("nl", "install", "g", &cfg).is_err());
    assert!(!marker.exists());
    m.set_no_launch(false);
    m.game_command("nl", "install", "g", &cfg).unwrap();
    assert!(marker.exists());
}

#[test]
fn timeout_kills_the_process_group() {
    let tmp = tempfile::tempdir().unwrap();
    let pids = tmp.path().join("pids");
    // A background child plus a foreground sleep: both must be gone after the timeout.
    let script = format!("sleep 30 &\necho $! > '{p}'\necho $$ >> '{p}'\nsleep 30\necho '{{}}'", p = pids.display());
    let m = installed(tmp.path(), "slow", "library", 1, &script);
    let t = Instant::now();
    let err = m.call("slow", "list", serde_json::json!({}), &enabled(&["slow"])).unwrap_err().to_string();
    let took = t.elapsed();
    assert!(err.contains("timed out"), "{err}");
    assert!(took < Duration::from_secs(5), "took {took:?}");
    std::thread::sleep(Duration::from_millis(100));
    let text = std::fs::read_to_string(&pids).unwrap();
    for pid in text.split_whitespace() {
        let pid: i32 = pid.parse().unwrap();
        // SAFETY: signal 0 only checks whether the process exists.
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        // A zombie would still answer; the stat file tells the state.
        let zombie = crate::procfs::read_stat(pid).is_none_or(|p| p.zombie);
        assert!(!alive || zombie, "process {pid} survived the timeout");
    }
    assert!(m.last_error("slow").unwrap().contains("timed out"));
}

#[test]
fn big_output_is_capped() {
    let tmp = tempfile::tempdir().unwrap();
    // 9 MB of output, more than the 8 MB cap.
    let m = installed(tmp.path(), "big", "library", 10, "head -c 9000000 /dev/zero | tr '\\0' 'a'");
    let t = Instant::now();
    let err = m.call("big", "list", serde_json::json!({}), &enabled(&["big"])).unwrap_err().to_string();
    assert!(err.contains("larger than 8 MB"), "{err}");
    assert!(t.elapsed() < Duration::from_secs(5));
    // Just under the cap is fine (a JSON string of ~7 MB).
    let m = installed(
        &tmp.path().join("2"),
        "big2",
        "library",
        10,
        "printf '{\"x\":\"'; head -c 7000000 /dev/zero | tr '\\0' 'a'; printf '\"}'",
    );
    let r = m.call("big2", "list", serde_json::json!({}), &enabled(&["big2"])).unwrap();
    assert_eq!(r["x"].as_str().unwrap().len(), 7_000_000);
}

#[test]
fn bad_replies_are_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let cases = [
        ("notjson", "echo 'this is not json'", "not valid JSON"),
        ("empty", "true", "no reply"),
        ("array", "echo '[1,2]'", "not a JSON object"),
        ("err", "echo '{\"error\":\"login needed\"}'", "login needed"),
        ("crash", "echo 'boom: missing tool' >&2; exit 3", "boom: missing tool"),
        ("noread", "exec 0<&-; echo '{}'", ""),
    ];
    for (id, script, want) in cases {
        let m = installed(&tmp.path().join(id), id, "library", 5, script);
        let r = m.call(id, "list", serde_json::json!({"big": "x".repeat(200_000)}), &enabled(&[id]));
        if want.is_empty() {
            assert!(r.is_ok(), "{id}: {r:?}");
        } else {
            let e = r.unwrap_err().to_string();
            assert!(e.contains(want), "{id}: {e}");
        }
    }
}

// ----- scanner -----

fn scan_ctx(tmp: &Path, cfg: AddonsConfig) -> crate::scan::ScanContext {
    crate::scan::ScanContext {
        home: tmp.join("home"),
        xdg_data_dirs: vec![],
        config: Default::default(),
        data_dir: tmp.join("data"),
        addons: cfg,
        emulation: crate::config::EmulationConfig::detect_nothing(),
        steam_names: Default::default(),
    }
}

#[test]
fn scanner_maps_list_replies() {
    let tmp = tempfile::tempdir().unwrap();
    let mut m = manager(tmp.path());
    m.install_from(&fixtures().join("fixture-library")).unwrap();
    let dir = m.get("fixture-library").unwrap().dir.clone();
    let games = crate::scan::addons::scan(&scan_ctx(tmp.path(), enabled(&["fixture-library"]))).unwrap();
    let names: Vec<&str> = games.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["Alpha Quest", "Numbered", "Beta Direct", "Relative Program"]);
    assert!(games.iter().all(|g| g.source == Source::Addon));

    let a = &games[0];
    assert_eq!(a.source_id, "fixture-library:alpha");
    assert_eq!(
        a.launch,
        Some(LaunchSpec::Command {
            program: "sh".into(),
            args: vec!["-c".into(), "sleep 1".into()],
            cwd: Some("/tmp".into()),
            env: vec![("FOO".into(), "bar".into())],
        })
    );
    assert_eq!(a.art.cover.as_deref(), Some(dir.join("art/cover.png").as_path()));
    assert!(a.art.hero.is_none(), "URLs aren't downloaded by a scan");
    assert!(a.art.logo.is_none(), "missing files are dropped");
    assert_eq!(a.playtime_secs, Some(3600));
    assert_eq!(a.last_played, Some(1767225600));
    assert_eq!(a.size_bytes, Some(1000));
    assert_eq!(a.meta.genres, ["RPG", "Action"]);
    assert_eq!(a.meta.developer.as_deref(), Some("Dev A"));
    assert_eq!(a.meta.release_year, Some(2024));
    assert_eq!(a.platform.as_deref(), Some("linux"));

    let n = &games[1];
    assert_eq!(n.source_id, "fixture-library:42");
    assert!(!n.installed);
    assert_eq!(n.launch, Some(LaunchSpec::Url("fixture://launch/42".into())));

    // No launch: the add-on runs as the game, secrets left out of the stored env.
    let b = &games[2];
    assert!(b.installed);
    let Some(LaunchSpec::Command { program, args, cwd, env }) = &b.launch else { panic!("{:?}", b.launch) };
    assert!(program.ends_with("bin/run.sh"));
    assert_eq!(args, &["launch", "beta"]);
    assert_eq!(cwd.as_deref(), Some(dir.as_path()));
    assert!(env.contains(&("LODESTAR_ADDON_LAUNCH".into(), "1".into())));
    let settings = &env.iter().find(|(k, _)| k == "LODESTAR_ADDON_SETTINGS").unwrap().1;
    assert!(!settings.contains("default-token"), "{settings}");
    assert!(settings.contains("region"));

    let r = &games[3];
    let Some(LaunchSpec::Command { program, .. }) = &r.launch else { panic!() };
    assert_eq!(program, &dir.join("./bin/game.sh").to_string_lossy());
    assert!(r.art.cover.is_none());
    assert_eq!(r.meta.release_year, None, "absurd years are dropped");
}

#[test]
fn scanner_tolerates_one_failing_addon_but_not_all() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let mut m = manager(tmp.path());
    for (id, script) in [
        ("lib-ok", r#"echo '{"games":[{"id":"1","name":"Fine Game"}]}'"#),
        ("lib-bad", "echo nope"),
        ("lib-slow", "sleep 30"),
    ] {
        m.install_from(&make_addon(&src, id, "library", 1, script)).unwrap();
    }
    m.install_from(&make_addon(&src, "meta-only", "metadata", 1, "echo '{\"games\":[{\"id\":\"x\",\"name\":\"X\"}]}'"))
        .unwrap();

    // Nothing enabled → nothing run, no error.
    assert!(crate::scan::addons::scan(&scan_ctx(tmp.path(), AddonsConfig::default())).unwrap().is_empty());

    let t = Instant::now();
    let games =
        crate::scan::addons::scan(&scan_ctx(tmp.path(), enabled(&["lib-ok", "lib-bad", "lib-slow", "meta-only"])))
            .unwrap();
    assert!(t.elapsed() < Duration::from_secs(4), "add-ons run in parallel");
    assert_eq!(games.len(), 1, "metadata add-ons aren't asked for games");
    assert_eq!(games[0].source_id, "lib-ok:1");
    assert!(m.last_error("lib-bad").unwrap().contains("JSON"));
    assert!(m.last_error("lib-slow").unwrap().contains("timed out"));
    assert!(m.last_error("lib-ok").is_none());
    m.refresh();
    assert!(m.get("lib-bad").unwrap().last_error.is_some(), "refresh picks up the scan's errors");

    let err = crate::scan::addons::scan(&scan_ctx(tmp.path(), enabled(&["lib-bad", "lib-slow"]))).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("lib-bad name") && msg.contains("lib-slow name"), "{msg}");
}

#[test]
fn example_addons_work() {
    let tmp = tempfile::tempdir().unwrap();
    let mut m = manager(tmp.path());
    assert_eq!(m.install_from(&examples().join("example-library")).unwrap(), "example-library");
    assert_eq!(m.install_from(&examples().join("example-metadata")).unwrap(), "example-metadata");
    let mut cfg = enabled(&["example-library", "example-metadata"]);
    cfg.settings.entry("example-library".into()).or_default().insert("prefix".into(), "Demo".into());

    let games = crate::scan::addons::scan(&scan_ctx(tmp.path(), cfg.clone())).unwrap();
    assert_eq!(games.len(), 2);
    assert_eq!(games[0].name, "Demo Tiny Game");
    assert_eq!(
        games[0].launch,
        Some(LaunchSpec::Command {
            program: "sh".into(),
            args: vec!["-c".into(), "sleep 2".into()],
            cwd: None,
            env: vec![]
        })
    );
    assert!(games[0].art.cover.is_some());
    // The second game is started through the add-on itself.
    let Some(LaunchSpec::Command { args, .. }) = &games[1].launch else { panic!() };
    assert_eq!(args, &["launch", "tiny-two"]);
    cfg.settings.get_mut("example-library").unwrap().insert("show_second".into(), "false".into());
    assert_eq!(crate::scan::addons::scan(&scan_ctx(tmp.path(), cfg.clone())).unwrap().len(), 1);

    // Protocol launch: the add-on says what to run (nothing is started here).
    let spec = m.resolve_launch("example-library", "tiny-one", &cfg).unwrap();
    assert!(matches!(spec, Some(LaunchSpec::Command { ref program, .. }) if program == "sh"));

    // Metadata add-on.
    let found = metadata::lookup(&m, &cfg, "Celeste", None).unwrap();
    assert_eq!(found.addon, "example-metadata");
    assert_eq!(found.meta.developer.as_deref(), Some("Example Studio"));
    assert_eq!(found.meta.genres, ["Adventure", "Demo"]);
    assert_eq!(found.meta.release_year, Some(2026));
    // Disabled → not asked.
    assert!(metadata::lookup(&m, &enabled(&["example-library"]), "Celeste", None).is_none());
}

#[test]
fn best_match_prefers_name_and_year() {
    let r = serde_json::json!([
        {"id": "a", "name": "Doom", "year": 1993},
        {"id": 7, "name": "DOOM", "year": 2016},
        {"id": "c", "name": "Doom Eternal"}
    ]);
    let r = r.as_array().unwrap();
    assert_eq!(metadata::best_match(r, "doom", Some(2016)).as_deref(), Some("7"));
    assert_eq!(metadata::best_match(r, "Doom", None).as_deref(), Some("a"));
    assert_eq!(metadata::best_match(r, "Quake", None).as_deref(), Some("a"));
    assert_eq!(metadata::best_match(&[], "Quake", None), None);
}

#[test]
fn split_source_ids() {
    assert_eq!(split_source_id("my-addon:game:1"), Some(("my-addon", "game:1")));
    assert_eq!(split_source_id("Bad:1"), None);
    assert_eq!(split_source_id("a:"), None);
    assert_eq!(split_source_id("plain"), None);
}
