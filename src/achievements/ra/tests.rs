//! RetroAchievements: ROM hashes against independently computed MD5s (Python's hashlib over the
//! same synthetic ROMs), answers shaped like the documented API responses, a mocked network
//! (nothing here goes online), pacing / back-off, local-only runs and the key kept out of logs.

use super::api::{ApiErr, Client, Pacer, USER_AGENT};
use super::hash::{Rule, console, hash_bytes, hash_cached, hash_file};
use super::*;
use crate::achievements::{Env, Request, State};
use crate::art::net::{HttpResponse, Net};
use crate::config::AchievementsConfig;
use crate::model::{Art, EmuRef, LaunchSpec};
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const KEY: &str = "SECRETkey1234567890abcdefXYZ";

/// The ROM bytes the expected hashes were computed from: `(i*31 + seed) & 0xFF`.
fn pat(n: usize, seed: u8) -> Vec<u8> {
    (0..n).map(|i| (i.wrapping_mul(31) as u8).wrapping_add(seed)).collect()
}

fn write(p: &Path, b: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, b).unwrap();
}

// ---------------------------------------------------------------------------------------------
// Hashes
// ---------------------------------------------------------------------------------------------

#[test]
fn hash_rules_match_rcheevos() {
    let n = Path::new("x.bin");
    // Whole file (Game Boy).
    assert_eq!(hash_bytes(Rule::Whole, &pat(32768, 1), n).unwrap(), "c12c4abbf931f5110e9a371560718b6a");
    // NES: the 16-byte iNES header is skipped; FDS too; a headerless dump is hashed whole.
    let body = pat(40960, 3);
    let nes = [b"NES\x1a".as_slice(), &pat(12, 2), &body].concat();
    assert_eq!(hash_bytes(Rule::Nes, &nes, n).unwrap(), "697fd4fb91b760b86a19463bef9207ad");
    let fds = [b"FDS\x1a".as_slice(), &pat(12, 2), &body].concat();
    assert_eq!(hash_bytes(Rule::Nes, &fds, n).unwrap(), "697fd4fb91b760b86a19463bef9207ad");
    assert_eq!(hash_bytes(Rule::Nes, &body, n).unwrap(), "697fd4fb91b760b86a19463bef9207ad");
    // SNES: a 512-byte copier header (size % 8192 == 512) is skipped.
    let snes = [pat(512, 4), pat(65536, 5)].concat();
    assert_eq!(hash_bytes(Rule::Snes, &snes, n).unwrap(), "5efd02636fb52c7ff470eacc1074d6f7");
    assert_eq!(hash_bytes(Rule::Snes, &pat(65536, 5), n).unwrap(), "5efd02636fb52c7ff470eacc1074d6f7");
    // …but the same bytes are hashed whole as a whole-file console.
    assert_eq!(hash_bytes(Rule::Whole, &snes, n).unwrap(), "01e1f2bfc2e015ed4e25da71e716d38e");
    // PC Engine: 512 bytes skipped when size & 512.
    let pce = [pat(512, 0), pat(32768, 8)].concat();
    assert_eq!(hash_bytes(Rule::Pce, &pce, n).unwrap(), "d4b54365614adc0541775737db958b48");
    assert_eq!(hash_bytes(Rule::Pce, &pat(32768, 8), n).unwrap(), "d4b54365614adc0541775737db958b48");
    // Lynx: the 64-byte LYNX header.
    let lynx = [b"LYNX\0".as_slice(), &pat(59, 1), &pat(4096, 9)].concat();
    assert_eq!(hash_bytes(Rule::Lynx, &lynx, n).unwrap(), "f6be51fb5571aa9e482434c0c2575337");
    // 7800: the 128-byte A78 header ("ATARI7800" at byte 1).
    let a78 = [b"\x01ATARI7800".as_slice(), &pat(118, 2), &pat(4096, 10)].concat();
    assert_eq!(hash_bytes(Rule::A7800, &a78, n).unwrap(), "5bae91964bb5f0344a23bcfb091377ae");
    // Disc consoles aren't hashed.
    assert_eq!(hash_bytes(Rule::NameOnly, &body, n), None);
}

#[test]
fn n64_byte_orders_hash_the_same() {
    let z64 = [vec![0x80, 0x37, 0x12, 0x40], pat(4092, 6)].concat();
    let v64: Vec<u8> = z64.chunks(2).flat_map(|c| [c[1], c[0]]).collect();
    let n64: Vec<u8> = z64.chunks(4).flat_map(|c| [c[3], c[2], c[1], c[0]]).collect();
    assert_eq!((v64[0], n64[0]), (0x37, 0x40));
    let n = Path::new("x.z64");
    for (name, data) in [("z64", &z64), ("v64", &v64), ("n64", &n64)] {
        assert_eq!(hash_bytes(Rule::N64, data, n).as_deref(), Some("f3911da3885d28f600947834421f2751"), "{name}");
    }
    assert_eq!(hash_bytes(Rule::N64, &pat(4096, 1), n), None, "not an N64 ROM");
}

fn nds_rom() -> Vec<u8> {
    let mut f = pat(0x4000, 7);
    let put = |f: &mut Vec<u8>, at: usize, v: u32| f[at..at + 4].copy_from_slice(&v.to_le_bytes());
    put(&mut f, 0x20, 0x200);
    put(&mut f, 0x2C, 0x100);
    put(&mut f, 0x30, 0x300);
    put(&mut f, 0x3C, 0x80);
    put(&mut f, 0x68, 0x3F00);
    f[0] = 0;
    f
}

