use super::parse::*;
use super::*;
use std::collections::HashMap;

macro_rules! fixture {
    ($name:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/metadata/", $name)).as_slice()
    };
}

const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F'];
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

/// Canned responses by URL; unknown URLs → 404. `Err` entries simulate network failure.
#[derive(Default)]
struct FakeHttp {
    routes: HashMap<String, Result<(u16, Vec<u8>), String>>,
    calls: Vec<(String, Option<String>)>,
}

impl FakeHttp {
    fn ok(mut self, url: &str, body: &[u8]) -> Self {
        self.routes.insert(url.to_string(), Ok((200, body.to_vec())));
        self
    }
    fn status(mut self, url: &str, s: u16) -> Self {
        self.routes.insert(url.to_string(), Ok((s, vec![])));
        self
    }
    fn net_err(mut self, url: &str) -> Self {
        self.routes.insert(url.to_string(), Err("connection refused".into()));
        self
    }
}

impl Http for FakeHttp {
    fn get(&mut self, url: &str, bearer: Option<&str>, _limit: u64) -> Result<HttpResponse, String> {
        self.calls.push((url.to_string(), bearer.map(str::to_string)));
        match self.routes.get(url) {
            Some(Ok((status, body))) => Ok(HttpResponse { status: *status, body: body.clone() }),
            Some(Err(e)) => Err(e.clone()),
            None => Ok(HttpResponse { status: 404, body: vec![] }),
        }
    }

    /// Routed as "POST <url>\n<body>" (the Authorization header goes in `calls`' second field).
    fn post(&mut self, url: &str, headers: &[(&str, String)], body: &str, limit: u64) -> Result<HttpResponse, String> {
        let auth = headers.iter().find(|(k, _)| *k == "Authorization").map(|(_, v)| v.clone());
        let key = format!("POST {url}\n{body}");
        let r = self.get(&key, None, limit);
        if let Some(last) = self.calls.last_mut() {
            last.1 = auth;
        }
        r
    }
}

fn igdb_cfg(id: &str, secret: &str) -> MetadataConfig {
    MetadataConfig {
        steam_store: false,
        igdb_client_id: Some(id.into()),
        igdb_client_secret: Some(secret.into()),
        ..Default::default()
    }
}

fn token_route(id: &str, secret: &str) -> String {
    format!("POST {}\n", crate::igdb::token_url(id, secret))
}

#[test]
fn igdb_fills_metadata_of_a_non_steam_game() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, secret) = ("md-cid", "md-secret-1");
    crate::igdb::forget_token(id, secret);
    let search = format!("POST {}\n{}", crate::igdb::games_url(), crate::igdb::search_body("Celeste"));
    let mut w = worker(
        FakeHttp::default()
            .ok(&token_route(id, secret), fixture!("twitch_token.json"))
            .ok(&search, fixture!("igdb_games_celeste.json")),
        tmp.path(),
    );
    let job = Job {
        id: 3,
        name: "Celeste™".into(),
        steam_appid: None,
        want: Wanted { meta: true, extra: true, ..Default::default() },
        protondb_max_age: protondb::MANUAL_SECS,
        known_protondb: None,
        backfill: None,
        igdb_platform: None,
        wiki: Default::default(),
        lookup: None,
    };
    let cfg = igdb_cfg(id, secret);
    assert!(eligible(&job, &cfg));
    let r = w.process(&job, &cfg).unwrap();
    let m = &r.meta;
    assert!(m.description.as_deref().unwrap().starts_with("Help Madeline"));
    assert_eq!(m.developer.as_deref(), Some("Maddy Makes Games"));
    assert_eq!(m.genres, vec!["Platform", "Adventure", "Indie"]);
    assert_eq!(m.release_date.as_deref(), Some("2018-01-25"));
    assert_eq!((m.critic_score, m.community_score), (Some(92), Some(87)));
    assert_eq!(m.series.as_deref(), Some("Celeste"));
    assert_eq!(m.features, vec!["Single player", "Side view"]);
    assert_eq!(w.http.calls.last().unwrap().1.as_deref(), Some("Bearer jostpf5q0uzmxmkba9iyug38kjtgh"));
    // Not exactly the name: nothing (a confident match only).
    let other = Job { name: "Celeste Classic".into(), ..job.clone() };
    let search2 = format!("POST {}\n{}", crate::igdb::games_url(), crate::igdb::search_body("Celeste Classic"));
    w.http = std::mem::take(&mut w.http).ok(&search2, fixture!("igdb_games_celeste.json"));
    let r = w.process(&other, &cfg).unwrap();
    assert_eq!(r.meta, Metadata::default());
    // Without IGDB set up the job isn't the fetcher's.
    assert!(!eligible(&job, &MetadataConfig { steam_store: false, ..Default::default() }));
    let mut off = cfg.clone();
    off.igdb_metadata = false;
    assert!(!eligible(&job, &off));
    crate::igdb::forget_token(id, secret);
}

#[test]
fn igdb_fills_gaps_steam_left_by_appid() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, secret) = ("md-cid2", "md-secret-2");
    crate::igdb::forget_token(id, secret);
    let ext = format!("POST {}\n{}", crate::igdb::external_games_url(), crate::igdb::steam_body(1245620));
    let by_id = format!(
        "POST {}
{}",
        crate::igdb::games_url(),
        crate::igdb::by_ids_body(&[26226])
    );
    let mut w = worker(
        FakeHttp::default()
            .ok(&token_route(id, secret), fixture!("twitch_token.json"))
            .ok(&ext, fixture!("igdb_external_steam.json"))
            .ok(&by_id, fixture!("igdb_games_celeste.json"))
            .ok(&format!("{STEAM_STORE}?appids=1245620&l=english"), fixture!("appdetails_1245620.json")),
        tmp.path(),
    );
    let job = Job { want: Wanted { meta: true, extra: true, ..Default::default() }, ..steam_job(1245620) };
    let mut cfg = igdb_cfg(id, secret);
    cfg.steam_store = true;
    let r = w.process(&job, &cfg).unwrap();
    // The Steam store's answer wins; IGDB (found by the appid) only adds what it lacked.
    assert!(!r.meta.description.as_deref().unwrap().starts_with("Help Madeline"));
    assert_ne!(r.meta.developer.as_deref(), Some("Maddy Makes Games"));
    assert_eq!(r.meta.series.as_deref(), Some("Celeste"));
    assert_eq!(r.meta.critic_score, Some(92));
    crate::igdb::forget_token(id, secret);
}

