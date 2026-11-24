use super::net::*;
use super::sources::*;
use super::worker::*;
use super::*;
use std::collections::HashMap;

macro_rules! fixture {
    ($name:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/art/", $name)).as_slice()
    };
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(w, h, image::Rgb([200, 30, 30]));
    let mut out = std::io::Cursor::new(vec![]);
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn jpg(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(w, h, image::Rgb([20, 30, 200]));
    let mut out = std::io::Cursor::new(vec![]);
    img.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
    out.into_inner()
}

const SVG: &[u8] = br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="10" height="20"><rect width="10" height="20" fill="red"/></svg>"#;

// ---------------------------------------------------------------------------------------------
// Parsing

#[test]
fn storesearch_parses_apps() {
    let hits = parse_storesearch(fixture!("storesearch_elden_ring.json")).unwrap();
    assert_eq!(hits[0], StoreHit { appid: 1245620, name: "ELDEN RING".into() });
    assert!(hits.iter().any(|h| h.appid == 2622380 && h.name == "ELDEN RING NIGHTREIGN"));
    assert_eq!(parse_storesearch(fixture!("storesearch_empty.json")).unwrap(), vec![]);
    assert_eq!(parse_storesearch(b"null"), None);
    assert_eq!(parse_storesearch(b"<html>"), None);
    // Non-app items and shortcut-like ids are skipped.
    let body = br#"{"items":[{"type":"sub","id":5,"name":"Bundle"},{"type":"app","id":4294967295,"name":"X"},{"id":7,"name":"No type"}]}"#;
    assert_eq!(parse_storesearch(body).unwrap(), vec![StoreHit { appid: 7, name: "No type".into() }]);
}

#[test]
fn get_items_parses_plain_and_hashed_assets() {
    let m = parse_get_items(fixture!("getitems.json")).unwrap();
    let er = m[&1245620].as_ref().unwrap();
    assert_eq!(er.name, "ELDEN RING");
    assert_eq!(er.base, format!("{STEAM_CDN}/1245620/"));
    assert_eq!(er.url("library_capsule_2x").unwrap(), format!("{STEAM_CDN}/1245620/library_600x900_2x.jpg"));
    assert_eq!(er.community_icon.as_deref(), Some("b6e290dd5a92ce98f89089a207733c70c41a1871"));
    let nr = m[&2622380].as_ref().unwrap();
    assert!(
        nr.url("library_capsule_2x")
            .unwrap()
            .ends_with("/2622380/d1ee7bc631966b1a99e232a9f84af736bb298395/library_600x900_2x.jpg")
    );
    assert_eq!(m[&99999999], None, "unknown app");
    assert_eq!(parse_get_items(b"{}"), None);
}

#[test]
fn get_items_rejects_odd_paths() {
    let body = br#"{"response":{"store_items":[{"id":5,"success":1,"name":"X","assets":{"asset_url_format":"https://evil/${FILENAME}","header":"../x.jpg","library_hero":"abc/library_hero.jpg","main_capsule":"a b.jpg"}}]}}"#;
    let a = parse_get_items(body).unwrap()[&5].clone().unwrap();
    assert_eq!(a.base, format!("{STEAM_CDN}/5/"));
    assert_eq!(a.files.len(), 1);
    assert_eq!(a.url("library_hero").unwrap(), format!("{STEAM_CDN}/5/abc/library_hero.jpg"));
}

#[test]
fn steam_urls_prefer_listed_assets_then_plain_paths() {
    let m = parse_get_items(fixture!("getitems.json")).unwrap();
    let nr = m[&2622380].clone().unwrap();
    let cover = steam_urls(2622380, Some(&nr), ArtSlot::Cover);
    assert!(cover[0].url.contains("d1ee7bc6") && cover[0].url.ends_with("library_600x900_2x.jpg"));
    assert!(cover[0].thumb.as_deref().unwrap().ends_with("library_600x900.jpg"));
    assert_eq!((cover[0].width, cover[0].height), (600, 900));
    assert_eq!(cover.last().unwrap().url, format!("{STEAM_CDN}/2622380/library_600x900.jpg"));
    let hero = steam_urls(2622380, Some(&nr), ArtSlot::Hero);
    assert_eq!((hero[0].width, hero[0].height), (3840, 1240));
    // No asset list: the classic paths only, no duplicates.
    let plain = steam_urls(10, None, ArtSlot::Banner);
    assert_eq!(plain, vec![UrlOpt { url: format!("{STEAM_CDN}/10/header.jpg"), thumb: None, width: 460, height: 215 }]);
    assert!(steam_urls(10, None, ArtSlot::Icon).is_empty());
    let er = m[&1245620].clone().unwrap();
    let icon = steam_urls(1245620, Some(&er), ArtSlot::Icon);
    assert_eq!(icon[0].url, format!("{STEAM_ICON_CDN}/1245620/b6e290dd5a92ce98f89089a207733c70c41a1871.jpg"));
    let logo = steam_urls(1245620, Some(&er), ArtSlot::Logo);
    assert_eq!(logo.iter().map(|o| o.url.rsplit('/').next().unwrap()).collect::<Vec<_>>(), ["logo_2x.png", "logo.png"]);
}

#[test]
fn appdetails_header() {
    let h = parse_appdetails_header(fixture!("appdetails_2622380.json"), 2622380).unwrap().unwrap();
    assert_eq!(h, format!("{STEAM_CDN}/2622380/header.jpg").replace("shared.", "shared.akamai."));
    assert!(!h.contains('?'));
    assert_eq!(parse_appdetails_header(b"null", 5), None);
    assert_eq!(parse_appdetails_header(br#"{"5":{"success":false}}"#, 5), Some(None));
}

#[test]
fn sgdb_parsing() {
    let games = parse_sgdb_games(fixture!("sgdb_autocomplete.json")).unwrap();
    assert_eq!(
        games,
        vec![SgdbGame { id: 5254936, name: "RuneLite".into() }, SgdbGame { id: 12, name: "RuneScape".into() }]
    );
    let a = parse_sgdb_assets(fixture!("sgdb_grids.json")).unwrap();
    assert_eq!(a.len(), 2, "nsfw, video and non-https are dropped: {a:?}");
    assert_eq!(a[0].url, "https://cdn2.steamgriddb.com/grid/aaa.png");
    assert_eq!(a[0].thumb.as_deref(), Some("https://cdn2.steamgriddb.com/thumb/aaa.jpg"));
    assert_eq!((a[0].width, a[0].height, a[0].style.as_str()), (600, 900, "alternate"));
    assert_eq!(a[1].thumb, None);
    assert_eq!(parse_sgdb_assets(fixture!("sgdb_not_found.json")), None);
    assert_eq!(parse_sgdb_games(b"garbage"), None);
}

#[test]
fn urls_are_encoded() {
    assert_eq!(
        storesearch_url(" Football Life 2026 "),
        format!("{STORE_SEARCH}?term=Football%20Life%202026&l=english&cc=US")
    );
    let u = get_items_url(&[1, 2]);
    assert!(u.starts_with(GET_ITEMS) && u.contains("%7B%22appid%22%3A1%7D") && !u.contains(' '));
    assert_eq!(sgdb_search_url("Half-Life: Alyx"), format!("{SGDB_API}/search/autocomplete/Half-Life%3A%20Alyx"));
    assert_eq!(
        sgdb_assets_url("game/5", ArtSlot::Cover),
        format!("{SGDB_API}/grids/game/5?dimensions=600x900,342x482,660x930&types=static&nsfw=false")
    );
    assert!(sgdb_assets_url("steam/1", ArtSlot::Banner).contains("dimensions=920x430,460x215"));
    assert!(sgdb_assets_url("steam/1", ArtSlot::Icon).starts_with(&format!("{SGDB_API}/icons/steam/1?")));
}

#[test]
fn hosts_are_classified() {
    assert_eq!(Host::of("https://store.steampowered.com/api/x"), Host::Store);
    assert_eq!(Host::of("https://api.steampowered.com/I/x"), Host::SteamApi);
    assert_eq!(Host::of("https://shared.steamstatic.com/a"), Host::SteamCdn);
    assert_eq!(Host::of("https://www.steamgriddb.com/api/v2/x"), Host::Sgdb);
    assert_eq!(Host::of("https://cdn2.steamgriddb.com/grid/x.png"), Host::SgdbCdn);
    assert_eq!(Host::of("https://example.com:8080/x"), Host::Other);
    assert!(USER_AGENT.starts_with("Lodestar/"));
}

// ---------------------------------------------------------------------------------------------
// Matching

#[test]
fn names_match_only_when_normalised_equal() {
    assert_eq!(norm_name("Assassin's Creed® II"), "assassins creed ii");
    assert!(same_name("Half-Life: Alyx", "Half Life Alyx"));
    assert!(same_name("ELDEN RING™", "Elden Ring"));
    assert!(same_name("Game (2019)", "Game"));
    assert!(same_name("Ratchet & Clank", "Ratchet and Clank"));
    assert!(!same_name("ELDEN RING", "ELDEN RING NIGHTREIGN"));
    assert!(!same_name("", ""));
    assert!(!same_name("Portal", "Portal 2"));
    let hits = parse_storesearch(fixture!("storesearch_elden_ring.json")).unwrap();
    assert_eq!(confident_steam("Elden Ring", &hits), Some(1245620));
    assert_eq!(confident_steam("Elden Ring Nightreign", &hits), Some(2622380));
    assert_eq!(confident_steam("ELDEN", &hits), None);
    assert_eq!(confident_steam("Football Life 2026", &[]), None);
    let games = parse_sgdb_games(fixture!("sgdb_autocomplete.json")).unwrap();
    assert_eq!(confident_sgdb("RuneLite", &games), Some(5254936));
    assert_eq!(confident_sgdb("Rune", &games), None);
}

#[test]
fn store_appids_exclude_shortcuts() {
    assert_eq!(store_appid(1245620), Some(1245620));
    assert_eq!(store_appid(0), None);
    assert_eq!(store_appid(0x8000_0001), None);
    let mut g = game(1, "X");
    g.source = Source::Steam;
    g.source_id = "1245620".into();
    assert_eq!(steam_appid(&g), Some(1245620));
    g.source_id = "shortcut:3000000000".into();
    assert_eq!(steam_appid(&g), None);
    g.launch = Some(LaunchSpec::Steam { appid: 3_000_000_000 });
    assert_eq!(steam_appid(&g), None);
}

#[test]
fn search_job_query_overrides() {
    let mut g = game(3, "My Game");
    g.source = Source::Steam;
    g.source_id = "70".into();
    let j = SearchJob::new(&g, ArtSlot::Cover, None);
    assert_eq!((j.name.as_str(), j.appid), ("My Game", Some(70)));
    let j = SearchJob::new(&g, ArtSlot::Cover, Some("  Half-Life  "));
    assert_eq!((j.name.as_str(), j.appid), ("Half-Life", None));
    let j = SearchJob::new(&g, ArtSlot::Cover, Some("220"));
    assert_eq!(j.appid, Some(220));
    let j = SearchJob::new(&g, ArtSlot::Cover, Some("steam:400"));
    assert_eq!(j.appid, Some(400));
    let j = SearchJob::new(&g, ArtSlot::Cover, Some("   "));
    assert_eq!(j.appid, Some(70));
}

// ---------------------------------------------------------------------------------------------
// Files

#[test]
fn file_naming() {
    let d = Path::new("/a/art");
    assert_eq!(user_art_path(d, 42, ArtSlot::Hero, "png"), Path::new("/a/art/user/42/hero.png"));
    assert_eq!(user_art_path(d, 42, ArtSlot::Cover, "jpg"), Path::new("/a/art/user/42/cover.jpg"));
    assert!(is_user_art_file(d, 42, Path::new("/a/art/user/42/logo.webp")));
    assert!(!is_user_art_file(d, 42, Path::new("/a/art/user/4/logo.webp")));
    assert!(!is_user_art_file(d, 42, Path::new("/home/me/Pictures/logo.png")));
    assert_eq!(ArtSlot::ALL.map(ArtSlot::key), ["cover", "hero", "logo", "icon", "banner"]);
}

#[test]
fn validate_accepts_images_and_rejects_others() {
    assert_eq!(validate_image(&png(3, 4)), Ok("png"));
    assert_eq!(validate_image(&jpg(8, 8)), Ok("jpg"));
    assert_eq!(validate_image(SVG), Ok("svg"));
    assert!(validate_image(b"hello world").is_err());
    assert!(validate_image(b"").is_err());
    let mut broken = png(50, 50);
    broken.truncate(40);
    assert!(validate_image(&broken).is_err(), "truncated PNG");
    assert!(validate_image(b"<svg nonsense").is_err());
}

#[test]
fn import_local_good_and_bad_files() {
    let tmp = tempfile::tempdir().unwrap();
    let art = tmp.path().join("art");
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    // Extension kept.
    std::fs::write(src.join("box.PNG"), png(6, 9)).unwrap();
    let p = import_local(&art, 7, ArtSlot::Cover, &src.join("box.PNG")).unwrap();
    assert_eq!(p, art.join("user/7/cover.png"));
    assert_eq!(std::fs::read(&p).unwrap(), png(6, 9));
    std::fs::write(src.join("bg.jpeg"), jpg(16, 5)).unwrap();
    assert_eq!(import_local(&art, 7, ArtSlot::Hero, &src.join("bg.jpeg")).unwrap(), art.join("user/7/hero.jpeg"));
    // Wrong extension: the real format wins.
    std::fs::write(src.join("logo.jpg"), png(4, 1)).unwrap();
    assert_eq!(import_local(&art, 7, ArtSlot::Logo, &src.join("logo.jpg")).unwrap(), art.join("user/7/logo.png"));
    std::fs::write(src.join("icon.svg"), SVG).unwrap();
    assert_eq!(import_local(&art, 7, ArtSlot::Icon, &src.join("icon.svg")).unwrap(), art.join("user/7/icon.svg"));
    // Re-importing the stored file itself is a no-op.
    assert_eq!(import_local(&art, 7, ArtSlot::Cover, &p).unwrap(), p);
    // Bad files.
    std::fs::write(src.join("notes.png"), b"not an image").unwrap();
    let e = import_local(&art, 7, ArtSlot::Banner, &src.join("notes.png")).unwrap_err();
    assert!(format!("{e:#}").contains("not a supported image"), "{e:#}");
    assert!(import_local(&art, 7, ArtSlot::Banner, &src.join("missing.png")).is_err());
    assert!(import_local(&art, 7, ArtSlot::Banner, &src).is_err(), "directory");
    assert!(
        !art.join("user/7")
            .read_dir()
            .unwrap()
            .any(|e| { e.unwrap().file_name().to_string_lossy().starts_with("banner") })
    );
    // No temp files left behind.
    assert!(
        !art.join("user/7").read_dir().unwrap().any(|e| e.unwrap().file_name().to_string_lossy().ends_with(".tmp"))
    );
}

#[test]
fn db_user_art_slot_merges() {
    let mut db = crate::db::Db::open_memory().unwrap();
    let id = db
        .add_manual("X", LaunchSpec::Url("x://".into()), Art { hero: Some("/h.png".into()), ..Default::default() })
        .unwrap();
    db.set_user_art_slot(id, |a| ArtSlot::Cover.set(a, Some("/c.png".into()))).unwrap();
    let ua = db.user_art(id).unwrap();
    assert_eq!((ua.cover.as_deref(), ua.hero.as_deref()), (Some(Path::new("/c.png")), Some(Path::new("/h.png"))));
    db.set_user_art_slot(id, |a| ArtSlot::Hero.set(a, None)).unwrap();
    assert_eq!(db.user_art(id).unwrap().hero, None);
    assert_eq!(db.get(id).unwrap().unwrap().art.cover.as_deref(), Some(Path::new("/c.png")));
}

// ---------------------------------------------------------------------------------------------
// Bulk option logic

fn game(id: GameId, name: &str) -> Game {
    let mut db = crate::db::Db::open_memory().unwrap();
    let real = db.add_manual(name, LaunchSpec::Url("x://".into()), Art::default()).unwrap();
    let mut g = db.get(real).unwrap().unwrap();
    g.id = id;
    g
}

fn avail(names: &[(&str, bool)]) -> Vec<SourceInfo> {
    names
        .iter()
        .map(|(n, a)| SourceInfo {
            name: n.to_string(),
            available: *a,
            reason: None,
            kind: SourceKind::from_name(n).unwrap_or(SourceKind::Steam),
            enabled: true,
        })
        .collect()
}

/// The extra artwork sources switched off, so the older tests see Steam + SteamGridDB only.
fn legacy_off() -> Vec<String> {
    SourceKind::ALL
        .iter()
        .filter(|k| !matches!(k, SourceKind::Steam | SourceKind::SteamGridDb))
        .map(|k| k.name().to_string())
        .collect()
}

fn legacy_cfg() -> MetadataConfig {
    MetadataConfig { sources_off: legacy_off(), ..Default::default() }
}

/// auto_request with the Steam / SteamGridDB switches of the older tests.
fn ar(g: &Game, tried: u16, steam: bool, sgdb: bool) -> Option<AutoRequest> {
    let mut s = vec![];
    if steam {
        s.push(SourceKind::Steam);
    }
    if sgdb {
        s.push(SourceKind::SteamGridDb);
    }
    crate::art_app::auto_request(g, tried, &s)
}

#[test]
fn bulk_slots_only_missing_vs_replace() {
    let tmp = tempfile::tempdir().unwrap();
    let have = tmp.path().join("c.png");
    std::fs::write(&have, png(2, 3)).unwrap();
    let mut g = game(1, "G");
    g.art.cover = Some(have);
    g.art.hero = Some(tmp.path().join("gone.jpg")); // file vanished → missing
    let opts = BulkOptions {
        slots: vec![ArtSlot::Cover, ArtSlot::Hero, ArtSlot::Logo, ArtSlot::Cover],
        only_missing: true,
        sources: vec![],
    };
    assert_eq!(slots_to_fill(&g, &opts), vec![ArtSlot::Hero, ArtSlot::Logo]);
    let opts = BulkOptions { only_missing: false, ..opts };
    assert_eq!(slots_to_fill(&g, &opts), vec![ArtSlot::Cover, ArtSlot::Hero, ArtSlot::Logo]);
    assert_eq!(BulkOptions::default().slots.len(), 5);
    assert!(BulkOptions::default().only_missing);
}

#[test]
fn bulk_sources_respect_order_and_availability() {
    let both = avail(&[(SRC_STEAM, true), (SRC_SGDB, true)]);
    let no_key = avail(&[(SRC_STEAM, true), (SRC_SGDB, false)]);
    assert_eq!(bulk_sources(&[], &both), vec![SourceKind::Steam, SourceKind::SteamGridDb]);
    assert_eq!(bulk_sources(&[], &no_key), vec![SourceKind::Steam]);
    let req = vec!["steamgriddb".to_string(), "Steam".into(), "Steam".into(), "TheGamesDB".into()];
    assert_eq!(bulk_sources(&req, &both), vec![SourceKind::SteamGridDb, SourceKind::Steam]);
    assert_eq!(bulk_sources(&[SRC_SGDB.into()], &no_key), vec![]);
}

#[test]
fn bulk_summary_text() {
    let mut b = crate::art_app::BulkStatus {
        total: 5,
        result: Some((3, 1, false)),
        not_found: vec!["A".into()],
        ..Default::default()
    };
    assert_eq!(crate::art_app::bulk_summary(&b), "Artwork: 3 games updated, 1 not found, 1 failed");
    b.result = Some((1, 0, true));
    b.not_found.clear();
    assert_eq!(crate::art_app::bulk_summary(&b), "Artwork: 1 game updated (cancelled)");
    b.total = 0;
    assert!(crate::art_app::bulk_summary(&b).contains("already has"));
}

// ---------------------------------------------------------------------------------------------
// Engine with a fake network

#[derive(Default)]
struct FakeNet {
    routes: HashMap<String, Result<(u16, Vec<u8>), String>>,
    calls: Calls,
}

type Calls = std::sync::Arc<parking_lot::Mutex<Vec<(String, Option<String>)>>>;

fn urls(c: &Calls) -> Vec<String> {
    c.lock().iter().map(|c| c.0.clone()).collect()
}

impl FakeNet {
    fn ok(mut self, url: &str, body: &[u8]) -> Self {
        self.routes.insert(url.into(), Ok((200, body.to_vec())));
        self
    }
    fn status(mut self, url: &str, s: u16) -> Self {
        self.routes.insert(url.into(), Ok((s, vec![])));
        self
    }
    fn net_err(mut self, url: &str) -> Self {
        self.routes.insert(url.into(), Err("connection refused".into()));
        self
    }
}

impl Net for FakeNet {
    fn get(&self, url: &str, bearer: Option<&str>, _limit: u64) -> Result<HttpResponse, String> {
        self.calls.lock().push((url.into(), bearer.map(str::to_string)));
        match self.routes.get(url) {
            Some(Ok((status, body))) => Ok(HttpResponse {
                status: *status,
                body: if *status == 200 { body.clone() } else { vec![] },
                ..Default::default()
            }),
            Some(Err(e)) => Err(e.clone()),
            None => Ok(HttpResponse { status: 404, ..Default::default() }),
        }
    }
    fn head(&self, url: &str) -> Result<u16, String> {
        self.calls.lock().push((format!("HEAD {url}"), None));
        match self.routes.get(url) {
            Some(Ok((status, _))) => Ok(*status),
            Some(Err(e)) => Err(e.clone()),
            None => Ok(404),
        }
    }
}

fn engine(net: FakeNet, dir: &Path, key: Option<&str>) -> Engine {
    let cfg = MetadataConfig {
        steam_store: true,
        steamgriddb_key: key.map(str::to_string),
        sources_off: legacy_off(),
        ..Default::default()
    };
    Engine::new(Box::new(net), Throttle::none(), dir.join("art"), dir.join("thumbs"), &cfg)
}

fn collect(e: &Engine, job: &SearchJob) -> (Vec<Candidate>, Vec<String>, usize) {
    let (mut items, mut errors, mut batches) = (vec![], vec![], 0);
    e.search(job, |i, _done, er| {
        batches += 1;
        items.extend(i);
        errors.extend(er);
        true
    });
    (items, errors, batches)
}

const GI_ER: &str = "1245620";

fn gi_url(ids: &[u32]) -> String {
    get_items_url(ids)
}

#[test]
fn steam_search_by_appid_uses_existing_files_only() {
    let tmp = tempfile::tempdir().unwrap();
    let cover = format!("{STEAM_CDN}/{GI_ER}/library_600x900_2x.jpg");
    let net = FakeNet::default()
        .ok(&gi_url(&[1245620]), fixture!("getitems.json"))
        .status(&cover, 200)
        .status(&format!("{STEAM_CDN}/{GI_ER}/library_600x900.jpg"), 200)
        .status(&format!("{STEAM_CDN}/{GI_ER}/logo.png"), 200);
    let log = net.calls.clone();
    let e = engine(net, tmp.path(), None);
    let mut g = game(9, "ELDEN RING");
    g.source = Source::Steam;
    g.source_id = GI_ER.into();
    let (items, errors, batches) = collect(&e, &SearchJob::new(&g, ArtSlot::Cover, None));
    assert_eq!(batches, 1, "no SteamGridDB key → one batch");
    assert!(errors.is_empty());
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].url, cover);
    assert_eq!(items[0].thumb_url.as_deref().unwrap(), format!("{STEAM_CDN}/{GI_ER}/library_600x900.jpg"));
    assert_eq!(items[0].label, "Steam · ELDEN RING");
    // Logo: logo_2x.png 404s, logo.png exists.
    let (items, _, _) = collect(&e, &SearchJob::new(&g, ArtSlot::Logo, None));
    assert_eq!(items[0].url, format!("{STEAM_CDN}/{GI_ER}/logo.png"));
    // Hero: nothing exists → no candidate.
    let (items, _, _) = collect(&e, &SearchJob::new(&g, ArtSlot::Hero, None));
    assert!(items.is_empty());
    // Session caches: GetItems asked once, each HEAD once.
    let _ = collect(&e, &SearchJob::new(&g, ArtSlot::Cover, None));
    let calls = urls(&log);
    assert_eq!(calls.iter().filter(|c| c.contains("GetItems")).count(), 1, "{calls:#?}");
    assert_eq!(calls.iter().filter(|c| *c == &format!("HEAD {cover}")).count(), 1);
}

#[test]
fn hashed_banner_falls_back_to_appdetails() {
    let tmp = tempfile::tempdir().unwrap();
    let hdr = "https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/2622380/header.jpg";
    let net = FakeNet::default()
        .status(&gi_url(&[2622380]), 500) // GetItems down: classic paths, then appdetails
        .ok(&appdetails_url(2622380), fixture!("appdetails_2622380.json"))
        .status(hdr, 200);
    let e = engine(net, tmp.path(), None);
    let c = e.steam_candidate(2622380, "NR", ArtSlot::Banner, Req::UI).unwrap().unwrap();
    assert_eq!(c.url, hdr);
    assert_eq!(c.label, "Steam · NR");
}

#[test]
fn name_search_without_matches_gives_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let net = FakeNet::default().ok(&storesearch_url("Football Life 2026"), fixture!("storesearch_empty.json"));
    let e = engine(net, tmp.path(), None);
    let (items, errors, _) = collect(&e, &SearchJob::new(&game(1, "Football Life 2026"), ArtSlot::Cover, None));
    assert!(items.is_empty() && errors.is_empty());
}

