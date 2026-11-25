//! Tests for the Steam scanner, against tests/fixtures/steam (copied to a temp home) plus
//! generated binary files (appinfo.vdf v29, shortcuts.vdf).

use super::appinfo::testgen::{self, N, s};
use super::direct::direct_launch;
use super::*;
use crate::config::ScanConfig;
use crate::scan::vdf::write as bw;

/// `tests/fixtures/steam/home`, found relative to this source file (works from the crate root).
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/steam/home")
}

fn copy_tree(src: &Path, dst: &Path, home: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for ent in std::fs::read_dir(src).unwrap().flatten() {
        let p = ent.path();
        let d = dst.join(ent.file_name());
        if ent.file_type().unwrap().is_dir() {
            copy_tree(&p, &d, home);
        } else if matches!(p.extension().and_then(|e| e.to_str()), Some("vdf" | "acf")) {
            let t = std::fs::read_to_string(&p).unwrap_or_default();
            std::fs::write(&d, t.replace("@HOME@", &home.display().to_string())).unwrap();
        } else {
            std::fs::copy(&p, &d).unwrap();
        }
    }
}

fn appinfo_apps() -> Vec<(u32, Vec<N>)> {
    use testgen::common;
    vec![
        (10, vec![common("Game Ten (appinfo name)", "Game", "windows,linux")]),
        (30, vec![common("Thirty", "game", "windows")]),
        (500, vec![common("Some Tool", "Tool", "linux")]),
        (60, vec![common("Wallpaper App", "Application", "windows")]),
        (61, vec![common("Coverless App", "Application", "windows")]),
        (70, vec![common("Seventy", "Game", "windows")]),
        (71, vec![common("Seventy DLC", "DLC", "windows")]),
        (12, vec![N::O("common", vec![s("name", "Unrelated"), s("type", "game")])]),
    ]
}

fn appinfo_file() -> Vec<u8> {
    testgen::build(appinfo_apps(), true)
}

fn shortcuts_file() -> Vec<u8> {
    let mut b = Vec::new();
    bw::obj(&mut b, "shortcuts");
    bw::obj(&mut b, "0");
    bw::int(&mut b, "appid", 3_000_000_000u32 as i32);
    bw::str(&mut b, "AppName", "My Emulator");
    bw::str(&mut b, "Exe", "\"/usr/bin/true\"");
    bw::str(&mut b, "StartDir", "\"/usr/bin/\"");
    bw::str(&mut b, "LaunchOptions", "gamemoderun %command% --fullscreen");
    bw::str(&mut b, "icon", "");
    bw::int(&mut b, "LastPlayTime", 1_700_000_100);
    bw::obj(&mut b, "tags");
    bw::str(&mut b, "0", "favorite");
    bw::end(&mut b);
    bw::end(&mut b);
    bw::obj(&mut b, "1");
    bw::str(&mut b, "appname", "Legacy Game");
    bw::str(&mut b, "exe", "\"C:\\Games\\legacy.exe\"");
    bw::end(&mut b);
    bw::obj(&mut b, "2");
    bw::int(&mut b, "appid", 123);
    bw::str(&mut b, "AppName", "Football Life 2026");
    bw::str(&mut b, "Exe", "\"/usr/bin/faugus-launcher\"");
    bw::str(&mut b, "LaunchOptions", "--game football-life-2026");
    bw::end(&mut b);
    bw::obj(&mut b, "3");
    bw::str(&mut b, "AppName", "");
    bw::end(&mut b);
    bw::end(&mut b);
    bw::end(&mut b);
    b
}

struct Fx {
    _dir: tempfile::TempDir,
    home: PathBuf,
}