#[test]
fn nds_hashes_header_code_and_icon() {
    let tmp = tempfile::tempdir().unwrap();
    let rom = nds_rom();
    // The icon block runs past the end: zero-padded.
    let want = "dc2022fc57ae82503330c8f03c2cfc86";
    assert_eq!(hash_bytes(Rule::Nds, &rom, Path::new("a.nds")).as_deref(), Some(want));
    let p = tmp.path().join("Game.nds");
    write(&p, &rom);
    assert_eq!(hash_file("nintendo_ds", &p).as_deref(), Some(want));
    // A SuperCard header in front changes nothing.
    let mut sc = vec![0u8; 512];
    sc[..4].copy_from_slice(&[0x2E, 0, 0, 0xEA]);
    sc[0xB0..0xB4].copy_from_slice(&[0x44, 0x46, 0x96, 0]);
    let p2 = tmp.path().join("Game (SuperCard).nds");
    write(&p2, &[sc, rom].concat());
    assert_eq!(hash_file("nintendo_ds", &p2).as_deref(), Some(want));
}

#[test]
fn files_zips_m3u_and_arcade_names() {
    let tmp = tempfile::tempdir().unwrap();
    let body = pat(40960, 3);
    let nes = [b"NES\x1a".as_slice(), &pat(12, 2), &body].concat();
    // A plain file.
    let p = tmp.path().join("nes/Game (USA).nes");
    write(&p, &nes);
    assert_eq!(hash_file("nintendo_nes", &p).as_deref(), Some("697fd4fb91b760b86a19463bef9207ad"));
    // Zipped (deflated) next to a readme: the ROM inside is hashed, not the zip.
    let z = tmp.path().join("nes/Game (USA).zip");
    crate::emulation::zipread::tests::write_zip(&z, &[("readme.txt", b"hello", true), ("Game (USA).nes", &nes, true)]);
    assert_eq!(hash_file("nintendo_nes", &z).as_deref(), Some("697fd4fb91b760b86a19463bef9207ad"));
    // A `<zip>/<entry>` path names the entry.
    assert_eq!(
        hash_file("nintendo_nes", &z.join("Game (USA).nes")).as_deref(),
        Some("697fd4fb91b760b86a19463bef9207ad")
    );
    // A zip with a single file of an unexpected extension.
    let z1 = tmp.path().join("gb/One.zip");
    crate::emulation::zipread::tests::write_zip(&z1, &[("One.rom", &pat(32768, 1), false)]);
    assert_eq!(hash_file("nintendo_gameboy", &z1).as_deref(), Some("c12c4abbf931f5110e9a371560718b6a"));
    // m3u: its first entry.
    let g = tmp.path().join("md/Disk 1.md");
    write(&g, &pat(8192, 11));
    let m = tmp.path().join("md/Game.m3u");
    write(&m, b"# list\n\nDisk 1.md\nDisk 2.md\n");
    assert_eq!(hash_file("sega_genesis", &m).as_deref(), Some("731219337f592f2a77fb580605953b64"));
    // Arcade: the file name; FBNeo console folders prefix it.
    assert_eq!(
        hash_file("arcade", Path::new("/roms/arcade/sf2.zip")).as_deref(),
        Some("a7946d9b269b67f7a6b1148d3f5d8155")
    );
    assert_eq!(
        hash_file("arcade", Path::new("/roms/NES/smb.zip")).as_deref(),
        Some("bb23ace7414f08c52f38e2565a06148e")
    );
    // Disc consoles and unknown platforms: nothing.
    assert_eq!(hash_file("sony_playstation", &p), None);
    assert_eq!(hash_file("pc_windows", &p), None);
    // 7z can't be opened.
    let s = tmp.path().join("nes/x.7z");
    write(&s, b"7z\xbc\xaf\x27\x1c");
    assert_eq!(hash_file("nintendo_nes", &s), None);
}

#[test]
fn hashes_are_cached_per_path_size_and_time() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("ra");
    let p = tmp.path().join("roms/a.gb");
    write(&p, &pat(32768, 1));
    assert_eq!(hash_cached(&dir, "nintendo_gameboy", &p).as_deref(), Some("c12c4abbf931f5110e9a371560718b6a"));
    let text = std::fs::read_to_string(dir.join("rom-hashes.json")).unwrap();
    assert!(text.contains("c12c4abbf931f5110e9a371560718b6a"), "{text}");
    // Tamper with the cache: a same-size, same-time file answers from it…
    std::fs::write(dir.join("rom-hashes.json"), text.replace("c12c4abbf931f5110e9a371560718b6a", "cached")).unwrap();
    assert_eq!(hash_cached(&dir, "nintendo_gameboy", &p).as_deref(), Some("cached"));
    // …a changed file is hashed again.
    write(&p, &pat(1000, 1));
    let f = std::fs::File::options().write(true).open(&p).unwrap();
    f.set_modified(std::time::SystemTime::now() + Duration::from_secs(5)).unwrap();
    assert_ne!(hash_cached(&dir, "nintendo_gameboy", &p).as_deref(), Some("cached"));
}

#[test]
fn every_platform_maps_to_a_real_console() {
    for p in crate::emulation::platforms::all() {
        if let Some(c) = console(p.id) {
            assert!(!c.ids.is_empty() && c.ids.iter().all(|i| (1..=82).contains(i)), "{}", p.id);
        }
    }
    assert_eq!(console("nintendo_nes").unwrap().ids, &[7]);
    assert_eq!(console("nintendo_super_nes").unwrap().rule, Rule::Snes);
    assert_eq!(console("sony_playstation2").unwrap().rule, Rule::NameOnly);
    assert!(console("pc_windows").is_none() && console("nintendo_switch").is_none() && console("xbox").is_none());
}

// ---------------------------------------------------------------------------------------------
// A mocked RetroAchievements
// ---------------------------------------------------------------------------------------------

/// A request seen: its URL and headers.
type Asked = (String, Vec<(String, String)>);

/// Answers by the endpoint in the URL; each endpoint's answers are used in turn (the last one
/// repeats). Records every request.
#[derive(Default)]
struct Mock {
    answers: Mutex<Vec<(String, Vec<HttpResponse>)>>,
    asked: Mutex<Vec<Asked>>,
    in_flight: AtomicUsize,
    most_in_flight: AtomicUsize,
    delay: Duration,
}

fn ok(body: &str) -> HttpResponse {
    HttpResponse { status: 200, body: body.as_bytes().to_vec(), headers: vec![] }
}

fn status(s: u16) -> HttpResponse {
    HttpResponse { status: s, body: vec![], headers: vec![] }
}