#[test]
fn name_search_offers_top_hits_exact_first() {
    let tmp = tempfile::tempdir().unwrap();
    let hits = parse_storesearch(fixture!("storesearch_elden_ring.json")).unwrap();
    let ids: Vec<u32> = hits.iter().take(TOP_HITS).map(|h| h.appid).collect();
    let mut net =
        FakeNet::default().ok(&storesearch_url("elden ring nightreign"), fixture!("storesearch_elden_ring.json"));
    for id in &ids {
        net = net.status(&format!("{STEAM_CDN}/{id}/header.jpg"), 200);
    }
    let _ = gi_url(&ids); // GetItems not routed → 404 → classic paths
    let e = engine(net, tmp.path(), None);
    let (items, _, _) = collect(&e, &SearchJob::new(&game(1, "elden ring nightreign"), ArtSlot::Banner, None));
    assert_eq!(items.len(), 3);
    assert!(items[0].url.contains("/2622380/"), "exact match first: {items:#?}");
    assert_eq!(items[0].label, "Steam · ELDEN RING NIGHTREIGN");
}

#[test]
fn network_errors_are_reported_not_fatal() {
    let tmp = tempfile::tempdir().unwrap();
    let net = FakeNet::default().net_err(&storesearch_url("X"));
    let e = engine(net, tmp.path(), None);
    let (items, errors, _) = collect(&e, &SearchJob::new(&game(1, "X"), ArtSlot::Cover, None));
    assert!(items.is_empty());
    assert_eq!(errors, vec!["Steam: connection refused".to_string()]);
}

