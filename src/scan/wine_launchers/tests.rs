//! Tests for the Wine-prefix store launchers. Fixture: tests/fixtures/wine/home.

use super::battlenet::{Install, Wire, decode_product_db, fields, product_info};
use super::ea::*;
use super::stores::*;
use super::*;
use crate::scan::winprefix::Runner;
use std::path::Path;

fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    crate::scan::winprefix::tests::fixture_home()
}

fn ids(v: &[Found]) -> Vec<(Source, String, String)> {
    v.iter().map(|f| (f.source, f.id.clone(), f.name.clone())).collect()
}

fn s(src: Source, id: &str, name: &str) -> (Source, String, String) {
    (src, id.into(), name.into())
}

// ---------------------------------------------------------------------------------------------
// Protobuf

fn varint(mut n: u64) -> Vec<u8> {
    let mut out = vec![];
    loop {
        let b = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            return out;
        }
        out.push(b | 0x80);
    }
}

fn ln(field: u32, b: &[u8]) -> Vec<u8> {
    let mut v = varint((field as u64) << 3 | 2);
    v.extend(varint(b.len() as u64));
    v.extend_from_slice(b);
    v
}

#[test]
fn protobuf_wire_format() {
    let mut msg = varint(1 << 3); // field 1 varint
    msg.extend(varint(300));
    msg.extend(varint(2 << 3 | 1)); // fixed64
    msg.extend(7u64.to_le_bytes());
    msg.extend(varint(3 << 3 | 5)); // fixed32
    msg.extend(9u32.to_le_bytes());
    msg.extend(ln(4, b"hi"));
    let f = fields(&msg).unwrap();
    assert_eq!(f, vec![(1, Wire::Varint(300)), (2, Wire::Fixed64(7)), (3, Wire::Fixed32(9)), (4, Wire::Bytes(b"hi"))]);
    // Truncated length, truncated varint, groups, field 0: rejected, never a panic.
    assert!(fields(&[0x0a, 0x05, b'a']).is_none());
    assert!(fields(&[0x08, 0xff]).is_none());
    assert!(fields(&[0x0b]).is_none());
    assert!(fields(&[0x00, 0x00]).is_none());
    assert!(fields(&[0x0a, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]).is_none());
    assert_eq!(fields(&[]), Some(vec![]));
}

#[test]
fn decodes_product_db() {
    let (_t, home) = fixture();
    let bytes = std::fs::read(home.join("Faugus/battlenet/drive_c/ProgramData/Battle.net/Agent/product.db")).unwrap();
    let v = decode_product_db(&bytes).unwrap();
    let codes: Vec<(&str, &str, Option<bool>)> =
        v.iter().map(|i| (i.uid.as_str(), i.product_code.as_str(), i.installed)).collect();
    assert_eq!(
        codes,
        vec![
            ("agent", "agent", Some(true)),
            ("battle.net", "bna", Some(true)),
            ("prometheus", "pro", Some(true)),
            ("fenris", "Fen", Some(true)),
            ("hs_beta", "WTCG", Some(true)),
            ("s2", "S2", Some(false)),
            ("zzz", "ZZZ", Some(true)),
        ]
    );
    assert_eq!(v[2].install_path, "C:/Program Files (x86)/Overwatch");
    assert_eq!(product_info(&v[2]), ("Pro".into(), Some("Overwatch 2")));
    assert_eq!(product_info(&v[4]), ("WTCG".into(), Some("Hearthstone")));
    assert_eq!(product_info(&v[6]), ("ZZZ".into(), None));
    // A minimal install with only uid + code + settings.
    let mut inst = ln(1, b"wow");
    inst.extend(ln(2, b"wow"));
    inst.extend(ln(3, &ln(1, b"C:/WoW")));
    let one = decode_product_db(&ln(1, &inst)).unwrap();
    assert_eq!(
        one,
        vec![Install { uid: "wow".into(), product_code: "wow".into(), install_path: "C:/WoW".into(), installed: None }]
    );
    assert_eq!(product_info(&one[0]).0, "WoW");
    // Not protobuf at all → error; a broken install inside a good message → skipped.
    assert!(decode_product_db(b"\xff\xff\xff").is_err());
    assert_eq!(decode_product_db(&ln(1, &[0x0a, 0x09])).unwrap(), vec![]);
}

