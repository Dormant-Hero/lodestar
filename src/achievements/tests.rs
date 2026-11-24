//! The Hub's bookkeeping: one request per game on its way, stale answers asked again once,
//! forced refreshes kept, answers from before `clear` dropped, and never "loading" for ever.

use super::*;
use crate::art::net::{HttpResponse, Net};
use crossbeam_channel::{Receiver, Sender, bounded};
use std::time::Duration;

struct NoNet;

impl Net for NoNet {
    fn get(&self, _url: &str, _bearer: Option<&str>, _limit: u64) -> Result<HttpResponse, String> {
        Err("no network in tests".into())
    }
    fn head(&self, _url: &str) -> Result<u16, String> {
        Err("no network in tests".into())
    }
}

/// A fake lookup that reports each request it starts and waits until the test lets it answer.
struct Gate {
    started: Receiver<Request>,
    release: Sender<State>,
}

fn gated_hub() -> (Hub, Gate) {
    let (started_tx, started) = bounded::<Request>(64);
    let (release, release_rx) = bounded::<State>(64);
    let lookup = move |req: &Request, _env: &Env| {
        let _ = started_tx.send(req.clone());
        release_rx.recv_timeout(Duration::from_secs(10)).unwrap_or(State::None)
    };
    let hub =
        Hub::with_lookup(&egui::Context::default(), PathBuf::from("/nonexistent"), Arc::new(NoNet), Box::new(lookup));
    (hub, Gate { started, release })
}

fn game(id: GameId) -> Game {
    let mut g = Game::blank("Fixture");
    g.id = id;
    g
}

fn ready(n: u32) -> State {
    State::Ready(Arc::new(Progress {
        service: Service::Steam,
        unlocked: n,
        total: 10,
        list: vec![],
        page_url: None,
        note: None,
        synced: true,
        probable: false,
    }))
}