impl Fx {
    fn new() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        copy_tree(&fixture(), &home, &home);
        let root = home.join(".local/share/Steam");
        std::fs::write(root.join("appcache/appinfo.vdf"), appinfo_file()).unwrap();
        std::fs::write(root.join("userdata/12345/config/shortcuts.vdf"), shortcuts_file()).unwrap();
        // ~/.steam/steam -> native root (must be deduped)
        std::fs::create_dir_all(home.join(".steam")).unwrap();
        std::os::unix::fs::symlink(&root, home.join(".steam/steam")).unwrap();
        // Steam's desktop-shortcut icon (better than the 32px librarycache one)
        let icons = home.join(".local/share/icons/hicolor/128x128/apps");
        std::fs::create_dir_all(&icons).unwrap();
        std::fs::write(icons.join("steam_icon_30.png"), b"x").unwrap();
        Fx { _dir: dir, home }
    }
    fn root(&self) -> PathBuf {
        self.home.join(".local/share/Steam")
    }
    fn ctx(&self, include_uninstalled: bool) -> ScanContext {
        ScanContext {
            home: self.home.clone(),
            xdg_data_dirs: vec![],
            config: ScanConfig { include_uninstalled, ..ScanConfig::default() },
            data_dir: Default::default(),
            addons: Default::default(),
            emulation: crate::config::EmulationConfig::detect_nothing(),
            steam_names: Default::default(),
        }
    }
}

fn by_id<'a>(games: &'a [ScannedGame], id: &str) -> Option<&'a ScannedGame> {
    games.iter().find(|g| g.source_id == id)
}

#[test]
fn roots_are_deduped() {
    let fx = Fx::new();
    let roots = find_roots(&fx.ctx(false));
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].kind, InstallKind::Native);
    assert_eq!(roots[0].path, fx.root().canonicalize().unwrap());
}

#[test]
fn libraries_skip_missing() {
    let fx = Fx::new();
    let libs = library_folders(&fx.root().canonicalize().unwrap());
    assert_eq!(libs.len(), 2, "{libs:?}");
    assert!(libs[1].ends_with("lib2"));
}

