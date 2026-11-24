//! Tests of the extra artwork sources: parsers on trimmed real answers (tests/fixtures/art/), and
//! the engine with a fake network (credentials only on the wire, confident matches, bulk).

use super::net::*;
use super::sources::*;
use super::worker::*;
use super::*;
use parking_lot::Mutex as PMutex;
use std::collections::HashMap;
use std::sync::Arc;

macro_rules! fixture {
    ($name:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/art/", $name)).as_slice()
    };
}
macro_rules! meta_fixture {
    ($name:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/metadata/", $name)).as_slice()
    };
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(w, h, image::Rgb([10, 200, 30]));
    let mut out = std::io::Cursor::new(vec![]);
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

// ---------------------------------------------------------------------------------------------
// Parsers

#[test]
fn lutris_search_and_matching() {
    let games = parse_lutris(fixture!("lutris_search_celeste.json")).unwrap();
    assert_eq!(games.len(), 3);
    let c = &games[1];
    assert_eq!((c.name.as_str(), c.slug.as_str()), ("Celeste", "celeste"));
    assert_eq!(c.steam, vec![504230]);
    assert_eq!(c.coverart.as_deref(), Some("https://lutris.net/media/igdb/cover_big/co3byy.jpg"));
    assert_eq!(c.banner.as_deref(), Some("https://lutris.net/games/banner/celeste.jpg"));
    assert_eq!(games[0].banner, None, "empty strings are no image");
    // By Steam appid even under another name; else the exact name only.
    assert_eq!(confident_lutris("Something else", Some(504230), &games).map(|g| g.slug.as_str()), Some("celeste"));
    assert_eq!(confident_lutris("CELESTE", None, &games).map(|g| g.slug.as_str()), Some("celeste"));
    assert!(confident_lutris("Celeste", Some(1), &games).is_none(), "a different Steam game");
    assert!(confident_lutris("Celeste 64", None, &games).is_none());
    let pics = lutris_pics(c, ArtSlot::Icon);
    assert_eq!((pics[0].width, pics[0].height), (128, 128));
    assert!(lutris_pics(c, ArtSlot::Hero).is_empty());
    assert_eq!(lutris_search_url("Celeste"), "https://lutris.net/api/games?search=Celeste");
    assert_eq!(parse_lutris(b"{}"), None);
}

#[test]
fn gog_catalog_and_product() {
    let p = parse_gog_catalog(fixture!("gog_catalog_hollow_knight.json")).unwrap();
    assert_eq!(p[0].id, 1308320804);
    assert_eq!(p[0].title, "Hollow Knight");
    assert!(p[0].cover.as_deref().unwrap().starts_with("https://images.gog-statics.com/"));
    assert!(p[0].horizontal.is_some() && p[0].logo.is_some() && p[0].background.len() == 1);
    assert_eq!(confident_gog("Hollow Knight", &p).map(|p| p.id), Some(1308320804));
    assert!(confident_gog("Hollow Knight: Voidheart", &p).is_none());
    let g = parse_gog_game(fixture!("gog_v2_games.json")).unwrap();
    assert_eq!((g.id, g.title.as_str()), (1207664663, "The Witcher 3: Wild Hunt - Complete Edition"));
    assert!(
        g.cover.as_deref().unwrap().ends_with("43972f6e5406e50b6ac3e82b16500c601e8c45f9a3577d026d99acd7c291607c.jpg")
    );
    // The Galaxy background first, the square icon first.
    assert!(g.background[0].contains("8b55161d"));
    assert!(g.icon[0].contains("53a25d35"));
    assert_eq!(gog_pics(&g, ArtSlot::Hero).len(), 2);
    assert!(gog_pics(&g, ArtSlot::Banner).is_empty(), "no wide capsule in v2");
    assert_eq!(gog_game_url(5), "https://api.gog.com/v2/games/5?locale=en-US");
    assert!(gog_catalog_url("Hollow Knight").contains("query=like:Hollow%20Knight"));
    // Images only from GOG's hosts.
    let odd = br#"{"products":[{"id":"1","title":"X","coverVertical":"https://evil.example/x.jpg","logo":"//images.gog-statics.com/l.png"}]}"#;
    let p = parse_gog_catalog(odd).unwrap();
    assert_eq!(p[0].cover, None);
    assert_eq!(p[0].logo.as_deref(), Some("https://images.gog-statics.com/l.png"));
}

#[test]
fn pcgamingwiki_pages_and_files() {
    let page = parse_pcgw_page(fixture!("pcgw_wikitext_celeste.json")).unwrap().unwrap();
    assert_eq!(page.title, "Celeste");
    assert_eq!(page.cover.as_deref(), Some("Celeste cover.png"));
    assert_eq!(page.steam_appids, vec![504230]);
    assert!(!page.redirected);
    assert!(pcgw_confident("Celeste", None, &page));
    assert!(pcgw_confident("Anything", Some(504230), &page), "the appid decides");
    assert!(!pcgw_confident("Celeste", Some(1), &page));
    assert!(!pcgw_confident("Celeste 64", None, &page));
    let red = parse_pcgw_page(fixture!("pcgw_wikitext_redirect.json")).unwrap().unwrap();
    assert_eq!(red.title, "The Witcher 3: Wild Hunt");
    assert!(red.redirected);
    assert_eq!(red.cover.as_deref(), Some("The Witcher 3 Wild Hunt - cover.jpg"));
    assert!(pcgw_confident("The Witcher 3", None, &red), "the wiki's own redirect");
    let missing = br#"{"batchcomplete":true,"query":{"pages":[{"ns":0,"title":"No Such Game Xyz","missing":true}]}}"#;
    assert_eq!(parse_pcgw_page(missing), Some(None));
    let f = parse_pcgw_file(fixture!("pcgw_imageinfo.json")).unwrap().unwrap();
    assert_eq!(f.url, "https://images.pcgamingwiki.com/a/a6/Celeste_cover.png");
    assert_eq!((f.width, f.height), (2600, 4168));
    assert!(f.thumb.unwrap().contains("300px-Celeste_cover.png"));
    assert_eq!(
        parse_pcgw_search(fixture!("pcgw_search.json")).unwrap(),
        vec!["Celeste 64: Fragments of the Mountain".to_string(), "Celeste".to_string()]
    );
    assert!(pcgw_file_url("Celeste cover.png").contains("titles=File%3ACeleste%20cover.png"));
    assert!(pcgw_page_url("A&B").contains("titles=A%26B&redirects=1"));
}

#[test]
fn libretro_names_and_urls() {
    let stems = parse_libretro_tree(fixture!("libretro_tree_n64_boxarts.json")).unwrap();
    let idx = LrIndex::new(stems.clone());
    assert!(stems.contains(&"Super Mario 64 (USA)".to_string()));
    assert!(!stems.iter().any(|s| s.contains("(Beta)") && s.starts_with("007")), "symlinks skipped");
    assert_eq!(
        nointro_title("Legend of Zelda, The - Ocarina of Time (USA) (Rev 1)"),
        "The Legend of Zelda - Ocarina of Time"
    );
    assert_eq!(idx.matches("Super Mario 64")[0], "Super Mario 64 (USA)");
    assert_eq!(idx.matches("The Legend of Zelda: Ocarina of Time")[0], "Legend of Zelda, The - Ocarina of Time (USA)");
    // A ROM-style name picks that exact file.
    assert_eq!(idx.matches("Super Mario 64 (Japan)"), vec!["Super Mario 64 (Japan)"]);
    assert!(idx.matches("Super Mario").is_empty());
    let s = idx.search("mario", 5);
    assert_eq!(s, vec!["Mario Kart 64 (USA)", "Super Mario 64 (USA)"]);
    let p = console_platform(Some("Nintendo 64")).unwrap();
    assert_eq!(
        libretro_image_url(p.libretro.unwrap(), LR_BOXART, "Super Mario 64 (USA)"),
        "https://raw.githubusercontent.com/libretro-thumbnails/Nintendo_-_Nintendo_64/master/Named_Boxarts/Super%20Mario%2064%20%28USA%29.png"
    );
    assert_eq!(
        libretro_index_url(p.libretro.unwrap()),
        "https://api.github.com/repos/libretro-thumbnails/Nintendo_-_Nintendo_64/git/trees/master:Named_Boxarts"
    );
    let hero = libretro_pics(p.libretro.unwrap(), "Super Mario 64 (USA)", ArtSlot::Hero);
    assert!(hero[0].url.contains("/Named_Snaps/") && hero[1].url.contains("/Named_Titles/"));
    // A symlinked image: the raw host answers with the target's name.
    let u = libretro_image_url(p.libretro.unwrap(), LR_SNAP, "Mario Kart 64 (Europe) (Rev 1)");
    assert_eq!(
        libretro_symlink(&u, b"Mario Kart 64 (Europe).png").unwrap(),
        libretro_image_url(p.libretro.unwrap(), LR_SNAP, "Mario Kart 64 (Europe)")
    );
    assert_eq!(libretro_symlink(&u, b"../../x.png"), None);
    assert_eq!(libretro_symlink("https://example.com/a.png", b"b.png"), None);
}

#[test]
fn console_platforms() {
    let id = |p: &str| console_platform(Some(p)).and_then(|p| p.libretro);
    assert_eq!(id("nintendo 64"), Some("Nintendo - Nintendo 64"));
    assert_eq!(id("N64"), Some("Nintendo - Nintendo 64"));
    assert_eq!(id("Sony PlayStation"), Some("Sony - PlayStation"));
    assert_eq!(id("sega genesis"), Some("Sega - Mega Drive - Genesis"));
    assert_eq!(id("Mega Drive"), Some("Sega - Mega Drive - Genesis"));
    assert_eq!(id("Nintendo - Game Boy Advance"), Some("Nintendo - Game Boy Advance"));
    assert_eq!(id("windows"), None);
    assert_eq!(id("linux"), None);
    assert_eq!(console_platform(None), None);
    let p = console_platform(Some("snes")).unwrap();
    assert_eq!((p.tgdb, p.igdb), (Some(6), Some(19)));
}

#[test]
fn thegamesdb_games_and_images() {
    let g = parse_tgdb_games(fixture!("tgdb_bygamename_sonic.json")).unwrap();
    assert_eq!((g[0].id, g[0].title.as_str(), g[0].platform), (53, "Sonic the Hedgehog", Some(18)));
    let b = g[0].boxart.as_ref().unwrap();
    assert_eq!(b.url, "https://cdn.thegamesdb.net/images/original/boxart/front/53-1.jpg");
    assert_eq!(b.thumb.as_deref(), Some("https://cdn.thegamesdb.net/images/thumb/boxart/front/53-1.jpg"));
    assert_eq!((b.width, b.height), (1521, 2156));
    assert_eq!(confident_tgdb("Sonic The Hedgehog", &g, false).map(|g| g.id), Some(53));
    let mut pcg = g.clone();
    pcg.push(TgdbGame { id: 99, platform: Some(1), ..g[0].clone() });
    assert_eq!(confident_tgdb("Sonic The Hedgehog", &pcg, true).map(|g| g.id), Some(99));
    assert_eq!(confident_tgdb("Sonic The Hedgehog", &pcg, false).map(|g| g.id), Some(53));
    let imgs = parse_tgdb_images(fixture!("tgdb_images_53.json"), 53).unwrap();
    let kinds: Vec<&str> = imgs.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(kinds, vec!["boxart", "fanart", "clearlogo", "banner"], "back covers dropped");
    assert_eq!(parse_tgdb_images(fixture!("tgdb_images_53.json"), 99).unwrap(), vec![]);
    assert!(tgdb_search_url("Sonic", Some(18)).ends_with("include=boxart&filter%5Bplatform%5D=18"));
    assert!(!tgdb_search_url("Sonic", None).contains("apikey"));
}

#[test]
fn source_order_and_switches() {
    let mut c = MetadataConfig::default();
    assert_eq!(ordered_sources(&c).iter().map(|(k, _)| *k).collect::<Vec<_>>(), PC_ORDER.to_vec());
    // A partial list: the named ones first in that order, the rest in their places after.
    c.source_order = vec!["libretro".into(), "nonsense".into(), "steam".into()];
    c.sources_off = vec!["GOG".into()];
    let o = ordered_sources(&c);
    assert_eq!(o[0], (SourceKind::Libretro, true));
    assert_eq!(o[1], (SourceKind::Steam, true));
    assert_eq!(o[2], (SourceKind::SteamGridDb, true));
    assert_eq!(o[3], (SourceKind::Gog, false));
    // GameTDB (console-only) isn't in the PC list.
    assert_eq!(o.len(), PC_ORDER.len());
    assert!(!o.iter().any(|(k, _)| *k == SourceKind::GameTdb));
    // Bits are distinct (stored in the DB).
    let mut bits: Vec<u16> = SourceKind::ALL.iter().map(|k| k.bit()).collect();
    bits.dedup();
    assert_eq!(bits.len(), SourceKind::ALL.len());
    assert!(!SourceKind::TheGamesDb.configured(&c));
    c.thegamesdb_key = Some(" k ".into());
    assert!(SourceKind::TheGamesDb.configured(&c));
}

// ---------------------------------------------------------------------------------------------
// Engine

type Sent = Arc<PMutex<Vec<String>>>;

/// Routes by URL (GET / HEAD) or "POST <url>\n<body>"; unknown → 404. Records what was sent.
#[derive(Default)]
struct Net2 {
    routes: HashMap<String, Result<(u16, Vec<u8>), String>>,
    sent: Sent,
}

impl Net2 {
    fn ok(mut self, url: &str, body: &[u8]) -> Self {
        self.routes.insert(url.into(), Ok((200, body.to_vec())));
        self
    }
    fn status(mut self, url: &str, s: u16) -> Self {
        self.routes.insert(url.into(), Ok((s, vec![])));
        self
    }
    fn err(mut self, url: &str, e: &str) -> Self {
        self.routes.insert(url.into(), Err(e.into()));
        self
    }
    fn answer(&self, key: &str) -> Result<HttpResponse, String> {
        self.sent.lock().push(key.to_string());
        match self.routes.get(key) {
            Some(Ok((status, body))) => Ok(HttpResponse { status: *status, body: body.clone(), ..Default::default() }),
            Some(Err(e)) => Err(e.clone()),
            None => Ok(HttpResponse { status: 404, ..Default::default() }),
        }
    }
}

impl Net for Net2 {
    fn get(&self, url: &str, _bearer: Option<&str>, _limit: u64) -> Result<HttpResponse, String> {
        self.answer(url)
    }
    fn head(&self, url: &str) -> Result<u16, String> {
        self.answer(&format!("HEAD {url}")).map(|r| r.status)
    }
    fn post(&self, url: &str, headers: &[(&str, String)], body: &str, _limit: u64) -> Result<HttpResponse, String> {
        let auth: Vec<String> = headers.iter().map(|(k, v)| format!("{k}={v}")).collect();
        if !auth.is_empty() {
            self.sent.lock().push(format!("HEADERS {}", auth.join(";")));
        }
        self.answer(&format!("POST {url}\n{body}"))
    }
}

/// Only `on` switched on.
fn cfg_only(on: &[SourceKind]) -> MetadataConfig {
    let off: Vec<String> = SourceKind::ALL.iter().filter(|k| !on.contains(k)).map(|k| k.name().to_string()).collect();
    MetadataConfig { sources_off: off.clone(), emu_sources_off: Some(off), ..Default::default() }
}

fn engine(net: Net2, dir: &std::path::Path, cfg: MetadataConfig) -> Engine {
    Engine::new(Box::new(net), Throttle::none(), dir.join("art"), dir.join("thumbs"), &cfg)
}

fn job(name: &str, slot: ArtSlot, platform: Option<&str>) -> SearchJob {
    SearchJob {
        game: 1,
        slot,
        name: name.into(),
        appid: None,
        gog_id: None,
        platform: platform.map(str::to_string),
        rom_names: vec![],
        serial: None,
        list: ArtList::Pc,
    }
}

fn search(e: &Engine, j: &SearchJob) -> (Vec<Candidate>, Vec<String>, usize) {
    let (mut items, mut errors, mut batches) = (vec![], vec![], 0);
    e.search(j, |i, _, er| {
        batches += 1;
        items.extend(i);
        errors.extend(er);
        true
    });
    (items, errors, batches)
}

fn bulk_game(name: &str, need: Vec<ArtSlot>, dir: &std::path::Path) -> BulkGame {
    BulkGame {
        id: 9,
        name: name.into(),
        appid: None,
        gog_id: None,
        platform: None,
        rom_names: vec![],
        serial: None,
        need,
        dest: Some(dir.join("out")),
    }
}

#[test]
fn network_errors_never_show_secrets() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = cfg_only(&[SourceKind::TheGamesDb]);
    cfg.thegamesdb_key = Some("tgdbsecret42".into());
    let url = format!("{}&apikey=tgdbsecret42", tgdb_search_url("Halo", None));
    let net = Net2::default().err(&url, "connect failed for https://api.thegamesdb.net/?apikey=tgdbsecret42");
    let e = engine(net, tmp.path(), cfg);
    let (_, errors, _) = search(&e, &job("Halo", ArtSlot::Cover, None));
    assert_eq!(errors.len(), 1);
    assert!(errors[0].starts_with("TheGamesDB: ") && !errors[0].contains("tgdbsecret42"), "{errors:?}");
    assert!(errors[0].contains("***"));
}