/// Poll until `id` has an answer that isn't "loading" (the worker is a real thread).
fn wait_answer(hub: &mut Hub, id: GameId) -> State {
    for _ in 0..1000 {
        hub.poll();
        if let Some(s) = hub.state(id)
            && *s != State::Loading
            && !hub.pending.contains_key(&id)
        {
            return s.clone();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("no answer for game {id}");
}

fn make_stale(hub: &mut Hub, id: GameId) {
    let (_, at) = hub.states.get_mut(&id).unwrap();
    *at = Instant::now().checked_sub(STALE_AFTER + Duration::from_secs(1)).unwrap();
}

#[test]
fn a_stale_answer_kept_while_its_refresh_loads_is_asked_for_only_once() {
    let (mut hub, gate) = gated_hub();
    let cfg = AchievementsConfig::default();
    let g = game(7);
    hub.request(&g, &cfg);
    gate.started.recv_timeout(Duration::from_secs(5)).unwrap();
    gate.release.send(ready(1)).unwrap();
    assert_eq!(wait_answer(&mut hub, 7), ready(1));
    // The page stays open past the stale time: one new request, the old answer stays on screen,
    // and asking every frame meanwhile sends nothing more.
    make_stale(&mut hub, 7);
    hub.request(&g, &cfg);
    gate.started.recv_timeout(Duration::from_secs(5)).unwrap();
    for _ in 0..50 {
        hub.request(&g, &cfg);
    }
    assert_eq!(hub.tx.len(), 0, "requests piled up while one was on its way");
    assert_eq!(hub.state(7), Some(&ready(1)), "the earlier answer shows while the new one loads");
    gate.release.send(ready(2)).unwrap();
    assert_eq!(wait_answer(&mut hub, 7), ready(2));
    // The new answer is fresh: no more requests.
    hub.request(&g, &cfg);
    assert_eq!(hub.tx.len(), 0);
    assert!(gate.started.try_recv().is_err());
}

#[test]
fn a_forced_refresh_is_not_lost_behind_a_newer_plain_request() {
    let req = |id: GameId, force: bool| Request { game: game(id), config: AchievementsConfig::default(), force };
    let mut queue = vec![(0, req(1, true)), (0, req(2, false)), (0, req(1, false))];
    let (_, first) = take_newest(&mut queue).unwrap();
    assert_eq!((first.game.id, first.force), (1, true), "the newest request, still forced");
    let (_, second) = take_newest(&mut queue).unwrap();
    assert_eq!((second.game.id, second.force), (2, false));
    assert!(take_newest(&mut queue).is_none());
}

#[test]
fn the_game_on_screen_is_looked_up_before_pages_scrolled_past() {
    let (mut hub, gate) = gated_hub();
    let cfg = AchievementsConfig::default();
    hub.request(&game(1), &cfg);
    gate.started.recv_timeout(Duration::from_secs(5)).unwrap();
    // While game 1 loads, the user scrolls through 2, 3, 4 and stops on 5.
    for id in 2..=5 {
        hub.request(&game(id), &cfg);
    }
    gate.release.send(State::None).unwrap();
    let next = gate.started.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(next.game.id, 5, "the newest page comes next");
    for _ in 0..4 {
        gate.release.send(State::None).unwrap();
    }
}

#[test]
fn answers_to_requests_from_before_clear_are_dropped() {
    let (mut hub, gate) = gated_hub();
    let cfg = AchievementsConfig::default();
    hub.request(&game(3), &cfg);
    gate.started.recv_timeout(Duration::from_secs(5)).unwrap();
    // The user changes the key while the old lookup runs.
    hub.clear();
    gate.release.send(State::Hint("made with the old key".into())).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    hub.poll();
    assert_eq!(hub.state(3), None, "an answer for the old settings never shows");
    // Asking again goes through at once.
    hub.request(&game(3), &cfg);
    let again = gate.started.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(again.game.id, 3);
    gate.release.send(ready(4)).unwrap();
    assert_eq!(wait_answer(&mut hub, 3), ready(4));
}

#[test]
fn a_lookup_that_panics_shows_no_section_and_the_worker_keeps_answering() {
    let lookup = |req: &Request, _env: &Env| {
        if req.game.id == 1 {
            panic!("broken file");
        }
        State::Hint("fine".into())
    };
    let mut hub =
        Hub::with_lookup(&egui::Context::default(), PathBuf::from("/nonexistent"), Arc::new(NoNet), Box::new(lookup));
    let cfg = AchievementsConfig::default();
    hub.request(&game(1), &cfg);
    assert_eq!(wait_answer(&mut hub, 1), State::None);
    hub.request(&game(2), &cfg);
    assert_eq!(wait_answer(&mut hub, 2), State::Hint("fine".into()));
}

#[test]
fn without_a_worker_a_page_never_stays_loading() {
    let (mut hub, _gate) = gated_hub();
    let (tx, rx) = crossbeam_channel::unbounded();
    drop(rx);
    hub.tx = tx;
    hub.request(&game(9), &AchievementsConfig::default());
    assert_eq!(hub.state(9), Some(&State::None));
}

#[test]
fn an_account_check_that_ends_without_an_answer_says_so() {
    let (mut hub, _gate) = gated_hub();
    let (tx, rx) = crossbeam_channel::unbounded::<RaCheck>();
    hub.ra_check = Some((rx, None));
    assert_eq!(hub.ra_account_check(), Some(None), "still checking");
    drop(tx);
    assert!(matches!(hub.ra_account_check(), Some(Some(Err(_)))), "never \"checking…\" for ever");
}

#[test]
fn achievement_settings_never_show_their_keys_in_debug_output() {
    let cfg = AchievementsConfig {
        steam_web_api_key: Some("STEAMKEY0123456789".into()),
        ra_api_key: Some("RAKEY0123456789abc".into()),
        ra_username: "player".into(),
        ..Default::default()
    };
    let req = Request { game: game(1), config: cfg, force: false };
    let shown = format!("{req:?} {req:#?}");
    assert!(!shown.contains("STEAMKEY") && !shown.contains("RAKEY"), "{shown}");
    assert!(shown.contains("player") && shown.contains("<set>"));
}

#[test]
fn keys_are_scrubbed_also_in_the_form_they_take_in_a_url() {
    let key = "ab+c/d=e f".to_string();
    let cfg = AchievementsConfig { ra_api_key: Some(key.clone()), ..Default::default() };
    let in_url = format!("https://example.invalid/API_X.php?y={}", crate::art::net::encode(&key));
    let text = format!("connection failed: {in_url} (key {key})");
    let clean = crate::config::redact(&text, &cfg.secrets());
    assert!(!clean.contains(&crate::art::net::encode(&key)) && !clean.contains(&key), "{clean}");
    let c = ra::api::Client::new(&NoNet, &key, vec![]);
    let clean = c.redact(&text);
    assert!(!clean.contains(&crate::art::net::encode(&key)) && !clean.contains(&key), "{clean}");
}

/// The real transport sends a service's own User-Agent (RetroAchievements asks for
/// `Lodestar/<version> (Linux)`), not the agent-wide default.
#[test]
fn the_real_transport_sends_retroachievements_user_agent_on_the_wire() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut buf = vec![0u8; 8192];
        let mut got = Vec::new();
        while !got.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = s.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
        }
        s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").unwrap();
        String::from_utf8_lossy(&got).into_owned()
    });
    let net = crate::art::net::UreqNet::new();
    let headers = [("User-Agent", ra::api::USER_AGENT.to_string()), ("Accept", "application/json".to_string())];
    let r = net.get_with(&format!("http://127.0.0.1:{port}/API/API_GetUserProfile.php"), &headers, 1024).unwrap();
    assert_eq!(r.status, 200);
    let request = server.join().unwrap().to_ascii_lowercase();
    let want = format!("user-agent: {}", ra::api::USER_AGENT.to_ascii_lowercase());
    assert!(request.contains(&want), "{request}");
    assert_eq!(request.matches("user-agent:").count(), 1, "{request}");
    assert!(ra::api::USER_AGENT.starts_with("Lodestar/") && ra::api::USER_AGENT.ends_with(" (Linux)"));
}