#[test]
fn igdb_backfill_asks_only_igdb_and_only_fills_gaps() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, secret) = ("md-cid5", "md-secret-5");
    crate::igdb::forget_token(id, secret);
    let mut g = steam_game_for_fetcher();
    g.source_id = "1245620".into();
    g.launch = Some(LaunchSpec::Steam { appid: 1245620 });
    g.meta = Metadata {
        description: Some("From the Steam store".into()),
        developer: Some("Extremely OK Games".into()),
        genres: vec!["Indie".into()],
        release_year: Some(2018),
        ..Default::default()
    };
    // Everything the store gave and nothing IGDB-only missing: nothing to do.
    let mut full = g.clone();
    full.meta.series = Some("x".into());
    assert!(Job::igdb_backfill(&full).is_none());
    let job = Job::igdb_backfill(&g).unwrap();
    assert!(!job.want.meta && job.want.extra && !job.want.protondb && !job.want.any_art());
    let ext = format!("POST {}\n{}", crate::igdb::external_games_url(), crate::igdb::steam_body(1245620));
    let by_id = format!("POST {}\n{}", crate::igdb::games_url(), crate::igdb::by_ids_body(&[26226]));
    let mut w = worker(
        FakeHttp::default()
            .ok(&token_route(id, secret), fixture!("twitch_token.json"))
            .ok(&ext, fixture!("igdb_external_steam.json"))
            .ok(&by_id, fixture!("igdb_games_celeste.json")),
        tmp.path(),
    );
    let mut cfg = igdb_cfg(id, secret);
    cfg.steam_store = true;
    assert!(eligible(&job, &cfg));
    let r = w.process(&job, &cfg).unwrap();
    assert!(r.backfill);
    // The Steam store isn't asked again; what the game has is left out of the answer.
    assert!(w.http.calls.iter().all(|(u, _)| !u.contains("steampowered")), "{:?}", w.http.calls);
    assert_eq!((r.meta.description.as_ref(), r.meta.developer.as_ref()), (None, None));
    assert!(r.meta.genres.is_empty() && r.meta.release_year.is_none());
    assert_eq!(r.meta.series.as_deref(), Some("Celeste"));
    assert_eq!(r.meta.critic_score, Some(92));
    assert_eq!(r.meta.release_date.as_deref(), Some("2018-01-25"));
    assert_eq!(r.meta.publisher.as_deref(), Some("Maddy Makes Games"));
    // Without IGDB set up it isn't the fetcher's.
    assert!(!eligible(&job, &MetadataConfig::default()));
    crate::igdb::forget_token(id, secret);
}

#[test]
fn igdb_trouble_never_leaks_secrets_or_retries_bad_credentials() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, secret) = ("md-cid3", "md-secret-3");
    crate::igdb::forget_token(id, secret);
    let job = Job {
        id: 4,
        name: "Celeste".into(),
        steam_appid: None,
        want: Wanted { meta: true, ..Default::default() },
        protondb_max_age: protondb::MANUAL_SECS,
        known_protondb: None,
        backfill: None,
        igdb_platform: None,
        wiki: Default::default(),
        lookup: None,
    };
    let cfg = igdb_cfg(id, secret);
    let mut w = worker(
        FakeHttp {
            routes: [(token_route(id, secret), Err(format!("timeout: {}", crate::igdb::token_url(id, secret))))].into(),
            calls: vec![],
        },
        tmp.path(),
    );
    match w.process(&job, &cfg) {
        Err(Transient(e)) => assert!(!e.contains(secret) && e.contains("***"), "{e}"),
        Ok(_) => panic!("a network error is retried later"),
    }
    // Rejected credentials: one token request, then never again this session.
    let mut w = worker(FakeHttp::default().status(&token_route(id, secret), 400), tmp.path());
    assert_eq!(w.process(&job, &cfg).unwrap().meta, Metadata::default());
    assert_eq!(w.process(&job, &cfg).unwrap().meta, Metadata::default());
    assert_eq!(w.http.calls.len(), 1);
}

fn worker(http: FakeHttp, dir: &Path) -> Worker<FakeHttp> {
    Worker {
        http,
        art_dir: dir.to_path_buf(),
        throttle: Throttle::none(),
        sgdb_disabled: false,
        proton: Default::default(),
        protondb_paused_until: None,
    }
}

fn steam_job(appid: u32) -> Job {
    Job {
        id: 7,
        name: "ELDEN RING".into(),
        steam_appid: Some(appid),
        want: Wanted { cover: true, hero: true, logo: true, banner: true, meta: true, protondb: false, extra: false },
        protondb_max_age: protondb::MANUAL_SECS,
        known_protondb: None,
        backfill: None,
        igdb_platform: None,
        wiki: Default::default(),
        lookup: None,
    }
}

fn cfg(steam: bool, key: Option<&str>) -> MetadataConfig {
    MetadataConfig {
        steam_store: steam,
        steamgriddb_key: key.map(str::to_string),
        // Wikipedia has its own tests (below); here it would make every game eligible.
        wikipedia: false,
        ..Default::default()
    }
}

fn cdn(appid: u32, f: &str) -> String {
    format!("{STEAM_CDN}/{appid}/{f}")
}

#[test]
fn appdetails_full() {
    let AppDetails::Found(m) = parse_appdetails(fixture!("appdetails_1245620.json"), 1245620) else { panic!() };
    assert!(m.description.as_deref().unwrap().starts_with("THE NEW FANTASY ACTION RPG."));
    assert_eq!(m.developer.as_deref(), Some("FromSoftware, Inc."));
    assert_eq!(m.publisher.as_deref(), Some("FromSoftware, Inc., Bandai Namco Entertainment"));
    assert_eq!(m.genres, vec!["Action", "RPG"]);
    assert_eq!(m.release_year, Some(2022));
}

#[test]
fn appdetails_about_fallback_and_html() {
    let AppDetails::Found(m) = parse_appdetails(fixture!("appdetails_about_only.json"), 440) else { panic!() };
    let d = m.description.unwrap();
    assert_eq!(
        d,
        "Welcome to Example & friends — it's \"great\" fun.\n\n• Feature one\n\n• Feature two\n\nLast © line &unknown; ok"
    );
    assert!(!d.contains("alert"));
    assert_eq!(m.developer, None);
    assert!(m.genres.is_empty());
    assert_eq!(m.release_year, None);
}

#[test]
fn appdetails_failures() {
    assert_eq!(parse_appdetails(fixture!("appdetails_fail.json"), 999999), AppDetails::NotFound);
    assert_eq!(parse_appdetails(b"null", 1), AppDetails::Invalid);
    assert_eq!(parse_appdetails(b"<html>", 1), AppDetails::Invalid);
    assert_eq!(parse_appdetails(fixture!("appdetails_1245620.json"), 1), AppDetails::Invalid);
}