#[test]
fn full_scan() {
    let fx = Fx::new();
    let games = scan(&fx.ctx(false)).unwrap();
    let ids: Vec<&str> = games.iter().map(|g| g.source_id.as_str()).collect();

    // Tools / redistributables / coverless applications are filtered.
    for gone in ["228980", "1493710", "500", "501", "61", "70", "71", "12"] {
        assert!(!ids.contains(&gone), "{gone} should be filtered: {ids:?}");
    }

    let ten = by_id(&games, "10").unwrap();
    assert_eq!(ten.name, "Game Ten"); // manifest name wins
    assert!(ten.installed);
    assert_eq!(ten.launch, Some(LaunchSpec::Steam { appid: 10 }));
    assert_eq!(ten.playtime_secs, Some(125 * 60));
    assert_eq!(ten.last_played, Some(1_700_009_999));
    assert_eq!(ten.source_launch_options.as_deref(), Some("MANGOHUD=1 %command% -novid"));
    assert_eq!(ten.size_bytes, Some(123_456_789));
    assert_eq!(ten.platform.as_deref(), Some("linux"));
    assert_eq!(ten.meta.developer.as_deref(), Some("Dev Studio"));
    assert_eq!(ten.meta.publisher.as_deref(), Some("Pub Co"));
    assert_eq!(ten.meta.release_year, Some(2015));
    assert_eq!(ten.meta.genres, vec!["RPG", "Action"]);
    assert!(ten.install_dir.as_ref().unwrap().ends_with("steamapps/common/Game Ten"));
    let a = &ten.art;
    assert!(a.cover.as_ref().unwrap().ends_with("10/library_600x900.jpg"), "{a:?}");
    assert!(a.hero.as_ref().unwrap().ends_with("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/library_hero.jpg"));
    assert!(a.logo.as_ref().unwrap().ends_with("10/logo.png"));
    assert!(a.banner.as_ref().unwrap().ends_with("10/header.jpg"));
    assert!(a.icon.as_ref().unwrap().ends_with("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.jpg"));

    // Second library, StateFlags 6 (installed + update), Proton-forced, custom grid cover wins.
    let thirty = by_id(&games, "30").unwrap();
    assert!(thirty.installed);
    assert_eq!(thirty.platform.as_deref(), Some("windows"));
    assert_eq!(thirty.playtime_secs, Some(120));
    assert!(thirty.art.cover.as_ref().unwrap().ends_with("grid/30p.png"));
    assert!(thirty.art.hero.as_ref().unwrap().ends_with("30_library_hero.jpg")); // old flat layout
    assert_eq!(thirty.art.logo, None); // empty file ignored
    assert!(thirty.art.icon.as_ref().unwrap().ends_with("128x128/apps/steam_icon_30.png"));
    assert!(thirty.install_dir.as_ref().unwrap().ends_with("lib2/steamapps/common/Thirty"));

    // StateFlags 2: present but not installed.
    let forty = by_id(&games, "40").unwrap();
    assert!(!forty.installed);
    assert_eq!(forty.install_dir, None);

    // Application with library art is kept.
    assert!(by_id(&games, "60").is_some());

    // Shortcuts
    let emu = by_id(&games, &format!("shortcut:{}", 3_000_000_000u32)).unwrap();
    assert_eq!(emu.name, "My Emulator");
    let gid = (3_000_000_000u64 << 32) | 0x0200_0000;
    assert_eq!(emu.launch, Some(LaunchSpec::Url(format!("steam://rungameid/{gid}"))));
    assert_eq!(emu.source_launch_options.as_deref(), Some("gamemoderun %command% --fullscreen"));
    assert_eq!(emu.platform.as_deref(), Some("linux"));
    assert_eq!(emu.last_played, Some(1_700_000_100));
    assert_eq!(emu.install_dir.as_deref(), Some(Path::new("/usr/bin/")));
    assert!(emu.art.hero.as_ref().unwrap().ends_with("3000000000_hero.jpg"));

    let legacy_id = legacy_shortcut_appid("\"C:\\Games\\legacy.exe\"", "Legacy Game");
    assert!(legacy_id & 0x8000_0000 != 0);
    let legacy = by_id(&games, &format!("shortcut:{legacy_id}")).unwrap();
    assert_eq!(legacy.platform.as_deref(), Some("windows"));

    let fl = by_id(&games, "shortcut:123").unwrap();
    assert_eq!(fl.duplicate_of, Some((Source::Faugus, "football-life-2026".into())));

    assert_eq!(games.len(), 7, "{ids:?}");
}

#[test]
fn include_uninstalled_adds_owned_games() {
    let fx = Fx::new();
    let games = scan(&fx.ctx(true)).unwrap();
    let s70 = by_id(&games, "70").unwrap();
    assert!(!s70.installed);
    assert_eq!(s70.name, "Seventy");
    assert_eq!(s70.playtime_secs, Some(36_000));
    assert_eq!(s70.last_played, Some(1_600_000_000));
    assert!(s70.art.cover.is_some());
    assert!(by_id(&games, "71").is_none(), "DLC must not be added");
}

#[test]
fn fallback_filter_without_appinfo() {
    let fx = Fx::new();
    std::fs::write(fx.root().join("appcache/appinfo.vdf"), b"broken").unwrap();
    let games = scan(&fx.ctx(false)).unwrap();
    let ids: Vec<&str> = games.iter().map(|g| g.source_id.as_str()).collect();
    assert!(!ids.contains(&"228980") && !ids.contains(&"1493710") && !ids.contains(&"501"), "{ids:?}");
    assert!(ids.contains(&"10") && ids.contains(&"30"));
    // Without a type, the tool named "Some Tool" can't be told apart from a game: kept.
    assert!(ids.contains(&"500"));
}

#[test]
fn no_steam_is_ok_empty() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ScanContext {
        home: dir.path().to_path_buf(),
        xdg_data_dirs: vec![],
        config: ScanConfig::default(),
        data_dir: Default::default(),
        addons: Default::default(),
        emulation: crate::config::EmulationConfig::detect_nothing(),
        steam_names: Default::default(),
    };
    assert!(scan(&ctx).unwrap().is_empty());
    assert!(direct_launch(&ctx, 10).is_err());
}

