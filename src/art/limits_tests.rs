//! The API limits (art/limits.rs) with a fake transport: GitHub's hourly limit, TheGamesDB's
//! allowance, 429 + Retry-After, IGDB's window, and the per-list source choice of bulk runs and
//! searches.

use super::limits::*;
use super::net::*;
use super::sources::*;
use super::worker::*;
use super::*;
use crate::config::MetadataConfig;
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};

type Route = (u16, Vec<u8>, Vec<(String, String)>);

/// Answers by URL substring (the first route whose key the URL contains); records every URL
/// and the most requests that were in flight at once.
#[derive(Default)]
struct Fake {
    routes: Vec<(String, Route)>,
    calls: Arc<Mutex<Vec<String>>>,
    running: Arc<AtomicUsize>,
    most: Arc<AtomicUsize>,
    delay: Duration,
}

impl Fake {
    fn route(mut self, key: &str, status: u16, body: &str, headers: &[(&str, &str)]) -> Self {
        let h = headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        self.routes.push((key.into(), (status, body.as_bytes().to_vec(), h)));
        self
    }
    fn answer(&self, url: &str) -> HttpResponse {
        self.calls.lock().push(url.to_string());
        let n = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.most.fetch_max(n, Ordering::SeqCst);
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        self.running.fetch_sub(1, Ordering::SeqCst);
        match self.routes.iter().find(|(k, _)| url.contains(k.as_str())) {
            Some((_, (status, body, headers))) => HttpResponse {
                status: *status,
                body: if *status == 200 { body.clone() } else { vec![] },
                headers: headers.clone(),
            },
            None => HttpResponse { status: 404, ..Default::default() },
        }
    }
}

impl Net for Fake {
    fn get(&self, url: &str, _bearer: Option<&str>, _limit: u64) -> Result<HttpResponse, String> {
        Ok(self.answer(url))
    }
    fn head(&self, url: &str) -> Result<u16, String> {
        Ok(self.answer(url).status)
    }
    fn post(&self, url: &str, _h: &[(&str, String)], _b: &str, _l: u64) -> Result<HttpResponse, String> {
        Ok(self.answer(url))
    }
}

/// Only `on` switched on, in both lists.
fn cfg_only(on: &[SourceKind]) -> MetadataConfig {
    let off: Vec<String> = SourceKind::ALL.iter().filter(|k| !on.contains(k)).map(|k| k.name().to_string()).collect();
    MetadataConfig {
        sources_off: off.clone(),
        emu_sources_off: Some(off),
        thegamesdb_key: Some("tgdb-key-1".into()),
        steamgriddb_key: Some("sgdb-key-1".into()),
        ..Default::default()
    }
}

fn engine(net: Fake, dir: &std::path::Path, cfg: MetadataConfig) -> Engine {
    Engine::new(Box::new(net), Throttle::none(), dir.join("art"), dir.join("thumbs"), &cfg)
}

fn game(id: GameId, name: &str) -> Game {
    let mut db = crate::db::Db::open_memory().unwrap();
    let real = db.add_manual(name, LaunchSpec::Url("x://".into()), Art::default()).unwrap();
    let mut g = db.get(real).unwrap().unwrap();
    g.id = id;
    g
}

fn ps2_game(id: GameId, name: &str) -> Game {
    let mut g = game(id, name);
    g.platform = Some("Sony PlayStation 2".into());
    g
}

fn bulk(e: &Engine, games: &[Game]) -> BulkReport {
    let cancel = AtomicBool::new(false);
    let opts = BulkOptions { slots: vec![ArtSlot::Cover], ..Default::default() };
    e.bulk(games, &opts, &cancel, &mut |_, _, _| {}, &mut |_, _, _| {})
}

fn search_errors(e: &Engine, g: &Game) -> Vec<String> {
    let job = SearchJob::new(g, ArtSlot::Cover, None);
    let mut errors = vec![];
    e.search(&job, |_, _, er| {
        errors.extend(er);
        true
    });
    errors
}