#[test]
fn sgdb_search_with_key_and_rejection() {
    let tmp = tempfile::tempdir().unwrap();
    let net = FakeNet::default()
        .ok(&storesearch_url("RuneLite"), fixture!("storesearch_empty.json"))
        .ok(&sgdb_search_url("RuneLite"), fixture!("sgdb_autocomplete.json"))
        .ok(&sgdb_assets_url("game/5254936", ArtSlot::Cover), fixture!("sgdb_grids.json"))
        .ok(&sgdb_assets_url("game/12", ArtSlot::Cover), fixture!("sgdb_grids.json"));
    let log = net.calls.clone();
    let e = engine(net, tmp.path(), Some(" key123 "));
    assert!(e.sources().iter().filter(|s| s.enabled).all(|s| s.available));
    let (items, errors, batches) = collect(&e, &SearchJob::new(&game(1, "RuneLite"), ArtSlot::Cover, None));
    assert_eq!(batches, 2);
    assert!(errors.is_empty(), "{errors:?}");
    // 2 games × 2 usable grids; urls repeat in this fixture but labels name the game.
    assert_eq!(items.len(), 4);
    assert_eq!(items[0].source, SRC_SGDB);
    assert_eq!(items[0].label, "SteamGridDB · RuneLite · alternate");
    assert_eq!(items[0].thumb_url.as_deref(), Some("https://cdn2.steamgriddb.com/thumb/aaa.jpg"));
    let calls = log.lock().clone();
    assert!(calls.iter().filter(|c| c.0.starts_with(SGDB_API)).all(|c| c.1.as_deref() == Some("key123")));
    assert!(calls.iter().filter(|c| !c.0.starts_with(SGDB_API)).all(|c| c.1.is_none()), "key only sent to SteamGridDB");

    // A rejected key: reported, source becomes unavailable.
    let net = FakeNet::default().status(&sgdb_search_url("Y"), 401);
    let e = engine(net, tmp.path(), Some("bad"));
    let (_, errors, _) = collect(&e, &SearchJob::new(&game(1, "Y"), ArtSlot::Cover, None));
    assert!(errors.iter().any(|e| e.starts_with("SteamGridDB: the API key was rejected")), "{errors:?}");
    let s = e.sources();
    assert!(!s[1].available && s[1].reason.as_deref().unwrap().contains("rejected"));
    // Changing the key makes it available again.
    e.set_config(&MetadataConfig {
        steam_store: true,
        steamgriddb_key: Some("new".into()),
        sources_off: legacy_off(),
        ..Default::default()
    });
    assert!(e.sources()[1].available);
    e.set_config(&legacy_cfg());
    assert!(e.sources()[1].reason.as_deref().unwrap().contains("API key"));
}

