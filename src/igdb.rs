//! IGDB (api-docs.igdb.com), shared by the artwork engine (covers, artworks) and the metadata
//! fetcher (description, genres, themes, modes, release date, companies, scores, series).
//!
//! * The user registers a Twitch application and pastes its client id + secret into
//!   Settings → Metadata. `POST https://id.twitch.tv/oauth2/token?client_id=…&client_secret=…
//!   &grant_type=client_credentials` → `{access_token, expires_in, token_type}`; the token is
//!   kept in memory (one per process, shared by every thread) until shortly before it expires.
//! * Queries: `POST https://api.igdb.com/v4/<endpoint>` with headers `Client-ID` and
//!   `Authorization: Bearer <token>`, body in Apicalypse (`search "…"; fields …; limit n;`).
//!   4 requests per second at most: both callers take turns through one process-wide window
//!   (`wait_turn`).
//! * Steam games are found through `external_games` (`external_game_source = 1` is Steam).
//! * Images: `https://images.igdb.com/igdb/image/upload/t_<size>/<image_id>.jpg`
//!   (cover_big 264×374, 720p 1280×720, 1080p 1920×1080; `_2x` doubles).
//!
//! Pure parts here; the HTTP is done by each caller with its own throttle.

use crate::model::Metadata;
use parking_lot::Mutex;
use serde_json::Value;
use std::time::{Duration, Instant};

/// IGDB allows 4 requests a second per application.
pub const PER_SECOND: usize = 4;
/// The window is a little longer than a second (clocks and network jitter).
const WINDOW: Duration = Duration::from_millis(1100);

/// At most `per` requests in any `window`: reserve a slot, get how long to wait for it.
#[derive(Debug)]
pub struct RateWindow {
    per: usize,
    window: Duration,
    sent: std::collections::VecDeque<Instant>,
}

impl RateWindow {
    pub const fn new(per: usize, window: Duration) -> RateWindow {
        RateWindow { per, window, sent: std::collections::VecDeque::new() }
    }
    /// Reserve the next free slot at or after `now`; the wait until it.
    pub fn reserve(&mut self, now: Instant) -> Duration {
        while self.sent.front().is_some_and(|t| now.saturating_duration_since(*t) >= self.window) {
            self.sent.pop_front();
        }
        let mut slot = now;
        if self.per > 0 && self.sent.len() >= self.per {
            slot = slot.max(self.sent[self.sent.len() - self.per] + self.window);
        }
        self.sent.push_back(slot);
        slot.saturating_duration_since(now)
    }
}

/// The one IGDB window of the process (artwork engine + metadata fetcher).
static WINDOW_SHARED: Mutex<RateWindow> = Mutex::new(RateWindow::new(PER_SECOND, WINDOW));

/// Wait for IGDB's next free slot (shared by every thread). False when cancelled meanwhile.
pub fn wait_turn(cancel: Option<&std::sync::atomic::AtomicBool>) -> bool {
    let d = WINDOW_SHARED.lock().reserve(Instant::now());
    crate::art::limits::sleep_cancellable(d, cancel)
}

pub const TWITCH_TOKEN: &str = "https://id.twitch.tv/oauth2/token";
pub const IGDB_API: &str = "https://api.igdb.com/v4";
pub const IGDB_IMAGES: &str = "https://images.igdb.com/igdb/image/upload";
/// IGDB's id of Steam in `external_game_source`.
pub const EXTERNAL_STEAM: u32 = 1;

const FIELDS: &str = "name,url,summary,first_release_date,aggregated_rating,rating,\
genres.name,themes.name,game_modes.name,player_perspectives.name,\
multiplayer_modes.offlinemax,multiplayer_modes.offlinecoopmax,multiplayer_modes.onlinemax,\
multiplayer_modes.onlinecoopmax,\
involved_companies.company.name,involved_companies.developer,involved_companies.publisher,\
collections.name,franchise.name,franchises.name,platforms.name,\
cover.image_id,cover.width,cover.height,artworks.image_id,artworks.width,artworks.height,\
screenshots.image_id,screenshots.width,screenshots.height";

pub fn token_url(client_id: &str, secret: &str) -> String {
    let e = crate::metadata::parse::encode_path_segment;
    format!("{TWITCH_TOKEN}?client_id={}&client_secret={}&grant_type=client_credentials", e(client_id), e(secret))
}

