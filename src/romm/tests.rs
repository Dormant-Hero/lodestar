//! RomM client tests against a local mock HTTP server (std TcpListener; nothing leaves the
//! machine) serving recorded-style answers (tests/fixtures/romm/, trimmed from a RomM 5.4
//! server's real replies).

use super::download::{self, Progress, Target};
use super::*;
use crate::config::RommConfig;
use parking_lot::Mutex;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

macro_rules! fixture {
    ($name:literal) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/romm/", $name))
    };
}

/// One request the mock saw.
#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    auth: Option<String>,
    body: String,
}

type Handler = dyn Fn(&Seen) -> (u16, Vec<u8>) + Send + Sync;

struct Mock {
    base: String,
    log: Arc<Mutex<Vec<Seen>>>,
}

impl Mock {
    fn start(handler: impl Fn(&Seen) -> (u16, Vec<u8>) + Send + Sync + 'static) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let log: Arc<Mutex<Vec<Seen>>> = Arc::default();
        let handler: Arc<Handler> = Arc::new(handler);
        let l2 = log.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (log, handler) = (l2.clone(), handler.clone());
                std::thread::spawn(move || {
                    let mut r = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    if r.read_line(&mut line).is_err() {
                        return;
                    }
                    let mut parts = line.split_whitespace();
                    let method = parts.next().unwrap_or("").to_string();
                    let path = parts.next().unwrap_or("").to_string();
                    let (mut auth, mut len) = (None, 0usize);
                    loop {
                        let mut h = String::new();
                        if r.read_line(&mut h).is_err() || h.trim().is_empty() {
                            break;
                        }
                        let (k, v) = h.split_once(':').unwrap_or(("", ""));
                        match k.trim().to_ascii_lowercase().as_str() {
                            "authorization" => auth = Some(v.trim().to_string()),
                            "content-length" => len = v.trim().parse().unwrap_or(0),
                            _ => {}
                        }
                    }
                    let mut body = vec![0u8; len];
                    let _ = r.read_exact(&mut body);
                    let seen = Seen { method, path, auth, body: String::from_utf8_lossy(&body).into_owned() };
                    log.lock().push(seen.clone());
                    let (status, out) = handler(&seen);
                    let mut s = stream;
                    // A body starting with "<!doctype html" is served as a web page (sign-in proxies).
                    let ctype = if out.starts_with(b"<!doctype html") { "text/html" } else { "application/json" };
                    let _ = write!(
                        s,
                        "HTTP/1.1 {status} X\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        out.len()
                    );
                    let _ = s.write_all(&out);
                });
            }
        });
        Mock { base, log }
    }

    fn server(&self, token: &str) -> Server {
        Server::from_config(&RommConfig { url: self.base.clone(), token: Some(token.into()), ..Default::default() })
            .unwrap()
    }

    fn paths(&self) -> Vec<String> {
        self.log.lock().iter().map(|s| s.path.clone()).collect()
    }
}