#[test]
fn apply_downloads_validates_and_reuses_thumbs() {
    let tmp = tempfile::tempdir().unwrap();
    let img = "https://cdn2.steamgriddb.com/grid/aaa.png";
    let txt = "https://example.com/page.html";
    let net = FakeNet::default().ok(img, &png(6, 9)).ok(txt, b"<html>hi</html>");
    let log = net.calls.clone();
    let e = engine(net, tmp.path(), None);
    let p = e.apply(3, ArtSlot::Cover, img).unwrap();
    assert_eq!(p, tmp.path().join("art/user/3/cover.png"));
    assert!(e.apply(3, ArtSlot::Cover, txt).unwrap_err().contains("isn't an image"));
    assert!(e.apply(3, ArtSlot::Cover, "https://example.com/404.png").unwrap_err().contains("HTTP 404"));
    assert!(e.apply(3, ArtSlot::Cover, "https://example.com/404.png").is_err());
    // A thumbnail of the full URL is reused instead of downloading again.
    let t = e.thumb(img).unwrap();
    assert!(t.starts_with(tmp.path().join("thumbs")) && t.extension().unwrap() == "png");
    let n0 = log.lock().len();
    e.apply(4, ArtSlot::Cover, img).unwrap();
    assert_eq!(log.lock().len(), n0);
    // Local paths and file:// URLs are imported.
    let local = tmp.path().join("my cover.jpg");
    std::fs::write(&local, jpg(4, 6)).unwrap();
    assert_eq!(e.apply(5, ArtSlot::Cover, local.to_str().unwrap()).unwrap(), tmp.path().join("art/user/5/cover.jpg"));
    let url = format!("file://{}", local.to_str().unwrap().replace(' ', "%20"));
    assert_eq!(e.apply(6, ArtSlot::Banner, &url).unwrap(), tmp.path().join("art/user/6/banner.jpg"));
}