impl Mock {
    fn on(self, part: &str, r: Vec<HttpResponse>) -> Mock {
        self.answers.lock().push((part.to_string(), r));
        self
    }
    fn urls(&self) -> Vec<String> {
        self.asked.lock().iter().map(|(u, _)| u.clone()).collect()
    }
    fn count(&self, part: &str) -> usize {
        self.urls().iter().filter(|u| u.contains(part)).count()
    }
}

impl Net for Mock {
    fn get(&self, url: &str, _: Option<&str>, limit: u64) -> Result<HttpResponse, String> {
        self.get_with(url, &[], limit)
    }
    fn head(&self, _: &str) -> Result<u16, String> {
        Err("no".into())
    }
    fn get_with(&self, url: &str, headers: &[(&str, String)], _: u64) -> Result<HttpResponse, String> {
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.most_in_flight.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        self.asked.lock().push((url.to_string(), headers.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()));
        let mut a = self.answers.lock();
        let r = match a.iter_mut().find(|(p, _)| url.contains(p.as_str())) {
            Some((_, list)) if list.len() > 1 => Ok(list.remove(0)),
            Some((_, list)) => Ok(list[0].clone()),
            // An error text that has the URL in it (as ureq's do).
            None => Err(format!("connection refused: {url}")),
        };
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        r
    }
}

fn no_sleep(_: Duration) {}

/// A client of its own (its own pacers: one test's "server is down" pause never reaches another).
fn client<'a>(net: &'a dyn Net) -> Client<'a> {
    let fast: &'static Pacer = Box::leak(Box::new(Pacer::new(Duration::ZERO)));
    let mut c = Client::new(net, KEY, vec![KEY.to_string()]);
    c.api = fast;
    c.media = fast;
    c.sleep = &no_sleep;
    c.local_only = false;
    c
}

const GAME_LIST: &str = r#"[
  {"Title":"~Hack~ Super Mario Bros.: Extra","ID":9001,"ConsoleID":7,"ConsoleName":"NES/Famicom","ImageIcon":"/Images/1.png","NumAchievements":10,"NumLeaderboards":0,"Points":100,"DateModified":"2024-01-01 00:00:00","ForumTopicID":1,"Hashes":["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]},
  {"Title":"Legend of Zelda, The","ID":1995,"ConsoleID":7,"ConsoleName":"NES/Famicom","ImageIcon":"/Images/2.png","NumAchievements":"40","NumLeaderboards":2,"Points":400,"DateModified":"2024-01-01 00:00:00","ForumTopicID":2,"Hashes":["697fd4fb91b760b86a19463bef9207ad","bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"]},
  {"Title":"Super Mario Bros.","ID":1446,"ConsoleID":7,"ConsoleName":"NES/Famicom","ImageIcon":"/Images/3.png","NumAchievements":30,"NumLeaderboards":1,"Points":300,"DateModified":null,"ForumTopicID":null,"Hashes":["cccccccccccccccccccccccccccccccc"]}
]"#;

const HASHES_1446: &str = r#"{"Results":[
  {"Name":"Super Mario Bros. (World).nes","MD5":"cccccccccccccccccccccccccccccccc","Labels":["nointro"],"PatchUrl":null}
]}"#;

/// Shaped like the documented `API_GetGameInfoAndUserProgress` answer (fields trimmed).
const PROGRESS_1995: &str = r#"{
  "ID":1995,"Title":"Legend of Zelda, The","ConsoleID":7,"ConsoleName":"NES/Famicom","ParentGameID":null,
  "NumDistinctPlayers":200,"NumAchievements":3,"NumAwardedToUser":2,"NumAwardedToUserHardcore":1,
  "UserCompletion":"66.67%","UserCompletionHardcore":"33.33%","UserTotalPlaytime":3600,
  "ImageIcon":"/Images/2.png","HighestAwardKind":null,"HighestAwardDate":null,
  "Achievements":{
    "300":{"ID":300,"Title":"Triforce","Description":"Get it","Points":25,"TrueRatio":60,"BadgeName":"30003","Type":"win_condition","NumAwarded":20,"NumAwardedHardcore":10,"DisplayOrder":3,"Author":"x","AuthorULID":"01","DateCreated":"2020-01-01 00:00:00","DateModified":"2020-01-01 00:00:00","MemAddr":"abc"},
    "100":{"ID":100,"Title":"Sword","Description":"Take it","Points":5,"TrueRatio":5,"BadgeName":"10001","Type":"progression","NumAwarded":150,"NumAwardedHardcore":90,"DisplayOrder":1,"Author":"x","AuthorULID":"01","DateCreated":"2020-01-01 00:00:00","DateModified":"2020-01-01 00:00:00","MemAddr":"abc","DateEarned":"2024-03-01 10:00:00","DateEarnedHardcore":"2024-03-01 10:00:00"},
    "200":{"ID":200,"Title":"Shield","Description":"Buy it","Points":10,"TrueRatio":12,"BadgeName":"20002","Type":null,"NumAwarded":"100","NumAwardedHardcore":50,"DisplayOrder":"2","Author":"x","AuthorULID":"01","DateCreated":"2020-01-01 00:00:00","DateModified":"2020-01-01 00:00:00","MemAddr":"abc","DateEarned":"2024-03-02 12:30:00"}
  }
}"#;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nfake";

fn nes_game(rom: PathBuf) -> crate::model::Game {
    let mut db = crate::db::Db::open_memory().unwrap();
    let id = db.add_manual("The Legend of Zelda", LaunchSpec::Url("x://".into()), Art::default()).unwrap();
    let mut g = db.get(id).unwrap().unwrap();
    g.emu = Some(EmuRef { platform: "nintendo_nes".into(), rom, ..Default::default() });
    g
}

fn cfg() -> AchievementsConfig {
    AchievementsConfig {
        ra_username: "Tester".into(),
        ra_api_key: Some(KEY.into()),
        ra_ulid: Some("00003EMFWR7XB8SDPEHB3K56ZQ".into()),
        ..Default::default()
    }
}