#[test]
fn extra_and_flatpak_roots() {
    let fx = Fx::new();
    // Move the native install to a Flatpak location and an extra root.
    let flat = fx.home.join(".var/app/com.valvesoftware.Steam/.local/share");
    std::fs::create_dir_all(&flat).unwrap();
    std::fs::remove_file(fx.home.join(".steam/steam")).unwrap();
    std::fs::rename(fx.root(), flat.join("Steam")).unwrap();
    let roots = find_roots(&fx.ctx(false));
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].kind, InstallKind::Flatpak);

    let extra = fx.home.join("custom/Steam");
    std::fs::create_dir_all(extra.join("steamapps")).unwrap();
    let mut ctx = fx.ctx(false);
    ctx.config.extra_steam_roots = vec![extra, fx.home.join("does-not-exist")];
    let roots = find_roots(&ctx);
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[1].kind, InstallKind::Custom);
}

#[test]
fn active_user_prefers_most_recent_then_timestamp() {
    let fx = Fx::new();
    let u = active_user(&fx.root()).unwrap();
    assert!(u.ends_with("userdata/12345"));
    // Without loginusers.vdf: the user dir with the newest localconfig.vdf.
    std::fs::remove_file(fx.root().join("config/loginusers.vdf")).unwrap();
    let newer = fx.root().join("userdata/999/config/localconfig.vdf");
    let f = std::fs::File::options().write(true).open(&newer).unwrap();
    f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60)).unwrap();
    assert!(active_user(&fx.root()).unwrap().ends_with("userdata/999"));
}

#[test]
fn old_libraryfolders_format() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Steam");
    let lib = dir.path().join("Lib");
    std::fs::create_dir_all(root.join("steamapps")).unwrap();
    std::fs::create_dir_all(lib.join("steamapps")).unwrap();
    std::fs::write(
        root.join("steamapps/libraryfolders.vdf"),
        format!(
            "\"LibraryFolders\"\n{{\n\t\"TimeNextStatsReport\"\t\t\"1\"\n\t\"ContentStatsID\"\t\t\"2\"\n\t\"1\"\t\t\"{}\"\n}}\n",
            lib.display()
        ),
    )
    .unwrap();
    let libs = library_folders(&root);
    assert_eq!(libs.len(), 2);
}

#[test]
fn manifest_parse_errors() {
    assert!(parse_manifest("garbage {{{", Path::new("/")).is_err());
    assert!(parse_manifest("\"AppState\" { \"name\" \"x\" }", Path::new("/")).is_err());
    let m = parse_manifest(
        "\"AppState\" { \"appid\" \"5\" \"name\" \" Spaced \" \"StateFlags\" \"1028\" }",
        Path::new("/lib"),
    )
    .unwrap();
    assert!(m.installed());
    assert_eq!(m.name, "Spaced");
    assert_eq!(m.install_path(), None);
}

#[test]
fn crc32_known_value() {
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
}

#[test]
fn shortcut_garbage_is_err() {
    assert!(parse_shortcuts(&[0x00, b's']).is_err());
    assert!(parse_shortcuts(&[]).unwrap().is_empty());
}