#[test]
fn bulk_fills_confident_matches_and_skips_the_rest() {
    let tmp = tempfile::tempdir().unwrap();
    let cover = format!("{STEAM_CDN}/1245620/library_600x900_2x.jpg");
    let net = FakeNet::default()
        .ok(&storesearch_url("ELDEN RING"), fixture!("storesearch_elden_ring.json"))
        .ok(&storesearch_url("Football Life 2026"), fixture!("storesearch_empty.json"))
        .ok(&storesearch_url("Elden"), fixture!("storesearch_elden_ring.json"))
        .ok(&gi_url(&[1245620]), fixture!("getitems.json"))
        .ok(&cover, &jpg(6, 9))
        .ok(&format!("{STEAM_CDN}/70/library_600x900_2x.jpg"), &jpg(6, 9))
        .ok(&format!("{STEAM_CDN}/70/library_hero.jpg"), &jpg(19, 6));
    let e = engine(net, tmp.path(), None);
    let mut steam = game(4, "Half-Life");
    steam.source = Source::Steam;
    steam.source_id = "70".into();
    let have = tmp.path().join("have.png");
    std::fs::write(&have, png(2, 3)).unwrap();
    steam.art.cover = Some(have);
    let games = vec![game(1, "ELDEN RING"), game(2, "Football Life 2026"), game(3, "Elden"), steam];
    let opts = BulkOptions { slots: vec![ArtSlot::Cover, ArtSlot::Hero], only_missing: true, sources: vec![] };
    let cancel = AtomicBool::new(false);
    let mut applied = vec![];
    let mut prog = vec![];
    let rep = e.bulk(&games, &opts, &cancel, &mut |d, t, _| prog.push((d, t)), &mut |g, s, p| applied.push((g, s, p)));
    assert_eq!(rep.changed, 2, "{rep:?}");
    assert_eq!(rep.failed, 0);
    assert_eq!(rep.not_found, vec!["Football Life 2026".to_string(), "Elden".into()]);
    assert!(!rep.cancelled);
    assert!(applied.contains(&(1, ArtSlot::Cover, tmp.path().join("art/user/1/cover.jpg"))));
    // Only missing: the Steam game keeps its cover, gets a hero.
    assert_eq!(applied.iter().filter(|a| a.0 == 4).map(|a| a.1).collect::<Vec<_>>(), vec![ArtSlot::Hero]);
    assert_eq!(prog.first(), Some(&(0, 4)));
    assert_eq!(prog.last(), Some(&(4, 4)));
}

