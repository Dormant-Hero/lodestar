//! Steam achievements against synthetic Steam folders (binary KeyValues written with the vdf test
//! writer) and a mocked network.

use super::*;
use crate::achievements::Env;
use crate::art::net::{HttpResponse, Net};
use crate::config::AchievementsConfig;
use crate::scan::vdf::write as bw;
use crate::test_support::write_file;

const APPID: u32 = 620;
/// Account 101 = Steam64 76561197960265829.
const ACCOUNT: u32 = 101;
const SID64: u64 = 76_561_197_960_265_829;
const KEY: &str = "TESTKEY0123456789ABCDEF";
const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F'];

// ---------------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------------

/// Mocked transport: exact URL → (status, body); pictures answer with a JPEG unless switched off.
#[derive(Default)]
struct FakeNet {
    routes: parking_lot::Mutex<HashMap<String, (u16, Vec<u8>)>>,
    calls: parking_lot::Mutex<Vec<String>>,
    icons_fail: bool,
}

impl FakeNet {
    fn route(self, url: &str, status: u16, body: &str) -> Self {
        self.routes.lock().insert(url.into(), (status, body.as_bytes().to_vec()));
        self
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().clone()
    }
    fn icon_calls(&self) -> usize {
        self.calls().iter().filter(|u| u.contains("steamstatic.com")).count()
    }
    fn api_calls(&self) -> Vec<String> {
        self.calls().into_iter().filter(|u| u.contains("api.steampowered.com")).collect()
    }
}

impl Net for FakeNet {
    fn get(&self, url: &str, _bearer: Option<&str>, _limit: u64) -> Result<HttpResponse, String> {
        self.calls.lock().push(url.to_string());
        if url.starts_with("https://shared.steamstatic.com/") {
            if self.icons_fail {
                return Err(format!("connection refused: {url}"));
            }
            return Ok(HttpResponse { status: 200, body: JPEG.to_vec(), ..Default::default() });
        }
        match self.routes.lock().get(url) {
            Some((status, body)) => Ok(HttpResponse {
                status: *status,
                body: if *status == 200 { body.clone() } else { vec![] },
                ..Default::default()
            }),
            None => Ok(HttpResponse { status: 404, ..Default::default() }),
        }
    }
    fn head(&self, _url: &str) -> Result<u16, String> {
        Ok(404)
    }
}

fn games_url() -> String {
    format!("https://api.steampowered.com/IPlayerService/GetGameAchievements/v1/?appid={APPID}&language=english")
}
fn global_url() -> String {
    format!("https://api.steampowered.com/ISteamUserStats/GetGlobalAchievementPercentagesForApp/v2/?gameid={APPID}")
}
fn player_url() -> String {
    format!(
        "https://api.steampowered.com/ISteamUserStats/GetPlayerAchievements/v1/?key={KEY}&steamid={SID64}&appid={APPID}&l=english"
    )
}

/// One achievement in a schema file.
struct A<'a> {
    bit: &'a str,
    id: &'a str,
    names: &'a [(&'a str, &'a str)],
    desc: &'a str,
    hidden: bool,
}

fn a<'a>(bit: &'a str, id: &'a str, english: &'a str) -> A<'a> {
    // Leaked so the fixture can borrow it for the test's lifetime.
    let names: &'a [(&'a str, &'a str)] =
        Box::leak(vec![("english", english), ("token", "NEW_ACH_TOKEN")].into_boxed_slice());
    A { bit, id, names, desc: "", hidden: false }
}

/// `"<appid>" { stats { <statid> { type, bits { … } } } }`; `stats`: (statid, type as written, bits).
/// A stat block of a schema fixture: (statid, `type` as a string, `type` as a number, bits).
type Stat<'a> = (&'a str, Option<&'a str>, Option<i32>, Vec<A<'a>>);

fn schema_bytes(stats: &[Stat]) -> Vec<u8> {
    let mut b = Vec::new();
    bw::obj(&mut b, &APPID.to_string());
    bw::obj(&mut b, "stats");
    for (id, ty_str, ty_int, bits) in stats {
        bw::obj(&mut b, id);
        if let Some(t) = ty_str {
            bw::str(&mut b, "type", t);
        }
        if let Some(t) = ty_int {
            bw::int(&mut b, "type", *t);
        }
        if bits.is_empty() {
            bw::str(&mut b, "name", "SOME_INT_STAT");
        } else {
            bw::obj(&mut b, "bits");
            for x in bits {
                bw::obj(&mut b, x.bit);
                bw::str(&mut b, "name", x.id);
                bw::obj(&mut b, "display");
                bw::obj(&mut b, "name");
                for (lang, s) in x.names {
                    bw::str(&mut b, lang, s);
                }
                bw::end(&mut b);
                bw::obj(&mut b, "desc");
                bw::str(&mut b, "english", if x.desc.is_empty() { "Do the thing." } else { x.desc });
                bw::str(&mut b, "token", "NEW_ACH_DESC");
                bw::end(&mut b);
                bw::int(&mut b, "hidden", x.hidden as i32);
                bw::str(&mut b, "icon", &format!("{}c0ffee.jpg", x.id.to_ascii_lowercase()));
                bw::str(&mut b, "icon_gray", &format!("{}9a11.jpg", x.id.to_ascii_lowercase()));
                bw::end(&mut b);
                bw::end(&mut b);
            }
            bw::end(&mut b);
        }
        bw::end(&mut b);
    }
    bw::end(&mut b);
    bw::int(&mut b, "version", 3);
    bw::str(&mut b, "gamename", "Fixture Game");
    bw::end(&mut b);
    bw::end(&mut b);
    b
}