// ---------------------------------------------------------------------------------------------
// Per launcher

#[test]
fn battlenet_games() {
    let (_t, home) = fixture();
    let pfx = home.join("Faugus/battlenet");
    let v = battlenet::scan(&pfx).unwrap();
    assert_eq!(
        ids(&v),
        vec![
            s(Source::BattleNet, "Pro", "Overwatch 2"),
            s(Source::BattleNet, "Fen", "Diablo IV"),
            s(Source::BattleNet, "ZZZ", "Mystery Game"),
        ]
    );
    assert_eq!(v[0].exe, pfx.join("drive_c/Program Files (x86)/Battle.net/Battle.net.exe"));
    assert_eq!(v[0].args, vec!["--exec=launch Pro".to_string()]);
    assert_eq!(v[1].install_dir, pfx.join("drive_c/Program Files (x86)/Diablo IV"));
    // No client → nothing; no product.db → nothing; garbage product.db → error.
    std::fs::remove_file(pfx.join("drive_c/Program Files (x86)/Battle.net/Battle.net.exe")).unwrap();
    assert!(battlenet::scan(&pfx).unwrap().is_empty());
    assert!(battlenet::scan(&home.join("Games/plain")).unwrap().is_empty());
    std::fs::write(pfx.join("drive_c/ProgramData/Battle.net/Agent/product.db"), b"\xff\xff").unwrap();
    assert!(battlenet::scan(&pfx).is_err());
}

#[test]
fn ubisoft_ea_rockstar_games() {
    let (_t, home) = fixture();
    let pfx = home.join("Games/ubisoft");
    let p = PrefixData::new(&pfx);
    let ubi = ubisoft(&p);
    assert_eq!(
        ids(&ubi),
        vec![s(Source::Ubisoft, "5595", "Far Cry 6"), s(Source::Ubisoft, "635", "Assassin's Creed Origins")]
    );
    assert_eq!(ubi[1].exe, pfx.join("drive_c/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/UbisoftConnect.exe"));
    assert_eq!(ubi[1].args, vec!["uplay://launch/635/0".to_string()]);
    assert_eq!(
        ubi[1].install_dir,
        pfx.join("drive_c/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/games/Assassin's Creed Origins")
    );

    let e = ea(&p);
    assert_eq!(
        ids(&e),
        vec![
            s(Source::Ea, "Origin.OFR.50.0000557", "Battlefield 1"),
            s(Source::Ea, "196749", "Mass Effect Legendary Edition"),
        ]
    );
    assert_eq!(e[0].exe, pfx.join("drive_c/Program Files/Electronic Arts/EA Desktop/EA Desktop/EADesktop.exe"));
    assert_eq!(e[0].args, vec!["origin2://game/launch?offerIds=Origin.OFR.50.0000557&autoDownload=1".to_string()]);

    let r = rockstar(&p);
    assert_eq!(
        ids(&r),
        vec![s(Source::Rockstar, "gta5", "Grand Theft Auto V"), s(Source::Rockstar, "mp3", "Max Payne 3")]
    );
    assert_eq!(r[0].exe, pfx.join("drive_c/Program Files/Rockstar Games/Launcher/Launcher.exe"));
    assert_eq!(
        r[0].args,
        vec!["-launchTitleInFolder".to_string(), "C:\\Program Files\\Rockstar Games\\Grand Theft Auto V".into()]
    );
    // Prefixes without these launchers don't even read the registry.
    let plain = PrefixData::new(&home.join("Games/plain"));
    assert!(ubisoft(&plain).is_empty() && ea(&plain).is_empty() && rockstar(&plain).is_empty());
    assert!(plain.system.get().is_none());
}