#[test]
fn html_and_text_helpers() {
    assert_eq!(strip_html("a<br>b<br/>c"), "a\nb\nc");
    assert_eq!(strip_html("1 &lt; 2 &amp;&amp; 3 &gt; 2"), "1 < 2 && 3 > 2");
    assert_eq!(strip_html("x &#8212; y &#x2014; z"), "x — y — z");
    assert_eq!(strip_html("  lots   of\t\tspace  "), "lots of space");
    assert_eq!(strip_html("a < b"), "a < b");
    assert_eq!(strip_html("<STYLE>p{}</style>ok"), "ok");
    assert_eq!(decode_entities("AT&T"), "AT&T");
    assert_eq!(decode_entities("&#0;"), "&#0;");
    let long = "word ".repeat(400);
    let t = truncate_words(&long, ABOUT_MAX_CHARS);
    assert!(t.chars().count() <= ABOUT_MAX_CHARS + 1);
    assert!(t.ends_with("word…"));
    assert_eq!(truncate_words("short", 10), "short");
    assert_eq!(parse_year("Feb 24, 2022"), Some(2022));
    assert_eq!(parse_year("Q3 1998"), Some(1998));
    assert_eq!(parse_year("12345 0042"), None);
    assert_eq!(encode_path_segment("Half-Life 2: Ep/1 ü"), "Half-Life%202%3A%20Ep%2F1%20%C3%BC");
    assert_eq!(search_name("DOOM™ (2016)"), "DOOM");
    assert_eq!(search_name("RuneScape ®"), "RuneScape");
    assert_eq!(image_ext(JPEG), Some("jpg"));
    assert_eq!(image_ext(PNG), Some("png"));
    assert_eq!(image_ext(b"<html>"), None);
}

#[test]
fn sgdb_parsers() {
    assert_eq!(parse_sgdb_search(fixture!("sgdb_search.json")), Some(Some(5247)));
    assert_eq!(parse_sgdb_search(fixture!("sgdb_search_empty.json")), Some(None));
    assert_eq!(parse_sgdb_search(fixture!("sgdb_unauthorized.json")), None);
    assert_eq!(
        parse_sgdb_assets(fixture!("sgdb_grids.json")),
        Some(Some("https://cdn2.steamgriddb.com/grid/abc123.png".into()))
    );
    assert_eq!(parse_sgdb_assets(fixture!("sgdb_logos_empty.json")), Some(None));
    assert_eq!(parse_sgdb_assets(b"garbage"), None);
}

#[test]
fn steam_job_fills_missing_with_fallbacks() {
    let tmp = tempfile::tempdir().unwrap();
    let id = 1245620;
    let http = FakeHttp::default()
        .status(&cdn(id, "library_600x900_2x.jpg"), 404)
        .ok(&cdn(id, "library_600x900.jpg"), JPEG)
        .ok(&cdn(id, "library_hero.jpg"), JPEG)
        // logo.png -> 404 by default
        .ok(&cdn(id, "header.jpg"), b"<html>not an image</html>")
        .ok(&format!("{STEAM_STORE}?appids={id}&l=english"), fixture!("appdetails_1245620.json"));
    let mut w = worker(http, tmp.path());
    let r = w.process(&steam_job(id), &cfg(true, None)).unwrap();
    let dir = tmp.path().join("steam/1245620");
    assert_eq!(r.art.cover, Some(dir.join("library_600x900.jpg")));
    assert_eq!(r.art.hero, Some(dir.join("library_hero.jpg")));
    assert_eq!(r.art.logo, None);
    assert_eq!(r.art.banner, None, "html body must not be saved as art");
    assert!(dir.join("library_600x900.jpg").exists());
    assert_eq!(std::fs::read(dir.join("library_hero.jpg")).unwrap(), JPEG);
    assert_eq!(r.meta.developer.as_deref(), Some("FromSoftware, Inc."));
    assert!(w.http.calls.iter().all(|(_, b)| b.is_none()), "no auth header to Steam");
    // Leftover temp files?
    assert!(std::fs::read_dir(&dir).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().contains("part")));

    // Second run reuses the files on disk: only the missing ones are asked for again.
    let mut w2 = worker(FakeHttp::default(), tmp.path());
    let job = Job { want: Wanted { cover: true, hero: true, ..Default::default() }, ..steam_job(id) };
    let r2 = w2.process(&job, &cfg(true, None)).unwrap();
    assert_eq!(r2.art.cover, r.art.cover);
    assert!(w2.http.calls.is_empty(), "{:?}", w2.http.calls);
}

#[test]
fn steam_only_requests_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let mut w = worker(FakeHttp::default().ok(&cdn(10, "logo.png"), PNG), tmp.path());
    let job = Job { want: Wanted { logo: true, ..Default::default() }, ..steam_job(10) };
    let r = w.process(&job, &cfg(true, None)).unwrap();
    assert_eq!(r.art.logo, Some(tmp.path().join("steam/10/logo.png")));
    assert_eq!(w.http.calls.len(), 1);
}

#[test]
fn network_error_is_transient() {
    let tmp = tempfile::tempdir().unwrap();
    let http = FakeHttp::default().net_err(&cdn(5, "library_600x900_2x.jpg"));
    let mut w = worker(http, tmp.path());
    assert!(w.process(&steam_job(5), &cfg(true, None)).is_err());
    // 5xx and 429 too; 404 for everything is definitive (empty result).
    let mut w = worker(FakeHttp::default().status(&cdn(5, "library_600x900_2x.jpg"), 503), tmp.path());
    assert!(w.process(&steam_job(5), &cfg(true, None)).is_err());
    let url = format!("{STEAM_STORE}?appids=5&l=english");
    let mut w = worker(FakeHttp::default().status(&url, 429), tmp.path());
    assert!(w.process(&steam_job(5), &cfg(true, None)).is_err());
    let mut w = worker(FakeHttp::default().ok(&url, b"null"), tmp.path());
    assert!(w.process(&steam_job(5), &cfg(true, None)).is_err());
    let mut w = worker(FakeHttp::default().ok(&url, fixture!("appdetails_fail.json")), tmp.path());
    let r = w.process(&Job { steam_appid: Some(999999), ..steam_job(5) }, &cfg(true, None)).unwrap();
    assert!(r.art.is_empty());
    assert_eq!(r.meta, Metadata::default());
}

fn sgdb_job() -> Job {
    Job {
        id: 42,
        name: "Old School RuneScape™".into(),
        steam_appid: None,
        want: Wanted { cover: true, hero: true, logo: true, banner: true, meta: true, protondb: false, extra: false },
        protondb_max_age: protondb::MANUAL_SECS,
        known_protondb: None,
        backfill: None,
        igdb_platform: None,
        wiki: Default::default(),
        lookup: None,
    }
}

#[test]
fn sgdb_non_steam_game() {
    let tmp = tempfile::tempdir().unwrap();
    let http = FakeHttp::default()
        .ok(&format!("{SGDB_API}/search/autocomplete/Old%20School%20RuneScape"), fixture!("sgdb_search.json"))
        .ok(&format!("{SGDB_API}/grids/game/5247?dimensions=600x900&types=static"), fixture!("sgdb_grids.json"))
        .ok(&format!("{SGDB_API}/heroes/game/5247?types=static"), fixture!("sgdb_heroes.json"))
        .ok(&format!("{SGDB_API}/logos/game/5247?types=static"), fixture!("sgdb_logos_empty.json"))
        .ok("https://cdn2.steamgriddb.com/grid/abc123.png", PNG)
        .ok("https://cdn2.steamgriddb.com/hero/fff000.jpg", JPEG);
    let mut w = worker(http, tmp.path());
    let r = w.process(&sgdb_job(), &cfg(true, Some("KEY"))).unwrap();
    let dir = tmp.path().join("sgdb/42");
    assert_eq!(r.art.cover, Some(dir.join("cover.png")));
    assert_eq!(r.art.hero, Some(dir.join("hero.jpg")));
    assert_eq!(r.art.logo, None);
    // API calls carry the key; the image CDN downloads don't.
    for (url, bearer) in &w.http.calls {
        assert_eq!(bearer.is_some(), url.starts_with(SGDB_API), "{url}");
        if let Some(b) = bearer {
            assert_eq!(b, "KEY");
        }
    }
}