#[test]
fn rejected_key_marks_the_source_unavailable_until_it_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = cfg_only(&[SourceKind::TheGamesDb, SourceKind::Lutris]);
    cfg.thegamesdb_key = Some("bad".into());
    let url = format!("{}&apikey=bad", tgdb_search_url("Halo", None));
    let net = Net2::default().status(&url, 403);
    let e = engine(net, tmp.path(), cfg.clone());
    let (_, errors, batches) = search(&e, &job("Halo", ArtSlot::Cover, None));
    assert_eq!(batches, 2, "Lutris still answers");
    assert!(errors[0].contains("rejected"), "{errors:?}");
    let s = e.sources();
    let t = s.iter().find(|s| s.kind == SourceKind::TheGamesDb).unwrap();
    assert!(!t.available && t.reason.as_deref().unwrap().contains("rejected"));
    cfg.thegamesdb_key = Some("good".into());
    e.set_config(&cfg);
    assert!(e.available_sources(ArtList::Pc).contains(&SourceKind::TheGamesDb));
}

#[test]
fn unconfigured_keyed_sources_are_skipped_with_a_reason() {
    let tmp = tempfile::tempdir().unwrap();
    let e = engine(Net2::default(), tmp.path(), MetadataConfig::default());
    let s = e.sources();
    for k in [SourceKind::Igdb, SourceKind::TheGamesDb, SourceKind::SteamGridDb] {
        let i = s.iter().find(|s| s.kind == k).unwrap();
        assert!(!i.available && i.enabled);
        assert!(i.reason.as_deref().unwrap().contains("Settings → Metadata"), "{i:?}");
    }
    // Switched-off sources come last.
    e.set_config(&MetadataConfig { sources_off: vec!["Steam".into()], ..Default::default() });
    let s = e.sources();
    assert_eq!(s.last().unwrap().kind, SourceKind::Steam);
    assert!(!s.last().unwrap().enabled);
}