#[test]
fn ea_files() {
    let d = parse_installerdata(
        "<DiPManifest><contentIDs><contentID> 71867 </contentID></contentIDs><gameTitles>\
         <gameTitle locale=\"fr_FR\">Jeu®</gameTitle></gameTitles></DiPManifest>",
    );
    assert_eq!(d, InstallerData { content_ids: vec!["71867".into()], title: Some("Jeu".into()), launcher: None });
    assert_eq!(parse_installerdata("not xml"), InstallerData::default());
    assert_eq!(
        parse_mfst("?id=OFB-EAST%3a109552677&dipinstallpath=C%3a%5cGames%5cX%5c&x=%zz%"),
        Some(("OFB-EAST:109552677".into(), Some("C:\\Games\\X\\".into())))
    );
    assert_eq!(parse_mfst("?currentstate=kReadyToStart"), None);
    assert_eq!(clean_title("Battlefield™  2042"), "Battlefield 2042");
}

#[test]
fn riot_and_hoyoplay_games() {
    let (_t, home) = fixture();
    let pfx = home.join(".local/share/bottles/bottles/Riot");
    let r = riot_hoyo::riot(&PrefixData::new(&pfx));
    assert_eq!(
        ids(&r),
        vec![
            s(Source::Riot, "league_of_legends", "League of Legends"),
            s(Source::Riot, "valorant", "VALORANT"),
            s(Source::Riot, "bacon", "Legends of Runeterra"),
        ]
    );
    assert_eq!(r[0].exe, pfx.join("drive_c/Riot Games/Riot Client/RiotClientServices.exe"));
    assert_eq!(r[1].args, vec!["--launch-product=valorant".to_string(), "--launch-patchline=live".into()]);
    assert_eq!(r[1].install_dir, pfx.join("drive_c/Riot Games/VALORANT/live"));

    let pfx = home.join("Games/Heroic/Prefixes/default/HoYo/pfx");
    let h = riot_hoyo::hoyoplay(&PrefixData::new(&pfx));
    assert_eq!(
        ids(&h),
        vec![
            s(Source::HoYoPlay, "hk4e_global", "Genshin Impact"),
            s(Source::HoYoPlay, "nap_global", "Zenless Zone Zero"),
            s(Source::HoYoPlay, "hkrpg_global", "Honkai: Star Rail"),
        ]
    );
    let hy = pfx.join("drive_c/Program Files/HoYoPlay");
    assert_eq!(h[0].exe, hy.join("games/Genshin Impact game/GenshinImpact.exe"));
    assert_eq!(h[1].exe, hy.join("launcher.exe")); // game exe missing → the launcher
    assert!(h.iter().all(|f| f.args.is_empty()));
}

// ---------------------------------------------------------------------------------------------
// Whole scan