fn run(mock: Arc<Mock>, cache: &Path, game: &crate::model::Game, cfg: AchievementsConfig, force: bool) -> State {
    let env = Env { cache_dir: cache.to_path_buf(), net: mock.clone() };
    let req = Request { game: game.clone(), config: cfg, force };
    load_with(&req, &env, &|_, secrets| {
        let mut c = client(mock.as_ref());
        assert!(secrets.contains(&KEY.to_string()));
        c.local_only = false;
        c
    })
}

fn full_mock() -> Mock {
    Mock::default()
        .on("API_GetGameList", vec![ok(GAME_LIST)])
        .on("API_GetGameInfoAndUserProgress", vec![ok(PROGRESS_1995)])
        .on("API_GetGameHashes", vec![ok(HASHES_1446)])
        .on("media.retroachievements.org", vec![HttpResponse { status: 200, body: PNG.to_vec(), headers: vec![] }])
}

#[test]
fn a_rom_is_found_by_hash_and_its_progress_shown() {
    let tmp = tempfile::tempdir().unwrap();
    let rom = tmp.path().join("roms/nes/Zelda.zip");
    let nes = [b"NES\x1a".as_slice(), &pat(12, 2), &pat(40960, 3)].concat();
    crate::emulation::zipread::tests::write_zip(&rom, &[("Zelda.nes", &nes, true)]);
    let mock = Arc::new(full_mock());
    let cache = tmp.path().join("cache");
    let st = run(mock.clone(), &cache, &nes_game(rom), cfg(), false);
    let State::Ready(p) = st else { panic!("{st:?}") };
    assert_eq!((p.unlocked, p.total, p.note.as_deref()), (2, 3, None));
    assert_eq!(p.page_url.as_deref(), Some("https://retroachievements.org/game/1995"));
    let names: Vec<&str> = p.list.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Sword", "Shield", "Triforce"], "display order");
    let (sword, shield, tri) = (&p.list[0], &p.list[1], &p.list[2]);
    assert!(sword.hardcore && !shield.hardcore && !tri.hardcore);
    assert_eq!(sword.unlocked_at, Some(1709287200));
    assert_eq!(shield.unlocked_at, Some(1709382600));
    assert_eq!(tri.unlocked_at, None);
    assert_eq!(sword.rarity, Some(75.0));
    assert_eq!(p.recent()[0].name, "Shield");
    // Badges: local files — the unlocked look for unlocked ones, the grey one for locked ones.
    let badges = cache.join("ra/badges");
    assert_eq!(sword.icon.as_deref(), Some(badges.join("10001.png").to_str().unwrap()));
    assert_eq!(sword.icon_locked, None);
    assert_eq!(tri.icon, None);
    assert_eq!(tri.icon_locked.as_deref(), Some(badges.join("30003_lock.png").to_str().unwrap()));
    assert!(mock.urls().iter().any(|u| u == "https://media.retroachievements.org/Badge/30003_lock.png"));
    // What was asked, and how: the user's ULID, the key as `y`, RetroAchievements' User-Agent.
    let urls = mock.urls();
    assert!(urls[0].starts_with("https://retroachievements.org/API/API_GetGameList.php?i=7&f=1&h=1&y="), "{urls:?}");
    assert!(
        urls.iter()
            .any(|u| u.contains("API_GetGameInfoAndUserProgress.php?g=1995&u=00003EMFWR7XB8SDPEHB3K56ZQ&a=1&y="))
    );
    assert_eq!(mock.count("GetGameHashes"), 0, "a hash match needs no names");
    for (_, h) in mock.asked.lock().iter() {
        assert!(h.contains(&("User-Agent".to_string(), USER_AGENT.to_string())), "{h:?}");
    }
    assert!(USER_AGENT.starts_with("Lodestar/") && USER_AGENT.ends_with(" (Linux)"));
    // Shown again: everything from the cache (lists a week, progress 10 minutes, badges for good).
    let before = mock.urls().len();
    let st2 = run(mock.clone(), &cache, &nes_game(tmp.path().join("roms/nes/Zelda.zip")), cfg(), false);
    assert_eq!(st2, State::Ready(p.clone()));
    assert_eq!(mock.urls().len(), before, "{:?}", &mock.urls()[before..]);
    // After playing (force): only the progress is asked again.
    run(mock.clone(), &cache, &nes_game(tmp.path().join("roms/nes/Zelda.zip")), cfg(), true);
    assert_eq!(mock.urls().len(), before + 1);
    assert!(mock.urls()[before].contains("GetGameInfoAndUserProgress"));
    // The cached progress is per user.
    let other = AchievementsConfig { ra_ulid: None, ra_username: "Someone".into(), ..cfg() };
    run(mock.clone(), &cache, &nes_game(tmp.path().join("roms/nes/Zelda.zip")), other, false);
    assert!(mock.urls().last().unwrap().contains("&u=Someone&"));
}