#[test]
fn bulk_stops_on_cancel_and_repeated_network_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let mut net = FakeNet::default();
    for n in ["A", "B", "C", "D"] {
        net = net.net_err(&storesearch_url(n));
    }
    let e = engine(net, tmp.path(), None);
    let games: Vec<Game> = ["A", "B", "C", "D"].iter().enumerate().map(|(i, n)| game(i as i64 + 1, n)).collect();
    let cancel = AtomicBool::new(false);
    let rep = e.bulk(&games, &BulkOptions::default(), &cancel, &mut |_, _, _| {}, &mut |_, _, _| {});
    assert_eq!(rep.failed, 3);
    assert!(rep.error.as_deref().unwrap().contains("network"));
    cancel.store(true, Ordering::Relaxed);
    let rep = e.bulk(&games, &BulkOptions::default(), &cancel, &mut |_, _, _| {}, &mut |_, _, _| {});
    assert!(rep.cancelled && rep.failed == 0);
}

#[test]
fn service_search_events_and_supersede() {
    let tmp = tempfile::tempdir().unwrap();
    let net = FakeNet::default().ok(&storesearch_url("Football Life 2026"), fixture!("storesearch_empty.json"));
    let ctx = egui::Context::default();
    let mut svc =
        ArtService::with_net(&ctx, Box::new(net), tmp.path().join("art"), tmp.path().join("thumbs"), &legacy_cfg());
    let g = game(1, "Football Life 2026");
    svc.search(&g, ArtSlot::Cover, None);
    let mut evs = vec![];
    let t0 = std::time::Instant::now();
    while !evs.iter().any(|e| matches!(e, ArtEvent::Candidates { done: true, .. })) && t0.elapsed().as_secs() < 5 {
        evs.extend(svc.poll());
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(evs[0], ArtEvent::SearchStarted { game: 1, slot: ArtSlot::Cover });
    assert!(matches!(&evs[1], ArtEvent::Candidates { items, done: true, .. } if items.is_empty()));
    // Import through the worker.
    let f = tmp.path().join("x.png");
    std::fs::write(&f, png(2, 2)).unwrap();
    svc.import_file(1, ArtSlot::Icon, &f);
    svc.import_file(1, ArtSlot::Logo, &tmp.path().join("nope.png"));
    let mut got = vec![];
    let t0 = std::time::Instant::now();
    while got.len() < 2 && t0.elapsed().as_secs() < 5 {
        got.extend(svc.poll());
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        got[0],
        ArtEvent::Applied { game: 1, slot: ArtSlot::Icon, path: tmp.path().join("art/user/1/icon.png") }
    );
    assert!(matches!(&got[1], ArtEvent::Failed { slot: ArtSlot::Logo, .. }));
}

// ---------------------------------------------------------------------------------------------
// Live checks (network): cargo test art_live -- --ignored --nocapture

/// Engine on a temporary folder (set TMPDIR to choose where).
fn live_engine(key: Option<String>) -> (Engine, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let cfg =
        MetadataConfig { steam_store: true, steamgriddb_key: key, sources_off: legacy_off(), ..Default::default() };
    let e = Engine::new(
        Box::new(UreqNet::new()),
        Throttle::polite(),
        dir.path().join("art"),
        dir.path().join("thumbs"),
        &cfg,
    );
    (e, dir)
}

#[test]
#[ignore]
fn art_live_search() {
    let (e, _dir) = live_engine(std::env::var("LODESTAR_SGDB_KEY").ok());
    let mut er = game(1, "ELDEN RING");
    er.source = Source::Steam;
    er.source_id = "1245620".into();
    let cases = [
        ("ELDEN RING (Steam 1245620)", er, None),
        ("ELDEN RING NIGHTREIGN by name", game(2, "x"), Some("ELDEN RING NIGHTREIGN")),
        ("RuneLite", game(3, "RuneLite"), None),
        ("Football Life 2026", game(4, "Football Life 2026"), None),
    ];
    for (title, g, q) in cases {
        println!("== {title}");
        for slot in ArtSlot::ALL {
            let t0 = std::time::Instant::now();
            let job = SearchJob::new(&g, slot, q);
            let (items, errors, _) = collect(&e, &job);
            println!("  {:<10} {} candidates in {:?} {:?}", slot.label(), items.len(), t0.elapsed(), errors);
            for c in items.iter().take(4) {
                println!("     {} {}x{} {} (thumb {:?})", c.label, c.width, c.height, c.url, c.thumb_url);
            }
        }
        let name = q.unwrap_or(g.display_name());
        let hits = e.store_search(name, Req::UI).unwrap_or_default();
        println!(
            "  confident Steam match for {name:?}: {:?} (hits: {:?})",
            confident_steam(name, &hits),
            hits.iter().take(3).map(|h| &h.name).collect::<Vec<_>>()
        );
    }
}

#[test]
#[ignore]
fn art_live_bulk() {
    let (e, _dir) = live_engine(std::env::var("LODESTAR_SGDB_KEY").ok());
    let mut er = game(1, "ELDEN RING");
    er.source = Source::Steam;
    er.source_id = "1245620".into();
    let games = vec![er, game(2, "Portal 2"), game(3, "RuneLite"), game(4, "Football Life 2026")];
    let opts =
        BulkOptions { slots: vec![ArtSlot::Cover, ArtSlot::Hero, ArtSlot::Logo], only_missing: true, sources: vec![] };
    let cancel = AtomicBool::new(false);
    let t0 = std::time::Instant::now();
    let rep = e.bulk(&games, &opts, &cancel, &mut |d, t, c| println!("  progress {d}/{t} {c}"), &mut |g, s, p| {
        let ok = validate_image(&std::fs::read(&p).unwrap());
        println!("  applied game {g} {:<10} {} {:?}", s.label(), p.display(), ok);
    });
    println!("report {rep:?} in {:?}", t0.elapsed());
    assert_eq!(rep.changed, 2);
    assert_eq!(rep.not_found, vec!["RuneLite".to_string(), "Football Life 2026".into()]);
}

// ---------------------------------------------------------------------------------------------
// Automatic fill after a scan

fn search_body(hits: &[(u32, &str)]) -> Vec<u8> {
    let items: Vec<serde_json::Value> =
        hits.iter().map(|(id, n)| serde_json::json!({"type": "app", "id": id, "name": n})).collect();
    serde_json::to_vec(&serde_json::json!({"total": items.len(), "items": items})).unwrap()
}

#[test]
fn auto_request_picks_games_without_art_once_per_source() {
    let g = game(1, "Brawlhalla");
    let r = ar(&g, 0, true, false).unwrap();
    assert_eq!(r.sources, vec![SourceKind::Steam]);
    assert_eq!(r.need, ArtSlot::ALL.to_vec());
    assert_eq!(r.appid, None);
    // Both sources, then only the one not asked yet, then none.
    assert_eq!(ar(&g, 0, true, true).unwrap().sources, vec![SourceKind::Steam, SourceKind::SteamGridDb]);
    assert_eq!(ar(&g, 1, true, true).unwrap().sources, vec![SourceKind::SteamGridDb]);
    assert!(ar(&g, 1, true, false).is_none());
    assert!(ar(&g, 3, true, true).is_none());
    assert!(ar(&g, 0, false, false).is_none());
    // Only the slots nothing fills.
    let mut c = g.clone();
    c.art.cover = Some("/x/cover.jpg".into());
    c.art.logo = Some("/x/logo.png".into());
    assert_eq!(ar(&c, 0, true, false).unwrap().need, vec![ArtSlot::Hero, ArtSlot::Icon, ArtSlot::Banner]);
    // Cover and background present: left alone.
    c.art.hero = Some("/x/hero.jpg".into());
    assert!(ar(&c, 0, true, false).is_none());
    // Hidden (removed) games and Steam store games (the metadata fetcher's) are skipped.
    let mut h = g.clone();
    h.hidden = true;
    assert!(ar(&h, 0, true, true).is_none());
    let mut s = g.clone();
    s.source = Source::Steam;
    s.source_id = "291550".into();
    assert!(ar(&s, 0, true, true).is_none());
    // A non-Steam shortcut in Steam has no store appid: looked up by name.
    s.source_id = (0x8000_0000u32 | 5).to_string();
    assert!(ar(&s, 0, true, false).is_some());
}

#[test]
fn auto_fill_stores_confident_matches_in_the_auto_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let net = FakeNet::default()
        .ok(&storesearch_url("Brawlhalla"), &search_body(&[(291550, "Brawlhalla")]))
        .ok(&storesearch_url("Diablo II: Resurrected"), &search_body(&[(2344520, "Diablo® IV"), (2300, "DOOM II")]))
        .ok(&format!("{STEAM_CDN}/291550/library_600x900_2x.jpg"), &jpg(6, 9))
        .ok(&format!("{STEAM_CDN}/291550/library_hero.jpg"), &jpg(19, 6))
        .net_err(&storesearch_url("Offline Game"));
    let calls = net.calls.clone();
    let e = engine(net, tmp.path(), None);
    let req = |id, name: &str, need: Vec<ArtSlot>| AutoRequest {
        id,
        name: name.into(),
        appid: None,
        gog_id: None,
        platform: None,
        rom_names: vec![],
        serial: None,
        need,
        sources: vec![SourceKind::Steam, SourceKind::SteamGridDb],
    };
    let reqs = vec![
        req(1, "Brawlhalla", vec![ArtSlot::Cover, ArtSlot::Hero]),
        req(2, "Diablo II: Resurrected", vec![ArtSlot::Cover]),
        req(3, "Offline Game", vec![ArtSlot::Cover]),
    ];
    let cancel = AtomicBool::new(false);
    let mut got = vec![];
    let rep = e.auto_fill(&reqs, &cancel, &mut |g, a, t| got.push((g, a, t)));
    assert_eq!(rep.changed, 1);
    assert!(rep.error.is_none() && !rep.cancelled);
    // Brawlhalla: cover and hero; no key, so only Steam counts as asked.
    assert_eq!(got[0].0, 1);
    assert_eq!(got[0].1.cover, Some(tmp.path().join("art/auto/1/cover.jpg")));
    assert_eq!(got[0].1.hero, Some(tmp.path().join("art/auto/1/hero.jpg")));
    assert!(got[0].1.logo.is_none());
    assert_eq!(got[0].2, vec![SourceKind::Steam]);
    assert!(tmp.path().join("art/auto/1/cover.jpg").is_file());
    assert!(!tmp.path().join("art/user/1").exists());
    // No confident match: nothing, but Steam answered (not asked again).
    assert_eq!(got[1], (2, Art::default(), vec![SourceKind::Steam]));
    // Network error: not reported (retried another time).
    assert_eq!(got.len(), 2);
    assert!(!urls(&calls).iter().any(|u| u.contains("steamgriddb")));
    // Cancelled before the first game: nothing.
    cancel.store(true, Ordering::Relaxed);
    got.clear();
    let rep = e.auto_fill(&reqs, &cancel, &mut |g, a, t| got.push((g, a, t)));
    assert!(rep.cancelled && got.is_empty());
}