#[test]
fn lutris_and_gog_bulk_confident_only() {
    let tmp = tempfile::tempdir().unwrap();
    let cover = "https://lutris.net/media/igdb/cover_big/co3byy.jpg";
    let gog_box = "https://images.gog-statics.com/43972f6e5406e50b6ac3e82b16500c601e8c45f9a3577d026d99acd7c291607c.jpg";
    let net = Net2::default()
        .ok(&lutris_search_url("Celeste"), fixture!("lutris_search_celeste.json"))
        .ok(cover, &png(4, 6))
        .ok(&gog_game_url(1207664663), fixture!("gog_v2_games.json"))
        .ok(gog_box, &png(4, 6));
    let sent = net.sent.clone();
    let e = engine(net, tmp.path(), MetadataConfig::default());
    // Lutris: by name.
    let g = bulk_game("Celeste", vec![ArtSlot::Cover], tmp.path());
    let mut got = vec![];
    let r = e.bulk_game(&g, &[SourceKind::Lutris], Req::UI, &mut |s, p| got.push((s, p)));
    assert_eq!(r.unwrap(), (1, true));
    // No exact name: nothing, and no image requested.
    let g2 = bulk_game("Celest", vec![ArtSlot::Cover], tmp.path());
    let before = sent.lock().len();
    assert_eq!(e.bulk_game(&g2, &[SourceKind::Lutris], Req::UI, &mut |_, _| {}).unwrap(), (0, false));
    assert!(sent.lock()[before..].iter().all(|u| u.starts_with(LUTRIS_API)));
    // GOG game by its product id, whatever it is called here.
    let mut g3 = bulk_game("Witcher 3 GOTY", vec![ArtSlot::Cover], tmp.path());
    g3.gog_id = Some(1207664663);
    let mut got = vec![];
    let r = e.bulk_game(&g3, &[SourceKind::Gog], Req::UI, &mut |s, p| got.push((s, p)));
    assert_eq!(r.unwrap(), (1, true));
    // Console games skip the PC stores.
    let mut g4 = bulk_game("Celeste", vec![ArtSlot::Cover], tmp.path());
    g4.platform = Some("Nintendo 64".into());
    let before = sent.lock().len();
    assert_eq!(
        e.bulk_game(&g4, &[SourceKind::Gog, SourceKind::PcGamingWiki], Req::UI, &mut |_, _| {}).unwrap(),
        (0, false)
    );
    assert_eq!(sent.lock().len(), before);
}