#[test]
fn scan_all_prefixes() {
    let (_t, home) = fixture();
    let ctx = crate::scan::winprefix::tests::ctx(&home);
    let games = scan(&ctx).unwrap();
    let got: Vec<(Source, &str)> = games.iter().map(|g| (g.source, g.source_id.as_str())).collect();
    let steam_pfx = home.join(".local/share/Steam/steamapps/compatdata/3000000001/pfx");
    let dup = format!("Pro@{}", winprefix::prefix_hash(&steam_pfx));
    assert_eq!(
        got,
        vec![
            (Source::BattleNet, "Pro"),
            (Source::BattleNet, "Fen"),
            (Source::BattleNet, "ZZZ"),
            (Source::Ea, "Origin.OFR.50.0000557"),
            (Source::Ea, "196749"),
            (Source::Ubisoft, "5595"),
            (Source::Ubisoft, "635"),
            (Source::Rockstar, "gta5"),
            (Source::Rockstar, "mp3"),
            (Source::Riot, "league_of_legends"),
            (Source::Riot, "valorant"),
            (Source::Riot, "bacon"),
            (Source::HoYoPlay, "hk4e_global"),
            (Source::HoYoPlay, "nap_global"),
            (Source::HoYoPlay, "hkrpg_global"),
            (Source::BattleNet, dup.as_str()),
        ]
    );
    for g in &games {
        assert!(g.installed && g.platform.as_deref() == Some("windows"), "{}", g.name);
        assert!(g.install_dir.as_ref().is_some_and(|d| d.is_dir()), "{}", g.name);
        // Every launcher game goes through Lodestar's store-client helper (client_launch.rs):
        // a request handed over through a new umu-run never reaches a running client.
        let Some(LaunchSpec::Command { program, args, env, .. }) = &g.launch else { panic!("{}", g.name) };
        assert_eq!(program, crate::client_launch::SELF_PROGRAM, "{}", g.name);
        assert!(env.is_empty(), "{}: {env:?}", g.name);
        let a = helper(args);
        assert!(a.prefix.join("drive_c").is_dir(), "{}: {:?}", g.name, a.prefix);
        assert!(a.exe.is_file(), "{}: {:?}", g.name, a.exe);
        match g.source {
            Source::BattleNet => {
                assert_eq!(a.kind, crate::client_launch::Kind::BattleNet);
                // A copy in a second prefix has `@<prefix hash>` in its id; the code stays plain.
                assert_eq!(a.code, g.source_id.split('@').next().unwrap_or_default(), "{args:?}");
            }
            Source::Ea if a.kind == crate::client_launch::Kind::Ea => assert!(!a.ids.is_empty()),
            src => {
                assert_eq!(a.kind, crate::client_launch::Kind::Generic, "{}", g.name);
                assert_eq!(Some(a.client.as_str()), crate::client_launch::client_name(src), "{}", g.name);
            }
        }
    }
    // Wine runner from Lutris for the Ubisoft prefix; the link goes to Ubisoft Connect.
    let ac = games.iter().find(|g| g.source_id == "635").unwrap();
    let a = launch_helper(ac);
    assert_eq!(
        a.runner,
        crate::scan::winprefix::Runner::Wine {
            path: home.join(".local/share/lutris/runners/wine/wine-ge-8-26-x86_64/bin/wine")
        }
    );
    assert_eq!(a.args, vec!["uplay://launch/635/0".to_string()]);
    assert_eq!(a.prefix, home.join("Games/ubisoft"));
    assert!(!a.exe_is_game);
    assert_eq!(a.exe.file_name().unwrap(), "UbisoftConnect.exe");
    // Proton (Heroic, pfx layout); HoYoPlay games start their own exe.
    let gi = games.iter().find(|g| g.source_id == "hk4e_global").unwrap();
    let a = launch_helper(gi);
    assert!(
        matches!(&a.runner, crate::scan::winprefix::Runner::Proton { path } if path.ends_with("Fixture-Proton")),
        "{:?}",
        a.runner
    );
    assert!(a.prefix.starts_with(home.join("Games/Heroic/Prefixes/default/HoYo")), "{:?}", a.prefix);
    assert!(a.exe_is_game);
    assert_eq!(a.client, "hoyoplay");
}

fn helper(args: &[String]) -> crate::client_launch::Args {
    assert_eq!(args[0], "--client-launch", "{args:?}");
    let kind = crate::client_launch::Kind::parse(&args[1]).unwrap();
    crate::client_launch::parse_kind_args(kind, &mut args[2..].iter().cloned()).unwrap()
}

fn launch_helper(g: &ScannedGame) -> crate::client_launch::Args {
    let Some(LaunchSpec::Command { args, .. }) = &g.launch else { panic!("{}", g.name) };
    helper(args)
}