#[test]
fn direct_launch_native_and_proton() {
    let fx = Fx::new();
    let root = fx.root();
    // appinfo with launch entries for 10 (native linux) and 30 (windows via Proton-Test)
    let launch = |exe: &str, args: &str, os: &str| {
        N::O(
            "config",
            vec![N::O(
                "launch",
                vec![N::O(
                    "0",
                    vec![
                        s("executable", exe),
                        s("arguments", args),
                        s("type", "default"),
                        N::O("config", vec![s("oslist", os)]),
                    ],
                )],
            )],
        )
    };
    let info = testgen::build(
        vec![
            (10, vec![testgen::common("Game Ten", "game", "linux"), launch("bin/game.sh", "-a \"b c\"", "linux")]),
            (30, vec![testgen::common("Thirty", "game", "windows"), launch("Bin\\Thirty.exe", "", "windows")]),
        ],
        true,
    );
    std::fs::write(root.join("appcache/appinfo.vdf"), info).unwrap();
    let tool = root.join("compatibilitytools.d/ptest");
    std::fs::create_dir_all(&tool).unwrap();
    std::fs::write(tool.join("proton"), "#!/bin/sh\n").unwrap();
    std::fs::write(
        tool.join("compatibilitytool.vdf"),
        "\"compatibilitytools\" { \"compat_tools\" { \"Proton-Test\" // name\n { \"install_path\" \".\" \"display_name\" \"Proton Test\" } } }",
    )
    .unwrap();
    let ctx = fx.ctx(false);

    let d = direct_launch(&ctx, 10).unwrap();
    assert!(d.program.ends_with("steamapps/common/Game Ten/bin/game.sh"), "{d:?}");
    assert_eq!(d.args, vec!["-a", "b c"]);
    assert!(d.cwd.ends_with("Game Ten/bin"));
    assert_eq!(d.compat_tool, None);
    assert!(d.command_line().starts_with("env SteamAppId=10 SteamGameId=10 "), "{}", d.command_line());

    let d = direct_launch(&ctx, 30).unwrap();
    assert!(d.program.ends_with("compatibilitytools.d/ptest/proton"), "{d:?}");
    assert_eq!(d.args[0], "waitforexitandrun");
    assert!(d.args[1].ends_with("lib2/steamapps/common/Thirty/Bin/Thirty.exe"));
    assert!(d.env.iter().any(|(k, v)| k == "STEAM_COMPAT_DATA_PATH" && v.ends_with("lib2/steamapps/compatdata/30")));
    assert_eq!(d.compat_tool.as_deref(), Some("Proton-Test"));

    assert!(direct_launch(&ctx, 40).is_err(), "not installed");

    // Shortcut: native exe, no Steam launch options in the command.
    let d = direct_launch_for_source_id(&ctx, &format!("shortcut:{}", 3_000_000_000u32)).unwrap();
    assert_eq!(d.program, "/usr/bin/true");
    assert_eq!(d.args, vec!["--fullscreen"]); // wrappers before %command% dropped
    assert_eq!(d.cwd, Path::new("/usr/bin/"));
    assert!(direct_launch_for_source_id(&ctx, "shortcut:1").is_err());
    assert!(direct_launch_for_source_id(&ctx, "nope").is_err());
}

#[test]
fn non_utf8_localconfig_keeps_playtime() {
    let fx = Fx::new();
    let lc = fx.root().join("userdata/12345/config/localconfig.vdf");
    let mut bytes = std::fs::read(&lc).unwrap();
    // A stray Latin-1 byte somewhere unrelated (e.g. a friend's nickname).
    let pos = bytes.windows(4).position(|w| w == b"\"10\"").unwrap();
    bytes.splice(pos..pos, b"\"nick\" \"J\xf6rg\"\n".iter().copied());
    std::fs::write(&lc, bytes).unwrap();
    let games = scan(&fx.ctx(false)).unwrap();
    let ten = by_id(&games, "10").unwrap();
    assert_eq!(ten.playtime_secs, Some(125 * 60));
    assert_eq!(ten.source_launch_options.as_deref(), Some("MANGOHUD=1 %command% -novid"));
}

#[test]
fn empty_oslist_means_windows() {
    // Old Windows-only games (Bully, Fallout: New Vegas) have no common/oslist at all.
    let info = appinfo::AppInfo { kind: Some("game".into()), ..Default::default() };
    assert_eq!(platform_for(1, Some(&info), &HashMap::new()).as_deref(), Some("windows"));
    assert_eq!(platform_for(1, None, &HashMap::new()), None);
}