// ------------------------------------------------------------------ library counts

fn progress_of(service: Service, unlocked: u32, total: u32, synced: bool, probable: bool) -> State {
    State::Ready(Arc::new(Progress {
        service,
        unlocked,
        total,
        list: vec![],
        page_url: None,
        note: None,
        synced,
        probable,
    }))
}

#[test]
fn game_page_answers_feed_the_library_counts_but_not_title_matches_or_unsynced_lists() {
    let (mut hub, gate) = gated_hub();
    let cfg = AchievementsConfig::default();
    for (id, st) in [
        (1, progress_of(Service::Steam, 31, 54, true, false)),
        (2, progress_of(Service::RetroAchievements, 3, 10, true, true)),
        (3, progress_of(Service::Steam, 0, 40, false, false)),
    ] {
        hub.request(&game(id), &cfg);
        gate.started.recv_timeout(Duration::from_secs(5)).unwrap();
        gate.release.send(st).unwrap();
        wait_answer(&mut hub, id);
    }
    hub.lib.fake = true; // no library pass in this test
    hub.sync(&[], 0, &cfg);
    assert_eq!(hub.count(1), Some((31, 54)));
    assert_eq!(hub.count(2), None, "matched by title: the game page only");
    assert_eq!(hub.count(3), None, "the player's progress isn't known");
    // The game turns out to have none (Steam switched off for it …): its count goes.
    hub.refresh(&game(1), &cfg);
    gate.started.recv_timeout(Duration::from_secs(5)).unwrap();
    gate.release.send(State::None).unwrap();
    wait_answer(&mut hub, 1);
    hub.sync(&[], 0, &cfg);
    assert_eq!(hub.count(1), None);
}