pub fn games_url() -> String {
    format!("{IGDB_API}/games")
}

pub fn external_games_url() -> String {
    format!("{IGDB_API}/external_games")
}

/// Apicalypse string literal (quotes and backslashes escaped, control characters dropped).
fn quoted(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
pub fn search_body(name: &str) -> String {
    search_body_on(name, None)
}

/// A name search limited to games released on one IGDB platform (consoles: an emulated
/// "Tekken 5" is the PlayStation 2 game, not the PC one).
pub fn search_body_on(name: &str, platform: Option<u32>) -> String {
    match platform {
        Some(p) => {
            let ids: Vec<String> =
                std::iter::once(p).chain(same_games_on(p).iter().copied()).map(|i| i.to_string()).collect();
            format!("search {}; fields {FIELDS}; where platforms = ({}); limit 10;", quoted(name.trim()), ids.join(","))
        }
        None => format!("search {}; fields {FIELDS}; limit 10;", quoted(name.trim())),
    }
}

/// IGDB platforms that hold the same console's games under another name: Japan-only NES / SNES
/// games are on "Family Computer" (99) / "Super Famicom" (58), New 3DS-only games on "New Nintendo
/// 3DS" (137), DSiWare on "Nintendo DSi" (159), Neo Geo games often only on "Neo Geo MVS" (79).
fn same_games_on(platform: u32) -> &'static [u32] {
    match platform {
        18 => &[99],
        19 => &[58],
        37 => &[137],
        20 => &[159],
        159 => &[20],
        80 => &[79],
        _ => &[],
    }
}

/// The IGDB platform of a console game (None for PC games, whose IGDB entries aren't filtered):
/// an emulated game's emulation platform, else a console its library platform names.
pub fn platform_of(g: &crate::model::Game) -> Option<u32> {
    g.emu
        .as_ref()
        .and_then(|e| crate::emulation::platforms::by_id(&e.platform))
        .and_then(|p| p.igdb)
        .or_else(|| crate::art::sources::console_platform(crate::art::sources::art_platform(g).as_deref())?.igdb)
}

pub fn by_ids_body(ids: &[u64]) -> String {
    let ids: Vec<String> = ids.iter().map(u64::to_string).collect();
    format!("fields {FIELDS}; where id = ({}); limit 10;", ids.join(","))
}

pub fn steam_body(appid: u32) -> String {
    format!("fields game; where external_game_source = {EXTERNAL_STEAM} & uid = \"{appid}\"; limit 5;")
}

/// The token answer → (token, lifetime). None = malformed.
pub fn parse_token(body: &[u8]) -> Option<(String, Duration)> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let t = v.get("access_token")?.as_str()?.trim();
    if t.is_empty() || !t.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    let secs = v.get("expires_in").and_then(Value::as_u64).unwrap_or(3600);
    Some((t.to_string(), Duration::from_secs(secs)))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IgdbImage {
    pub image_id: String,
    pub width: u32,
    pub height: u32,
}