#[test]
fn names_then_titles_when_the_hash_is_unknown() {
    let tmp = tempfile::tempdir().unwrap();
    let rom = tmp.path().join("roms/nes/Super Mario Bros. (World).nes");
    write(&rom, &pat(4096, 99));
    let cache = tmp.path().join("cache");
    let mut g = nes_game(rom.clone());
    g.name = "Super Mario Bros.".into();
    let progress = PROGRESS_1995.replace("\"ID\":1995", "\"ID\":1446");
    // (The more specific answer first: the mock takes the first that fits.)
    let mock = Arc::new(Mock::default().on("GetGameInfoAndUserProgress.php?g=1446", vec![ok(&progress)]));
    for (k, v) in std::mem::take(&mut *full_mock().answers.lock()) {
        mock.answers.lock().push((k, v));
    }
    // The ROM's file name is one of the game's supported files: no note.
    let State::Ready(p) = run(mock.clone(), &cache, &g, cfg(), false) else { panic!() };
    assert_eq!((p.note.as_deref(), p.page_url.as_deref()), (None, Some("https://retroachievements.org/game/1446")));
    assert!(mock.urls().iter().any(|u| u.contains("API_GetGameHashes.php?i=1446&")));
    // A differently named file of the same title: matched by title, with the note.
    let rom2 = tmp.path().join("roms/nes/SMB (hacked).nes");
    write(&rom2, &pat(4096, 98));
    let mut g2 = nes_game(rom2);
    g2.name = "Super Mario Bros".into();
    let State::Ready(p) = run(mock.clone(), &cache, &g2, cfg(), false) else { panic!() };
    assert_eq!(p.note.as_deref(), Some("Matched by title — your version may not be supported"));
    assert_eq!(mock.count("API_GetGameHashes.php?i=1446"), 1, "file names cached");
    // A disc game: the Redump name against the supported files, no hash needed.
    let mut disc = nes_game(tmp.path().join("roms/psx/smb.chd"));
    disc.name = "Super Mario Bros.".into();
    let e = disc.emu.as_mut().unwrap();
    e.platform = "sony_playstation".into();
    e.dat_name = Some("Super Mario Bros. (World)".into());
    let State::Ready(p) = run(mock.clone(), &cache, &disc, cfg(), false) else { panic!() };
    assert_eq!(p.note, None);
    assert!(mock.urls().iter().any(|u| u.contains("API_GetGameList.php?i=12&")));
    // Nothing like it: no section.
    let rom3 = tmp.path().join("roms/nes/Unknown Thing.nes");
    write(&rom3, &pat(4096, 97));
    let mut g3 = nes_game(rom3);
    g3.name = "Unknown Thing".into();
    assert_eq!(run(mock.clone(), &cache, &g3, cfg(), false), State::None);
}

#[test]
fn hand_set_id_wins_and_unknown_ids_say_so() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");
    let mut g = nes_game(tmp.path().join("missing.nes"));
    g.meta.ra_game_id = Some(1995);
    let mock = Arc::new(full_mock());
    assert!(matches!(run(mock.clone(), &cache, &g, cfg(), false), State::Ready(_)));
    assert_eq!(mock.count("GetGameList"), 0, "no matching needed");
    // An id RetroAchievements doesn't know (`{}`).
    let mock = Arc::new(Mock::default().on("GetGameInfoAndUserProgress", vec![ok("{}")]));
    g.meta.ra_game_id = Some(77);
    let State::Hint(h) = run(mock, &cache, &g, cfg(), false) else { panic!() };
    assert!(h.contains("no game with the id 77"), "{h}");
    // A game without achievements: no section.
    let none = r#"{"ID":5,"Title":"x","NumAchievements":0,"Achievements":{}}"#;
    let mock = Arc::new(Mock::default().on("GetGameInfoAndUserProgress", vec![ok(none)]));
    g.meta.ra_game_id = Some(5);
    assert_eq!(run(mock, &cache, &g, cfg(), false), State::None);
    // A platform RetroAchievements has nothing for: no section, nothing asked.
    let mock = Arc::new(full_mock());
    let mut pc = nes_game(tmp.path().join("x.exe"));
    pc.emu.as_mut().unwrap().platform = "nintendo_switch".into();
    assert_eq!(run(mock.clone(), &cache, &pc, cfg(), false), State::None);
    assert!(mock.urls().is_empty());
}

#[test]
fn missing_account_and_failures_are_plain_hints() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");
    let g = nes_game(tmp.path().join("a.nes"));
    write(&tmp.path().join("a.nes"), &pat(100, 1));
    for c in [
        AchievementsConfig { ra_api_key: None, ..cfg() },
        AchievementsConfig { ra_username: " ".into(), ..cfg() },
        AchievementsConfig { ra_api_key: Some("  ".into()), ..cfg() },
    ] {
        let mock = Arc::new(full_mock());
        assert_eq!(run(mock.clone(), &cache, &g, c, false), State::Hint(HINT_SET_UP.into()));
        assert!(mock.urls().is_empty());
    }
    let hint_for = |m: Mock| match run(Arc::new(m), &cache, &g, cfg(), false) {
        State::Hint(h) => h,
        s => panic!("{s:?}"),
    };
    assert!(hint_for(Mock::default()).contains("Couldn't reach"));
    assert!(hint_for(Mock::default().on("API_", vec![status(401)])).contains("Web API key"));
    assert!(hint_for(Mock::default().on("API_", vec![status(503)])).contains("isn't answering"));
    // An unknown user: `{}` from the progress call.
    let m = Mock::default().on("GetGameInfoAndUserProgress", vec![ok("[]")]).on("API_GetGameList", vec![ok(GAME_LIST)]);
    let mut g2 = g.clone();
    g2.meta.ra_game_id = None;
    g2.emu.as_mut().unwrap().rom = tmp.path().join("roms/z.nes");
    let nes = [b"NES\x1a".as_slice(), &pat(12, 2), &pat(40960, 3)].concat();
    write(&tmp.path().join("roms/z.nes"), &nes);
    let State::Hint(h) = run(Arc::new(m), &cache, &g2, cfg(), false) else { panic!() };
    assert!(h.contains("doesn't know the user “Tester”"), "{h}");
}

#[test]
fn local_only_runs_never_go_online_but_show_what_is_cached() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");
    let rom = tmp.path().join("roms/z.nes");
    write(&rom, &[b"NES\x1a".as_slice(), &pat(12, 2), &pat(40960, 3)].concat());
    let g = nes_game(rom);
    let env_run = |mock: Arc<Mock>, local: bool, force: bool| {
        let env = Env { cache_dir: cache.clone(), net: mock.clone() };
        let req = Request { game: g.clone(), config: cfg(), force };
        load_with(&req, &env, &|_, _| {
            let mut c = client(mock.as_ref());
            c.local_only = local;
            c
        })
    };
    // Nothing cached: no section, nothing asked.
    let mock = Arc::new(full_mock());
    assert_eq!(env_run(mock.clone(), true, false), State::None);
    assert!(mock.urls().is_empty());
    // Cached once online, then shown in a local-only run — even when it's stale (force).
    let online = env_run(Arc::new(full_mock()), false, false);
    assert!(matches!(online, State::Ready(_)));
    let mock = Arc::new(full_mock());
    assert_eq!(env_run(mock.clone(), true, true), online);
    assert!(mock.urls().is_empty(), "{:?}", mock.urls());
    // Offline later: the saved progress too.
    assert_eq!(env_run(Arc::new(Mock::default()), false, true), online);
    // The account check says why.
    let mock = Mock::default();
    let mut c = client(&mock);
    c.local_only = true;
    assert_eq!(check_with("Tester", &c).unwrap_err(), "Lodestar doesn't go online in this run.");
    assert!(mock.urls().is_empty());
}

