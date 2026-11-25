//! The background Steam name / owned-games lookup against a mocked transport.

use super::*;
use crate::art::net::HttpResponse;
use parking_lot::Mutex;

const KEY: &str = "0123456789ABCDEF0123456789ABCDEF";
const ACCOUNT: u32 = 12345;

/// Answers by URL prefix; records every URL asked.
#[derive(Default)]
struct Mock {
    calls: Mutex<Vec<String>>,
    routes: Vec<(String, u16, String)>,
    fail: bool,
}

impl Mock {
    fn route(mut self, prefix: &str, status: u16, body: &str) -> Mock {
        self.routes.push((prefix.to_string(), status, body.to_string()));
        self
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().clone()
    }
}

impl Net for Mock {
    fn get(&self, url: &str, _b: Option<&str>, _l: u64) -> Result<HttpResponse, String> {
        self.calls.lock().push(url.to_string());
        if self.fail {
            return Err(format!("connection reset while fetching {url}"));
        }
        for (p, status, body) in &self.routes {
            if url.starts_with(p.as_str()) {
                return Ok(HttpResponse { status: *status, body: body.clone().into_bytes(), ..Default::default() });
            }
        }
        Ok(HttpResponse { status: 404, ..Default::default() })
    }
    fn head(&self, _url: &str) -> Result<u16, String> {
        Ok(404)
    }
}

const ITEMS: &str = r#"{"response":{"store_items":[
  {"item_type":0,"id":440,"success":1,"visible":true,"name":"Team Fortress 2","appid":440,"type":0},
  {"item_type":0,"id":378648,"success":1,"name":"The Witcher 3: Wild Hunt - Blood and Wine","appid":378648,"type":4},
  {"item_type":0,"id":1493710,"success":15,"visible":false,"name":"","appid":0},
  {"item_type":0,"id":50300,"success":1,"visible":true,"unlisted":true,"name":"Spec Ops: The Line","type":0}
]}}"#;

const OWNED: &str = r#"{"response":{"game_count":3,"games":[
  {"appid":220,"name":"Half-Life 2","playtime_forever":745,"img_icon_url":"x","rtime_last_played":1700000000},
  {"appid":70,"name":"Half-Life","playtime_forever":0,"rtime_last_played":0},
  {"appid":70,"name":"Half-Life","playtime_forever":0},
  {"appid":99,"name":""}
]}}"#;

fn job(dir: &std::path::Path) -> Job {
    Job {
        cache_dir: dir.to_path_buf(),
        names: vec![440, 378648, 1493710, 50300, 777],
        owned: vec![ACCOUNT],
        key: Some(KEY.into()),
        secrets: vec![KEY.into()],
        local_only: false,
    }
}

const ITEMS_URL: &str = "https://api.steampowered.com/IStoreBrowseService/GetItems/v1/?input_json=";
const OWNED_URL: &str = "https://api.steampowered.com/IPlayerService/GetOwnedGames/v1/?key=";

#[test]
fn store_types_and_urls() {
    assert_eq!(store_kind(0), Some("game"));
    assert_eq!(store_kind(4), Some("dlc"));
    assert_eq!(store_kind(6), Some("application"));
    assert_eq!(store_kind(11), Some("music"));
    assert_eq!(store_kind(99), None);
    let u = get_items_url(&[440, 570]);
    assert!(u.starts_with(ITEMS_URL), "{u}");
    assert!(u.contains("%22appid%22%3A440") && u.contains("570"), "{u}");
    let u = owned_url(KEY, ACCOUNT);
    assert!(u.contains("steamid=76561197960278073"), "{u}");
    assert!(u.contains("include_appinfo=1") && u.contains("include_played_free_games=1"));
    assert!(key_looks_complete(KEY) && key_looks_complete(&format!(" {KEY} ")));
    assert!(!key_looks_complete("0123") && !key_looks_complete(&KEY.replace('A', "Z")));
}

#[test]
fn parses_get_items() {
    let v: serde_json::Value = serde_json::from_str(ITEMS).unwrap();
    let got: std::collections::HashMap<u32, StoreName> =
        parse_get_items(&v, &[440, 378648, 1493710, 50300, 777], 5).into_iter().collect();
    assert_eq!(got.len(), 5);
    assert_eq!(got[&440].name.as_deref(), Some("Team Fortress 2"));
    assert_eq!(got[&440].kind.as_deref(), Some("game"));
    assert_eq!(got[&378648].kind.as_deref(), Some("dlc"));
    assert_eq!(got[&50300].name.as_deref(), Some("Spec Ops: The Line"), "delisted games keep their name");
    for id in [1493710, 777] {
        assert_eq!(got[&id], StoreName { name: None, kind: None, checked: 5 }, "{id}");
    }
}

#[test]
fn parses_owned_games() {
    let v: serde_json::Value = serde_json::from_str(OWNED).unwrap();
    let g = parse_owned(&v).unwrap();
    assert_eq!(g.len(), 2, "duplicates and nameless entries dropped");
    assert_eq!(g[0], OwnedGame { appid: 70, name: "Half-Life".into(), playtime_mins: 0, last_played: None });
    assert_eq!(g[1].playtime_mins, 745);
    assert_eq!(g[1].last_played, Some(1_700_000_000));
    // A private profile (another account's key): an empty response.
    let e = parse_owned(&serde_json::json!({"response": {}})).unwrap_err();
    assert!(e.contains("Game details"), "{e}");
    assert!(parse_owned(&serde_json::json!({"response": {"game_count": 0}})).unwrap().is_empty());
}