impl IgdbImage {
    pub fn url(&self, size: &str) -> String {
        format!("{IGDB_IMAGES}/t_{size}/{}.jpg", self.image_id)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IgdbGame {
    pub id: u64,
    pub name: String,
    pub summary: Option<String>,
    /// Unix seconds.
    pub first_release: Option<i64>,
    pub critic: Option<f64>,
    pub community: Option<f64>,
    /// The game's page on igdb.com.
    pub url: Option<String>,
    pub genres: Vec<String>,
    pub themes: Vec<String>,
    pub modes: Vec<String>,
    /// Most players a mode allows (local first, else online; 0 = unknown).
    pub max_players: u32,
    pub developers: Vec<String>,
    pub publishers: Vec<String>,
    pub series: Option<String>,
    /// Platform names ("PlayStation 3", "PC (Microsoft Windows)"), to tell same-named games apart.
    pub platforms: Vec<String>,
    pub cover: Option<IgdbImage>,
    pub artworks: Vec<IgdbImage>,
    pub screenshots: Vec<IgdbImage>,
}

fn names(v: &Value, k: &str) -> Vec<String> {
    v.get(k)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.get("name").and_then(Value::as_str))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn image(v: &Value) -> Option<IgdbImage> {
    let id = v.get("image_id")?.as_str()?.trim();
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
    Some(IgdbImage { image_id: id.to_string(), width: n("width"), height: n("height") })
}

fn images(v: &Value, k: &str) -> Vec<IgdbImage> {
    v.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(image).collect()).unwrap_or_default()
}

/// `/games` → games. None = malformed.
pub fn parse_games(body: &[u8]) -> Option<Vec<IgdbGame>> {
    let v: Value = serde_json::from_slice(body).ok()?;
    Some(
        v.as_array()?
            .iter()
            .filter_map(|g| {
                let id = g.get("id")?.as_u64()?;
                let mut developers = vec![];
                let mut publishers = vec![];
                for ic in g.get("involved_companies").and_then(Value::as_array).into_iter().flatten() {
                    let Some(n) =
                        ic.pointer("/company/name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty())
                    else {
                        continue;
                    };
                    let flag = |k: &str| ic.get(k).and_then(Value::as_bool) == Some(true);
                    if flag("developer") && !developers.iter().any(|d| d == n) {
                        developers.push(n.to_string());
                    }
                    if flag("publisher") && !publishers.iter().any(|d| d == n) {
                        publishers.push(n.to_string());
                    }
                }
                let game_modes = names(g, "game_modes");
                let max_of = |keys: &[&str]| {
                    g.get("multiplayer_modes")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .flat_map(|m| keys.iter().filter_map(move |k| m.get(*k).and_then(Value::as_u64)))
                        .max()
                        .unwrap_or(0)
                        .min(64) as u32
                };
                let local = max_of(&["offlinemax", "offlinecoopmax"]);
                let max_players = if local > 1 { local } else { max_of(&["onlinemax", "onlinecoopmax"]) };
                let mut modes = game_modes;
                modes.extend(names(g, "player_perspectives"));
                let series = names(g, "collections")
                    .into_iter()
                    .next()
                    .or_else(|| g.pointer("/franchise/name").and_then(Value::as_str).map(|s| s.trim().to_string()))
                    .or_else(|| names(g, "franchises").into_iter().next())
                    .filter(|s| !s.is_empty());
                let num = |k: &str| g.get(k).and_then(Value::as_f64).filter(|n| n.is_finite());
                Some(IgdbGame {
                    max_players,
                    id,
                    name: g.get("name").and_then(Value::as_str).unwrap_or("").trim().to_string(),
                    summary: g
                        .get("summary")
                        .and_then(Value::as_str)
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty()),
                    first_release: g.get("first_release_date").and_then(Value::as_i64).filter(|t| *t > 0),
                    critic: num("aggregated_rating"),
                    community: num("rating"),
                    url: g
                        .get("url")
                        .and_then(Value::as_str)
                        .filter(|u| u.starts_with("https://www.igdb.com/"))
                        .map(str::to_string),
                    genres: names(g, "genres"),
                    themes: names(g, "themes"),
                    modes,
                    developers,
                    publishers,
                    series,
                    platforms: names(g, "platforms"),
                    cover: g.get("cover").and_then(image),
                    artworks: images(g, "artworks"),
                    screenshots: images(g, "screenshots"),
                })
            })
            .collect(),
    )
}

/// `/external_games` → IGDB game ids. None = malformed.
pub fn parse_external(body: &[u8]) -> Option<Vec<u64>> {
    let v: Value = serde_json::from_slice(body).ok()?;
    Some(v.as_array()?.iter().filter_map(|e| e.get("game").and_then(Value::as_u64)).collect())
}

/// The game named `name` among search hits (exact normalised name only).
pub fn confident<'a>(name: &str, games: &'a [IgdbGame]) -> Option<&'a IgdbGame> {
    games.iter().find(|g| crate::art::sources::same_name(name, &g.name))
}