#[test]
fn pcgamingwiki_cover_by_title_checked_by_appid() {
    let tmp = tempfile::tempdir().unwrap();
    let full = "https://images.pcgamingwiki.com/a/a6/Celeste_cover.png";
    let net = Net2::default()
        .ok(&pcgw_page_url("Celeste"), fixture!("pcgw_wikitext_celeste.json"))
        .ok(&pcgw_file_url("Celeste cover.png"), fixture!("pcgw_imageinfo.json"))
        .ok(full, &png(4, 6));
    let e = engine(net, tmp.path(), MetadataConfig::default());
    let mut g = bulk_game("Celeste", vec![ArtSlot::Cover, ArtSlot::Hero], tmp.path());
    g.appid = Some(504230);
    let mut got = vec![];
    let r = e.bulk_game(&g, &[SourceKind::PcGamingWiki], Req::UI, &mut |s, p| got.push((s, p)));
    assert_eq!(r.unwrap(), (1, true));
    assert!(got[0].1.ends_with("cover.png"));
    // Another Steam game of the same name is not this page.
    g.appid = Some(42);
    assert_eq!(e.bulk_game(&g, &[SourceKind::PcGamingWiki], Req::UI, &mut |_, _| {}).unwrap(), (0, false));
    // Search: the cover with its 300 px thumbnail.
    let (items, _, _) = search(&e, &SearchJob { appid: None, ..job("Celeste", ArtSlot::Cover, None) });
    let c = items.iter().find(|c| c.source == "PCGamingWiki").unwrap();
    assert_eq!(c.url, full);
    assert!(c.thumb_url.as_deref().unwrap().contains("300px"));
}