#[test]
fn sgdb_no_match_is_definitive_and_bad_key_is_not() {
    let tmp = tempfile::tempdir().unwrap();
    let search = format!("{SGDB_API}/search/autocomplete/Old%20School%20RuneScape");
    let mut w = worker(FakeHttp::default().ok(&search, fixture!("sgdb_search_empty.json")), tmp.path());
    let r = w.process(&sgdb_job(), &cfg(false, Some("KEY"))).unwrap();
    assert!(r.art.is_empty());

    let mut w = worker(FakeHttp::default().status(&search, 401), tmp.path());
    assert!(w.process(&sgdb_job(), &cfg(false, Some("KEY"))).is_err());
    assert!(w.sgdb_disabled);
    // Not asked again this session.
    let n = w.http.calls.len();
    let _ = w.process(&sgdb_job(), &cfg(false, Some("KEY")));
    assert_eq!(w.http.calls.len(), n);
}

#[test]
fn sgdb_fallback_for_steam_game_without_cdn_art() {
    let tmp = tempfile::tempdir().unwrap();
    let http = FakeHttp::default()
        .ok(&format!("{STEAM_STORE}?appids=77&l=english"), br#"{"77":{"success":false}}"#)
        .ok(&format!("{SGDB_API}/grids/steam/77?dimensions=600x900&types=static"), fixture!("sgdb_grids.json"))
        .ok("https://cdn2.steamgriddb.com/grid/abc123.png", PNG);
    let mut w = worker(http, tmp.path());
    let job = Job { id: 3, steam_appid: Some(77), ..sgdb_job() };
    let r = w.process(&job, &cfg(true, Some("KEY"))).unwrap();
    assert_eq!(r.art.cover, Some(tmp.path().join("sgdb/3/cover.png")));
    assert!(w.http.calls.iter().all(|(u, _)| !u.contains("search")), "steam games are addressed by appid");
}

#[test]
fn eligibility_and_wanted() {
    let mut g = Game {
        id: 1,
        source: Source::Steam,
        source_id: "620".into(),
        launch: Some(LaunchSpec::Steam { appid: 620 }),
        ..Game::blank("Portal 2")
    };
    let j = Job::from_game(&g);
    assert_eq!(j.steam_appid, Some(620));
    assert!(eligible(&j, &cfg(true, None)));
    assert!(!eligible(&j, &cfg(false, None)));
    g.source = Source::Desktop;
    g.launch = None;
    let j = Job::from_game(&g);
    assert_eq!(j.steam_appid, None);
    assert!(!eligible(&j, &cfg(true, None)));
    assert!(eligible(&j, &cfg(true, Some("k"))));
    assert!(!eligible(&j, &cfg(true, Some("  "))));
    // Everything present → nothing to do.
    g.art = Art {
        cover: Some("/a".into()),
        hero: Some("/b".into()),
        logo: Some("/c".into()),
        icon: None,
        banner: Some("/d".into()),
    };
    g.meta = Metadata {
        description: Some("x".into()),
        developer: Some("y".into()),
        publisher: None,
        genres: vec!["z".into()],
        release_year: Some(2000),
        ..Default::default()
    };
    assert!(!eligible(&Job::from_game(&g), &cfg(true, Some("k"))));
}

#[test]
fn fetcher_dedups_and_does_not_block() {
    // Real fetcher with networking disabled by config: requests are ignored, nothing pending.
    let tmp = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut f = MetadataFetcher::new(&ctx, tmp.path().to_path_buf(), &cfg(false, None));
    let g = Game { id: 9, source: Source::Desktop, source_id: "x".into(), ..Game::blank("X") };
    f.request(&g);
    assert_eq!(f.pending(), 0);
    assert!(f.poll().is_empty());
    f.set_config(&cfg(true, None));
    f.request(&g); // still not eligible (not Steam, no key)
    assert_eq!(f.pending(), 0);
}

/// Live check against the real Steam store (run manually:
/// `cargo test metadata::tests::live -- --ignored --nocapture`).
#[test]
#[ignore]
fn live_steam_1245620() {
    let tmp = tempfile::tempdir().unwrap();
    let mut w = Worker {
        http: UreqHttp::new(),
        art_dir: tmp.path().to_path_buf(),
        throttle: Throttle::polite(),
        sgdb_disabled: false,
        proton: Default::default(),
        protondb_paused_until: None,
    };
    let t0 = Instant::now();
    let r = w.process(&steam_job(1245620), &cfg(true, None));
    eprintln!("took {:?}", t0.elapsed());
    match r {
        Ok(r) => {
            eprintln!("art: {:#?}", r.art);
            eprintln!("meta: {:#?}", r.meta);
            for p in [&r.art.cover, &r.art.hero, &r.art.logo, &r.art.banner].into_iter().flatten() {
                eprintln!("{} {} bytes", p.display(), std::fs::metadata(p).map(|m| m.len()).unwrap_or(0));
            }
        }
        Err(e) => eprintln!("transient: {e:?}"),
    }
}

/// Steam's appdetails API allows ~200 requests per 5 minutes; the polite throttle must stay
/// under that so a large library isn't cut off by 429s part way through.
#[test]
fn polite_store_spacing_respects_steam_rate_limit() {
    let t = Throttle::polite();
    assert!(t.store * 200 >= Duration::from_secs(300), "{:?}", t.store);
}

// ---------------------------------------------------------------------------------------------
// ProtonDB

fn proton_job(appid: u32) -> Job {
    Job {
        want: Wanted { protondb: true, extra: false, ..Default::default() },
        protondb_max_age: protondb::REFRESH_SECS,
        ..steam_job(appid)
    }
}

#[test]
fn protondb_summary_parsing() {
    use protondb::parse_summary;
    assert_eq!(parse_summary(fixture!("protondb_1245620.json")), Some(Some("gold".into())));
    assert_eq!(parse_summary(fixture!("protondb_pending.json")), Some(Some("pending".into())));
    // Unknown "tier" → the trending one, case-folded.
    assert_eq!(parse_summary(fixture!("protondb_odd.json")), Some(Some("silver".into())));
    assert_eq!(parse_summary(br#"{"total":0}"#), Some(None));
    assert_eq!(parse_summary(b"<html>Cloudflare</html>"), None);
    assert_eq!(parse_summary(b"[1,2]"), None);
    assert_eq!(protondb::tier_label("gold"), Some("Gold"));
    assert_eq!(protondb::tier_label("Borked"), Some("Borked"));
    assert_eq!(protondb::tier_label("shiny"), None);
    assert!(protondb::tier_rgb("platinum").is_some());
    for t in protondb::TIERS {
        assert!(protondb::tier_label(t).is_some() && protondb::tier_rgb(t).is_some());
    }
}

#[test]
fn protondb_fetch_cache_and_refresh_window() {
    let tmp = tempfile::tempdir().unwrap();
    let url = protondb::url(1245620);
    let mut w = worker(FakeHttp::default().ok(&url, fixture!("protondb_1245620.json")), tmp.path());
    let r = w.process(&proton_job(1245620), &cfg(true, None)).unwrap();
    assert_eq!(r.meta.protondb.as_deref(), Some("gold"));
    assert_eq!(w.http.calls.len(), 1);
    assert_eq!(w.http.calls[0], (url.clone(), None));
    // Fresh: answered from the cache, no request.
    let r = w.process(&proton_job(1245620), &cfg(true, None)).unwrap();
    assert_eq!(r.meta.protondb.as_deref(), Some("gold"));
    assert_eq!(w.http.calls.len(), 1);
    // Saved and loaded again.
    w.save_protondb();
    let c = protondb::Cache::load(&protondb::Cache::path(tmp.path()));
    let e = c.get(1245620).unwrap();
    assert_eq!(e.tier.as_deref(), Some("gold"));
    assert!(c.is_fresh(1245620, protondb::REFRESH_SECS, now_unix()));
    // ~2 weeks later it is asked again, but not after 13 days.
    let at = e.at;
    assert!(c.is_fresh(1245620, protondb::REFRESH_SECS, at + 13 * 86400));
    assert!(!c.is_fresh(1245620, protondb::REFRESH_SECS, at + 15 * 86400));
    // A timestamp from a wrong (future) clock doesn't block refreshes for good.
    assert!(!c.is_fresh(1245620, protondb::REFRESH_SECS, at - 3 * 86400));
    assert!(!c.is_fresh(1, protondb::REFRESH_SECS, at));
}

#[test]
fn protondb_no_report_is_definitive_and_errors_are_not() {
    let tmp = tempfile::tempdir().unwrap();
    // 404 = no reports yet: remembered (not asked again within the window), no tier.
    let mut w = worker(FakeHttp::default(), tmp.path());
    let r = w.process(&proton_job(5), &cfg(true, None)).unwrap();
    assert_eq!(r.meta.protondb, None);
    assert!(w.proton.lock().is_fresh(5, protondb::REFRESH_SECS, now_unix()));
    w.process(&proton_job(5), &cfg(true, None)).unwrap();
    assert_eq!(w.http.calls.len(), 1);

    // Server trouble: nothing remembered, ProtonDB paused, but the job's other results stand.
    let mut w = worker(FakeHttp::default().status(&protondb::url(6), 503).ok(&cdn(6, "logo.png"), PNG), tmp.path());
    let job = Job { want: Wanted { logo: true, protondb: true, extra: false, ..Default::default() }, ..steam_job(6) };
    let r = w.process(&job, &cfg(true, None)).unwrap();
    assert!(r.art.logo.is_some());
    assert_eq!(r.meta.protondb, None);
    assert!(w.proton.lock().get(6).is_none());
    assert!(w.protondb_paused_until.is_some());
    // While paused, no further ProtonDB requests.
    let n = w.http.calls.len();
    w.process(&proton_job(7), &cfg(true, None)).unwrap();
    assert_eq!(w.http.calls.len(), n);

    // Network failure and junk bodies behave the same.
    for http in [
        FakeHttp::default().net_err(&protondb::url(8)),
        FakeHttp::default().ok(&protondb::url(8), b"<html>"),
        FakeHttp::default().status(&protondb::url(8), 429),
    ] {
        let mut w = worker(http, tmp.path());
        let r = w.process(&proton_job(8), &cfg(true, None)).unwrap();
        assert_eq!(r.meta.protondb, None);
        assert!(w.proton.lock().get(8).is_none());
        assert!(w.protondb_paused_until.is_some());
    }

    // Steam store lookups switched off: ProtonDB isn't asked either.
    let mut w = worker(FakeHttp::default(), tmp.path());
    w.process(&proton_job(9), &cfg(false, None)).unwrap();
    assert!(w.http.calls.is_empty());
    assert!(!eligible(&proton_job(9), &cfg(false, None)));
    assert!(eligible(&proton_job(9), &cfg(true, None)));
}

#[test]
fn protondb_jobs_for_games() {
    let mut g = Game {
        id: 3,
        source: Source::Steam,
        source_id: "620".into(),
        launch: Some(LaunchSpec::Steam { appid: 620 }),
        art: Art {
            cover: Some("/a".into()),
            hero: Some("/b".into()),
            logo: Some("/c".into()),
            icon: None,
            banner: Some("/d".into()),
        },
        meta: Metadata {
            description: Some("x".into()),
            developer: Some("y".into()),
            genres: vec!["z".into()],
            release_year: Some(2011),
            protondb: Some("platinum".into()),
            ..Default::default()
        },
        ..Game::blank("Portal 2")
    };
    // Download metadata… on a complete Steam game still refreshes the tier (daily at most).
    let j = Job::from_game(&g);
    assert!(j.want.protondb && j.want.protondb_only());
    assert_eq!(j.protondb_max_age, protondb::MANUAL_SECS);
    assert_eq!(j.known_protondb.as_deref(), Some("platinum"));
    assert!(eligible(&j, &cfg(true, None)));
    let b = Job::protondb_refresh(&g).unwrap();
    assert!(b.want.protondb_only());
    assert_eq!(b.protondb_max_age, protondb::REFRESH_SECS);
    // Not a Steam game: no ProtonDB.
    g.source = Source::Gog;
    g.launch = None;
    assert!(Job::protondb_refresh(&g).is_none());
    assert!(!Job::from_game(&g).want.protondb);
}

#[test]
fn fetcher_background_refresh_is_silent_when_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut f = MetadataFetcher::new(&ctx, tmp.path().to_path_buf(), &cfg(false, None));
    let g = Game { meta: Metadata::default(), ..steam_game_for_fetcher() };
    f.request_protondb(&g);
    assert_eq!(f.pending(), 0);
    assert!(f.poll().is_empty());
    assert_eq!(f.protondb_cached(620), None);
}

fn steam_game_for_fetcher() -> Game {
    Game {
        id: 11,
        source: Source::Steam,
        source_id: "620".into(),
        installed: false,
        launch: Some(LaunchSpec::Steam { appid: 620 }),
        ..Game::blank("Portal 2")
    }
}

/// Live check against ProtonDB (run manually:
/// `cargo test metadata::tests::live_protondb -- --ignored --nocapture`).
#[test]
#[ignore]
fn live_protondb_1245620() {
    let tmp = tempfile::tempdir().unwrap();
    let mut w = Worker {
        http: UreqHttp::new(),
        art_dir: tmp.path().to_path_buf(),
        throttle: Throttle::polite(),
        sgdb_disabled: false,
        proton: Default::default(),
        protondb_paused_until: None,
    };
    let t0 = Instant::now();
    let r = w.process(&proton_job(1245620), &cfg(true, None)).unwrap();
    eprintln!("took {:?}: tier {:?}", t0.elapsed(), r.meta.protondb);
    assert!(r.meta.protondb.is_some(), "paused: {:?}", w.protondb_paused_until);
}

/// A non-Steam game with no built-in source: an enabled metadata add-on fills its description
/// and art (a local file and a downloaded URL); built-in answers are never overwritten.
#[test]
fn metadata_addons_fill_gaps() {
    use crate::test_support::write_exe;
    let tmp = tempfile::tempdir().unwrap();
    let addons_dir = tmp.path().join("addons");
    let dir = addons_dir.join("meta-test");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("addon.toml"),
        "id = \"meta-test\"\nname = \"Meta Test\"\nkind = \"metadata\"\nexec = \"run.sh\"\ntimeout_secs = 5\n",
    )
    .unwrap();
    std::fs::write(dir.join("hero.png"), PNG).unwrap();
    write_exe(
        &dir.join("run.sh"),
        "#!/bin/sh\ncat >/dev/null\ncase \"$1\" in\n\
         search) echo '{\"results\":[{\"id\":\"1\",\"name\":\"Diablo II: Resurrected\"}]}' ;;\n\
         fetch) echo '{\"description\":\"From the add-on.\",\"developer\":\"Blizzard\",\"cover\":\"https://img.test/c.png\",\"hero\":\"hero.png\"}' ;;\n\
         esac\n",
    );
    let mut mgr = crate::addons::AddonManager::new(addons_dir);
    mgr.refresh();
    let acfg = crate::config::AddonsConfig { enabled: vec!["meta-test".into()], ..Default::default() };
    let a = AddonSources { mgr, cfg: acfg };
    assert!(a.any());

    let art_dir = tmp.path().join("art");
    let mut w = worker(FakeHttp::default().ok("https://img.test/c.png", PNG), &art_dir);
    let job = Job {
        id: 42,
        name: "Diablo II: Resurrected".into(),
        steam_appid: None,
        want: Wanted { cover: true, hero: true, logo: false, banner: false, meta: true, protondb: false, extra: false },
        protondb_max_age: protondb::MANUAL_SECS,
        known_protondb: None,
        backfill: None,
        igdb_platform: None,
        wiki: Default::default(),
        lookup: None,
    };
    assert!(!eligible(&job, &cfg(true, None)) && addon_eligible(&job));
    let mut res = FetchResult::empty(&job);
    res.meta.developer = Some("Built-in".into());
    w.ask_addons(&job, &a, &mut res);
    assert_eq!(res.meta.description.as_deref(), Some("From the add-on."));
    assert_eq!(res.meta.developer.as_deref(), Some("Built-in"), "built-in answers win");
    assert_eq!(res.art.hero, Some(dir.join("hero.png")));
    assert_eq!(res.art.cover, Some(art_dir.join("addon/42/cover.png")));
    assert!(res.art.cover.as_ref().unwrap().is_file());

    // Switched off: not asked.
    let off = AddonSources { mgr: a.mgr.clone(), cfg: Default::default() };
    assert!(!off.any());
}

#[test]
fn igdb_name_search_of_a_console_game_stays_on_its_platform() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, secret) = ("md-cid-plat", "md-secret-plat");
    crate::igdb::forget_token(id, secret);
    // Only the PlayStation 2 search is answered: an unfiltered one would find the PC game.
    let search = format!("POST {}\n{}", crate::igdb::games_url(), crate::igdb::search_body_on("Celeste", Some(8)));
    assert!(search.contains("where platforms = (8);"));
    let mut w = worker(
        FakeHttp::default()
            .ok(&token_route(id, secret), fixture!("twitch_token.json"))
            .ok(&search, fixture!("igdb_games_celeste.json")),
        tmp.path(),
    );
    let mut g = Game { id: 7, source: Source::Emulator, ..Game::blank("Celeste") };
    g.emu = Some(crate::model::EmuRef { platform: "sony_playstation2".into(), ..Default::default() });
    let mut job = Job::from_game(&g);
    assert_eq!(job.igdb_platform, Some(8));
    job.want = Wanted { meta: true, ..Default::default() };
    let r = w.process(&job, &igdb_cfg(id, secret)).unwrap();
    assert_eq!(r.meta.developer.as_deref(), Some("Maddy Makes Games"));
    crate::igdb::forget_token(id, secret);
}