/// "YYYY-MM-DD" (UTC) of unix seconds.
pub fn ymd(secs: i64) -> (i32, u32, u32) {
    // Civil-from-days (Howard Hinnant).
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

/// IGDB's facts as Lodestar metadata (only what IGDB knows; callers fill gaps with it).
pub fn to_metadata(g: &IgdbGame) -> Metadata {
    let join = |v: &[String]| (!v.is_empty()).then(|| v.join(", "));
    let score = |n: Option<f64>| n.map(|n| n.round().clamp(0.0, 100.0) as u8);
    let (year, date) = match g.first_release {
        Some(t) => {
            let (y, m, d) = ymd(t);
            (Some(y), Some(format!("{y:04}-{m:02}-{d:02}")))
        }
        None => (None, None),
    };
    Metadata {
        description: g.summary.clone(),
        developer: join(&g.developers),
        publisher: join(&g.publishers),
        genres: g.genres.clone(),
        release_year: year,
        release_date: date,
        themes: g.themes.clone(),
        features: g.modes.clone(),
        players: {
            let (single, multi) = crate::model::player_modes(&g.modes);
            crate::model::players_text(single, multi || g.max_players > 1, g.max_players)
        },
        critic_score: score(g.critic),
        community_score: score(g.community),
        igdb_url: g.url.clone(),
        series: g.series.clone(),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------------------------
// Token cache (process-wide)

struct Cached {
    token: String,
    until: Instant,
}

/// Tokens by credentials (normally one entry).
static TOKENS: Mutex<Vec<(u64, Cached)>> = Mutex::new(Vec::new());

pub(crate) fn cache_key(client_id: &str, secret: &str) -> u64 {
    crate::db::fnv1a(format!("{client_id}\n{secret}").as_bytes())
}

/// The cached token for these credentials, while it is good for another minute.
pub fn cached_token(client_id: &str, secret: &str) -> Option<String> {
    let k = cache_key(client_id, secret);
    TOKENS
        .lock()
        .iter()
        .find(|(key, c)| *key == k && c.until > Instant::now() + Duration::from_secs(60))
        .map(|(_, c)| c.token.clone())
}

pub fn store_token(client_id: &str, secret: &str, token: String, lifetime: Duration) {
    let k = cache_key(client_id, secret);
    let mut t = TOKENS.lock();
    t.retain(|(key, _)| *key != k);
    if t.len() >= 4 {
        t.remove(0);
    }
    t.push((k, Cached { token, until: Instant::now() + lifetime }));
}

/// IGDB said 401: the token for these credentials is no longer good.
pub fn forget_token(client_id: &str, secret: &str) {
    let k = cache_key(client_id, secret);
    TOKENS.lock().retain(|(key, _)| *key != k);
}

/// Credentials Twitch / IGDB rejected this session (so neither caller keeps asking; changing
/// them in Settings makes them be tried again).
static REJECTED: Mutex<Vec<u64>> = Mutex::new(Vec::new());

pub fn mark_rejected(client_id: &str, secret: &str) {
    let k = cache_key(client_id, secret);
    let mut r = REJECTED.lock();
    if !r.contains(&k) {
        r.push(k);
    }
}

pub fn is_rejected(client_id: &str, secret: &str) -> bool {
    REJECTED.lock().contains(&cache_key(client_id, secret))
}

/// What getting a token can end with.
#[derive(Debug, PartialEq)]
pub enum TokenErr {
    /// Twitch rejected the client id / secret (400 / 401 / 403).
    BadCredentials,
    Transient(String),
}

/// A token for the credentials: cached, or asked from Twitch through `post(url)` (which returns
/// (status, body) or a network error).
pub fn token(
    client_id: &str,
    secret: &str,
    post: impl FnOnce(&str) -> Result<(u16, Vec<u8>), String>,
) -> Result<String, TokenErr> {
    if let Some(t) = cached_token(client_id, secret) {
        return Ok(t);
    }
    if is_rejected(client_id, secret) {
        return Err(TokenErr::BadCredentials);
    }
    let (status, body) = post(&token_url(client_id, secret)).map_err(TokenErr::Transient)?;
    match status {
        200 => {
            let (t, life) =
                parse_token(&body).ok_or_else(|| TokenErr::Transient("Twitch: unexpected token response".into()))?;
            store_token(client_id, secret, t.clone(), life);
            Ok(t)
        }
        400 | 401 | 403 => {
            mark_rejected(client_id, secret);
            Err(TokenErr::BadCredentials)
        }
        s => Err(TokenErr::Transient(format!("Twitch: HTTP {s}"))),
    }
}

/// Headers of an IGDB request.
pub fn headers<'a>(client_id: &'a str, token: &'a str) -> [(&'static str, String); 2] {
    [("Client-ID", client_id.to_string()), ("Authorization", format!("Bearer {token}"))]
}

// ---------------------------------------------------------------------------------------------
// Games looked up before IGDB was set up (AppState::queue_igdb_backfill)

/// Which games had their IGDB-only background lookup for the current credentials (kept in the
/// DB `kv` as JSON, so each game is asked once per Twitch app, ever).
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct BackfillDone {
    /// `cache_key` of the credentials the list is for.
    pub key: u64,
    pub done: std::collections::HashSet<crate::model::GameId>,
}

/// The app side of the backfill: off in screenshot / test runs.
#[derive(Debug, Default)]
pub struct Backfill {
    pub enabled: bool,
    /// Loaded from the DB on first use.
    pub state: Option<BackfillDone>,
    /// Credentials the games were queued for this session.
    pub queued_for: Option<u64>,
}

pub const BACKFILL_KV: &str = "igdb_backfill";

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! fixture {
        ($name:literal) => {
            include_bytes!(concat!("../tests/fixtures/metadata/", $name)).as_slice()
        };
    }

    #[test]
    fn parses_games_with_expanded_fields() {
        let g = parse_games(fixture!("igdb_games_celeste.json")).unwrap();
        let c = &g[0];
        assert_eq!((c.id, c.name.as_str()), (26226, "Celeste"));
        assert_eq!(c.developers, vec!["Maddy Makes Games".to_string()]);
        assert_eq!(c.publishers, vec!["Maddy Makes Games".to_string()]);
        assert_eq!(c.genres, vec!["Platform", "Adventure", "Indie"]);
        assert_eq!(c.themes, vec!["Action"]);
        assert_eq!(c.modes, vec!["Single player", "Side view"]);
        assert_eq!(c.series.as_deref(), Some("Celeste"));
        assert_eq!(c.cover.as_ref().unwrap().url("cover_big_2x"), format!("{IGDB_IMAGES}/t_cover_big_2x/co3byy.jpg"));
        assert_eq!(c.artworks.len(), 1);
        assert_eq!(c.screenshots.len(), 2);
        let m = to_metadata(c);
        assert_eq!(m.release_year, Some(2018));
        assert_eq!(m.release_date.as_deref(), Some("2018-01-25"));
        assert_eq!(m.critic_score, Some(92));
        assert_eq!(m.community_score, Some(87));
        assert!(m.description.unwrap().starts_with("Help Madeline"));
        assert_eq!(confident("CELESTE™", &g).map(|g| g.id), Some(26226));
        assert_eq!(confident("Celeste 64", &g), None);
        assert_eq!(parse_games(b"{\"message\":\"x\"}"), None);
        assert_eq!(parse_games(b"[]"), Some(vec![]));
    }

    #[test]
    fn bodies_escape_names() {
        assert_eq!(
            search_body("Say \"Hi\" \\ now\n"),
            format!("search \"Say \\\"Hi\\\" \\\\ now\"; fields {FIELDS}; limit 10;")
        );
        assert!(by_ids_body(&[1, 22]).contains("where id = (1,22);"));
        assert_eq!(steam_body(570), "fields game; where external_game_source = 1 & uid = \"570\"; limit 5;");
        assert_eq!(parse_external(br#"[{"id":5,"game":1020},{"id":6}]"#), Some(vec![1020]));
    }

    #[test]
    fn token_parsing_and_cache() {
        let (t, life) = parse_token(fixture!("twitch_token.json")).unwrap();
        assert_eq!(t, "jostpf5q0uzmxmkba9iyug38kjtgh");
        assert_eq!(life, Duration::from_secs(5011271));
        assert_eq!(parse_token(br#"{"access_token":"a b"}"#), None);
        assert_eq!(parse_token(b"x"), None);
        // Fetched once, then cached; bad credentials are reported as such.
        forget_token("cid", "sec");
        let mut calls = 0;
        let mut ask = |status: u16| {
            calls += 1;
            let body = fixture!("twitch_token.json").to_vec();
            move |url: &str| {
                assert!(url.starts_with(TWITCH_TOKEN) && url.contains("grant_type=client_credentials"));
                Ok((status, body))
            }
        };
        assert_eq!(token("cid", "sec", ask(200)).unwrap(), t);
        assert_eq!(token("cid", "sec", |_| panic!("cached")).unwrap(), t);
        assert_eq!(token("cid", "other", ask(403)), Err(TokenErr::BadCredentials));
        assert_eq!(calls, 2);
        forget_token("cid", "sec");
        assert!(cached_token("cid", "sec").is_none());
    }

    #[test]
    fn dates() {
        assert_eq!(ymd(0), (1970, 1, 1));
        assert_eq!(ymd(1516838400), (2018, 1, 25));
        assert_eq!(ymd(951782400), (2000, 2, 29));
    }
}