// ---------------------------------------------------------------------------------------------
// Account check
// ---------------------------------------------------------------------------------------------

const PROFILE: &str = r#"{"User":"TeSTer","ULID":"00003EMFWR7XB8SDPEHB3K56ZQ","UserPic":"/UserPic/TeSTer.png","MemberSince":"2015-01-01 00:00:00","RichPresenceMsg":null,"LastGameID":1,"ContribCount":0,"ContribYield":0,"TotalPoints":10,"TotalSoftcorePoints":0,"TotalTruePoints":12,"Permissions":1,"Untracked":0,"ID":7,"UserWallActive":1,"Motto":""}"#;

#[test]
fn account_check_answers_in_plain_words() {
    let mock = Mock::default().on("API_GetUserProfile", vec![ok(PROFILE)]);
    let a = check_with("  tester ", &client(&mock)).unwrap();
    assert_eq!((a.display_name.as_str(), a.ulid.as_str()), ("TeSTer", "00003EMFWR7XB8SDPEHB3K56ZQ"));
    assert!(mock.urls()[0].starts_with("https://retroachievements.org/API/API_GetUserProfile.php?u=tester&y="));
    let err = |m: Mock, user: &str| check_with(user, &client(&m)).unwrap_err();
    assert!(err(Mock::default().on("API_", vec![status(401)]), "a").contains("didn't accept this Web API key"));
    assert!(err(Mock::default().on("API_", vec![status(404)]), "nobody").contains("doesn't know the user “nobody”"));
    assert!(err(Mock::default().on("API_", vec![ok("[]")]), "nobody").contains("doesn't know the user"));
    assert!(err(Mock::default(), "a").contains("Check your internet connection"));
    assert!(err(Mock::default().on("API_", vec![status(502)]), "a").contains("isn't answering right now"));
    assert!(err(Mock::default().on("API_", vec![ok("<html>")]), "a").contains("unexpected answer"));
    assert!(err(Mock::default(), " ").contains("username first"));
    let m = Mock::default();
    let keyless = Client::new(&m, " ", vec![]);
    assert!(check_with("a", &keyless).unwrap_err().contains("Web API key first"));
    assert!(m.urls().is_empty());
}

#[test]
fn the_key_never_shows_in_texts_meant_for_logs() {
    let mock = Mock::default();
    let c = client(&mock);
    let url = format!("https://retroachievements.org/API/API_GetUserProfile.php?u=a&y={KEY}");
    assert!(!c.redact(&url).contains(KEY));
    assert!(!c.redact(&format!("connection refused: {url}")).contains(KEY));
    // Every error the user sees is ours, never a transport text with the URL in it.
    let e = check_with("a", &c).unwrap_err();
    assert!(!e.contains(KEY) && !e.contains("http"), "{e}");
    // The key is scrubbed with the config's own secret list.
    let cfg = cfg();
    assert!(cfg.secrets().contains(&KEY.to_string()));
}

// ---------------------------------------------------------------------------------------------
// Pacing and back-off
// ---------------------------------------------------------------------------------------------

#[test]
fn requests_go_one_at_a_time_and_a_gap_apart() {
    let pacer = Pacer::new(Duration::from_millis(40));
    let mock = Mock { delay: Duration::from_millis(5), ..Default::default() }.on("API_", vec![ok("{}")]);
    let start = Instant::now();
    std::thread::scope(|s| {
        for _ in 0..2 {
            s.spawn(|| {
                let mut c = Client::new(&mock, KEY, vec![]);
                c.api = &pacer;
                c.local_only = false;
                for _ in 0..2 {
                    c.call("GetUserProfile", &[("u", "a".into())]).unwrap();
                }
            });
        }
    });
    assert_eq!(mock.urls().len(), 4);
    assert_eq!(mock.most_in_flight.load(Ordering::SeqCst), 1, "never two at once");
    assert!(start.elapsed() >= Duration::from_millis(3 * 40), "{:?}", start.elapsed());
}

#[test]
fn too_many_requests_backs_off_gives_up_then_rests_a_while() {
    let slept = Mutex::new(vec![]);
    let record = |d: Duration| slept.lock().push(d);
    let limited = HttpResponse { status: 429, body: vec![], headers: vec![("retry-after".into(), "3".into())] };
    let mock = Mock::default().on("API_", vec![limited.clone(), status(503), ok("{\"a\":1}")]);
    let mut c = client(&mock);
    c.backoff = Duration::from_millis(500);
    c.sleep = &record;
    assert_eq!(c.call("GetUserProfile", &[]).unwrap()["a"], 1);
    // Retry-After honoured, else doubling from the first back-off.
    assert_eq!(*slept.lock(), [Duration::from_secs(3), Duration::from_secs(1)]);
    // Always busy: three retries, then "down".
    slept.lock().clear();
    let mock = Mock::default().on("API_", vec![status(429)]);
    let mut c = client(&mock);
    c.backoff = Duration::from_millis(100);
    c.sleep = &record;
    assert_eq!(c.call("GetUserProfile", &[]), Err(ApiErr::Down(429)));
    assert_eq!(mock.urls().len(), 4);
    assert_eq!(*slept.lock(), [100, 200, 400].map(Duration::from_millis));
    // Then a pause: the next pages ask nothing (no 14 s of back-off each, no hammering).
    assert_eq!(c.call("GetGameInfoAndUserProgress", &[]), Err(ApiErr::Down(0)));
    assert_eq!(mock.urls().len(), 4, "nothing asked while resting");
    // A Retry-After that is too long: give up now (try again on the next page view).
    let long = HttpResponse { status: 429, body: vec![], headers: vec![("retry-after".into(), "3600".into())] };
    let mock = Mock::default().on("API_", vec![long]);
    assert_eq!(client(&mock).call("GetUserProfile", &[]), Err(ApiErr::Down(429)));
    assert_eq!(mock.urls().len(), 1);
}