#[test]
fn igdb_platform_of_games() {
    let mut g = Game { source: Source::Lutris, ..Game::blank("X") };
    assert_eq!(crate::igdb::platform_of(&g), None);
    g.platform = Some("Linux".into());
    assert_eq!(crate::igdb::platform_of(&g), None, "PC games are not filtered");
    g.platform = Some("Nintendo 64".into());
    assert_eq!(crate::igdb::platform_of(&g), Some(4));
    // Emulated: the emulation platform wins over a hand-set library platform.
    g.platform = Some("Whatever".into());
    g.emu = Some(crate::model::EmuRef { platform: "nintendo_switch".into(), ..Default::default() });
    assert_eq!(crate::igdb::platform_of(&g), Some(130));
    assert_eq!(crate::igdb::search_body_on("Tekken 5", None), crate::igdb::search_body("Tekken 5"));
    // Japan-only NES games are IGDB "Family Computer" games.
    assert!(crate::igdb::search_body_on("Zelda no Densetsu", Some(18)).contains("where platforms = (18,99);"));
}

// ---------------------------------------------------------------------------------------------
// Wikipedia

fn wiki_cfg(steam: bool, prefer: bool) -> MetadataConfig {
    MetadataConfig { steam_store: steam, wikipedia: true, prefer_wikipedia: prefer, ..Default::default() }
}