#[test]
fn store_match_retries_with_the_plain_words() {
    let tmp = tempfile::tempdir().unwrap();
    let name = "Nioh 2 – The Complete Edition";
    let net = FakeNet::default()
        .ok(&storesearch_url(name), &search_body(&[]))
        .ok(&storesearch_url("nioh 2 the complete edition"), &search_body(&[(1325200, name)]))
        .ok(&format!("{STEAM_CDN}/1325200/library_600x900_2x.jpg"), &jpg(6, 9));
    let e = engine(net, tmp.path(), None);
    let reqs = vec![AutoRequest {
        id: 7,
        name: name.into(),
        appid: None,
        gog_id: None,
        platform: None,
        rom_names: vec![],
        serial: None,
        need: vec![ArtSlot::Cover],
        sources: vec![SourceKind::Steam],
    }];
    let mut got = vec![];
    e.auto_fill(&reqs, &AtomicBool::new(false), &mut |g, a, _| got.push((g, a)));
    assert_eq!(got[0].1.cover, Some(tmp.path().join("art/auto/7/cover.jpg")));
}

#[test]
fn auto_fill_stops_when_offline() {
    let tmp = tempfile::tempdir().unwrap();
    let mut net = FakeNet::default();
    for n in ["A", "B", "C", "D"] {
        net = net.net_err(&storesearch_url(n));
    }
    let calls = net.calls.clone();
    let e = engine(net, tmp.path(), None);
    let reqs: Vec<AutoRequest> = ["A", "B", "C", "D"]
        .iter()
        .enumerate()
        .map(|(i, n)| AutoRequest {
            id: i as i64 + 1,
            name: n.to_string(),
            appid: None,
            gog_id: None,
            platform: None,
            rom_names: vec![],
            serial: None,
            need: vec![ArtSlot::Cover],
            sources: vec![SourceKind::Steam],
        })
        .collect();
    let mut got = 0;
    let rep = e.auto_fill(&reqs, &AtomicBool::new(false), &mut |_, _, _| got += 1);
    assert_eq!(got, 0);
    assert!(rep.error.as_deref().unwrap().contains("network"));
    assert_eq!(urls(&calls).len(), 3, "stops after 3 failures");
}

/// AppState with a fake network for the art service (the metadata fetcher stays offline).
fn auto_state(tmp: &Path, net: FakeNet) -> (crate::app::AppState, Calls) {
    let calls = net.calls.clone();
    let mut st = crate::app::AppState::for_test(tmp);
    st.config.metadata.steam_store = true;
    st.art = ArtService::with_net(
        st.ctx(),
        Box::new(net),
        st.paths.art_dir(),
        tmp.join("cache/art-thumbs"),
        &st.config.metadata,
    );
    (st, calls)
}