/// A Steam library on a drive that is not mounted: its games become "not installed" (as Steam
/// shows them) but keep everything else, and come back unchanged once the drive is back.
#[test]
fn unmounted_library_keeps_games_and_user_data() {
    let fx = Fx::new();
    let ctx = fx.ctx(false);
    let mut db = crate::db::Db::open_memory().unwrap();
    db.merge_scan(&scan(&ctx).unwrap(), &[Source::Steam]).unwrap();
    let thirty = db.load_all().unwrap().into_iter().find(|g| g.source_id == "30").unwrap();
    assert!(thirty.installed);
    db.set_favorite(thirty.id, true).unwrap();
    db.set_tags(thirty.id, &["couch".into()]).unwrap();

    let lib2 = fx.home.join("lib2");
    let away = fx.home.join("lib2-unmounted");
    std::fs::rename(&lib2, &away).unwrap();
    let games = scan(&ctx).unwrap();
    assert!(by_id(&games, "30").is_none());
    assert!(by_id(&games, "10").is_some());
    db.merge_scan(&games, &[Source::Steam]).unwrap();
    let g = db.get(thirty.id).unwrap().unwrap();
    assert!(!g.installed);
    assert!(g.favorite && g.tags == ["couch"]);
    assert_eq!(
        (g.install_dir.clone(), g.art.clone(), g.playtime_secs),
        (thirty.install_dir.clone(), thirty.art.clone(), thirty.playtime_secs)
    );

    std::fs::rename(&away, &lib2).unwrap();
    db.merge_scan(&scan(&ctx).unwrap(), &[Source::Steam]).unwrap();
    let g = db.get(thirty.id).unwrap().unwrap();
    assert!(g.installed && g.favorite);
    assert_eq!(g.added, thirty.added);
}

#[test]
fn the_account_that_owns_the_installed_games_is_used_not_just_the_most_recent_one() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Steam");
    for account in ["111", "222"] {
        std::fs::create_dir_all(root.join("userdata").join(account).join("config")).unwrap();
    }
    let base = 76_561_197_960_265_728u64;
    let m = |appid: u32, owner: u64| Manifest {
        appid,
        name: format!("Game {appid}"),
        installdir: String::new(),
        state_flags: 4,
        size_on_disk: None,
        last_played: None,
        last_owner: Some(base + owner),
        library: root.clone(),
    };
    // Two of three installed games belong to account 222 (say, the main one; 111 logged in last).
    let manifests = [m(1, 222), m(2, 222), m(3, 111)];
    assert!(library_owner(&root, &manifests).unwrap().ends_with("userdata/222"));
    // An owner without a userdata folder on this PC isn't chosen.
    let elsewhere = [m(1, 333), m(2, 333), m(3, 111)];
    assert!(library_owner(&root, &elsewhere).unwrap().ends_with("userdata/111"));
    // No owners recorded: nothing to go on (the caller falls back to the most recent user).
    let unknown: Vec<Manifest> = manifests
        .iter()
        .cloned()
        .map(|mut x| {
            x.last_owner = None;
            x
        })
        .collect();
    assert_eq!(library_owner(&root, &unknown), None);
}

#[test]
fn a_manifest_reads_its_owner() {
    let text = r#""AppState" { "appid" "10" "name" "X" "StateFlags" "4" "LastOwner" "76561197960266727" }"#;
    let m = parse_manifest(text, Path::new("/lib")).unwrap();
    assert_eq!(m.last_owner, Some(76_561_197_960_266_727));
}

// ---------------------------------------------------------------------------------------------
// Newer Steam clients: the sharded app cache, and apps no app cache describes
// ---------------------------------------------------------------------------------------------

/// The app cache as shard files (Steam beta, November 2026): the same games and details.
#[test]
fn sharded_app_cache_gives_the_same_library() {
    let fx = Fx::new();
    let before = scan(&fx.ctx(true)).unwrap();
    std::fs::remove_file(fx.root().join("appcache/appinfo.vdf")).unwrap();
    testgen::write_shards(&fx.root().join("appcache/appinfo"), appinfo_apps());
    let after = scan(&fx.ctx(true)).unwrap();
    assert_eq!(after, before);
    let ten = by_id(&after, "10").unwrap();
    assert_eq!(ten.meta.developer.as_deref(), Some("Dev Studio"));
    assert_eq!(ten.meta.genres, vec!["RPG", "Action"]);
    assert_eq!(by_id(&after, "70").unwrap().name, "Seventy");
    assert!(by_id(&after, "71").is_none());
}