#[test]
fn number_and_time_words() {
    assert_eq!(grouped(9412), "9 412");
    assert_eq!(grouped(10000), "10 000");
    assert_eq!(grouped(60), "60");
    assert_eq!(grouped(1234567), "1 234 567");
    let now = 1_700_000_000;
    assert!(when(now + 60, now).starts_with("at "));
    assert!(when(now + 86400, now).starts_with("tomorrow at "));
    assert!(when(now + 20 * 86400, now).starts_with("on "));
}

#[test]
fn github_hourly_limit_for_libretro() {
    let now = unix_now();
    let l = Limits::with_saved(Saved::default(), None);
    let reset = (now + 1800).to_string();
    l.observe(
        Host::GitHubApi,
        200,
        &[
            ("x-ratelimit-remaining".into(), "52".into()),
            ("x-ratelimit-limit".into(), "60".into()),
            ("x-ratelimit-reset".into(), reset.clone()),
        ],
        b"{}",
        now,
    );
    assert_eq!(l.summary(now), vec![(SourceKind::Libretro, "52 of 60 lookups left this hour".to_string())]);
    assert_eq!(l.check(Host::GitHubApi, true, now), Verdict::Go);
    // Nearly used up: a bulk run waits for the reset, a search goes on until 0.
    l.observe(
        Host::GitHubApi,
        200,
        &[("x-ratelimit-remaining".into(), "2".into()), ("x-ratelimit-reset".into(), reset.clone())],
        b"{}",
        now,
    );
    assert!(
        matches!(l.check(Host::GitHubApi, true, now), Verdict::Wait(s, ref w) if s >= 1800 && w.contains("continues ") && w.contains(" at "))
    );
    assert_eq!(l.check(Host::GitHubApi, false, now), Verdict::Go);
    // A 403 without headers: the hour is used up.
    l.observe(Host::GitHubApi, 403, &[], b"", now);
    assert!(matches!(l.check(Host::GitHubApi, false, now), Verdict::Stop(w) if w.contains("used up")));
    // After the reset it goes again.
    assert_eq!(l.check(Host::GitHubApi, true, now + 1900), Verdict::Go);
}

#[test]
fn github_limit_reaches_the_search_and_waits_in_bulk_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let now = unix_now();
    let tree = r#"{"tree":[{"path":"Game One (USA).png","type":"blob","mode":"100644"}],"truncated":false}"#;
    let soon = (now + 1).to_string();
    let net = Fake::default().route(
        "api.github.com",
        200,
        tree,
        &[("x-ratelimit-remaining", "0"), ("x-ratelimit-limit", "60"), ("x-ratelimit-reset", &soon)],
    );
    let calls = net.calls.clone();
    let e = engine(net, tmp.path(), cfg_only(&[SourceKind::Libretro]));
    // Remaining 0 until a reset far away: the search says when it continues, without asking.
    e.limits.observe(
        Host::GitHubApi,
        200,
        &[("x-ratelimit-remaining".into(), "0".into()), ("x-ratelimit-reset".into(), (now + 1800).to_string())],
        b"",
        now,
    );
    let errors = search_errors(&e, &ps2_game(1, "Game One"));
    assert!(errors.iter().any(|w| w.contains("GitHub's hourly limit") && w.contains("continues")), "{errors:?}");
    assert!(calls.lock().is_empty());
    // A reset in two seconds: the bulk run waits for it (saying so), then goes on. Two, not one:
    // `unix_now` counts whole seconds, so a one-second reset may be over before the run starts.
    let now = unix_now();
    e.limits.observe(
        Host::GitHubApi,
        200,
        &[("x-ratelimit-remaining".into(), "0".into()), ("x-ratelimit-reset".into(), (now + 2).to_string())],
        b"",
        now,
    );
    let rep = bulk(&e, &[ps2_game(2, "Game One")]);
    let waited = e.limits.waited();
    assert!(waited.iter().any(|w| w.contains("GitHub's hourly limit") && w.contains("continues")), "{waited:?}");
    assert!(calls.lock().iter().any(|u| u.contains("api.github.com")), "asked after the reset");
    assert!(rep.error.is_none(), "{rep:?}");
    assert!(e.limits.waiting().is_none());
}