#[test]
fn libretro_by_platform_with_disk_cached_index() {
    let tmp = tempfile::tempdir().unwrap();
    let p = console_platform(Some("n64")).unwrap();
    let box_url = libretro_image_url(p.libretro.unwrap(), LR_BOXART, "Super Mario 64 (USA)");
    let snap = libretro_image_url(p.libretro.unwrap(), LR_SNAP, "Super Mario 64 (USA)");
    let title = libretro_image_url(p.libretro.unwrap(), LR_TITLE, "Super Mario 64 (USA)");
    let net = Net2::default()
        .ok(&libretro_index_url(p.libretro.unwrap()), fixture!("libretro_tree_n64_boxarts.json"))
        .ok(&box_url, &png(5, 7))
        .ok(&format!("HEAD {snap}"), b"")
        // A symlinked snap: the raw host answers with the target's file name.
        .ok(&snap, b"Super Mario 64 (Europe) (En,Fr,De).png")
        .ok(&libretro_image_url(p.libretro.unwrap(), LR_SNAP, "Super Mario 64 (Europe) (En,Fr,De)"), &png(8, 6));
    let sent = net.sent.clone();
    let e = engine(net, tmp.path(), MetadataConfig::default());
    // PC games never ask libretro; console games do.
    let (items, _, _) = search(&e, &job("Super Mario 64", ArtSlot::Hero, None));
    assert!(items.iter().all(|c| c.source != "libretro"));
    let (items, _, _) = search(&e, &job("Super Mario 64", ArtSlot::Hero, Some("Nintendo 64")));
    let lr: Vec<&Candidate> = items.iter().filter(|c| c.source == "libretro").collect();
    assert_eq!(lr.len(), 1, "the title screen doesn't exist (HEAD 404)");
    assert_eq!(lr[0].url, snap);
    assert!(lr[0].label.contains("Super Mario 64") && lr[0].label.contains("screenshot"));
    let _ = title;
    // Bulk: box art + the symlinked snap (followed).
    let mut g = bulk_game("Super Mario 64", vec![ArtSlot::Cover, ArtSlot::Hero], tmp.path());
    g.platform = Some("Nintendo 64".into());
    let mut got = vec![];
    let r = e.bulk_game(&g, &[SourceKind::Libretro], Req::UI, &mut |s, p| got.push((s, p)));
    assert_eq!(r.unwrap(), (2, true));
    // The name list was asked once and is on disk for the next session.
    let idx = libretro_index_url(p.libretro.unwrap());
    assert_eq!(sent.lock().iter().filter(|u| **u == idx).count(), 1);
    let e2 = engine(Net2::default(), tmp.path(), MetadataConfig::default());
    let q = Query { name: "Super Mario 64", appid: None, gog_id: None, console: Some(p), rom_names: &[], serial: None };
    let found = e2.find(SourceKind::Libretro, &q, ArtSlot::Cover, true, Req::UI).unwrap().unwrap();
    assert_eq!(found[0].1.url, box_url);
}

#[test]
fn igdb_token_and_queries() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = cfg_only(&[SourceKind::Igdb]);
    cfg.igdb_client_id = Some("art-cid".into());
    cfg.igdb_client_secret = Some("art-secret-1".into());
    crate::igdb::forget_token("art-cid", "art-secret-1");
    let token_key = format!("POST {}\n", crate::igdb::token_url("art-cid", "art-secret-1"));
    let search_key = format!("POST {}\n{}", crate::igdb::games_url(), crate::igdb::search_body("Celeste"));
    let net = Net2::default()
        .ok(&token_key, meta_fixture!("twitch_token.json"))
        .ok(&search_key, meta_fixture!("igdb_games_celeste.json"));
    let sent = net.sent.clone();
    let e = engine(net, tmp.path(), cfg);
    let (items, errors, _) = search(&e, &job("Celeste", ArtSlot::Cover, None));
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(items[0].url, "https://images.igdb.com/igdb/image/upload/t_cover_big_2x/co3byy.jpg");
    assert_eq!(items[0].label, "IGDB · Celeste");
    assert!(
        sent.lock().iter().any(|s| s == "HEADERS Client-ID=art-cid;Authorization=Bearer jostpf5q0uzmxmkba9iyug38kjtgh")
    );
    // Heroes: artworks, then screenshots; a confident match only in bulk.
    let q = Query { name: "Celeste", appid: None, gog_id: None, console: None, rom_names: &[], serial: None };
    let hero = e.find(SourceKind::Igdb, &q, ArtSlot::Hero, true, Req::UI).unwrap().unwrap();
    assert_eq!(hero.len(), 3);
    assert!(hero[0].1.url.ends_with("t_1080p/ar4vo.jpg"));
    // One token request for everything (cached).
    let tokens = sent.lock().iter().filter(|s| s.contains("oauth2/token")).count();
    assert_eq!(tokens, 1);
    crate::igdb::forget_token("art-cid", "art-secret-1");
}

#[test]
fn igdb_bad_credentials_are_reported_once() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = cfg_only(&[SourceKind::Igdb]);
    cfg.igdb_client_id = Some("bad-cid".into());
    cfg.igdb_client_secret = Some("bad-secret-2".into());
    let token_key = format!("POST {}\n", crate::igdb::token_url("bad-cid", "bad-secret-2"));
    let net = Net2::default().status(&token_key, 403);
    let e = engine(net, tmp.path(), cfg);
    let (_, errors, _) = search(&e, &job("Celeste", ArtSlot::Cover, None));
    assert!(errors[0].contains("rejected"), "{errors:?}");
    assert!(!e.available_sources(ArtList::Pc).contains(&SourceKind::Igdb));
}