/// `cache { crc, PendingChanges, <statid> { data, AchievementTimes { <bit> <t> } } }`.
/// A block of a player's file: (statid, unlock bitfield, (bit, unix time) pairs).
type Block<'a> = (&'a str, u32, &'a [(u32, i32)]);

fn user_bytes(blocks: &[Block]) -> Vec<u8> {
    let mut b = Vec::new();
    bw::obj(&mut b, "cache");
    bw::int(&mut b, "crc", 12345);
    bw::int(&mut b, "PendingChanges", 0);
    for (stat, field, times) in blocks {
        bw::obj(&mut b, stat);
        bw::int(&mut b, "data", *field as i32);
        if !times.is_empty() {
            bw::obj(&mut b, "AchievementTimes");
            for (bit, t) in *times {
                bw::int(&mut b, &bit.to_string(), *t);
            }
            bw::end(&mut b);
        }
        bw::end(&mut b);
    }
    bw::end(&mut b);
    bw::end(&mut b);
    b
}

struct Fx {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    cache: PathBuf,
}

impl Fx {
    /// A Steam folder with account 101 signed in (no achievement files yet).
    fn new() -> Fx {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Steam");
        write_file(
            &root.join("config/loginusers.vdf"),
            format!(
                "\"users\"\n{{\n\t\"{SID64}\"\n\t{{\n\t\t\"MostRecent\"\t\t\"1\"\n\t\t\"Timestamp\"\t\t\"100\"\n\t}}\n}}\n"
            ),
        );
        std::fs::create_dir_all(root.join(format!("userdata/{ACCOUNT}/config"))).unwrap();
        std::fs::create_dir_all(root.join("appcache/stats")).unwrap();
        let cache = tmp.path().join("cache/achievements");
        Fx { _tmp: tmp, root, cache }
    }
    fn schema(&self, bytes: &[u8]) -> &Self {
        write_file(&self.stats().join(format!("UserGameStatsSchema_{APPID}.bin")), bytes);
        self
    }
    fn user(&self, account: u32, bytes: &[u8]) -> &Self {
        write_file(&self.stats().join(format!("UserGameStats_{account}_{APPID}.bin")), bytes);
        self
    }
    fn library_json(&self, text: &str) -> &Self {
        write_file(&self.root.join(format!("userdata/{ACCOUNT}/config/librarycache/{APPID}.json")), text);
        self
    }
    fn stats(&self) -> PathBuf {
        self.root.join("appcache/stats")
    }
    fn icon_dir(&self) -> PathBuf {
        self.cache.join("steam").join(APPID.to_string())
    }
    fn load(&self, net: &Arc<FakeNet>, cfg: AchievementsConfig, force: bool, local_only: bool) -> State {
        let mut game = Game::blank("Fixture Game");
        game.source = Source::Steam;
        game.source_id = APPID.to_string();
        let req = Request { game, config: cfg, force };
        let env = Env { cache_dir: self.cache.clone(), net: net.clone() };
        load_from(&req, &env, std::slice::from_ref(&self.root), "english", local_only)
    }
}

fn cfg() -> AchievementsConfig {
    AchievementsConfig::default()
}

fn cfg_key() -> AchievementsConfig {
    AchievementsConfig { steam_web_api_key: Some(format!(" {KEY} ")), ..Default::default() }
}

fn ready(s: State) -> Arc<Progress> {
    match s {
        State::Ready(p) => p,
        other => panic!("expected a list, got {other:?}"),
    }
}

fn find<'a>(p: &'a Progress, id: &str) -> &'a Achievement {
    p.list.iter().find(|a| a.id == id).unwrap_or_else(|| panic!("{id} missing"))
}

/// Four achievements in block 1 (bit 31 included), one in an old-style block 2 (`type` 4).
fn standard_schema() -> Vec<u8> {
    let hidden = A { hidden: true, desc: "The secret ending.", ..a("2", "SECRET", "Secret") };
    let hidden_done = A { hidden: true, desc: "Found it.", ..a("3", "SECRET_DONE", "Secret Done") };
    schema_bytes(&[
        (
            "1",
            Some("ACHIEVEMENTS"),
            None,
            vec![a("0", "FIRST", "First"), a("1", "SECOND", "Second"), hidden, hidden_done, a("31", "LAST", "Last")],
        ),
        ("2", None, Some(4), vec![a("0", "OLD", "Old Style")]),
        ("3", Some("INT"), None, vec![]),
    ])
}

fn standard_user() -> Vec<u8> {
    // Block 1: bits 0, 3, 31 and 5 (retired: not in the schema); block 2: bit 0.
    let field1 = 1 | 1 << 3 | 1 << 5 | 1 << 31;
    user_bytes(&[("1", field1, &[(0, 1_000), (3, 3_000), (5, 9_000), (31, 2_000)]), ("2", 1, &[(0, 4_000)])])
}

// ---------------------------------------------------------------------------------------------
// Local files
// ---------------------------------------------------------------------------------------------