fn tiny_png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(4, 6, image::Rgba([200, 40, 40, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn query(path: &str, key: &str) -> Option<usize> {
    path.split_once('?')?.1.split('&').find_map(|kv| kv.strip_prefix(&format!("{key}="))?.parse().ok())
}

/// A RomM 5.x server: paged /api/roms honouring limit / offset, token "rmm_good" only.
fn romm_server() -> Mock {
    let roms: Vec<Value> = serde_json::from_str(fixture!("roms.json")).unwrap();
    Mock::start(move |s| {
        if s.path == "/api/heartbeat" {
            return (200, fixture!("heartbeat.json").as_bytes().to_vec());
        }
        // Pairing needs no sign-in (that is what it is for).
        if s.path == "/api/client-tokens/exchange" {
            return if s.body.contains("\"12345678\"") {
                (200, br#"{"id":3,"name":"Lodestar","scopes":["roms.read"],"raw_token":"rmm_paired"}"#.to_vec())
            } else {
                (400, br#"{"detail":"Invalid or expired code"}"#.to_vec())
            };
        }
        if s.auth.as_deref() != Some("Bearer rmm_good") {
            return (401, br#"{"detail":"Invalid token"}"#.to_vec());
        }
        let p = s.path.split('?').next().unwrap();
        match p {
            "/api/platforms" => (200, fixture!("platforms.json").as_bytes().to_vec()),
            "/api/roms" => {
                let limit = query(&s.path, "limit").unwrap_or(50);
                let offset = query(&s.path, "offset").unwrap_or(0);
                let items: Vec<Value> = roms.iter().skip(offset).take(limit).cloned().collect();
                let page = serde_json::json!({"items": items, "total": roms.len(), "limit": limit, "offset": offset,
                    "char_index": {}, "rom_id_index": [], "filter_values": {}});
                (200, page.to_string().into_bytes())
            }
            "/api/roms/201" => (200, fixture!("rom_201.json").as_bytes().to_vec()),
            "/api/roms/159/content/Christmas%20Craze.smc" => {
                (200, b"SNES-ROM-BYTES-0123456789-abcdefghijklmn".to_vec())
            }
            "/api/roms/901/files/content/Final%20Fantasy%20VII%20%28USA%29.m3u" => {
                (200, b"discs/Disc 1.chd\n".to_vec())
            }
            "/api/roms/902/files/content/Disc%201.chd" => (200, b"DISC-ONE..".to_vec()),
            "/api/roms/903/files/content/Disc%202.chd" => (200, b"DISC-TWO..".to_vec()),
            p if p.starts_with("/assets/romm/resources/roms/12/159/cover/big.png") => (200, tiny_png()),
            _ => (404, br#"{"detail":"Not found"}"#.to_vec()),
        }
    })
}

#[test]
fn addresses_are_tidied() {
    assert_eq!(normalize_url("romm.lan:8080/").as_deref(), Some("http://romm.lan:8080"));
    assert_eq!(normalize_url(" https://romm.example.com/api/roms ").as_deref(), Some("https://romm.example.com"));
    assert_eq!(normalize_url("https://host/romm/").as_deref(), Some("https://host/romm"));
    assert_eq!(normalize_url("http://192.168.1.5:3000/platform/12").as_deref(), Some("http://192.168.1.5:3000"));
    assert_eq!(normalize_url("ftp://x"), None);
    assert_eq!(normalize_url(""), None);
    assert_eq!(normalize_url("a b"), None);
}

#[test]
fn sign_in_headers() {
    let tok =
        Server::from_config(&RommConfig { url: "h:1".into(), token: Some(" rmm_abc ".into()), ..Default::default() })
            .unwrap();
    assert_eq!(tok.auth.as_deref(), Some("Bearer rmm_abc"));
    let basic = Server::from_config(&RommConfig {
        url: "h:1".into(),
        username: Some("alice".into()),
        password: Some("s3cret".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(basic.auth.as_deref(), Some("Basic YWxpY2U6czNjcmV0"));
    assert_eq!(base64(b"ab"), "YWI=");
    assert_eq!(base64(b"a"), "YQ==");
    assert!(matches!(Server::from_config(&RommConfig::default()), Err(ApiError::NotSetUp)));
    // Sign-in goes only to the server itself, not to a host whose name merely starts the same.
    assert!(tok.on_server("http://h:1/assets/x.png") && tok.on_server("http://h:1"));
    assert!(!tok.on_server("http://h:10/x") && !tok.on_server("http://h:1.example.com/x"));
    // The token never shows in logs.
    assert!(!format!("{tok:?}").contains("rmm_abc"));
}

#[test]
fn platforms_map_to_lodestar_consoles() {
    let plats: Vec<Value> = serde_json::from_str(fixture!("platforms.json")).unwrap();
    let ids: Vec<Option<&str>> = plats.iter().map(|p| map_platform(p).map(|x| x.id)).collect();
    assert_eq!(
        ids,
        vec![Some("atari_2600"), Some("nintendo_super_nes"), Some("sega_genesis"), Some("sony_playstation"), None]
    );
    // Without IGDB ids: RomM's slugs and names.
    for (slug, want) in [
        ("ngc", "nintendo_gamecube"),
        ("ps2", "sony_playstation2"),
        ("n64", "nintendo_64"),
        ("gba", "nintendo_gameboyadvance"),
    ] {
        let p = serde_json::json!({"slug": slug, "fs_slug": slug, "name": ""});
        assert_eq!(map_platform(&p).map(|x| x.id), Some(want), "{slug}");
    }
    let by_name = serde_json::json!({"slug": "x-unknown", "name": "Nintendo Switch"});
    assert_eq!(map_platform(&by_name).map(|x| x.id), Some("nintendo_switch"));
    assert_eq!(map_platform(&serde_json::json!({"slug": "win", "name": "PC (Microsoft Windows)"})).map(|x| x.id), None);
    // RomM's "win" platform carries IGDB id 6 (PC Windows): still not an emulated console.
    assert_eq!(
        map_platform(&serde_json::json!({"slug": "win", "igdb_id": 6, "name": "PC (Microsoft Windows)"})).map(|x| x.id),
        None
    );
}

#[test]
fn reads_every_page_with_the_token() {
    let m = romm_server();
    let lib = fetch_library(&m.server("rmm_good"), 2, None).unwrap();
    assert_eq!(lib.games.len(), 5);
    let paged: Vec<String> = m.paths().into_iter().filter(|p| p.starts_with("/api/roms?")).collect();
    assert_eq!(paged.len(), 3, "{paged:?}");
    assert!(paged[2].contains("offset=4"), "{paged:?}");
    assert!(m.log.lock().iter().all(|s| s.auth.as_deref() == Some("Bearer rmm_good")));
    let xmas = lib.games.iter().find(|g| g.id == 159).unwrap();
    assert_eq!(xmas.name, "16 BIT XMAS 2011: Christmas Craze");
    assert_eq!(xmas.platform.as_deref(), Some("nintendo_super_nes"));
    assert_eq!(xmas.fs_name, "Christmas Craze.smc");
    assert_eq!(xmas.genres, vec!["Arcade".to_string()]);
    assert_eq!(xmas.year, Some(2011));
    assert_eq!(xmas.developer.as_deref(), Some("Shiru"));
    assert_eq!(xmas.publisher.as_deref(), Some("RetroUSB"));
    assert_eq!(xmas.igdb_id, Some(134472));
    // The server's own cover, not the third-party URL.
    assert!(xmas.cover.as_deref().unwrap().starts_with("/assets/romm/resources/roms/12/159/cover/big.png"));
    let ff7 = lib.games.iter().find(|g| g.id == 201).unwrap();
    assert!(ff7.multi);
    assert_eq!(ff7.platform.as_deref(), Some("sony_playstation"));
    assert_eq!(ff7.year, Some(1997));
    let pico = lib.games.iter().find(|g| g.id == 300).unwrap();
    assert_eq!(pico.platform, None);
    assert_eq!(pico.platform_name, "PICO-8 Fantasy Console");
}

#[test]
fn older_servers_answer_one_list() {
    let m = Mock::start(|s| match s.path.split('?').next().unwrap() {
        "/api/platforms" => (200, fixture!("platforms.json").as_bytes().to_vec()),
        "/api/roms" => (200, fixture!("roms.json").as_bytes().to_vec()),
        _ => (404, vec![]),
    });
    let lib = fetch_library(&m.server("t"), 2, None).unwrap();
    assert_eq!(lib.games.len(), 5);
    assert_eq!(m.paths().iter().filter(|p| p.starts_with("/api/roms")).count(), 1);
}

#[test]
fn a_bad_token_says_so() {
    let m = romm_server();
    let e = fetch_library(&m.server("rmm_wrong"), 50, None).unwrap_err();
    assert_eq!(e, ApiError::BadAuth);
    let msg = e.to_string();
    assert!(msg.contains("didn't accept the token") && msg.contains("Client API Tokens"), "{msg}");
    assert_eq!(test_connection(&m.server("rmm_wrong")).unwrap_err(), ApiError::BadAuth);
    let forbidden =
        Mock::start(|s| if s.path == "/api/heartbeat" { (200, b"{\"SYSTEM\":{}}".to_vec()) } else { (403, vec![]) });
    assert!(test_connection(&forbidden.server("t")).unwrap_err().to_string().contains("missing a permission"));
}

#[test]
fn test_connection_counts_the_library() {
    let m = romm_server();
    let info = test_connection(&m.server("rmm_good")).unwrap();
    assert_eq!(info, ServerInfo { version: Some("5.4.0".into()), platforms: 5, games: 5 });
    assert_eq!(info.summary(), "Connected to RomM 5.4.0: 5 games on 5 consoles");
    // Not RomM / not there.
    let other = Mock::start(|_| (404, b"<html>".to_vec()));
    assert_eq!(test_connection(&other.server("t")).unwrap_err(), ApiError::NotRomm);
    let gone = Server::from_config(&RommConfig { url: "http://127.0.0.1:1".into(), ..Default::default() }).unwrap();
    assert!(matches!(test_connection(&gone), Err(ApiError::Unreachable(_))));
}

#[test]
fn pairing_code_gets_the_token() {
    let m = romm_server();
    let s = Server::from_config(&RommConfig { url: m.base.clone(), ..Default::default() }).unwrap();
    assert_eq!(exchange_code(&s, "1234 5678").unwrap(), "rmm_paired");
    assert!(exchange_code(&s, "999").unwrap_err().to_string().contains("pairing code"));
    let seen = m.log.lock().iter().find(|x| x.path == "/api/client-tokens/exchange").cloned().unwrap();
    assert_eq!(seen.method, "POST");
    assert_eq!(seen.auth, None);
}

#[test]
fn sync_caches_and_scan_lists_not_installed_games() {
    let m = romm_server();
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path();
    let server = m.server("rmm_good");
    let (lib, first) = sync(&server, data, None).unwrap();
    assert!(first.changed);
    assert_eq!(first.games, 5);
    let (_, again) = sync(&server, data, None).unwrap();
    assert!(!again.changed);
    assert_eq!(fetch_covers(&server, data, &lib, None), 1);
    assert_eq!(fetch_covers(&server, data, &lib, None), 0, "already there");

    let ctx = crate::scan::desktop::test_support::ctx(tmp.path());
    let ctx = crate::scan::ScanContext { data_dir: data.to_path_buf(), ..ctx };
    let games = crate::scan::romm::scan(&ctx).unwrap();
    assert_eq!(games.len(), 5);
    assert!(games.iter().all(|g| g.source == crate::model::Source::Romm && !g.installed));
    let xmas = games.iter().find(|g| g.source_id == "159").unwrap();
    assert_eq!(xmas.platform.as_deref(), Some("Nintendo SNES"));
    assert_eq!(xmas.meta.description.as_deref().map(|d| d.starts_with("Gather up")), Some(true));
    assert_eq!(xmas.size_bytes, Some(40));
    let cover = xmas.art.cover.as_ref().unwrap();
    assert!(cover.starts_with(covers_dir(data)) && cover.extension().unwrap() == "png");
    let pico = games.iter().find(|g| g.source_id == "300").unwrap();
    assert_eq!(pico.platform.as_deref(), Some("PICO-8 Fantasy Console"));
    // Cleared: the server's games go.
    clear_cache(data);
    assert!(crate::scan::romm::scan(&ctx).unwrap().is_empty());
}

fn game(lib_json: &str, id: u64) -> RommGame {
    let plats: Vec<Value> = serde_json::from_str(fixture!("platforms.json")).unwrap();
    let pl: Vec<(u64, Option<&'static Platform>, String)> =
        plats.iter().map(|p| (p["id"].as_u64().unwrap(), map_platform(p), String::new())).collect();
    let roms: Vec<Value> = serde_json::from_str(lib_json).unwrap();
    roms.iter().filter_map(|r| map_rom(r, &pl)).find(|g| g.id == id).unwrap()
}

#[test]
fn downloads_one_file() {
    let m = romm_server();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("roms/snes");
    let g = game(fixture!("roms.json"), 159);
    let prog = Progress::default();
    let path = download::download(&m.server("rmm_good"), &g, &dir, &prog).unwrap();
    assert_eq!(path, dir.join("Christmas Craze.smc"));
    assert_eq!(std::fs::read(&path).unwrap(), b"SNES-ROM-BYTES-0123456789-abcdefghijklmn");
    assert_eq!(prog.done.load(std::sync::atomic::Ordering::Relaxed), 40);
    assert_eq!(prog.fraction(), Some(1.0));
    assert!(!dir.join("Christmas Craze.smc.part").exists());
    let dl = m.log.lock().iter().find(|s| s.path.contains("/content/")).cloned().unwrap();
    assert_eq!(dl.auth.as_deref(), Some("Bearer rmm_good"));
    // Never overwritten: a second download keeps the file.
    std::fs::write(&path, b"mine").unwrap();
    download::download(&m.server("rmm_good"), &g, &dir, &Progress::default()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"mine");
}

#[test]
fn downloads_a_multi_file_game_into_its_folder() {
    let m = romm_server();
    let tmp = tempfile::tempdir().unwrap();
    let g = game(fixture!("roms.json"), 201);
    let path = download::download(&m.server("rmm_good"), &g, tmp.path(), &Progress::default()).unwrap();
    let folder = tmp.path().join("Final Fantasy VII (USA)");
    assert_eq!(path, folder);
    assert_eq!(std::fs::read(folder.join("Final Fantasy VII (USA).m3u")).unwrap(), b"discs/Disc 1.chd\n");
    assert_eq!(std::fs::read(folder.join("discs/Disc 1.chd")).unwrap(), b"DISC-ONE..");
    assert_eq!(std::fs::read(folder.join("discs/Disc 2.chd")).unwrap(), b"DISC-TWO..");
    // The ".." entry is never followed.
    let mut names: Vec<String> = walk(tmp.path());
    names.sort();
    assert_eq!(names.len(), 3, "{names:?}");
}

fn walk(dir: &Path) -> Vec<String> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        if e.path().is_dir() {
            out.extend(walk(&e.path()));
        } else {
            out.push(e.path().display().to_string());
        }
    }
    out
}

#[test]
fn older_servers_send_multi_file_games_as_a_zip() {
    let m = Mock::start(|s| match s.path.as_str() {
        "/api/roms/201" => (200, br#"{"id":201,"files":[]}"#.to_vec()),
        "/api/roms/201/content/Final%20Fantasy%20VII%20%28USA%29.zip" => (200, b"PK-zip".to_vec()),
        _ => (404, vec![]),
    });
    let tmp = tempfile::tempdir().unwrap();
    let g = game(fixture!("roms.json"), 201);
    let path = download::download(&m.server("t"), &g, tmp.path(), &Progress::default()).unwrap();
    assert_eq!(path, tmp.path().join("Final Fantasy VII (USA).zip"));
    assert_eq!(std::fs::read(&path).unwrap(), b"PK-zip");
    assert!(!tmp.path().join("Final Fantasy VII (USA)").exists());
}

#[test]
fn cancel_and_errors_leave_nothing_behind() {
    let m = romm_server();
    let tmp = tempfile::tempdir().unwrap();
    let g = game(fixture!("roms.json"), 159);
    let prog = Progress::default();
    prog.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(download::download(&m.server("rmm_good"), &g, tmp.path(), &prog).unwrap_err(), ApiError::Cancelled);
    assert!(walk(tmp.path()).is_empty());
    let e = download::download(&m.server("rmm_wrong"), &g, tmp.path(), &Progress::default()).unwrap_err();
    assert_eq!(e, ApiError::BadAuth);
    assert!(walk(tmp.path()).is_empty());
    // A multi-file game failing half-way: its new folder goes again.
    let half = Mock::start(|s| match s.path.as_str() {
        "/api/roms/201" => (200, fixture!("rom_201.json").as_bytes().to_vec()),
        "/api/roms/901/files/content/Final%20Fantasy%20VII%20%28USA%29.m3u" => (200, b"m3u".to_vec()),
        _ => (500, vec![]),
    });
    let ff7 = game(fixture!("roms.json"), 201);
    assert_eq!(
        download::download(&half.server("t"), &ff7, tmp.path(), &Progress::default()).unwrap_err(),
        ApiError::Http(500)
    );
    assert!(!tmp.path().join("Final Fantasy VII (USA)").exists());
}

fn root(path: &Path, layout: crate::scan::emulation::roots::Layout, user: bool) -> crate::scan::emulation::roots::Root {
    crate::scan::emulation::roots::Root {
        id: "f1".into(),
        path: path.to_path_buf(),
        real: path.to_path_buf(),
        layout,
        folder: user.then(|| crate::config::RomFolder { path: path.to_path_buf(), ..Default::default() }),
        explicit: user,
        how: String::new(),
    }
}

#[test]
fn downloads_go_to_the_consoles_rom_folder() {
    use crate::scan::emulation::roots::Layout;
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let snes = platforms::by_id("nintendo_super_nes").unwrap();
    let psx = platforms::by_id("sony_playstation").unwrap();
    assert_eq!(download::console_folder_name(snes), "snes");
    assert_eq!(download::console_folder_name(psx), "psx");

    // A folder set to SNES.
    let mine = tmp.path().join("SNES games");
    std::fs::create_dir_all(&mine).unwrap();
    let roots = vec![root(&mine, Layout::Platform(snes), true)];
    assert_eq!(download::pick_target(&roots, snes, None, &home), Target::Dir { dir: mine.clone(), add_folder: None });

    // Sub-folders are consoles: its "SNES" folder.
    let all = tmp.path().join("ROMs");
    std::fs::create_dir_all(all.join("SNES")).unwrap();
    std::fs::create_dir_all(all.join("gba")).unwrap();
    let roots = vec![root(&all, Layout::Consoles, false)];
    assert_eq!(
        download::pick_target(&roots, snes, None, &home),
        Target::Dir { dir: all.join("SNES"), add_folder: None }
    );

    // No PS1 folder: ask, suggesting that collection.
    assert_eq!(download::pick_target(&roots, psx, None, &home), Target::Ask { suggest: all.clone() });
    assert_eq!(download::pick_target(&[], psx, None, &home), Target::Ask { suggest: home.join("ROMs") });

    // The chosen folder: inside a scanned collection → no new ROM folder; elsewhere → added.
    assert_eq!(
        download::pick_target(&roots, psx, Some(&all), &home),
        Target::Dir { dir: all.join("psx"), add_folder: None }
    );
    let other = tmp.path().join("RomM games");
    assert_eq!(
        download::pick_target(&roots, psx, Some(&other), &home),
        Target::Dir { dir: other.join("psx"), add_folder: Some(other.clone()) }
    );
    // Picking a console's own folder means its parent.
    assert_eq!(download::download_root(&other.join("snes")), other);
    assert_eq!(download::download_root(&other), other);
}

#[test]
fn downloaded_games_become_the_emulated_game() {
    use crate::model::{EmuRef, ScannedGame, Source};
    let m = romm_server();
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let (lib, _) = sync(&m.server("rmm_good"), &data, None).unwrap();
    fetch_covers(&m.server("rmm_good"), &data, &lib, None);
    let ctx =
        crate::scan::ScanContext { data_dir: data.clone(), ..crate::scan::desktop::test_support::ctx(tmp.path()) };
    let mut games = crate::scan::romm::scan(&ctx).unwrap();
    let emu = |rom: &str, platform: &str| {
        let mut g = ScannedGame::new(Source::Emulator, format!("f1/{rom}"), "x");
        g.emu = Some(EmuRef {
            platform: platform.into(),
            folder: "f1".into(),
            rom: PathBuf::from("/roms").join(rom),
            files: vec![],
            emulator: String::new(),
            profile: String::new(),
            serial: None,
            crc: None,
            dat_name: None,
            region: None,
        });
        g
    };
    games.push(emu("snes/Christmas Craze.smc", "nintendo_super_nes"));
    games.push(emu("psx/Final Fantasy VII (USA)/Final Fantasy VII (USA).m3u", "sony_playstation"));
    // Same file name on another console: not the same game.
    games.push(emu("megadrive/Aardvark.bin", "sega_genesis"));
    let dropped = crate::scan::romm::dedupe_with_emulation(&ctx, &mut games);
    assert_eq!(dropped, 2);
    let ids: Vec<&str> = games.iter().filter(|g| g.source == Source::Romm).map(|g| g.source_id.as_str()).collect();
    assert_eq!(ids, vec!["1", "167", "300"]);
    let snes = games.iter().find(|g| g.source_id == "f1/snes/Christmas Craze.smc").unwrap();
    assert!(snes.art.cover.is_some(), "the server's cover is lent");
    assert_eq!(snes.meta.genres, vec!["Arcade".to_string()]);
    // The RomM entry's library row (favourite, tags …) is handed to the ROM's (Db::merge_scan).
    assert_eq!(snes.absorbs.len(), 1);
    assert_eq!(snes.absorbs[0].0, Source::Romm);
    assert!(!ids.contains(&snes.absorbs[0].1.as_str()));
}

#[test]
fn year_from_release_dates() {
    assert_eq!(year_of_ms(1325289600000), Some(2011)); // 2011-12-31
    assert_eq!(year_of_ms(1546214400000), Some(2018)); // 2018-12-31
    assert_eq!(year_of_ms(0), Some(1970));
    assert_eq!(year_of_ms(-9_000_000_000_000), None);
}

#[test]
fn app_tests_syncs_and_offers_download() {
    use crate::model::{Game, Source};
    let m = romm_server();
    let tmp = tempfile::tempdir().unwrap();
    let mut st = crate::app::AppState::for_test(tmp.path());
    assert!(!st.romm_set_up());
    st.config.romm.url = m.base.clone();
    st.config.romm.token = Some("rmm_wrong".into());
    st.romm_test();
    assert!(st.poll_until(std::time::Duration::from_secs(10), |s| !s.romm.testing));
    let err = st.romm.test_result.clone().unwrap().unwrap_err();
    assert!(err.contains("didn't accept the token"), "{err}");
    assert_eq!(st.romm.rescans, 0);

    st.config.romm.token = Some("rmm_good".into());
    st.romm_settings_changed();
    st.romm_test();
    assert!(st.poll_until(std::time::Duration::from_secs(10), |s| !s.romm.testing
        && !s.romm.syncing
        && s.romm.sync_result.is_some()));
    assert_eq!(st.romm.test_result.clone().unwrap().unwrap(), "Connected to RomM 5.4.0: 5 games on 5 consoles");
    assert_eq!(st.romm.sync_result.clone().unwrap().unwrap(), "5 games on the server");
    assert!(cache_file(&st.paths.data_dir).is_file());
    // The new list (and then the covers) rescan.
    assert!(st.poll_until(std::time::Duration::from_secs(10), |s| s.romm.rescans >= 2));

    let mut g = Game::blank("Christmas Craze");
    g.id = 7;
    g.source = Source::Romm;
    g.source_id = "159".into();
    g.installed = false;
    g.platform = Some("Nintendo SNES".into());
    let mut pico = g.clone();
    pico.id = 8;
    pico.source_id = "300".into();
    pico.platform = Some("PICO-8 Fantasy Console".into());
    st.set_games(vec![g, pico]);
    assert!(st.can_install(7));
    assert!(!st.can_install(8), "a console Lodestar can't play");
    // Screenshot runs never download.
    st.no_launch = true;
    st.install_game(7);
    assert!(st.romm_progress(7).is_none());

    // Not set up any more: the cached games go with the next scan.
    st.config.romm.url.clear();
    st.romm_on_rescan();
    assert!(!cache_file(&st.paths.data_dir).exists());
}

#[test]
fn app_downloads_into_rom_folders_and_asks_once() {
    use crate::model::{Game, Source};
    let m = romm_server();
    let tmp = tempfile::tempdir().unwrap();
    let mut st = crate::app::AppState::for_test(tmp.path());
    // Only the folders this test makes (never the machine's drives).
    st.config.emulation.auto_folders = false;
    let roms = tmp.path().join("roms");
    std::fs::create_dir_all(roms.join("snes")).unwrap();
    st.config
        .emulation
        .rom_folders
        .insert("f1".into(), crate::config::RomFolder { path: roms.clone(), ..Default::default() });
    st.config.romm.url = m.base.clone();
    st.config.romm.token = Some("rmm_good".into());
    st.romm_sync();
    assert!(st.poll_until(std::time::Duration::from_secs(10), |s| !s.romm.syncing));
    let mk = |id: i64, sid: &str, plat: &str| {
        let mut g = Game::blank("x");
        g.id = id;
        g.source = Source::Romm;
        g.source_id = sid.into();
        g.installed = false;
        g.platform = Some(plat.into());
        g
    };
    st.set_games(vec![mk(7, "159", "Nintendo SNES"), mk(8, "201", "Sony PlayStation")]);
    let before = st.romm.rescans;
    st.install_game(7);
    assert!(st.romm_progress(7).is_some());
    assert!(st.poll_until(std::time::Duration::from_secs(10), |s| s.romm_progress(7).is_none()));
    assert_eq!(
        std::fs::read(roms.join("snes/Christmas Craze.smc")).unwrap(),
        b"SNES-ROM-BYTES-0123456789-abcdefghijklmn"
    );
    assert_eq!(st.romm.rescans, before + 1);

    // No PlayStation folder: asked once, suggesting the ROM folder.
    st.romm_download(8);
    assert!(st.poll_until(std::time::Duration::from_secs(10), |s| s.romm.ask.is_some()));
    assert_eq!(st.romm.ask.clone().unwrap(), (8, roms.clone()));
    let dl = tmp.path().join("From RomM");
    st.romm_folder_chosen(8, Some(dl.clone()));
    assert_eq!(st.config.romm.download_dir.as_deref(), Some(dl.as_path()));
    assert!(st.poll_until(std::time::Duration::from_secs(10), |s| s.romm_progress(8).is_none()));
    assert!(dl.join("psx/Final Fantasy VII (USA)/discs/Disc 2.chd").is_file());
    // The new folder is a ROM folder now, so the scan finds the game.
    assert!(st.config.emulation.rom_folders.values().any(|f| f.path == dl));
}

#[test]
fn a_sign_in_page_or_a_short_file_is_not_saved_as_the_game() {
    let tmp = tempfile::tempdir().unwrap();
    let mut g = game(fixture!("roms.json"), 159);
    // A proxy's sign-in page instead of the ROM.
    let m = Mock::start(|_| (200, b"<!doctype html><html>Sign in</html>".to_vec()));
    let err = download::download(&m.server("t"), &g, tmp.path(), &Progress::default()).unwrap_err();
    assert!(format!("{err}").contains("web page"), "{err}");
    assert!(std::fs::read_dir(tmp.path()).unwrap().next().is_none(), "nothing left behind");
    // Fewer bytes than the server said the game has.
    g.size = 1000;
    let m = Mock::start(|_| (200, b"short".to_vec()));
    let err = download::download(&m.server("t"), &g, tmp.path(), &Progress::default()).unwrap_err();
    assert!(format!("{err}").contains("whole game"), "{err}");
    assert!(std::fs::read_dir(tmp.path()).unwrap().next().is_none(), "nothing left behind");
}