#[test]
fn auto_requests_follow_the_platform() {
    let mut db = crate::db::Db::open_memory().unwrap();
    let id = db.add_manual("Super Mario 64", LaunchSpec::Url("x://".into()), Art::default()).unwrap();
    let mut g = db.get(id).unwrap().unwrap();
    let all = SourceKind::ALL.to_vec();
    let r = crate::art_app::auto_request(&g, 0, &all).unwrap();
    assert!(!r.sources.contains(&SourceKind::Libretro) && r.sources.contains(&SourceKind::Gog));
    g.platform = Some("nintendo 64".into());
    let r = crate::art_app::auto_request(&g, 0, &all).unwrap();
    assert!(r.sources.contains(&SourceKind::Libretro));
    assert!(!r.sources.iter().any(|k| matches!(k, SourceKind::Steam | SourceKind::Gog | SourceKind::PcGamingWiki)));
    assert_eq!(r.platform.as_deref(), Some("nintendo 64"));
    // Each source once per game.
    let tried = SourceKind::Libretro.bit() | SourceKind::Lutris.bit();
    let r = crate::art_app::auto_request(&g, tried, &all).unwrap();
    assert!(!r.sources.contains(&SourceKind::Libretro) && !r.sources.contains(&SourceKind::Lutris));
}

#[test]
fn one_failing_source_does_not_stop_the_others() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = cfg_only(&[SourceKind::Lutris, SourceKind::Gog]);
    // Lutris is down; GOG answers (404: it doesn't know the game).
    let net = Net2::default().err(&lutris_search_url("Some Game"), "connection refused");
    let e = engine(net, tmp.path(), cfg);
    let reqs: Vec<AutoRequest> = (1..=4)
        .map(|id| AutoRequest {
            id,
            name: "Some Game".into(),
            appid: None,
            gog_id: None,
            platform: None,
            rom_names: vec![],
            serial: None,
            need: vec![ArtSlot::Cover],
            sources: vec![SourceKind::Lutris, SourceKind::Gog],
        })
        .collect();
    let mut got = vec![];
    let rep = e.auto_fill(&reqs, &std::sync::atomic::AtomicBool::new(false), &mut |g, _, t| got.push((g, t)));
    // Every game is reported with only GOG asked (Lutris is asked again another time), and the
    // run isn't stopped as "offline".
    assert_eq!(got.len(), 4);
    assert!(got.iter().all(|(_, t)| *t == vec![SourceKind::Gog]), "{got:?}");
    assert!(rep.error.is_none(), "{rep:?}");
}

// ---------------------------------------------------------------------------------------------
// Emulated games

#[test]
fn every_playnite_console_name_is_known() {
    let lr = |p: &str| console_platform(Some(p)).and_then(|p| p.libretro);
    for (name, system) in [
        ("Sony PlayStation 3", "Sony - PlayStation 3"),
        ("Sony PlayStation Vita", "Sony - PlayStation Vita"),
        ("Nintendo Wii U", "Nintendo - Wii U"),
        ("Nintendo DSi", "Nintendo - Nintendo DSi"),
        ("Nintendo DS", "Nintendo - Nintendo DS"),
        ("Nintendo 3DS", "Nintendo - Nintendo 3DS"),
        ("New Nintendo 3DS", "Nintendo - Nintendo 3DS"),
        ("Microsoft Xbox", "Microsoft - Xbox"),
        ("Microsoft Xbox 360", "Microsoft - Xbox 360"),
        ("Nintendo SNES", "Nintendo - Super Nintendo Entertainment System"),
        ("Sega Genesis", "Sega - Mega Drive - Genesis"),
        ("NEC TurboGrafx-CD", "NEC - PC Engine CD - TurboGrafx-CD"),
        ("Atari ST/STE", "Atari - ST"),
        ("Commodore Plus/4", "Commodore - Plus-4"),
        ("3DO Interactive Multiplayer", "The 3DO Company - 3DO"),
        ("SNK Neo Geo AES", "SNK - Neo Geo"),
        ("GCE Vectrex", "GCE - Vectrex"),
        ("Thomson TO7", "Thomson - MOTO"),
        ("Sinclair ZX Spectrum +3", "Sinclair - ZX Spectrum"),
        // ES-DE / Batocera folder names and Playnite ids.
        ("psvita", "Sony - PlayStation Vita"),
        ("wiiu", "Nintendo - Wii U"),
        ("n3ds", "Nintendo - Nintendo 3DS"),
        ("sony_playstation3", "Sony - PlayStation 3"),
        ("nintendo_gamecube", "Nintendo - GameCube"),
    ] {
        assert_eq!(lr(name), Some(system), "{name}");
    }
    // Switch: a console without a libretro-thumbnails repo.
    let sw = console_platform(Some("Nintendo Switch")).unwrap();
    assert_eq!((sw.libretro, sw.tgdb, sw.igdb), (None, Some(4971), Some(130)));
    let ps3 = console_platform(Some("ps3")).unwrap();
    assert_eq!((ps3.tgdb, ps3.igdb), (Some(12), Some(9)));
    // PCs are not consoles (their games keep the PC stores).
    for pc in ["PC (Windows)", "PC (Linux)", "PC (DOS)", "Windows", "Linux", "pc", "Macintosh"] {
        assert_eq!(console_platform(Some(pc)), None, "{pc}");
    }
    // Every row is reachable by its own name and id, and no two rows share a libretro system.
    let mut seen = HashMap::new();
    for p in PLATFORMS {
        assert_eq!(console_platform(Some(p.name)).map(|c| c.id), Some(p.id), "{}", p.name);
        assert_eq!(console_platform(Some(p.id)).map(|c| c.id), Some(p.id), "{}", p.id);
        if let Some(lr) = p.libretro {
            assert!(seen.insert(lr, p.id).is_none(), "{lr} twice");
        }
    }
}