#[test]
fn run_saves_both_caches() {
    let dir = tempfile::tempdir().unwrap();
    let net = Mock::default().route(ITEMS_URL, 200, ITEMS).route(OWNED_URL, 200, OWNED);
    let out = run(&net, &Throttle::none(), &job(dir.path()));
    assert_eq!(out.error, None);
    assert_eq!(out.names_asked, 5);
    assert_eq!(out.names_found, 3);
    assert!(out.owned_changed && out.library_changed());
    assert_eq!(out.owned_count, Some(2));
    assert_eq!(net.calls().len(), 2);
    let names = names::load_names(dir.path());
    assert_eq!(names.apps[&440].name.as_deref(), Some("Team Fortress 2"));
    assert_eq!(names.apps[&777].name, None);
    let owned = names::load_owned(dir.path());
    assert_eq!(owned.accounts[&ACCOUNT].games.len(), 2);
    // The key is in no cache file.
    for f in [names::NAMES_FILE, names::OWNED_FILE] {
        assert!(!std::fs::read_to_string(dir.path().join(f)).unwrap().contains(KEY));
    }
    // The same answers again: nothing new, nothing to rescan.
    let out = run(&net, &Throttle::none(), &job(dir.path()));
    assert_eq!(out.names_found, 0);
    assert!(!out.owned_changed && !out.library_changed());
    // The owned list is fresh now: not due for a day, unless forced (the key changed).
    let now = names::now();
    assert!(owned_due(dir.path(), &[ACCOUNT], false, now).is_empty());
    assert_eq!(owned_due(dir.path(), &[ACCOUNT, 7], false, now), vec![7]);
    assert_eq!(owned_due(dir.path(), &[ACCOUNT], false, now + OWNED_TTL + 1), vec![ACCOUNT]);
    assert_eq!(owned_due(dir.path(), &[ACCOUNT], true, now), vec![ACCOUNT]);
}

#[test]
fn big_requests_are_split() {
    let dir = tempfile::tempdir().unwrap();
    let net = Mock::default().route(ITEMS_URL, 200, r#"{"response":{"store_items":[]}}"#);
    let j = Job { names: (1..=250).collect(), key: None, ..job(dir.path()) };
    let out = run(&net, &Throttle::none(), &j);
    assert_eq!(net.calls().len(), 3);
    assert_eq!(out.names_asked, 250);
}

/// Screenshot runs never go online.
#[test]
fn local_only_asks_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let net = Mock::default().route(ITEMS_URL, 200, ITEMS).route(OWNED_URL, 200, OWNED);
    let out = run(&net, &Throttle::none(), &Job { local_only: true, ..job(dir.path()) });
    assert_eq!(out, Outcome::default());
    assert!(net.calls().is_empty());
    assert!(!dir.path().join(names::NAMES_FILE).exists());
}

/// Failures keep the old caches and never show the key.
#[test]
fn failures_are_redacted_and_keep_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let good = Mock::default().route(ITEMS_URL, 200, ITEMS).route(OWNED_URL, 200, OWNED);
    run(&good, &Throttle::none(), &job(dir.path()));
    let before = names::load_owned(dir.path());

    let down = Mock { fail: true, ..Default::default() };
    let out = run(&down, &Throttle::none(), &job(dir.path()));
    let e = out.error.clone().unwrap();
    assert!(!e.contains(KEY), "{e}");
    assert!(!out.library_changed());
    assert_eq!(names::load_owned(dir.path()), before);

    let refused = Mock::default().route(OWNED_URL, 403, "");
    let out = run(&refused, &Throttle::none(), &Job { names: vec![], ..job(dir.path()) });
    assert_eq!(out.error.as_deref(), Some("Steam didn't accept the Web API key"));
    assert_eq!(names::load_owned(dir.path()), before);

    let private = Mock::default().route(OWNED_URL, 200, r#"{"response":{}}"#);
    let out = run(&private, &Throttle::none(), &Job { names: vec![], ..job(dir.path()) });
    assert!(out.error.unwrap().contains("Game details"));
    assert_eq!(names::load_owned(dir.path()), before);
}

#[test]
fn job_debug_hides_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let s = format!("{:?}", job(dir.path()));
    assert!(!s.contains(KEY), "{s}");
}

/// Against the real store: `LODESTAR_STEAM_NAMES_CACHE=<dir> LODESTAR_STEAM_NAMES_IDS=10,20 cargo test
/// --release real_store_lookup -- --ignored --nocapture` (keyless; no account data is sent).
#[test]
#[ignore]
fn real_store_lookup() {
    let (Some(dir), Some(ids)) =
        (std::env::var_os("LODESTAR_STEAM_NAMES_CACHE"), std::env::var("LODESTAR_STEAM_NAMES_IDS").ok())
    else {
        return;
    };
    let names: Vec<u32> = ids.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    let job = Job { cache_dir: dir.into(), names, ..Default::default() };
    let out = run(&crate::art::net::UreqNet::new(), &Throttle::polite(), &job);
    eprintln!("{out:?}");
    for (id, e) in names::load_names(&job.cache_dir).apps {
        eprintln!("{id}: {:?} {:?}", e.name, e.kind);
    }
}