#[test]
fn launcher_for_install_uninstall() {
    let (_t, home) = fixture();
    let ctx = crate::scan::winprefix::tests::ctx(&home);
    let Some(LaunchSpec::Command { args, .. }) = launcher_command(&ctx, Source::Ubisoft, "635") else { panic!() };
    assert_eq!(
        args,
        vec![
            home.join("Games/ubisoft/drive_c/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/UbisoftConnect.exe")
                .to_string_lossy()
                .into_owned()
        ]
    );
    // The copy in the second prefix opens that prefix's client.
    let steam_pfx = home.join(".local/share/Steam/steamapps/compatdata/3000000001/pfx");
    let id = format!("Pro@{}", winprefix::prefix_hash(&steam_pfx));
    let Some(LaunchSpec::Command { args, .. }) = launcher_command(&ctx, Source::BattleNet, &id) else { panic!() };
    assert!(args.iter().any(|a| a.starts_with(&*steam_pfx.to_string_lossy())), "{args:?}");
    assert!(launcher_command(&ctx, Source::Ea, "nope").is_none());
    let ls: Vec<Source> = launchers_in(&home.join("Games/ubisoft")).into_iter().map(|(s, _)| s).collect();
    assert_eq!(ls, vec![Source::Ea, Source::Ubisoft, Source::Rockstar]);
}

#[test]
fn missing_everything_is_empty() {
    let t = tempfile::tempdir().unwrap();
    assert!(scan(&crate::scan::desktop::test_support::ctx(t.path())).unwrap().is_empty());
    let _ = Runner::Unknown;
}

/// The raw-header pre-check never drops a key `wanted_key` keeps (every fixture hive, plus
/// hand-written headers in both cases and with / without Wow6432Node).
#[test]
fn raw_prefilter_keeps_every_wanted_key() {
    use crate::scan::winprefix::registry::Registry;
    let extra = "WINE REGISTRY Version 2\n\n\
        [Software\\\\Wow6432Node\\\\Ubisoft\\\\Launcher] 1\n\"InstallDir\"=\"C:\\\\x\"\n\n\
        [SOFTWARE\\\\EA Games\\\\Battlefield 1] 1\n\"Install Dir\"=\"C:\\\\bf1\"\n\n\
        [Software\\\\Microsoft\\\\Windows\\\\CurrentVersion\\\\Uninstall\\\\Uplay Install 635] 1\n\"InstallLocation\"=\"C:\\\\ac\"\n\n\
        [Software\\\\Cognosphere\\\\HYP\\\\1_0\\\\hk4e_global] 1\n\"GameInstallPath\"=\"C:\\\\g\"\n\n\
        [Software\\\\Classes\\\\.exe] 1\n@=\"exefile\"\n\n\
        [Software\\\\Origin] 1\n\"ClientPath\"=\"C:\\\\o\"\n\n\
        [Software\\\\Maxis\\\\The Sims 4] 1\n\"Install Dir\"=\"C:\\\\s\"\n\n\
        [Software\\\\Maxis\\\\The Sims 4\\\\InstallOptSelect] 1\n@=\"1\"\n\n\
        [Software\\\\Microsoft\\\\Windows\\\\CurrentVersion\\\\Uninstall\\\\{GUID}] 1\n\"InstallLocation\"=\"C:\\\\s\"\n";
    let (_tmp, home) = fixture();
    let mut texts = vec![extra.to_string()];
    fn walk(d: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "reg") {
                out.push(std::fs::read_to_string(&p).unwrap_or_default());
            }
        }
    }
    walk(&home, &mut texts);
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wine/ea_app"), &mut texts);
    assert!(texts.len() > 5);
    for t in &texts {
        let full = Registry::parse_filtered(t, wanted_key);
        let fast = Registry::parse_prefiltered(t, maybe_wanted_raw, wanted_key);
        assert_eq!(format!("{:?}", sorted(&full)), format!("{:?}", sorted(&fast)));
    }
    let r = Registry::parse_prefiltered(extra, maybe_wanted_raw, wanted_key);
    for k in [
        "Software\\Wow6432Node\\Ubisoft\\Launcher",
        "Software\\EA Games\\Battlefield 1",
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Uplay Install 635",
        "Software\\Cognosphere\\HYP\\1_0\\hk4e_global",
        "Software\\Origin",
        "Software\\Maxis\\The Sims 4",
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{GUID}",
    ] {
        assert!(r.key(k).is_some(), "{k}");
    }
    assert!(r.key("Software\\Classes\\.exe").is_none());
    assert!(r.key("Software\\Maxis\\The Sims 4\\InstallOptSelect").is_none());
}