#[test]
fn thegamesdb_monthly_allowance() {
    let now = 1_700_000_000;
    assert_eq!(
        parse_tgdb_allowance(
            br#"{"code":200,"remaining_monthly_allowance":2950,"extra_allowance":0,"allowance_refresh_timer":86400}"#
        ),
        Some((2950, 0, Some(86400)))
    );
    let l = Limits::with_saved(Saved::default(), None);
    l.observe(
        Host::Tgdb,
        200,
        &[],
        br#"{"remaining_monthly_allowance":2950,"extra_allowance":12,"allowance_refresh_timer":86400}"#,
        now,
    );
    assert_eq!(l.summary(now), vec![(SourceKind::TheGamesDb, "2 950 left this month (+ 12 extra)".to_string())]);
    l.observe(Host::Tgdb, 200, &[], br#"{"remaining_monthly_allowance":8,"extra_allowance":0}"#, now);
    // Bulk runs keep 20 for searches; the refresh time from before is kept.
    assert!(
        matches!(l.check(Host::Tgdb, true, now), Verdict::Stop(w) if w.contains("TheGamesDB") && w.contains("continues"))
    );
    assert_eq!(l.check(Host::Tgdb, false, now), Verdict::Go);
    l.observe(Host::Tgdb, 200, &[], br#"{"remaining_monthly_allowance":0,"extra_allowance":0}"#, now);
    assert!(matches!(l.check(Host::Tgdb, false, now), Verdict::Stop(_)));
    // The month starts over.
    assert_eq!(l.check(Host::Tgdb, false, now + 90000), Verdict::Go);
}

#[test]
fn thegamesdb_bulk_run_stops_before_the_allowance_is_gone() {
    let tmp = tempfile::tempdir().unwrap();
    let body = r#"{"code":200,"data":{"count":0,"games":[]},"remaining_monthly_allowance":21,"extra_allowance":0,"allowance_refresh_timer":3600}"#;
    let net = Fake::default().route("Games/ByGameName", 200, body, &[]);
    let calls = net.calls.clone();
    let e = engine(net, tmp.path(), cfg_only(&[SourceKind::TheGamesDb]));
    let games: Vec<Game> = (1..=5).map(|i| ps2_game(i, &format!("Game {i}"))).collect();
    // 21 left (bulk runs keep 20): every game is asked while the replies say 21.
    let rep = bulk(&e, &games);
    assert_eq!(calls.lock().len(), 5, "21 > 20: each game is asked while the reply says 21");
    assert!(rep.error.is_none());
    e.limits.observe(Host::Tgdb, 200, &[], br#"{"remaining_monthly_allowance":20}"#, unix_now());
    let others: Vec<Game> = (6..=9).map(|i| ps2_game(i, &format!("Game {i}"))).collect();
    let rep = bulk(&e, &others);
    assert_eq!(calls.lock().len(), 5, "no more requests");
    assert!(rep.error.unwrap().contains("TheGamesDB's monthly allowance"));
}

#[test]
fn counts_saved_by_an_older_version_still_load() {
    // 0.16.2 and before also kept ScreenScraper's quota and could have it paused: both are
    // dropped, the rest is kept.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("api-limits.json");
    let now = unix_now();
    let old = format!(
        r#"{{"screenscraper":{{"day":20000,"requests":9412,"max_per_day":10000,"ko":3,"max_ko_per_day":2000,"max_threads":1,"max_per_min":20}},
"thegamesdb":{{"remaining":2950,"extra":0,"refresh_at":{},"seen_at":{now}}},
"paused":{{"ScreenScraper":{},"Sgdb":{}}}}}"#,
        now + 86400,
        now + 600,
        now + 600
    );
    std::fs::write(&path, old).unwrap();
    let l = Limits::new(Some(path));
    assert_eq!(l.saved().thegamesdb.unwrap().remaining, 2950);
    let kinds: Vec<SourceKind> = l.summary(now).into_iter().map(|(k, _)| k).collect();
    assert_eq!(kinds, [SourceKind::TheGamesDb, SourceKind::SteamGridDb], "the old pause isn't shown as another source");
}

#[test]
fn retry_after_pauses_a_host() {
    let now = 1_700_000_000;
    let l = Limits::with_saved(Saved::default(), None);
    assert_eq!(retry_after(&[("Retry-After".into(), "120".into())]), Some(120));
    assert_eq!(retry_after(&[("retry-after".into(), "Wed, 21 Oct 2015 07:28:00 GMT".into())]), None);
    l.observe(Host::Sgdb, 429, &[("retry-after".into(), "120".into())], b"", now);
    assert!(matches!(l.check(Host::Sgdb, true, now), Verdict::Wait(120, _)));
    assert!(
        matches!(l.check(Host::Sgdb, false, now), Verdict::Stop(w) if w.contains("SteamGridDB asked Lodestar to wait"))
    );
    assert_eq!(l.check(Host::Sgdb, false, now + 121), Verdict::Go);
    assert!(l.summary(now).iter().any(|(k, t)| *k == SourceKind::SteamGridDb && t.contains("asked to wait")));
    // SteamGridDB without Retry-After: a minute.
    let l = Limits::with_saved(Saved::default(), None);
    l.observe(Host::Sgdb, 429, &[], b"", now);
    assert!(matches!(l.check(Host::Sgdb, true, now), Verdict::Wait(60, _)));
    // X-RateLimit-Remaining 0 + Reset.
    let l = Limits::with_saved(Saved::default(), None);
    l.observe(
        Host::Sgdb,
        200,
        &[("x-ratelimit-remaining".into(), "0".into()), ("x-ratelimit-reset".into(), "30".into())],
        b"{}",
        now,
    );
    assert!(matches!(l.check(Host::Sgdb, true, now), Verdict::Wait(30, _)));
}

#[test]
fn steamgriddb_429_reaches_the_search() {
    let tmp = tempfile::tempdir().unwrap();
    let net = Fake::default().route("steamgriddb.com/api/v2/search", 429, "", &[("retry-after", "600")]);
    let calls = net.calls.clone();
    let e = engine(net, tmp.path(), cfg_only(&[SourceKind::SteamGridDb]));
    let g = game(1, "Some Game");
    let first = search_errors(&e, &g);
    assert_eq!(first.len(), 1, "{first:?}");
    let n = calls.lock().len();
    let errors = search_errors(&e, &game(2, "Other Game"));
    assert!(errors.iter().any(|w| w.contains("asked Lodestar to wait")), "{errors:?}");
    assert_eq!(calls.lock().len(), n, "left alone while paused");
}

#[test]
fn igdb_window_is_four_a_second() {
    let mut w = crate::igdb::RateWindow::new(4, Duration::from_millis(1100));
    let t0 = Instant::now();
    for _ in 0..4 {
        assert_eq!(w.reserve(t0), Duration::ZERO);
    }
    assert_eq!(w.reserve(t0), Duration::from_millis(1100));
    assert_eq!(w.reserve(t0 + Duration::from_millis(100)), Duration::from_millis(1000));
    assert_eq!(w.reserve(t0 + Duration::from_secs(5)), Duration::ZERO);
}

#[test]
fn igdb_requests_of_every_thread_share_one_window() {
    // The art throttle and the metadata fetcher both go through `igdb::wait_turn`: 8 requests
    // from two threads take at least one window.
    let t0 = Instant::now();
    let hs: Vec<_> = (0..2)
        .map(|i| {
            std::thread::spawn(move || {
                for _ in 0..4 {
                    if i == 0 {
                        assert!(Throttle::polite().wait("https://api.igdb.com/v4/games", false, None));
                    } else {
                        assert!(crate::igdb::wait_turn(None));
                    }
                }
            })
        })
        .collect();
    hs.into_iter().for_each(|h| h.join().unwrap());
    assert!(t0.elapsed() >= Duration::from_millis(1000), "{:?}", t0.elapsed());
}

#[test]
fn emulated_games_use_their_own_list() {
    let tmp = tempfile::tempdir().unwrap();
    // Steam on for PC games; for emulated games only libretro (the default list has Steam off).
    let cfg = MetadataConfig::default();
    let e = engine(Fake::default(), tmp.path(), cfg);
    let ps2 = ps2_game(1, "Okami");
    assert_eq!(art_list(&ps2), ArtList::Emulated);
    let mut emu = game(2, "Rom only");
    emu.emu = Some(crate::model::EmuRef { platform: "nintendo_gba".into(), ..Default::default() });
    assert_eq!(art_list(&emu), ArtList::Emulated);
    assert_eq!(art_list(&game(3, "Portal")), ArtList::Pc);
    let names = |l| -> Vec<SourceKind> { e.available_sources(l) };
    assert_eq!(names(ArtList::Pc)[0], SourceKind::Steam);
    let emu_on = names(ArtList::Emulated);
    assert_eq!(emu_on[0], SourceKind::Libretro);
    assert!(!emu_on.contains(&SourceKind::Steam) && !emu_on.contains(&SourceKind::Gog));
    assert_eq!(SearchJob::new(&ps2, ArtSlot::Cover, None).list, ArtList::Emulated);
}

#[test]
fn bulk_run_asks_each_game_by_its_own_list() {
    let tmp = tempfile::tempdir().unwrap();
    let net = Fake::default();
    let calls = net.calls.clone();
    // PC: Steam only. Emulated: libretro only.
    let mut cfg = cfg_only(&[SourceKind::Steam]);
    cfg.emu_sources_off =
        Some(SourceKind::ALL.iter().filter(|k| **k != SourceKind::Libretro).map(|k| k.name().to_string()).collect());
    let e = engine(net, tmp.path(), cfg);
    bulk(&e, &[game(1, "Portal"), ps2_game(2, "Okami")]);
    let calls = calls.lock().clone();
    assert!(calls.iter().any(|u| u.contains("store.steampowered.com") && u.contains("Portal")), "{calls:?}");
    assert!(!calls.iter().any(|u| u.contains("store.steampowered.com") && u.contains("Okami")), "{calls:?}");
    assert!(calls.iter().any(|u| u.contains("api.github.com")), "{calls:?}");
}

#[test]
fn games_whose_list_ran_out_are_not_called_not_found() {
    let tmp = tempfile::tempdir().unwrap();
    // PC: Steam. Emulated: TheGamesDB only, whose first reply says the month is almost used up.
    let body = r#"{"code":200,"data":{"count":0,"games":[]},"remaining_monthly_allowance":5,"extra_allowance":0}"#;
    let net = Fake::default().route("Games/ByGameName", 200, body, &[]);
    let calls = net.calls.clone();
    let mut cfg = cfg_only(&[SourceKind::Steam]);
    cfg.emu_sources_off =
        Some(SourceKind::ALL.iter().filter(|k| **k != SourceKind::TheGamesDb).map(|k| k.name().to_string()).collect());
    let e = engine(net, tmp.path(), cfg);
    let games = [ps2_game(1, "Okami"), ps2_game(2, "Jak"), ps2_game(3, "Ico"), game(4, "Portal")];
    let rep = bulk(&e, &games);
    // Okami was asked (nothing found), Jak hit the limit, Ico wasn't asked: neither of those
    // two is "not found"; Portal (PC list) still was asked.
    assert!(!rep.not_found.iter().any(|n| n == "Jak" || n == "Ico"), "{rep:?}");
    assert_eq!(rep.failed, 2, "{rep:?}");
    assert!(rep.error.as_deref().unwrap().contains("TheGamesDB's monthly allowance"), "{rep:?}");
    assert!(calls.lock().iter().any(|u| u.contains("store.steampowered.com") && u.contains("Portal")));
}