fn doom_game(year: Option<i32>) -> Game {
    let mut g = Game { id: 21, source: Source::Desktop, source_id: "doom".into(), ..Game::blank("DOOM") };
    g.meta.release_year = year;
    g
}

#[test]
fn wikipedia_fills_a_missing_description_with_its_credit() {
    let tmp = tempfile::tempdir().unwrap();
    let url = wikipedia::search_url("DOOM");
    let mut w = worker(FakeHttp::default().ok(&url, fixture!("wikipedia_search_doom.json")), tmp.path());
    let job = Job::from_game(&doom_game(Some(1993)));
    let cfg = wiki_cfg(false, false);
    assert!(eligible(&job, &cfg), "Wikipedia alone makes a game without a description eligible");
    assert!(!eligible(&job, &MetadataConfig { wikipedia: false, ..cfg.clone() }));
    let r = w.process(&job, &cfg).unwrap();
    assert!(r.meta.description.as_deref().unwrap().starts_with("Doom is a 1993 first-person shooter"));
    assert_eq!(r.meta.wikipedia.as_deref(), Some("https://en.wikipedia.org/wiki/Doom_(1993_video_game)"));
    assert_eq!(w.http.calls.len(), 1, "one request per game");
    assert!(r.wiki_backfill, "asked once: no background lookup later");
    // Same name, no year: two games of that name → nothing (and no error).
    let r = w.process(&Job::from_game(&doom_game(None)), &cfg).unwrap();
    assert_eq!((r.meta.description, r.meta.wikipedia), (None, None));
}