fn sorted(r: &crate::scan::winprefix::registry::Registry) -> Vec<String> {
    let mut v: Vec<String> = r.debug_keys();
    v.sort();
    v
}

/// An EA app prefix laid out like a real one (Lutris `ea-app`, The Sims 4): the game's key is
/// under its publisher (`Maxis`), the offer id only in the client's logs.
#[test]
fn ea_app_real_layout() {
    let pfx = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wine/ea_app");
    let p = PrefixData::new(&pfx);
    let e = ea(&p);
    assert_eq!(
        ids(&e),
        vec![
            s(Source::Ea, "1015875", "The Sims 4"),
            s(Source::Ea, "196749", "Mass Effect Legendary Edition"),
            s(Source::Ea, "1031469", "Unravel"),
        ],
        "{e:#?}"
    );
    let client = pfx.join("drive_c/Program Files/Electronic Arts/EA Desktop/EA Desktop");
    // Offer id from `Logs/<offer>_install_log.txt`, launched through the origin2 handler.
    assert_eq!(e[0].exe, client.join("EALauncher.exe"));
    assert_eq!(e[0].args, vec!["origin2://game/launch?offerIds=1015875&autoDownload=1".to_string()]);
    // The EA app looks a link's ids up as content ids: content id first, the offer id to retry.
    assert_eq!(e[0].launch_ids, vec!["1015875".to_string(), "OFB-EAST:109552408".into()]);
    assert_eq!(e[0].install_dir, pfx.join("drive_c/Program Files/EA Games/The Sims 4"));
    // No offer id anywhere: the game's own (non-trial) launcher from installerdata.xml.
    assert_eq!(e[1].exe, pfx.join("drive_c/Games/EA/MELE/Game/Launcher/MassEffectLauncher.exe"));
    assert!(e[1].args.is_empty());
    // Found through InstallData + the library folder; offer from EADesktop.log.
    assert_eq!(e[2].args, vec!["origin2://game/launch?offerIds=1031469&autoDownload=1".to_string()]);
    assert_eq!(e[2].launch_ids, vec!["1031469".to_string(), "Origin.OFR.50.0001469".into()]);
    assert!(e[1].launch_ids.is_empty());
    assert_eq!(ea_launcher(&p), Some((client.join("EADesktop.exe"), true)));
    assert_eq!(launchers_in(&pfx), vec![(Source::Ea, client.join("EADesktop.exe"))]);
}

#[test]
fn ea_logs_and_installerdata() {
    use super::ea::{client_log_installs, install_log_location};
    let d = parse_installerdata(
        "<DiPManifest><contentIDs><contentID>1</contentID><contentID>2</contentID><contentID>1</contentID></contentIDs>\
         <runtime><launcher><filePath>[HKEY_LOCAL_MACHINE\\SOFTWARE\\X\\Y\\Install Dir]a.exe</filePath>\
         <hidden>1</hidden></launcher><launcher><filePath>[K]b.exe</filePath></launcher></runtime></DiPManifest>",
    );
    assert_eq!(d.content_ids, vec!["1".to_string(), "2".into()]);
    assert_eq!(d.launcher.as_deref(), Some("[K]b.exe"));
    assert_eq!(install_log_location("x\r\n12:00  Install Location: C:\\G\\X\\\r\n"), Some("C:\\G\\X\\".into()));
    assert_eq!(
        client_log_installs(
            "a initiateDownload() called for offerId=[OFB-EAST:1] slug[s] installPath=[C:\\G\\S] x\n\
             b initiateDownload() called for offerId=[] installPath=[C:\\G]\nc other offerId=[9]"
        ),
        vec![("OFB-EAST:1".to_string(), "C:\\G\\S".to_string())]
    );
}

#[test]
fn ea_skips_steam_copies() {
    use super::ea::in_steam_library;
    assert!(in_steam_library(Path::new("/home/u/.local/share/Steam/steamapps/common/Battlefield 2042")));
    assert!(!in_steam_library(Path::new("/home/u/Games/ea-app/drive_c/Program Files/EA Games/The Sims 4")));
}