const MORE_APPS: &str = r#""UserLocalConfigStore"
{
	"Software" { "Valve" { "Steam" { "apps" {
		"10" { "Playtime" "125" }
		"30" { "Playtime" "2" }
		"70" { "LastPlayed" "1600000000" "Playtime" "600" }
		"71" { "Playtime" "1" }
		"80" { "Playtime" "5" }
		"81" { "Playtime" "0" }
		"82" { "Playtime" "0" }
		"83" { "Playtime" "0" }
		"84" { "Playtime" "0" }
		"85" { "Playtime" "0" }
		"1493710" { "Playtime" "0" }
	} } } }
}
"#;

/// A home whose Steam has no app cache Lodestar can read, with the other name sources.
fn no_appinfo_fixture() -> (Fx, tempfile::TempDir) {
    use super::names::*;
    let fx = Fx::new();
    std::fs::remove_file(fx.root().join("appcache/appinfo.vdf")).unwrap();
    std::fs::write(fx.root().join("userdata/12345/config/localconfig.vdf"), MORE_APPS).unwrap();
    // Steam's desktop shortcuts.
    let apps = fx.home.join(".local/share/applications");
    std::fs::create_dir_all(&apps).unwrap();
    let entry = |file: &str, name: &str, id: u32| {
        std::fs::write(
            apps.join(file),
            format!("[Desktop Entry]\nName={name}\nExec=steam steam://rungameid/{id}\nType=Application\n"),
        )
        .unwrap();
    };
    entry("Seventy.desktop", "Seventy", 70);
    entry("OST.desktop", "Eighty-Three Original Soundtrack", 83);
    // The background job's caches.
    let cache = tempfile::tempdir().unwrap();
    let now = now();
    let store = |name: Option<&str>, kind: Option<&str>| StoreName {
        name: name.map(str::to_string),
        kind: kind.map(str::to_string),
        checked: now - 60,
    };
    let mut names = NameCache::default();
    names.apps.insert(71, store(Some("Seventy DLC"), Some("dlc")));
    names.apps.insert(80, store(Some("Eighty"), Some("game")));
    names.apps.insert(81, store(None, None)); // the store doesn't know it (a tool / server)
    names.apps.insert(84, store(None, None));
    names.apps.get_mut(&84).unwrap().checked = now - RECHECK_SECS - 1; // … but asked long ago
    names.apps.insert(85, store(Some("Eighty-Five Demo"), Some("demo")));
    save_names(cache.path(), &names).unwrap();
    let mut owned = OwnedCache::default();
    owned.accounts.insert(
        12345,
        OwnedList {
            fetched: now,
            games: vec![
                OwnedGame { appid: 10, name: "Game Ten".into(), playtime_mins: 1, last_played: None },
                OwnedGame { appid: 70, name: "Seventy (owned)".into(), playtime_mins: 1, last_played: None },
                OwnedGame { appid: 90, name: "Ninety".into(), playtime_mins: 30, last_played: Some(1_650_000_000) },
            ],
        },
    );
    // Another account's list is not this library's.
    owned.accounts.insert(
        999,
        OwnedList { fetched: now, games: vec![OwnedGame { appid: 95, name: "Not Mine".into(), ..Default::default() }] },
    );
    save_owned(cache.path(), &owned).unwrap();
    (fx, cache)
}

fn ctx_with_cache(fx: &Fx, cache: &Path) -> ScanContext {
    ScanContext { steam_names: names::Shared::new(cache.to_path_buf()), ..fx.ctx(true) }
}