#[test]
fn emulation_platforms_resolve_to_their_libretro_system() {
    const PCS: &[&str] = &["pc_dos", "pc_windows", "pc_linux", "macintosh", "adobe_flash"];
    for p in crate::emulation::platforms::all() {
        if PCS.contains(&p.id) {
            continue;
        }
        let c = console_platform(Some(p.name));
        if let Some(lr) = p.libretro {
            assert_eq!(c.and_then(|c| c.libretro), Some(lr), "{} ({})", p.name, p.id);
            assert_eq!(console_platform(Some(p.id)).and_then(|c| c.libretro), Some(lr), "{}", p.id);
        }
        // Both tables agree on the other services' ids where both know them.
        if let Some(c) = c {
            for (what, a, b) in [("TheGamesDB", p.tgdb, c.tgdb), ("IGDB", p.igdb, c.igdb)] {
                if let (Some(a), Some(b)) = (a, b) {
                    assert_eq!(a, b, "{what} id of {}", p.id);
                }
            }
        }
    }
}

#[test]
fn rom_names_pick_the_regional_image() {
    let idx = LrIndex::new(
        [
            "Tekken 5 (USA)",
            "Tekken 5 (Europe, Australia)",
            "Tekken 5 (Japan)",
            "Ratchet _ Clank (USA)",
            "Ratchet _ Clank (Europe, Australia)",
            "Mario Kart 64 (Europe) (Rev 1)",
            "Mario Kart 64 (Europe)",
            "Mario Kart 64 (USA)",
        ]
        .map(String::from)
        .to_vec(),
    );
    let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    // Without ROM names: the usual USA-first ranking.
    assert_eq!(idx.matches_rom(&[], "Tekken 5")[0], "Tekken 5 (USA)");
    // The Redump name is an image name: that one.
    assert_eq!(
        idx.matches_rom(&n(&["Tekken 5 (Europe, Australia)"]), "Tekken 5"),
        vec!["Tekken 5 (Europe, Australia)"]
    );
    // libretro's file-name spelling (& → _).
    assert_eq!(idx.matches_rom(&n(&["Ratchet & Clank (USA)"]), "Ratchet & Clank"), vec!["Ratchet _ Clank (USA)"]);
    // Not an image name, but the region is known: same title, that region first.
    assert_eq!(idx.matches_rom(&n(&["Tekken 5 (Japan) (Rev 1)"]), "Tekken 5")[0], "Tekken 5 (Japan)");
    assert_eq!(idx.matches_rom(&n(&["Mario Kart 64 (Europe) (Rev 2)"]), "Mario Kart 64")[0], "Mario Kart 64 (Europe)");
    // The file name is the second candidate; a renamed game still finds its title by the DAT name.
    assert_eq!(idx.matches_rom(&n(&["Nope (USA)", "Tekken 5 (Japan)"]), "Tekken 5"), vec!["Tekken 5 (Japan)"]);
    assert_eq!(
        idx.matches_rom(&n(&["Tekken 5 (Europe, Australia) (Alt)"]), "My Tekken")[0],
        "Tekken 5 (Europe, Australia)"
    );
}

fn emulated(platform: &str, dat: Option<&str>, rom: &str) -> Game {
    let mut g = Game { id: 5, source: crate::model::Source::Emulator, ..Game::blank("Tekken 5") };
    g.emu = Some(crate::model::EmuRef {
        platform: platform.into(),
        rom: rom.into(),
        dat_name: dat.map(str::to_string),
        ..Default::default()
    });
    g
}

#[test]
fn emulated_games_send_their_rom_names_and_console() {
    let mut g = emulated("sony_playstation2", Some("Tekken 5 (Europe, Australia)"), "/roms/ps2/Tekken 5 (Europe).chd");
    assert_eq!(rom_names(&g), vec!["Tekken 5 (Europe, Australia)", "Tekken 5 (Europe)"]);
    // No library platform: the emulation platform's name.
    assert_eq!(art_platform(&g).as_deref(), Some("Sony PlayStation 2"));
    let all = SourceKind::ALL.to_vec();
    let r = crate::art_app::auto_request(&g, 0, &all).unwrap();
    assert_eq!(r.platform.as_deref(), Some("Sony PlayStation 2"));
    assert_eq!(r.rom_names, rom_names(&g));
    assert!(r.sources.contains(&SourceKind::Libretro) && r.sources.contains(&SourceKind::TheGamesDb));
    assert!(!r.sources.contains(&SourceKind::Steam));
    // Switch: console sources but no libretro.
    g = emulated("nintendo_switch", None, "/roms/switch/Some Game [0100000000010000][v0].nsp");
    g.platform = Some("Nintendo Switch".into());
    let r = crate::art_app::auto_request(&g, 0, &all).unwrap();
    assert!(!r.sources.contains(&SourceKind::Libretro) && !r.sources.contains(&SourceKind::Gog));
    assert!(r.sources.contains(&SourceKind::Igdb) && r.sources.contains(&SourceKind::TheGamesDb));
    // Folder games: the folder's name; generic launch files say nothing.
    g = emulated("sony_playstation3", None, "/roms/ps3/Demon's Souls (USA)");
    assert_eq!(rom_names(&g), vec!["Demon's Souls (USA)"]);
    g = emulated("sony_playstation3", None, "/roms/ps3/X/PS3_GAME/USRDIR/EBOOT.BIN");
    assert!(rom_names(&g).is_empty());
    // A typed search is the user's: no ROM names.
    let g = emulated("sony_playstation2", Some("Tekken 5 (Japan)"), "/r/t.iso");
    assert_eq!(SearchJob::new(&g, ArtSlot::Cover, None).rom_names, vec!["Tekken 5 (Japan)", "t"]);
    assert!(SearchJob::new(&g, ArtSlot::Cover, Some("Tekken")).rom_names.is_empty());
    assert_eq!(SearchJob::new(&g, ArtSlot::Cover, None).platform.as_deref(), Some("Sony PlayStation 2"));
}