#[test]
fn wikipedia_respects_existing_and_user_descriptions() {
    let mut g = doom_game(Some(1993));
    g.meta.description = Some("From the launcher.".into());
    let cfg = wiki_cfg(false, false);
    assert!(!eligible(&Job::from_game(&g), &cfg), "has a description, Wikipedia not preferred");
    assert!(eligible(&Job::from_game(&g), &wiki_cfg(false, true)), "Wikipedia first");
    // The user's own text is never replaced.
    g.edits.description = Some("Mine.".into());
    let g = g.with_edits_applied();
    assert!(!eligible(&Job::from_game(&g), &wiki_cfg(false, true)));
    // Already Wikipedia's: not asked again.
    let mut g = doom_game(Some(1993));
    g.meta.description = Some("Doom is …".into());
    g.meta.wikipedia = Some("https://en.wikipedia.org/wiki/Doom_(1993_video_game)".into());
    assert!(!eligible(&Job::from_game(&g), &wiki_cfg(false, true)));
    assert!(Job::wikipedia_backfill(&g, true).is_none());
}

#[test]
fn wikipedia_first_replaces_the_store_text() {
    let tmp = tempfile::tempdir().unwrap();
    let id = 1245620;
    let mut job = steam_job(id);
    job.name = "Doom".into();
    job.want = Wanted { meta: true, ..Default::default() };
    job.wiki = WikiJob { allowed: true, missing: true, year: Some(2016), ..Default::default() };
    let http = FakeHttp::default()
        .ok(&format!("{STEAM_STORE}?appids={id}&l=english"), fixture!("appdetails_1245620.json"))
        .ok(&wikipedia::search_url("Doom"), fixture!("wikipedia_search_doom.json"));
    let mut w = worker(http, tmp.path());
    // Not preferred: the store's description stays, Wikipedia isn't asked.
    let r = w.process(&job, &wiki_cfg(true, false)).unwrap();
    assert!(r.meta.wikipedia.is_none() && r.meta.description.is_some() && !r.wiki_backfill);
    assert!(!w.http.calls.iter().any(|(u, _)| u.contains("wikipedia")));
    // Preferred: Wikipedia's text, the store's other facts.
    let r = w.process(&job, &wiki_cfg(true, true)).unwrap();
    assert_eq!(r.meta.wikipedia.as_deref(), Some("https://en.wikipedia.org/wiki/Doom_(2016_video_game)"));
    assert!(r.meta.description.as_deref().unwrap().starts_with("Doom is a 2016"));
    assert_eq!(r.meta.developer.as_deref(), Some("FromSoftware, Inc."));
}

#[test]
fn wikipedia_backfill_asks_only_wikipedia_and_errors_retry() {
    let tmp = tempfile::tempdir().unwrap();
    let g = doom_game(Some(1993));
    let job = Job::wikipedia_backfill(&g, false).unwrap();
    assert!(job.wiki.only && !job.want.meta);
    let url = wikipedia::search_url("DOOM");
    let mut w = worker(FakeHttp::default().ok(&url, fixture!("wikipedia_search_doom.json")), tmp.path());
    let r = w.process(&job, &wiki_cfg(true, false)).unwrap();
    assert!(r.wiki_backfill && !r.backfill);
    assert!(r.meta.wikipedia.is_some());
    assert_eq!(w.http.calls.len(), 1);
    // Off: nothing queued; a network error / 5xx → try again another time; 4xx → no match.
    assert!(!eligible(&job, &MetadataConfig { wikipedia: false, ..Default::default() }));
    let mut w = worker(FakeHttp::default().net_err(&url), tmp.path());
    assert!(w.process(&job, &wiki_cfg(false, false)).is_err());
    let mut w = worker(FakeHttp::default().status(&url, 503), tmp.path());
    assert!(w.process(&job, &wiki_cfg(false, false)).is_err());
    let mut w = worker(FakeHttp::default().status(&url, 404), tmp.path());
    assert_eq!(w.process(&job, &wiki_cfg(false, false)).unwrap().meta.description, None);
}

#[test]
fn wikipedia_credit_follows_the_description_through_merges() {
    let fetched = Metadata {
        description: Some("Wiki text".into()),
        wikipedia: Some("https://en.wikipedia.org/wiki/X".into()),
        ..Default::default()
    };
    // A scanner description wins, and the credit isn't carried over to it.
    let mut scanner = Metadata { description: Some("Launcher text".into()), ..Default::default() };
    scanner.fill_from(&fetched);
    assert_eq!(scanner.wikipedia, None);
    let mut empty = Metadata::default();
    empty.fill_from(&fetched);
    assert_eq!(empty.wikipedia, fetched.wikipedia);
    // A user edit drops the credit.
    let mut g = Game::blank("X");
    g.meta = fetched.clone();
    g.edits.description = Some("Mine".into());
    assert_eq!(g.with_edits_applied().meta.wikipedia, None);
    // keep_only_missing: a game that has a description gets neither.
    let mut m = fetched.clone();
    m.keep_only_missing(&Metadata { description: Some("x".into()), ..Default::default() });
    assert_eq!((m.description, m.wikipedia), (None, None));
    // Saved with the description (old rows without it read fine).
    let json = serde_json::to_string(&fetched).unwrap();
    assert!(json.contains("\"wikipedia\""));
    let old: Metadata = serde_json::from_str(
        r#"{"description":"d","developer":null,"publisher":null,"genres":[],"release_year":null}"#,
    )
    .unwrap();
    assert_eq!(old.wikipedia, None);
}

// ---------------------------------------------------------------------------------------------
// Lookups the user asks for (Edit Game Details → Download metadata…)

fn lookup_req(pick: Option<Pick>) -> LookupReq {
    LookupReq { token: 9, pick, with_art: false, scan_description: false }
}