#[test]
fn switches_off_hide_the_library_counts() {
    let all: HashMap<GameId, summary::Summary> = [
        (1, summary::Summary { unlocked: 31, total: 54, service: Service::Steam }),
        (2, summary::Summary { unlocked: 3, total: 10, service: Service::RetroAchievements }),
    ]
    .into();
    let on = AchievementsConfig::default();
    assert_eq!(summary::visible(&all, &on).len(), 2);
    assert!(summary::visible(&all, &AchievementsConfig { show: false, ..on.clone() }).is_empty());
    assert!(summary::visible(&all, &AchievementsConfig { library_counts: false, ..on.clone() }).is_empty());
    let no_steam = summary::visible(&all, &AchievementsConfig { steam: false, ..on.clone() });
    assert_eq!(no_steam, [(2, (3, 10))].into());
    let no_ra = summary::visible(&all, &AchievementsConfig { retroachievements: false, ..on.clone() });
    assert_eq!(no_ra, [(1, (31, 54))].into());
    // The Hub follows a switch at once (no new lookup needed).
    let (mut hub, _gate) = gated_hub();
    hub.set_fake_counts(all);
    hub.sync(&[], 0, &on);
    assert_eq!(hub.count(1), Some((31, 54)));
    let g = hub.counts_generation();
    hub.sync(&[], 0, &AchievementsConfig { library_counts: false, ..on.clone() });
    assert_eq!((hub.count(1), hub.count(2)), (None, None));
    assert!(hub.counts().is_empty());
    assert_ne!(hub.counts_generation(), g, "lists sorted by achievements notice");
    // Old configs without the switch: on.
    let old: AchievementsConfig = toml::from_str("show = true\nsteam = true").unwrap();
    assert!(old.library_counts);
}

#[test]
fn library_counts_are_saved_and_show_at_once_at_the_next_start() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("achievements");
    let cfg = AchievementsConfig {
        ra_username: "Tester".into(),
        ra_api_key: Some("k".into()),
        ..AchievementsConfig::default()
    };
    let saved = summary::Saved {
        ra_user: summary::ra_user_key(&cfg),
        games: [
            (1, summary::Summary { unlocked: 31, total: 54, service: Service::Steam }),
            (2, summary::Summary { unlocked: 3, total: 10, service: Service::RetroAchievements }),
        ]
        .into(),
    };
    summary::save(&cache, &saved);
    assert_eq!(summary::load(&cache), saved);
    // A new run: the library pass's thread reads the file first.
    let mut hub = Hub::with_lookup(
        &egui::Context::default(),
        cache.clone(),
        Arc::new(NoNet),
        Box::new(|_: &Request, _: &Env| State::None),
    );
    for _ in 0..400 {
        hub.sync(&[], 0, &cfg);
        if hub.count(2).is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!((hub.count(1), hub.count(2)), (Some((31, 54)), Some((3, 10))));
    // Another RetroAchievements account: its counts aren't this one's (Steam's stay).
    let other = AchievementsConfig { ra_username: "Someone".into(), ..cfg.clone() };
    hub.sync(&[], 0, &other);
    assert_eq!((hub.count(1), hub.count(2)), (Some((31, 54)), None));
}

#[test]
fn a_steam_game_that_exits_is_asked_again_once_steam_has_written_its_files() {
    let (mut hub, gate) = gated_hub();
    hub.lib.fake = true;
    let cfg = AchievementsConfig::default();
    let mut g = game(9);
    g.source = crate::model::Source::Steam;
    g.source_id = "620".into();
    hub.game_exited(&g, &cfg);
    // At once …
    assert!(gate.started.recv_timeout(Duration::from_secs(5)).unwrap().force);
    gate.release.send(State::None).unwrap();
    wait_answer(&mut hub, 9);
    // … and again later (the files Steam writes a little after the game closed).
    assert_eq!(hub.lib.later.len(), 1);
    let (at, id) = hub.lib.later[0];
    assert_eq!(id, 9);
    assert!(at > Instant::now() + summary::STEAM_AFTER_EXIT - Duration::from_secs(5));
    hub.lib.later[0].0 = Instant::now();
    hub.sync(std::slice::from_ref(&g), 0, &cfg);
    assert!(gate.started.recv_timeout(Duration::from_secs(5)).unwrap().force);
    gate.release.send(State::None).unwrap();
    assert!(hub.lib.later.is_empty());
}