#[test]
fn libretro_uses_the_rom_name_and_thegamesdb_the_platform() {
    let tmp = tempfile::tempdir().unwrap();
    let p = console_platform(Some("Nintendo 64")).unwrap();
    let lr = p.libretro.unwrap();
    let japan = libretro_image_url(lr, LR_BOXART, "Mario Kart 64 (Japan)");
    let net =
        Net2::default().ok(&libretro_index_url(lr), fixture!("libretro_tree_n64_boxarts.json")).ok(&japan, &png(5, 7));
    let sent = net.sent.clone();
    let e = engine(net, tmp.path(), MetadataConfig::default());
    let mut g = bulk_game("Mario Kart 64", vec![ArtSlot::Cover], tmp.path());
    g.platform = Some("Nintendo 64".into());
    g.rom_names = vec!["Mario Kart 64 (Japan)".into(), "mk64".into()];
    let r = e.bulk_game(&g, &[SourceKind::Libretro], Req::UI, &mut |_, _| {});
    assert_eq!(r.unwrap(), (1, true));
    assert!(sent.lock().contains(&japan), "{:?}", sent.lock());
    // The editor's search offers the ROM's own image first.
    let mut j = job("Mario Kart 64", ArtSlot::Cover, Some("Nintendo 64"));
    j.rom_names = vec!["Mario Kart 64 (Japan)".into()];
    let (items, _, _) = search(&e, &j);
    assert_eq!(items.iter().find(|c| c.source == "libretro").unwrap().url, japan);
    // TheGamesDB searches carry the console's id.
    assert!(tgdb_search_url("Tekken 5", console_platform(Some("ps3")).and_then(|p| p.tgdb)).ends_with("%5D=12"));
}

#[test]
fn gametdb_finds_art_by_game_id_with_region_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let base = "https://art.gametdb.com/wii";
    // A European disc: no English cover there, the US one is.
    let net = Net2::default()
        .status(&format!("HEAD {base}/cover/US/RMGP01.png"), 200)
        .status(&format!("HEAD {base}/cover3D/US/RMGP01.png"), 200)
        .status(&format!("HEAD {base}/disc/JA/RMGP01.png"), 200);
    let sent = net.sent.clone();
    let e = engine(net, tmp.path(), cfg_only(&[SourceKind::GameTdb]));
    let wii = console_platform(Some("Nintendo Wii")).unwrap();
    let q = Query {
        name: "Super Mario Galaxy",
        appid: None,
        gog_id: None,
        console: Some(wii),
        rom_names: &[],
        serial: Some("RMGP01"),
    };
    assert!(q.fits(SourceKind::GameTdb) && q.searchable(SourceKind::GameTdb));
    // Automatic: the front cover of the first region that has one (EN → US).
    let found = e.find(SourceKind::GameTdb, &q, ArtSlot::Cover, true, Req::UI).unwrap().unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].1.url, format!("{base}/cover/US/RMGP01.png"));
    let heads: Vec<String> = sent.lock().iter().filter(|s| s.starts_with("HEAD")).cloned().collect();
    assert_eq!(heads, [format!("HEAD {base}/cover/EN/RMGP01.png"), format!("HEAD {base}/cover/US/RMGP01.png")]);
    // A search also offers the other kinds that exist in that region.
    let found = e.find(SourceKind::GameTdb, &q, ArtSlot::Cover, false, Req::UI).unwrap().unwrap();
    let urls: Vec<&str> = found.iter().map(|(_, p)| p.url.as_str()).collect();
    assert_eq!(urls, [format!("{base}/cover/US/RMGP01.png"), format!("{base}/cover3D/US/RMGP01.png")]);
    // Disc art for the icon, found in the last region tried.
    let disc = e.find(SourceKind::GameTdb, &q, ArtSlot::Icon, true, Req::UI).unwrap().unwrap();
    assert_eq!(disc[0].1.url, format!("{base}/disc/JA/RMGP01.png"));
    // No backgrounds; nothing anywhere = no confident match.
    assert_eq!(e.find(SourceKind::GameTdb, &q, ArtSlot::Hero, true, Req::UI).unwrap(), Some(vec![]));
    let other = Query { serial: Some("RSBE01"), ..q.clone() };
    assert_eq!(e.find(SourceKind::GameTdb, &other, ArtSlot::Cover, true, Req::UI).unwrap(), None);
    // Without a game ID (or on a console GameTDB doesn't cover) it isn't asked.
    let no_id = Query { serial: None, ..q.clone() };
    assert!(!no_id.fits(SourceKind::GameTdb));
    let n64 = Query { console: console_platform(Some("Nintendo 64")), serial: Some("NSME"), ..q.clone() };
    assert!(!n64.fits(SourceKind::GameTdb) && !n64.searchable(SourceKind::GameTdb));
    // Through the editor search: labelled with the kind and region.
    let j = SearchJob {
        serial: Some("RMGP01".into()),
        list: ArtList::Emulated,
        ..job("Super Mario Galaxy", ArtSlot::Cover, Some("Nintendo Wii"))
    };
    let (items, errors, _) = search(&e, &j);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(items[0].label, "GameTDB · Super Mario Galaxy · front · US");
    assert_eq!(items[0].source, "GameTDB");
}

#[test]
fn emulated_games_send_their_serial() {
    let mut g = Game::blank("Mario Kart 7");
    g.emu = Some(crate::model::EmuRef {
        platform: "nintendo_3ds".into(),
        serial: Some("CTR-P-AMKE".into()),
        ..Default::default()
    });
    assert_eq!(gametdb_key_of(&g), Some(("3ds", "AMKE".into())));
    assert_eq!(SearchJob::new(&g, ArtSlot::Cover, None).serial.as_deref(), Some("CTR-P-AMKE"));
    assert_eq!(SearchJob::new(&g, ArtSlot::Cover, Some("Mario Kart")).serial, None);
    let r = crate::art_app::auto_request(&g, 0, &[SourceKind::GameTdb]).unwrap();
    assert_eq!(r.serial.as_deref(), Some("CTR-P-AMKE"));
    // Already asked (its bit) or no ID: GameTDB is left out.
    assert!(crate::art_app::auto_request(&g, SourceKind::GameTdb.bit(), &[SourceKind::GameTdb]).is_none());
    g.emu.as_mut().unwrap().serial = None;
    assert!(crate::art_app::auto_request(&g, 0, &[SourceKind::GameTdb]).is_none());
}