fn run_auto(st: &mut crate::app::AppState) {
    st.auto_art_after_scan();
    let ok = st.poll_until(std::time::Duration::from_secs(10), |s| !s.auto_art.running && !s.auto_art.wanted);
    assert!(ok, "auto fill did not finish");
}

#[test]
fn auto_fill_after_scan_fills_missing_art_once_and_keeps_existing() {
    let tmp = tempfile::tempdir().unwrap();
    let net = FakeNet::default()
        .ok(&storesearch_url("Brawlhalla"), &search_body(&[(291550, "Brawlhalla")]))
        .ok(&storesearch_url("The Sims 4"), &search_body(&[(1222670, "The Sims™ 4")]))
        .ok(&storesearch_url("Diablo II: Resurrected"), &search_body(&[(2344520, "Diablo® IV")]))
        .ok(&format!("{STEAM_CDN}/291550/library_600x900_2x.jpg"), &jpg(6, 9))
        .ok(&format!("{STEAM_CDN}/291550/library_hero.jpg"), &jpg(19, 6))
        .ok(&format!("{STEAM_CDN}/1222670/library_hero.jpg"), &jpg(19, 6))
        .ok(&format!("{STEAM_CDN}/1222670/library_600x900_2x.jpg"), &jpg(6, 9));
    let (mut st, calls) = auto_state(tmp.path(), net);

    // Not allowed (screenshot runs / tests by default): nothing happens.
    st.add_manual_game("Brawlhalla", LaunchSpec::Url("x://".into()), Art::default());
    run_auto(&mut st);
    assert!(urls(&calls).is_empty());

    st.auto_art.allowed = true;
    let own = tmp.path().join("own-cover.png");
    std::fs::write(&own, png(2, 3)).unwrap();
    st.add_manual_game(
        "The Sims 4",
        LaunchSpec::Url("x://".into()),
        Art { cover: Some(own.clone()), ..Default::default() },
    );
    st.add_manual_game("Diablo II: Resurrected", LaunchSpec::Url("x://".into()), Art::default());
    run_auto(&mut st);
    let by_name = |st: &crate::app::AppState, n: &str| st.library.games.iter().find(|g| g.name == n).unwrap().clone();
    let b = by_name(&st, "Brawlhalla");
    let dir = auto_art_dir(&st.paths.art_dir(), b.id);
    assert_eq!(b.art.cover, Some(dir.join("cover.jpg")));
    assert_eq!(b.art.hero, Some(dir.join("hero.jpg")));
    assert!(b.art.logo.is_none());
    // Stored as fetched art, not as a user choice.
    assert_eq!(st.db.user_art(b.id).unwrap(), Art::default());
    // The Sims 4 keeps its own cover, gets a background.
    let s = by_name(&st, "The Sims 4");
    assert_eq!(s.art.cover, Some(own));
    assert!(s.art.hero.is_some());
    assert!(!calls.lock().iter().any(|c| c.0.contains("1222670/library_600x900")));
    // Diablo II: Resurrected isn't on Steam: no art.
    let d = by_name(&st, "Diablo II: Resurrected");
    assert!(d.art.cover.is_none() && d.art.hero.is_none());
    assert!(st.toasts.iter().any(|t| t.text.contains("Found artwork for 2 games")));

    // Once per game: the next scan asks nothing again, also after a restart (kept in the DB).
    let n = urls(&calls).len();
    run_auto(&mut st);
    drop(st);
    let (mut st2, calls2) = auto_state(tmp.path(), FakeNet::default());
    st2.auto_art.allowed = true;
    run_auto(&mut st2);
    assert_eq!(urls(&calls).len(), n);
    assert!(urls(&calls2).is_empty(), "{:?}", urls(&calls2));

    // Switched off: no run.
    st2.config.art.auto_fill = false;
    st2.add_manual_game("Brand New", LaunchSpec::Url("x://".into()), Art::default());
    run_auto(&mut st2);
    assert!(urls(&calls2).is_empty());
}

/// Real names against the real Steam store (and SteamGridDB with LODESTAR_SGDB_KEY):
/// cargo test art_live_auto -- --ignored --nocapture
#[test]
#[ignore]
fn art_live_auto() {
    let key = std::env::var("LODESTAR_SGDB_KEY").ok();
    let (e, dir) = live_engine(key.clone());
    let names = [("Brawlhalla", Some(291550)), ("The Sims 4", Some(1222670)), ("Diablo II: Resurrected", None)];
    for (n, want) in names {
        let hits = e.store_search(n, Req::UI).unwrap();
        let m = confident_steam(n, &hits);
        println!(
            "{n:?}: confident Steam match {m:?} (top hits {:?})",
            hits.iter().take(4).map(|h| (h.appid, &h.name)).collect::<Vec<_>>()
        );
        assert_eq!(m, want, "{n}");
    }
    let reqs: Vec<AutoRequest> = names
        .iter()
        .enumerate()
        .map(|(i, (n, _))| AutoRequest {
            id: i as i64 + 1,
            name: n.to_string(),
            appid: None,
            gog_id: None,
            platform: None,
            rom_names: vec![],
            serial: None,
            need: vec![ArtSlot::Cover, ArtSlot::Hero, ArtSlot::Logo],
            sources: vec![SourceKind::Steam, SourceKind::SteamGridDb],
        })
        .collect();
    let rep = e.auto_fill(&reqs, &AtomicBool::new(false), &mut |g, a, t| {
        println!("  game {g}: tried {t:?}");
        for s in ArtSlot::ALL {
            if let Some(p) = s.get(&a) {
                let ok = validate_image(&std::fs::read(p).unwrap());
                println!("     {:<10} {} {ok:?}", s.label(), p.display());
            }
        }
    });
    println!("report {rep:?} (key: {})", key.is_some());
    assert!(rep.changed >= 2);
    drop(dir);
}

/// What the automatic fill would ask for every game of a library DB, with live confident Steam
/// matches (read-only on the DB): LODESTAR_TEST_DB=<path>/library.db cargo test
/// art_live_auto_library -- --ignored --nocapture
#[test]
#[ignore]
fn art_live_auto_library() {
    let Ok(path) = std::env::var("LODESTAR_TEST_DB") else {
        println!("set LODESTAR_TEST_DB");
        return;
    };
    let db = crate::db::Db::open(Path::new(&path)).unwrap();
    let (e, _dir) = live_engine(None);
    for g in db.load_all().unwrap() {
        let Some(r) = ar(&g, 0, true, false) else {
            continue;
        };
        println!("{:<45} need {:<2} -> {:?}", r.name, r.need.len(), e.confident_store_match(&r.name, Req::UI));
    }
}