#[test]
fn unlock_bits_count_only_the_schemas_achievements_and_bit_31_works() {
    let fx = Fx::new();
    fx.schema(&standard_schema()).user(ACCOUNT, &standard_user());
    let net = Arc::new(FakeNet::default());
    let p = ready(fx.load(&net, cfg(), false, false));
    assert_eq!(p.service, Service::Steam);
    assert_eq!(p.total, 6, "the INT stat isn't an achievement; the retired bit isn't counted");
    assert_eq!(p.unlocked, 4);
    assert!(p.synced);
    assert_eq!(p.note, None);
    assert_eq!(find(&p, "FIRST").unlocked_at, Some(1_000));
    assert_eq!(find(&p, "SECOND").unlocked_at, None);
    assert_eq!(find(&p, "LAST").unlocked_at, Some(2_000));
    assert_eq!(find(&p, "OLD").unlocked_at, Some(4_000), "old schemas write type as a number");
    let order: Vec<&str> = p.list.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(order, ["FIRST", "SECOND", "SECRET", "SECRET_DONE", "LAST", "OLD"], "the developer's order");
}

#[test]
fn recent_unlocks_are_newest_first() {
    let fx = Fx::new();
    fx.schema(&standard_schema()).user(ACCOUNT, &standard_user());
    let p = ready(fx.load(&Arc::new(FakeNet::default()), cfg(), false, false));
    let recent: Vec<&str> = p.recent().iter().map(|a| a.id.as_str()).collect();
    assert_eq!(recent, ["OLD", "SECRET_DONE", "LAST", "FIRST"]);
}

#[test]
fn hidden_locked_achievement_keeps_its_secret_but_an_unlocked_one_shows_its_text() {
    let fx = Fx::new();
    fx.schema(&standard_schema()).user(ACCOUNT, &standard_user());
    let p = ready(fx.load(&Arc::new(FakeNet::default()), cfg(), false, false));
    let locked = find(&p, "SECRET");
    assert_eq!(locked.name, HIDDEN_NAME);
    assert_eq!(locked.description, "");
    assert!(locked.hidden);
    let done = find(&p, "SECRET_DONE");
    assert_eq!(done.name, "Secret Done");
    assert_eq!(done.description, "Found it.");
    assert!(done.hidden);
    assert_eq!(find(&p, "FIRST").description, "Do the thing.");
}

#[test]
fn names_come_in_the_users_language_then_english_never_the_token() {
    let de = A { names: &[("token", "TOK"), ("english", "Hello"), ("german", "Hallo")], ..a("0", "A", "") };
    let only_fr = A { names: &[("token", "TOK"), ("french", "Bonjour")], ..a("1", "B", "") };
    let none = A { names: &[("token", "TOK")], ..a("2", "C_API_NAME", "") };
    let fx = Fx::new();
    fx.schema(&schema_bytes(&[("1", Some("ACHIEVEMENTS"), None, vec![de, only_fr, none])]));
    let defs = local::read_schema(&fx.stats().join(format!("UserGameStatsSchema_{APPID}.bin")), "german").unwrap();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["Hallo", "Bonjour", "C_API_NAME"]);
    let defs = local::read_schema(&fx.stats().join(format!("UserGameStatsSchema_{APPID}.bin")), "koreana").unwrap();
    assert_eq!(defs[0].name, "Hello");
}

#[test]
fn desktop_locale_maps_to_steam_language_names() {
    for (loc, lang) in [
        ("de_DE.UTF-8", "german"),
        ("en_US.UTF-8", "english"),
        ("pt_BR.UTF-8", "brazilian"),
        ("pt_PT", "portuguese"),
        ("zh_TW.UTF-8", "tchinese"),
        ("zh_CN.UTF-8", "schinese"),
        ("es_ES.UTF-8", "spanish"),
        ("es_MX.UTF-8", "latam"),
        ("ko_KR.UTF-8", "koreana"),
        ("sr_RS@latin", "english"),
        ("", "english"),
    ] {
        assert_eq!(local::steam_language(loc), lang, "{loc}");
    }
}

#[test]
fn list_without_the_players_file_shows_everything_locked_with_a_hint() {
    let fx = Fx::new();
    fx.schema(&standard_schema());
    let net = Arc::new(FakeNet::default());
    let p = ready(fx.load(&net, cfg(), false, false));
    assert!(!p.synced);
    assert_eq!(p.unlocked, 0);
    assert_eq!(p.total, 6);
    assert_eq!(p.note.as_deref(), Some(PLAY_ONCE));
    assert!(p.list.iter().all(|a| a.unlocked_at.is_none()));
    assert!(net.api_calls().iter().all(|u| !u.contains("GetPlayerAchievements")), "no key: no player lookup");
}

#[test]
fn another_accounts_file_is_used_when_the_signed_in_one_has_none() {
    let fx = Fx::new();
    fx.schema(&standard_schema()).user(202, &standard_user());
    let p = ready(fx.load(&Arc::new(FakeNet::default()), cfg(), false, false));
    assert!(p.synced);
    assert_eq!(p.unlocked, 4);
    let sid = 76_561_197_960_265_728u64 + 202;
    assert_eq!(
        p.page_url.as_deref(),
        Some(format!("https://steamcommunity.com/profiles/{sid}/stats/{APPID}/?tab=achievements").as_str())
    );
}

#[test]
fn page_link_is_the_players_steam_achievements_page() {
    let fx = Fx::new();
    fx.schema(&standard_schema()).user(ACCOUNT, &standard_user());
    let p = ready(fx.load(&Arc::new(FakeNet::default()), cfg(), false, false));
    assert_eq!(
        p.page_url.as_deref(),
        Some(format!("https://steamcommunity.com/profiles/{SID64}/stats/{APPID}/?tab=achievements").as_str())
    );
}

#[test]
fn schema_with_only_stats_means_no_section() {
    let fx = Fx::new();
    fx.schema(&schema_bytes(&[("3", Some("INT"), None, vec![])]));
    assert_eq!(fx.load(&Arc::new(FakeNet::default()), cfg(), false, false), State::None);
}