#[test]
fn a_lookup_the_user_asked_for_asks_again_whatever_the_game_has() {
    // A game looked up before has everything: a background job wants nothing more …
    let mut g = Game::blank("PlayStation Home");
    g.id = 5;
    g.meta = Metadata {
        description: Some("Old match".into()),
        developer: Some("Old dev".into()),
        genres: vec!["Old".into()],
        release_year: Some(2001),
        critic_score: Some(50),
        ..Default::default()
    };
    let bg = Job::from_game(&g);
    assert!(!bg.want.meta && !bg.want.extra);
    // … the user's own lookup asks for everything again, with the name typed now.
    g.user_name = Some("Tomb Raider".into());
    let j = Job::for_lookup(&g, lookup_req(None));
    assert!(j.want.meta && j.want.extra && !j.want.cover, "no art for a lookup without with_art");
    assert_eq!(j.name, "Tomb Raider");
    assert_eq!(j.lookup.as_ref().map(|l| l.token), Some(9));
    assert!(j.wiki.allowed && j.wiki.missing, "Wikipedia may replace the old match's text");
    assert_eq!(j.wiki.year, None, "a fetched year (maybe the wrong game's) doesn't steer the match");
    // The user's own year does; their own description keeps Wikipedia out; a cleared one doesn't.
    g.edits.release_year = Some(1996);
    g.edits.description = Some("".into());
    let j = Job::for_lookup(&g, lookup_req(None));
    assert_eq!(j.wiki.year, Some(1996));
    assert!(j.wiki.allowed);
    g.edits.description = Some("Mine".into());
    g.edits.release_year = Some(CLEARED_YEAR);
    let j = Job::for_lookup(&g, LookupReq { scan_description: true, ..lookup_req(None) });
    assert!(!j.wiki.allowed && !j.wiki.missing);
    assert_eq!(j.wiki.year, None);
}

#[test]
fn an_ambiguous_igdb_name_is_offered_to_pick_only_when_the_user_asked() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, secret) = ("lk-cid", "lk-secret-1");
    crate::igdb::forget_token(id, secret);
    let search = format!("POST {}\n{}", crate::igdb::games_url(), crate::igdb::search_body("Tomb Raider"));
    let by_id = format!("POST {}\n{}", crate::igdb::games_url(), crate::igdb::by_ids_body(&[912]));
    let mut w = worker(
        FakeHttp::default()
            .ok(&token_route(id, secret), fixture!("twitch_token.json"))
            .ok(&search, fixture!("igdb_games_tomb_raider.json"))
            .ok(&by_id, fixture!("igdb_games_tomb_raider_1996.json")),
        tmp.path(),
    );
    let cfg = igdb_cfg(id, secret);
    let g = Game { id: 4, ..Game::blank("Tomb Raider") };
    // Background: the first game of that name, as before.
    let r = w.process(&Job::from_game(&g), &cfg).unwrap();
    assert_eq!(r.meta.developer.as_deref(), Some("Crystal Dynamics"));
    assert!(r.candidates.is_empty() && r.lookup.is_none());
    // Asked by the user: both games of that name to pick from (not the other-named hit).
    let r = w.process(&Job::for_lookup(&g, lookup_req(None)), &cfg).unwrap();
    assert_eq!(r.lookup, Some(9));
    assert_eq!(r.meta.developer, None, "nothing guessed");
    let c: Vec<(&str, Option<i32>, &str)> =
        r.candidates.iter().map(|c| (c.name.as_str(), c.year, c.platforms.as_str())).collect();
    assert_eq!(
        c,
        [
            ("Tomb Raider", Some(2013), "PC (Microsoft Windows), PlayStation 3, Xbox 360"),
            ("Tomb Raider", Some(1996), "PlayStation, Sega Saturn, DOS"),
        ]
    );
    assert_eq!(r.candidates[1].pick, Pick::Igdb(912));
    // The user's own year settles it without asking.
    let mut typed = g.clone();
    typed.edits.release_year = Some(1996);
    let r = w.process(&Job::for_lookup(&typed, lookup_req(None)), &cfg).unwrap();
    assert!(r.candidates.is_empty());
    assert_eq!(r.meta.developer.as_deref(), Some("Core Design"));
    // Picked: that game, by its id.
    let r = w.process(&Job::for_lookup(&g, lookup_req(Some(Pick::Igdb(912)))), &cfg).unwrap();
    assert!(r.candidates.is_empty());
    assert_eq!(r.meta.release_year, Some(1996));
    assert_eq!(r.meta.publisher.as_deref(), Some("Eidos Interactive"));
}

#[test]
fn an_ambiguous_wikipedia_name_is_offered_to_pick() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = wiki_cfg(false, false);
    let mut w = worker(
        FakeHttp::default()
            .ok(&wikipedia::search_url("DOOM"), fixture!("wikipedia_search_doom.json"))
            .ok(&wikipedia::search_url("Doom (1993 video game)"), fixture!("wikipedia_search_doom.json")),
        tmp.path(),
    );
    // A fetched year of 1993 would have picked one; the user's lookup ignores fetched years.
    let g = doom_game(Some(1993));
    let job = Job::for_lookup(&g, lookup_req(None));
    assert!(eligible(&job, &cfg));
    let r = w.process(&job, &cfg).unwrap();
    assert_eq!(r.meta.description, None);
    let titles: Vec<&str> = r.candidates.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(titles, ["Doom (1993 video game)", "Doom (2016 video game)"]);
    assert_eq!(r.candidates[0].year, Some(1993));
    let r = w
        .process(&Job::for_lookup(&g, lookup_req(Some(Pick::Wikipedia("Doom (1993 video game)".into())))), &cfg)
        .unwrap();
    assert!(r.candidates.is_empty());
    assert!(r.meta.description.as_deref().unwrap().starts_with("Doom is a 1993 first-person shooter"));
}

#[test]
fn a_lookup_no_source_can_answer_says_so_right_away() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let f = MetadataFetcher::new(&ctx, tmp.path().to_path_buf(), &cfg(false, None));
    let g = Game { id: 77, ..Game::blank("Nothing") };
    assert!(f.lookup(&g, lookup_req(None)).is_err());
    assert_eq!(f.pending(), 0);
}

#[test]
fn a_game_igdb_settled_picks_its_own_wikipedia_article_without_asking() {
    // "DOOM" asked by the user with Wikipedia preferred: IGDB knows one game of that name
    // (1993); Wikipedia has 1993 and 2016 articles. The year IGDB found picks the article, and
    // the user isn't asked which game it is (IGDB already said).
    let tmp = tempfile::tempdir().unwrap();
    let (id, secret) = ("lk-cid-doom", "lk-secret-doom");
    crate::igdb::forget_token(id, secret);
    let search = format!("POST {}\n{}", crate::igdb::games_url(), crate::igdb::search_body("DOOM"));
    let igdb = br#"[{"id":7,"name":"Doom","summary":"IGDB text about Doom.","first_release_date":755481600}]"#;
    let mut w = worker(
        FakeHttp::default()
            .ok(&token_route(id, secret), fixture!("twitch_token.json"))
            .ok(&search, igdb)
            .ok(&wikipedia::search_url("DOOM"), fixture!("wikipedia_search_doom.json")),
        tmp.path(),
    );
    let cfg = MetadataConfig { wikipedia: true, prefer_wikipedia: true, ..igdb_cfg(id, secret) };
    let r = w.process(&Job::for_lookup(&doom_game(None), lookup_req(None)), &cfg).unwrap();
    assert!(r.candidates.is_empty(), "{:?}", r.candidates);
    assert_eq!(r.meta.release_year, Some(1993));
    assert!(r.meta.wikipedia.as_deref().is_some_and(|u| u.ends_with("Doom_(1993_video_game)")), "{:?}", r.meta);
}