#[test]
fn without_app_cache_names_come_from_shortcuts_and_caches() {
    let (fx, cache) = no_appinfo_fixture();
    let ctx = ctx_with_cache(&fx, cache.path());
    let games = scan(&ctx).unwrap();
    let ids: Vec<&str> = games.iter().map(|g| g.source_id.as_str()).collect();

    // Installed games as always (names from their manifests).
    assert!(by_id(&games, "10").unwrap().installed);
    // From Steam's desktop shortcut, with the account's playtime.
    let s70 = by_id(&games, "70").unwrap();
    assert_eq!(s70.name, "Seventy");
    assert!(!s70.installed);
    assert_eq!(s70.playtime_secs, Some(36_000));
    assert_eq!(s70.launch, Some(LaunchSpec::Steam { appid: 70 }));
    // From the store name cache.
    assert_eq!(by_id(&games, "80").unwrap().name, "Eighty");
    // Owned but never started here: from the owned-games list, with Steam's playtime.
    let s90 = by_id(&games, "90").unwrap();
    assert_eq!(s90.name, "Ninety");
    assert!(!s90.installed);
    assert_eq!(s90.playtime_secs, Some(1800));
    assert_eq!(s90.last_played, Some(1_650_000_000));
    // Left out: DLC and demo by store type, a soundtrack by name, apps the store doesn't
    // know, a tool id, apps nobody names yet, another account's games.
    for gone in ["71", "85", "83", "81", "1493710", "82", "84", "95"] {
        assert!(!ids.contains(&gone), "{gone}: {ids:?}");
    }
    // No game twice (installed, the account's list and the owned list overlap).
    let unique: HashSet<&str> = ids.iter().copied().collect();
    assert_eq!(unique.len(), ids.len(), "{ids:?}");
    // What the background job should ask the store about: unnamed, or not known a week ago.
    assert_eq!(ctx.steam_names.needs.lock().names, [82, 84].into());
}

#[test]
fn without_app_cache_or_caches_only_named_games_show() {
    let (fx, _cache) = no_appinfo_fixture();
    let ctx = fx.ctx(true); // no cache folder at all
    let games = scan(&ctx).unwrap();
    let ids: Vec<&str> = games.iter().map(|g| g.source_id.as_str()).collect();
    assert!(ids.contains(&"70"), "named by its desktop shortcut: {ids:?}");
    for gone in ["80", "90", "71", "83"] {
        assert!(!ids.contains(&gone), "{gone}: {ids:?}");
    }
    assert_eq!(ctx.steam_names.needs.lock().names, [71, 80, 81, 82, 84, 85].into());
    // Owned games are only looked at with "include not installed".
    let ctx = ScanContext { config: ScanConfig { include_uninstalled: false, ..ScanConfig::default() }, ..ctx };
    let games = scan(&ctx).unwrap();
    assert!(games.iter().all(|g| g.installed || g.source_id == "40"), "{games:?}");
    assert!(ctx.steam_names.needs.lock().names.is_empty());
}

/// With an app cache, the apps it describes are decided by it exactly as before; the caches
/// only add what it doesn't know.
#[test]
fn app_cache_decides_what_it_knows() {
    let (fx, cache) = no_appinfo_fixture();
    std::fs::write(fx.root().join("appcache/appinfo.vdf"), appinfo_file()).unwrap();
    let mut names = names::load_names(cache.path());
    names.apps.insert(71, names::StoreName { name: Some("Seventy DLC".into()), kind: Some("game".into()), checked: 1 });
    names::save_names(cache.path(), &names).unwrap();
    let games = scan(&ctx_with_cache(&fx, cache.path())).unwrap();
    assert_eq!(by_id(&games, "70").unwrap().name, "Seventy");
    assert_eq!(by_id(&games, "70").unwrap().platform.as_deref(), Some("windows"));
    assert!(by_id(&games, "71").is_none(), "appinfo.vdf says DLC");
    assert_eq!(by_id(&games, "80").unwrap().name, "Eighty");
    assert_eq!(by_id(&games, "90").unwrap().name, "Ninety");
}