#[test]
fn broken_files_never_panic_and_fall_back_to_the_web() {
    let fx = Fx::new();
    let mut schema = standard_schema();
    schema.truncate(schema.len() / 2);
    fx.schema(&schema).user(ACCOUNT, &[0x00, b'c', b'a']);
    let net = Arc::new(FakeNet::default().route(&games_url(), 200, r#"{"response":{}}"#));
    assert_eq!(fx.load(&net, cfg(), false, false), State::None);
    assert_eq!(net.api_calls(), [games_url()]);
}

// ---------------------------------------------------------------------------------------------
// Rarity
// ---------------------------------------------------------------------------------------------

fn library_json(ids: &[(&str, f64)]) -> String {
    let items: Vec<String> = ids
        .iter()
        .map(|(id, p)| format!(r#"{{"strID":"{id}","strName":"x","bAchieved":false,"flAchieved":{p}}}"#))
        .collect();
    format!(
        r#"[["badge",{{"version":1,"data":{{}}}}],["achievements",{{"version":2,"data":{{"nTotal":6,"nAchieved":4,"vecHighlight":[{}],"vecUnachieved":[],"vecAchievedHidden":[]}}}}]]["usernews",{{"#,
        items.join(",")
    )
}

#[test]
fn rarity_comes_from_the_library_cache_first_value_even_with_trailing_bytes() {
    let fx = Fx::new();
    let all: Vec<(&str, f64)> =
        ["FIRST", "SECOND", "SECRET", "SECRET_DONE", "LAST", "OLD"].iter().map(|id| (*id, 12.5)).collect();
    fx.schema(&standard_schema()).user(ACCOUNT, &standard_user()).library_json(&library_json(&all));
    let net = Arc::new(FakeNet::default());
    let p = ready(fx.load(&net, cfg(), false, false));
    assert!(p.list.iter().all(|a| a.rarity == Some(12.5)));
    assert!(net.api_calls().is_empty(), "every percentage was local: {:?}", net.api_calls());
}

#[test]
fn rarity_falls_back_to_steams_keyless_percentages_and_is_cached() {
    let fx = Fx::new();
    fx.schema(&standard_schema()).user(ACCOUNT, &standard_user()).library_json(&library_json(&[("FIRST", 50.0)]));
    let body = r#"{"achievementpercentages":{"achievements":[{"name":"SECOND","percent":"3.5"},{"name":"LAST","percent":7}]}}"#;
    let net = Arc::new(FakeNet::default().route(&global_url(), 200, body));
    let p = ready(fx.load(&net, cfg(), false, false));
    assert_eq!(find(&p, "FIRST").rarity, Some(50.0), "gaps filled from the library cache");
    assert_eq!(find(&p, "SECOND").rarity, Some(3.5));
    assert_eq!(find(&p, "LAST").rarity, Some(7.0));
    assert_eq!(find(&p, "OLD").rarity, None);
    assert_eq!(net.api_calls(), [global_url()]);
    // A forced re-read (after playing) uses the week-long cache.
    let net2 = Arc::new(FakeNet::default());
    let p = ready(fx.load(&net2, cfg(), true, false));
    assert_eq!(find(&p, "SECOND").rarity, Some(3.5));
    assert!(net2.api_calls().is_empty());
}

// ---------------------------------------------------------------------------------------------
// Pictures and the in-memory cache
// ---------------------------------------------------------------------------------------------

#[test]
fn pictures_are_downloaded_once_and_given_as_local_files() {
    let fx = Fx::new();
    fx.schema(&standard_schema()).user(ACCOUNT, &standard_user());
    let net = Arc::new(FakeNet::default());
    let p = ready(fx.load(&net, cfg(), false, false));
    assert_eq!(net.icon_calls(), 12, "colour + grey for 6 achievements");
    let first = find(&p, "FIRST");
    let icon = PathBuf::from(first.icon.as_deref().unwrap());
    assert_eq!(icon, fx.icon_dir().join("firstc0ffee.jpg"));
    assert_eq!(std::fs::read(&icon).unwrap(), JPEG);
    assert_eq!(PathBuf::from(first.icon_locked.as_deref().unwrap()), fx.icon_dir().join("first9a11.jpg"));
    assert!(
        net.calls()
            .iter()
            .any(|u| u == "https://shared.steamstatic.com/community_assets/images/apps/620/firstc0ffee.jpg")
    );
    // Read again from the files (forced): the pictures are already there.
    let net2 = Arc::new(FakeNet::default());
    let p2 = ready(fx.load(&net2, cfg(), true, false));
    assert_eq!(net2.icon_calls(), 0);
    assert_eq!(p2.list, p.list);
}

#[test]
fn failed_picture_downloads_leave_no_picture() {
    let fx = Fx::new();
    fx.schema(&standard_schema()).user(ACCOUNT, &standard_user());
    let net = Arc::new(FakeNet { icons_fail: true, ..Default::default() });
    let p = ready(fx.load(&net, cfg(), false, false));
    assert!(p.list.iter().all(|a| a.icon.is_none() && a.icon_locked.is_none()));
    assert!(net.icon_calls() <= 4, "the first network failure stops the rest: {}", net.icon_calls());
    assert_eq!(p.unlocked, 4);
}

#[test]
fn opening_the_page_again_reuses_the_parsed_result_until_the_files_change() {
    let fx = Fx::new();
    fx.schema(&standard_schema()).user(ACCOUNT, &standard_user());
    let net =
        Arc::new(FakeNet::default().route(&global_url(), 200, r#"{"achievementpercentages":{"achievements":[]}}"#));
    let p1 = ready(fx.load(&net, cfg(), false, false));
    let n = net.calls().len();
    let p2 = ready(fx.load(&net, cfg(), false, false));
    assert!(Arc::ptr_eq(&p1, &p2), "same result, not parsed again");
    assert_eq!(net.calls().len(), n);
    // A new unlock rewrites the player's file.
    fx.user(ACCOUNT, &user_bytes(&[("1", 0b11, &[(0, 1_000), (1, 5_000)])]));
    let p3 = ready(fx.load(&net, cfg(), false, false));
    assert_eq!(p3.unlocked, 2);
    assert_eq!(p3.recent()[0].id, "SECOND");
}

// ---------------------------------------------------------------------------------------------
// No local files: the web
// ---------------------------------------------------------------------------------------------

const TWO: &str = r#"{"response":{"achievements":[
  {"internal_name":"WEB_A","localized_name":"Web A","localized_desc":"First one","icon":"aaaa.jpg","icon_gray":"aaaa_bw.jpg","hidden":false,"player_percent_unlocked":"74.0"},
  {"internal_name":"WEB_B","localized_name":"Web B","localized_desc":"Secret one","icon":"bbbb.jpg","icon_gray":"bbbb_bw.jpg","hidden":true,"player_percent_unlocked":"1.5"}]}}"#;

#[test]
fn no_files_and_steam_says_no_achievements_means_no_section() {
    let fx = Fx::new();
    let net = Arc::new(FakeNet::default().route(&games_url(), 200, r#"{"response":{}}"#));
    assert_eq!(fx.load(&net, cfg(), false, false), State::None);
}

#[test]
fn no_files_and_steam_lists_some_gives_a_plain_hint() {
    let fx = Fx::new();
    let net = Arc::new(FakeNet::default().route(&games_url(), 200, TWO));
    let s = fx.load(&net, cfg(), false, false);
    assert_eq!(s, State::Hint(has_achievements_hint(2)));
    let State::Hint(text) = s else { unreachable!() };
    assert!(text.starts_with("This game has 2 achievements. Play it once through Steam"), "{text}");
    assert!(has_achievements_hint(1).starts_with("This game has 1 achievement."));
    // The list is cached: the next look needs no network.
    let net2 = Arc::new(FakeNet::default());
    assert_eq!(fx.load(&net2, cfg(), true, false), State::Hint(has_achievements_hint(2)));
    assert!(net2.calls().is_empty());
}

#[test]
fn no_files_and_steam_unreachable_shows_nothing() {
    let fx = Fx::new();
    let net = Arc::new(FakeNet::default().route(&games_url(), 503, ""));
    assert_eq!(fx.load(&net, cfg(), false, false), State::None);
}

#[test]
fn web_api_key_reads_the_players_progress_when_steam_has_no_files() {
    let fx = Fx::new();
    let player = r#"{"playerstats":{"steamID":"x","gameName":"G","achievements":[
      {"apiname":"WEB_A","achieved":1,"unlocktime":1700000000},{"apiname":"WEB_B","achieved":0,"unlocktime":0}],"success":true}}"#;
    let net = Arc::new(FakeNet::default().route(&games_url(), 200, TWO).route(&player_url(), 200, player));
    let p = ready(fx.load(&net, cfg_key(), false, false));
    assert!(p.synced);
    assert_eq!((p.unlocked, p.total), (1, 2));
    assert_eq!(find(&p, "WEB_A").unlocked_at, Some(1_700_000_000));
    assert_eq!(find(&p, "WEB_A").rarity, Some(74.0));
    let b = find(&p, "WEB_B");
    assert_eq!((b.name.as_str(), b.description.as_str(), b.hidden), (HIDDEN_NAME, "", true));
    assert!(b.icon_locked.as_deref().is_some_and(|i| i.ends_with("bbbb_bw.jpg")));
    assert!(net.calls().contains(&player_url()), "the key (trimmed) goes in the request");
}

#[test]
fn web_api_key_fills_in_progress_when_only_the_list_is_on_disk() {
    let fx = Fx::new();
    fx.schema(&standard_schema());
    let player =
        r#"{"playerstats":{"achievements":[{"apiname":"LAST","achieved":1,"unlocktime":1234}],"success":true}}"#;
    let net = Arc::new(FakeNet::default().route(&player_url(), 200, player));
    let p = ready(fx.load(&net, cfg_key(), false, false));
    assert!(p.synced);
    assert_eq!(p.note, None);
    assert_eq!(p.unlocked, 1);
    assert_eq!(find(&p, "LAST").unlocked_at, Some(1234));
}

#[test]
fn private_profile_or_wrong_key_shows_the_list_locked_with_a_reason() {
    let fx = Fx::new();
    let net = Arc::new(FakeNet::default().route(&games_url(), 200, TWO).route(&player_url(), 403, ""));
    let p = ready(fx.load(&net, cfg_key(), false, false));
    assert!(!p.synced);
    assert_eq!(p.note.as_deref(), Some(NOT_SHARED));
    assert_eq!(p.unlocked, 0);
    assert!(!p.note.as_deref().unwrap_or_default().contains(KEY));
    // Steam's "success: false" answer counts the same.
    let net = Arc::new(FakeNet::default().route(&games_url(), 200, TWO).route(
        &player_url(),
        200,
        r#"{"playerstats":{"error":"Profile is not public","success":false}}"#,
    ));
    let p = ready(fx.load(&net, cfg_key(), true, false));
    assert_eq!(p.note.as_deref(), Some(NOT_SHARED));
}

#[test]
fn transport_errors_never_carry_the_key() {
    let fx = Fx::new();
    let w = Web { net: &FailNet, dir: fx.cache.clone(), appid: APPID, local_only: false, secrets: cfg_key().secrets() };
    let err = web::player_achievements(&w, KEY, ACCOUNT, "english", true).unwrap_err();
    let Fail::Transient(text) = err else { panic!("{err:?}") };
    assert!(!text.contains(KEY), "{text}");
    assert!(text.contains("***"), "{text}");
}

/// Fails every request quoting the URL, like ureq does.
struct FailNet;
impl Net for FailNet {
    fn get(&self, url: &str, _b: Option<&str>, _l: u64) -> Result<HttpResponse, String> {
        Err(format!("io: connection reset while fetching {url}"))
    }
    fn head(&self, _url: &str) -> Result<u16, String> {
        Err("no".into())
    }
}

// ---------------------------------------------------------------------------------------------
// Screenshot runs
// ---------------------------------------------------------------------------------------------

#[test]
fn local_only_runs_never_go_online() {
    let fx = Fx::new();
    fx.schema(&standard_schema()).user(ACCOUNT, &standard_user());
    let net = Arc::new(FakeNet::default().route(&global_url(), 200, "{}"));
    let p = ready(fx.load(&net, cfg_key(), false, true));
    assert_eq!(p.unlocked, 4);
    assert!(p.list.iter().all(|a| a.icon.is_none()));
    // No files at all, with a key: still nothing asked.
    let fx2 = Fx::new();
    let net2 = Arc::new(FakeNet::default().route(&games_url(), 200, TWO));
    assert_eq!(fx2.load(&net2, cfg_key(), false, true), State::None);
    assert!(net.calls().is_empty(), "{:?}", net.calls());
    assert!(net2.calls().is_empty(), "{:?}", net2.calls());
}

#[test]
fn local_only_runs_still_show_pictures_already_downloaded() {
    let fx = Fx::new();
    fx.schema(&standard_schema()).user(ACCOUNT, &standard_user());
    write_file(&fx.icon_dir().join("firstc0ffee.jpg"), JPEG);
    let net = Arc::new(FakeNet::default());
    let p = ready(fx.load(&net, cfg(), false, true));
    assert!(find(&p, "FIRST").icon.is_some());
    assert!(find(&p, "FIRST").icon_locked.is_none());
    assert!(net.calls().is_empty());
}

// ---------------------------------------------------------------------------------------------
// Pieces
// ---------------------------------------------------------------------------------------------

#[test]
fn picture_names_from_files_cannot_leave_the_cache_folder() {
    assert_eq!(web::icon_file_name("ab12.jpg").as_deref(), Some("ab12.jpg"));
    assert_eq!(web::icon_file_name("ab12").as_deref(), Some("ab12.jpg"));
    assert_eq!(web::icon_file_name("WAKE_UP_BW.jpg").as_deref(), Some("WAKE_UP_BW.jpg"));
    for bad in ["../x.jpg", "a/b.jpg", "..", ".hidden.jpg", "", "a b.jpg"] {
        assert_eq!(web::icon_file_name(bad), None, "{bad}");
    }
}

#[test]
fn only_steam_games_apply() {
    let mut g = Game::blank("x");
    g.source = Source::Steam;
    g.source_id = "620".into();
    assert!(applies(&g));
    g.source_id = "shortcut:123".into();
    assert!(!applies(&g));
    g.source = Source::Manual;
    g.source_id = "620".into();
    assert!(!applies(&g));
}

/// Manual check against the real Steam install (read-only, offline): `cargo test --release
/// real_steam_files -- --ignored --nocapture`. Prints counts only; compares them with Steam's own
/// library-page numbers where that file has them.
#[test]
#[ignore]
fn real_steam_files() {
    let mut g = Game::blank("probe");
    g.source = Source::Steam;
    let roots = local::candidate_roots(&g);
    println!("steam roots: {}", roots.len());
    let tmp = tempfile::tempdir().unwrap();
    let env = Env { cache_dir: tmp.path().to_path_buf(), net: Arc::new(FailNet) };
    let (mut games, mut agree, mut differ, mut shown) = (0, 0, 0, 0);
    for root in &roots {
        let Ok(rd) = std::fs::read_dir(root.join("appcache/stats")) else { continue };
        let mut appids: Vec<u32> = rd
            .flatten()
            .filter_map(|e| {
                e.file_name().to_str()?.strip_prefix("UserGameStatsSchema_")?.strip_suffix(".bin")?.parse().ok()
            })
            .collect();
        appids.sort();
        for appid in appids {
            g.source_id = appid.to_string();
            let req = Request { game: g.clone(), config: AchievementsConfig::default(), force: true };
            let State::Ready(p) = load_from(&req, &env, std::slice::from_ref(root), "english", true) else { continue };
            games += 1;
            let files = local::find(std::slice::from_ref(root), appid);
            if let Some(j) = files.library_json.and_then(|p| std::fs::read(p).ok())
                && let Some(Ok(v)) = serde_json::Deserializer::from_slice(&j).into_iter::<serde_json::Value>().next()
                && let Some(d) = v.as_array().and_then(|s| {
                    s.iter()
                        .find(|x| x.get(0).and_then(|n| n.as_str()) == Some("achievements"))?
                        .get(1)?
                        .get("data")
                        .cloned()
                })
                && let (Some(n), Some(t)) =
                    (d.get("nAchieved").and_then(|x| x.as_u64()), d.get("nTotal").and_then(|x| x.as_u64()))
                && t > 0
            {
                if (n, t) == (p.unlocked as u64, p.total as u64) {
                    agree += 1;
                } else {
                    differ += 1;
                }
            }
            if shown < 3 && p.synced && p.unlocked > 0 {
                shown += 1;
                let hidden_locked = p.list.iter().filter(|a| a.hidden && a.unlocked_at.is_none()).count();
                let rare = p.list.iter().filter(|a| a.rarity.is_some()).count();
                println!(
                    "game {shown}: {} of {} unlocked, {} recent, {hidden_locked} hidden locked, {rare} with rarity, newest unlock time known: {}",
                    p.unlocked,
                    p.total,
                    p.recent().len().min(5),
                    p.recent().first().is_some_and(|a| a.unlocked_at.unwrap_or(0) > 0)
                );
            }
        }
    }
    println!("games with achievements: {games}; library-page counts agree: {agree}, differ: {differ}");
    assert!(differ == 0);
}

/// Manual check of the online parts (pictures, rarity) for one real game: `cargo test --release
/// real_steam_online -- --ignored --nocapture`. Uses a temporary cache; prints counts only.
#[test]
#[ignore]
fn real_steam_online() {
    let mut g = Game::blank("probe");
    g.source = Source::Steam;
    let roots = local::candidate_roots(&g);
    let root = roots.first().expect("a Steam install");
    let appid = std::fs::read_dir(root.join("appcache/stats"))
        .unwrap()
        .flatten()
        .filter_map(|e| {
            let id = e
                .file_name()
                .to_str()?
                .strip_prefix("UserGameStatsSchema_")?
                .strip_suffix(".bin")?
                .parse::<u32>()
                .ok()?;
            Some((e.metadata().ok()?.len(), id))
        })
        .max()
        .unwrap()
        .1;
    g.source_id = appid.to_string();
    let tmp = tempfile::tempdir().unwrap();
    let env = Env { cache_dir: tmp.path().to_path_buf(), net: Arc::new(crate::art::net::UreqNet::new()) };
    let req = Request { game: g, config: AchievementsConfig::default(), force: false };
    let t = Instant::now();
    let State::Ready(p) = load_from(&req, &env, std::slice::from_ref(root), "english", false) else {
        panic!("no list")
    };
    let first = t.elapsed();
    let icons = p.list.iter().filter(|a| a.icon.is_some()).count();
    let grey = p.list.iter().filter(|a| a.icon_locked.is_some()).count();
    let rare = p.list.iter().filter(|a| a.rarity.is_some()).count();
    let t = Instant::now();
    let again = load_from(&req, &env, std::slice::from_ref(root), "english", false);
    println!(
        "{} achievements: {icons} pictures, {grey} grey pictures, {rare} with rarity; first open {first:?}, again {:?} (same: {})",
        p.total,
        t.elapsed(),
        again == State::Ready(p.clone())
    );
}

#[test]
fn pictures_the_page_shows_are_fetched_first_newest_unlocks_leading() {
    let ach = |id: &str, at: Option<i64>| Achievement {
        id: id.into(),
        icon: Some(format!("{id}.jpg")),
        icon_locked: Some(format!("{id}_bw.jpg")),
        unlocked_at: at,
        ..Default::default()
    };
    let list = [ach("A", None), ach("B", Some(10)), ach("C", Some(30)), ach("D", None)];
    assert_eq!(
        icon_order(&list),
        ["C.jpg", "B.jpg", "A_bw.jpg", "D_bw.jpg", "C_bw.jpg", "B_bw.jpg", "A.jpg", "D.jpg"].map(String::from)
    );
}

/// Transport that answers every picture slowly (a slow line).
struct SlowNet(parking_lot::Mutex<usize>);

impl Net for SlowNet {
    fn get(&self, _url: &str, _b: Option<&str>, _l: u64) -> Result<HttpResponse, String> {
        std::thread::sleep(std::time::Duration::from_millis(20));
        *self.0.lock() += 1;
        Ok(HttpResponse { status: 200, body: JPEG.to_vec(), ..Default::default() })
    }
    fn head(&self, _url: &str) -> Result<u16, String> {
        Ok(404)
    }
}

#[test]
fn a_game_with_hundreds_of_pictures_on_a_slow_line_stops_within_the_time_budget() {
    let fx = Fx::new();
    let net = SlowNet(parking_lot::Mutex::new(0));
    let w = Web { net: &net, dir: fx.icon_dir(), appid: APPID, local_only: false, secrets: vec![] };
    let names: Vec<String> = (0..400).map(|i| format!("pic{i:04}.jpg")).collect();
    let started = std::time::Instant::now();
    let (got, complete) = web::icons_within(&w, &names, std::time::Duration::from_millis(150));
    assert!(started.elapsed() < std::time::Duration::from_secs(2), "took {:?}", started.elapsed());
    assert!(!complete, "the rest is fetched next time");
    assert!(!got.is_empty() && got.len() < 200, "{} pictures", got.len());
    assert!(got.contains_key("pic0000.jpg"), "the first (most wanted) pictures come first");
    // The next open fetches the rest without asking again for what is on disk.
    let (all, complete) = web::icons_within(&w, &names, std::time::Duration::from_secs(60));
    assert!(complete);
    assert_eq!(all.len(), 400);
    assert_eq!(*net.0.lock(), 400, "every picture asked for once");
}

// ---------------------------------------------------------------------------------------------
// The library pass (counts for every Steam game at once)
// ---------------------------------------------------------------------------------------------

#[test]
fn the_library_pass_counts_each_game_as_its_page_does_without_going_online() {
    let fx = Fx::new();
    fx.schema(&standard_schema()).user(ACCOUNT, &standard_user());
    let mut memo = bulk::Memo::default();
    let counts = bulk::counts(std::slice::from_ref(&fx.root), &[APPID, 999], &mut memo);
    assert_eq!(counts.get(&APPID), Some(&Some((4, 6))), "the same numbers as the game page");
    assert!(!counts.contains_key(&999), "no files: left to the game page");
    // The page agrees.
    let p = ready(fx.load(&Arc::new(FakeNet::default()), cfg(), false, true));
    assert_eq!((p.unlocked, p.total), (4, 6));
}

#[test]
fn the_library_pass_shows_no_count_for_a_game_not_played_on_this_computer() {
    let fx = Fx::new();
    fx.schema(&standard_schema());
    let counts = bulk::counts(std::slice::from_ref(&fx.root), &[APPID], &mut bulk::Memo::default());
    assert_eq!(counts.get(&APPID), Some(&None));
    // Stats only (no achievements): nothing either.
    let fx = Fx::new();
    fx.schema(&schema_bytes(&[("3", Some("INT"), None, vec![])])).user(ACCOUNT, &standard_user());
    let counts = bulk::counts(std::slice::from_ref(&fx.root), &[APPID], &mut bulk::Memo::default());
    assert_eq!(counts.get(&APPID), Some(&None));
}

#[test]
fn the_library_pass_reads_a_game_again_only_when_its_files_change() {
    let fx = Fx::new();
    fx.schema(&standard_schema()).user(ACCOUNT, &standard_user());
    let roots = [fx.root.clone()];
    let mut memo = bulk::Memo::default();
    assert_eq!(bulk::counts(&roots, &[APPID], &mut memo)[&APPID], Some((4, 6)));
    // Same size and time: not parsed again (garbage of the same length still counts 4).
    let user = fx.stats().join(format!("UserGameStats_{ACCOUNT}_{APPID}.bin"));
    let meta = std::fs::metadata(&user).unwrap();
    std::fs::write(&user, vec![0u8; meta.len() as usize]).unwrap();
    std::fs::File::options().write(true).open(&user).unwrap().set_modified(meta.modified().unwrap()).unwrap();
    assert_eq!(bulk::counts(&roots, &[APPID], &mut memo)[&APPID], Some((4, 6)));
    // Steam wrote new unlocks: read again.
    let field1 = 1 | 1 << 1 | 1 << 3 | 1 << 31;
    let newer =
        user_bytes(&[("1", field1, &[(0, 1_000), (1, 5_000), (3, 3_000), (31, 2_000)]), ("2", 1, &[(0, 4_000)])]);
    std::fs::write(&user, newer).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&user)
        .unwrap()
        .set_modified(meta.modified().unwrap() + std::time::Duration::from_secs(60))
        .unwrap();
    assert_eq!(bulk::counts(&roots, &[APPID], &mut memo)[&APPID], Some((5, 6)));
}

#[test]
fn the_library_pass_takes_the_signed_in_accounts_file_over_a_newer_one_of_another_account() {
    let fx = Fx::new();
    fx.schema(&standard_schema()).user(ACCOUNT, &standard_user());
    fx.user(202, &user_bytes(&[("1", 1, &[(0, 1_000)])]));
    let counts = bulk::counts(std::slice::from_ref(&fx.root), &[APPID], &mut bulk::Memo::default());
    assert_eq!(counts[&APPID], Some((4, 6)));
}

/// The library pass against the user's real Steam files (read-only, offline): run with
/// `cargo test --release real_steam_library_pass -- --ignored --nocapture`. Prints counts and time
/// only; every count must equal the game page's.
#[test]
#[ignore]
fn real_steam_library_pass() {
    let roots = bulk::roots(&[]);
    let mut appids = vec![];
    for root in &roots {
        let Ok(rd) = std::fs::read_dir(root.join("appcache/stats")) else { continue };
        appids.extend(rd.flatten().filter_map(|e| {
            e.file_name().to_str()?.strip_prefix("UserGameStatsSchema_")?.strip_suffix(".bin")?.parse::<u32>().ok()
        }));
    }
    let t = std::time::Instant::now();
    let mut memo = bulk::Memo::default();
    let counts = bulk::counts(&roots, &appids, &mut memo);
    let first = t.elapsed();
    let t = std::time::Instant::now();
    let again = bulk::counts(&roots, &appids, &mut memo);
    let second = t.elapsed();
    assert_eq!(counts, again);
    let shown = counts.values().filter(|c| c.is_some()).count();
    println!("games with lists: {}, with counts: {shown}, first pass {first:?}, again {second:?}", counts.len());
    let tmp = tempfile::tempdir().unwrap();
    let env = Env { cache_dir: tmp.path().to_path_buf(), net: Arc::new(FakeNet::default()) };
    let mut g = Game::blank("probe");
    g.source = Source::Steam;
    let mut differ = 0;
    for (appid, c) in &counts {
        g.source_id = appid.to_string();
        let req = Request { game: g.clone(), config: AchievementsConfig::default(), force: true };
        if let State::Ready(p) = load_from(&req, &env, &roots, "english", true) {
            let page = p.synced.then_some((p.unlocked, p.total));
            if page != *c {
                differ += 1;
                println!("app {appid}: library {c:?}, page {page:?}");
            }
        }
    }
    assert_eq!(differ, 0);
}