// ---------------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------------

#[test]
fn game_lists_and_titles_parse_leniently() {
    let v: serde_json::Value = serde_json::from_str(GAME_LIST).unwrap();
    let l = matching::GameList::parse(&v, 5).unwrap();
    assert_eq!(l.games.len(), 3);
    assert_eq!(l.games[1], (1995, "Legend of Zelda, The".to_string(), 40));
    assert_eq!(l.hashes["697fd4fb91b760b86a19463bef9207ad"], 1995);
    assert_eq!(l.hashes.len(), 4);
    assert!(matching::GameList::parse(&serde_json::json!({"success": false}), 0).is_none());
    use matching::norm_title as n;
    assert_eq!(n("The Legend of Zelda (USA) (Rev 1) [!]"), n("Legend of Zelda, The"));
    assert_eq!(n("Ren & Stimpy"), "ren and stimpy");
    assert_eq!(n("Pokémon: Red Version"), "pokémon red version");
    assert_ne!(n("Super Mario Bros. 3"), n("Super Mario Bros."));
}

#[test]
fn progress_answers_parse() {
    assert!(progress::parse(&serde_json::json!({})).is_none());
    assert!(progress::parse(&serde_json::json!([])).is_none());
    let v: serde_json::Value = serde_json::from_str(PROGRESS_1995).unwrap();
    let p = progress::parse(&v).unwrap();
    assert_eq!((p.unlocked, p.total, p.synced), (2, 3, true));
    assert_eq!(p.list[0].icon.as_deref(), Some("10001"), "badge name until pictures are fetched");
    // `Achievements: []` (PHP's empty array) is an empty list.
    let p = progress::parse(&serde_json::json!({"ID": 3, "Achievements": []})).unwrap();
    assert!(p.list.is_empty());
}

/// Badges already on disk are never downloaded again; bad names are never used as file names.
#[test]
fn badges_are_downloaded_once() {
    let tmp = tempfile::tempdir().unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&PROGRESS_1995.replace("\"BadgeName\":\"20002\"", "\"BadgeName\":\"../x\"")).unwrap();
    let mock = Mock::default()
        .on("media.", vec![ok("not a png"), HttpResponse { status: 200, body: PNG.to_vec(), headers: vec![] }]);
    let c = client(&mock);
    let mut p = progress::parse(&v).unwrap();
    progress::badges(&c, tmp.path(), &mut p);
    // Shield (bad name): nothing. Sword's first answer wasn't a PNG: not saved.
    assert_eq!(p.list[1].icon, None);
    assert_eq!(p.list[0].icon, None);
    assert!(p.list[2].icon_locked.is_some());
    let mut p = progress::parse(&v).unwrap();
    progress::badges(&c, tmp.path(), &mut p);
    assert!(p.list[0].icon.is_some());
    let n = mock.urls().len();
    let mut p = progress::parse(&v).unwrap();
    progress::badges(&c, tmp.path(), &mut p);
    assert_eq!(mock.urls().len(), n, "all on disk");
    assert!(mock.urls().iter().all(|u| !u.contains("..")));
}

/// Expected values from Python's hashlib over the same bytes / names (rcheevos
/// `rc_hash_neogeo_cart` and the 128-byte buffer of `rc_hash_arcade`).
#[test]
fn neo_geo_neo_files_hash_their_rom_data_and_long_arcade_names_drop_the_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let neo = tmp.path().join("roms/neogeo/game.neo");
    let mut bytes = b"NEO\x01".to_vec();
    bytes.resize(4096, 0x20);
    for _ in 0..10 {
        bytes.extend(0u8..=255);
    }
    crate::test_support::write_file(&neo, &bytes);
    assert_eq!(hash::hash_file("snk_neogeo_aes", &neo).as_deref(), Some("9aec5fa312feff7a1d15b135181ffe04"));
    // Not a .neo file after all: no hash (never a file-name guess for it).
    let fake = tmp.path().join("roms/neogeo/other.neo");
    crate::test_support::write_file(&fake, b"PK\x03\x04 not a neo file");
    assert_eq!(hash::hash_file("snk_neogeo_aes", &fake), None);
    let long = format!("/roms/nes/{}.zip", "a".repeat(130));
    assert_eq!(hash::hash_file("arcade", Path::new(&long)).as_deref(), Some("e016e4ccc7fdaea56fc377600b58c4cb"));
}

// ---------------------------------------------------------------------------------------------
// The library pass: the player's completion list and cheap matches
// ---------------------------------------------------------------------------------------------

/// Shaped like the documented `API_GetUserCompletionProgress` answer (fields trimmed).
fn completion_page(total: u32, games: &[(u32, u32, u32, u32)]) -> String {
    let results: Vec<String> = games
        .iter()
        .map(|(id, max, soft, hard)| {
            format!(
                r#"{{"GameID":{id},"Title":"G{id}","ImageIcon":"/Images/1.png","ConsoleID":7,"ConsoleName":"NES/Famicom","MaxPossible":{max},"NumAwarded":{soft},"NumAwardedHardcore":"{hard}","MostRecentAwardedDate":"2024-03-01T10:00:00+00:00","HighestAwardKind":null,"HighestAwardDate":null}}"#
            )
        })
        .collect();
    format!(r#"{{"Count":{},"Total":{total},"Results":[{}]}}"#, games.len(), results.join(","))
}

#[test]
fn the_players_completion_list_is_read_page_by_page_and_kept_an_hour() {
    let tmp = tempfile::tempdir().unwrap();
    let mock = Arc::new(
        Mock::default()
            .on(
                "API_GetUserCompletionProgress.php?u=Tester&c=500&o=0&",
                vec![ok(&completion_page(3, &[(1995, 3, 2, 1), (5, 10, 0, 4)]))],
            )
            .on(
                "API_GetUserCompletionProgress.php?u=Tester&c=500&o=2&",
                vec![ok(&completion_page(3, &[(7, 0, 0, 0)]))],
            ),
    );
    let c = client(mock.as_ref());
    let done = bulk::completion(&c, tmp.path(), "Tester", false).unwrap();
    assert_eq!(done.games.get(&1995), Some(&(2, 3)));
    assert_eq!(done.games.get(&5), Some(&(4, 10)), "hardcore unlocks count too");
    assert_eq!(done.games.get(&7), Some(&(0, 0)));
    assert_eq!(mock.count("GetUserCompletionProgress"), 2, "{:?}", mock.urls());
    // Within the hour: from disk.
    assert_eq!(bulk::completion(&c, tmp.path(), "Tester", false).unwrap(), done);
    assert_eq!(mock.count("GetUserCompletionProgress"), 2);
    // After playing: asked again; offline: the saved list.
    bulk::completion(&c, tmp.path(), "Tester", true).unwrap();
    assert_eq!(mock.count("GetUserCompletionProgress"), 4);
    let offline = Mock::default();
    let saved = bulk::completion(&client(&offline), tmp.path(), "Tester", true).unwrap();
    assert_eq!(saved.games, done.games);
    // Not that answer: garbled, never a loop.
    assert_eq!(bulk::parse_page(&serde_json::json!({"Error": "x"})), None);
    let (total, page) = bulk::parse_page(
        &serde_json::json!({"Total": "1", "Results": [{"GameID": "12", "MaxPossible": "5", "NumAwarded": 9}]}),
    )
    .unwrap();
    assert_eq!((total, page), (1, vec![(12, 5, 5)]), "never more than all");
}

#[test]
fn library_counts_match_by_id_hash_or_saved_name_but_never_by_title_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");
    let dir = cache.join("ra");
    // By hash (Zelda, played: 2 of 3).
    let zelda_rom = tmp.path().join("roms/nes/Zelda.nes");
    write(&zelda_rom, &[b"NES\x1a".as_slice(), &pat(12, 2), &pat(40960, 3)].concat());
    let mut zelda = nes_game(zelda_rom);
    zelda.id = 1;
    // By the id set by hand (never played: none of the list's 30).
    let mut manual = nes_game(tmp.path().join("roms/nes/whatever.nes"));
    write(&manual.emu.as_ref().unwrap().rom.clone(), &pat(100, 1));
    manual.id = 2;
    manual.meta.ra_game_id = Some(1446);
    // By its file name, once a game page has saved the game's file names.
    let smb_rom = tmp.path().join("roms/nes/Super Mario Bros. (World).nes");
    write(&smb_rom, &pat(4096, 99));
    let mut smb = nes_game(smb_rom);
    smb.id = 3;
    smb.name = "Super Mario Bros.".into();
    // The same title, another file: a title-only match never counts in the library.
    let hack_rom = tmp.path().join("roms/nes/SMB (hacked).nes");
    write(&hack_rom, &pat(4096, 98));
    let mut hack = nes_game(hack_rom);
    hack.id = 4;
    hack.name = "Super Mario Bros".into();
    let mock = Arc::new(Mock::default().on("API_GetGameList", vec![ok(GAME_LIST)]));
    let c = client(mock.as_ref());
    let done = bulk::Completion { fetched: 0, games: [(1995, (2, 3))].into() };
    let games = [zelda, manual, smb.clone(), hack];
    let count = |hash_new: bool| {
        let mut lists = bulk::Lists::new(&c, &dir);
        let mut hashes = hash::HashCache::load(&dir);
        bulk::library_counts(&games, &dir, &mut lists, &mut hashes, &done, hash_new, &|| true)
    };
    // First pass: only hashes known already (none) — the id set by hand counts at once.
    let first = count(false);
    assert_eq!(first, [(2, (0, 30))].into());
    // With hashing: Zelda too.
    let second = count(true);
    assert_eq!(second, [(1, (2, 3)), (2, (0, 30))].into());
    // A game page looked Super Mario Bros. up (and saved its file names): now it counts.
    let full = Arc::new(full_mock());
    run(full, &cache, &smb, cfg(), false);
    let third = count(false);
    assert_eq!(third.get(&3), Some(&(0, 30)));
    assert!(!third.contains_key(&4), "title-only match: game page only");
    // The library pass asks only for console lists (once per console), never per game.
    assert_eq!(mock.urls().len(), 1, "{:?}", mock.urls());
    assert!(mock.urls().iter().all(|u| u.contains("API_GetGameList")));
}

#[test]
fn a_title_only_match_is_marked_probable_and_a_hash_match_is_not() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");
    let rom = tmp.path().join("roms/nes/SMB (hacked).nes");
    write(&rom, &pat(4096, 98));
    let mut g = nes_game(rom);
    g.name = "Super Mario Bros".into();
    let progress = PROGRESS_1995.replace("\"ID\":1995", "\"ID\":1446");
    let mock = Arc::new(Mock::default().on("GetGameInfoAndUserProgress.php?g=1446", vec![ok(&progress)]));
    for (k, v) in std::mem::take(&mut *full_mock().answers.lock()) {
        mock.answers.lock().push((k, v));
    }
    let State::Ready(p) = run(mock.clone(), &cache, &g, cfg(), false) else { panic!() };
    assert!(p.probable);
    assert_eq!(crate::achievements::summary::Summary::of(&p), None);
    let zelda_rom = tmp.path().join("roms/nes/Zelda.nes");
    write(&zelda_rom, &[b"NES\x1a".as_slice(), &pat(12, 2), &pat(40960, 3)].concat());
    let State::Ready(p) = run(mock, &cache, &nes_game(zelda_rom), cfg(), false) else { panic!() };
    assert!(!p.probable);
    assert!(crate::achievements::summary::Summary::of(&p).is_some());
}
