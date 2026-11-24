//! RetroAchievements' public Web API (`https://retroachievements.org/API/API_*.php`), the way
//! RetroAchievements asks third-party apps to use it: the user's own Web API key (`y=`), a
//! descriptive User-Agent, one request at a time about once a second, and backing off when the
//! server says "too many requests". Never the emulators' login API (`dorequest.php`).
//!
//! The key never reaches a log: every URL and error text is run through `config::redact`.

use crate::art::net::{HttpResponse, Net, encode};
use parking_lot::Mutex;
use std::time::{Duration, Instant};

pub const API_BASE: &str = "https://retroachievements.org/API/";
pub const MEDIA_BASE: &str = "https://media.retroachievements.org/";
pub const SITE: &str = "https://retroachievements.org";
/// `Lodestar/0.16.0 (Linux)`.
pub const USER_AGENT: &str = concat!("Lodestar/", env!("CARGO_PKG_VERSION"), " (Linux)");

/// Biggest answer read (a console's game list with hashes is a few MiB; arcade is the largest).
const MAX_LIST_BYTES: u64 = 48 * 1024 * 1024;
const MAX_JSON_BYTES: u64 = 8 * 1024 * 1024;
const MAX_BADGE_BYTES: u64 = 1024 * 1024;
/// Retries after a 429 / 5xx answer.
const RETRIES: u32 = 3;
/// The longest `Retry-After` honoured (longer: give up for now).
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);
/// After giving up on a busy / unavailable server, no request goes out for this long (every page
/// opened meanwhile shows its saved copy or the "isn't answering" line at once).
const COOL_OFF: Duration = Duration::from_secs(2 * 60);

/// Keeps requests of one kind apart: one at a time, at least `gap` between them. The lock is
/// held during the request, so every thread (the achievements worker, the account check) waits
/// its turn.
pub struct Pacer {
    last: Mutex<Option<Instant>>,
    gap: Duration,
    /// After the server kept saying "too many requests" / "unavailable": no requests until then.
    quiet_until: Mutex<Option<Instant>>,
}

impl Pacer {
    pub const fn new(gap: Duration) -> Pacer {
        Pacer { last: Mutex::new(None), gap, quiet_until: Mutex::new(None) }
    }

    /// The server asked us to stay away a while longer.
    fn resting(&self) -> bool {
        self.quiet_until.lock().is_some_and(|t| Instant::now() < t)
    }

    fn rest(&self) {
        *self.quiet_until.lock() = Some(Instant::now() + COOL_OFF);
    }

    fn run<T>(&self, sleep: &dyn Fn(Duration), f: impl FnOnce() -> T) -> T {
        let mut last = self.last.lock();
        if let Some(t) = *last {
            let since = t.elapsed();
            if since < self.gap {
                sleep(self.gap - since);
            }
        }
        let out = f();
        *last = Some(Instant::now());
        out
    }
}

/// The API: about one request a second for the whole process.
pub static API_PACER: Pacer = Pacer::new(Duration::from_millis(1000));
/// Badge pictures (a static file server): one at a time, a little apart.
pub static MEDIA_PACER: Pacer = Pacer::new(Duration::from_millis(100));

/// Why a call produced nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiErr {
    /// This run doesn't go online (screenshot runs).
    LocalOnly,
    /// No connection (DNS, refused, timeout).
    Offline,
    /// 401 / 403: the key is wrong or was reset.
    BadKey,
    /// 404 / 422 (an unknown user, game or a malformed name).
    NotFound,
    /// Still 429 / 5xx after backing off.
    Down(u16),
    /// Another status.
    Status(u16),
    /// Not the JSON expected.
    Garbled,
}

/// Talks to RetroAchievements for one user.
pub struct Client<'a> {
    pub net: &'a dyn Net,
    key: String,
    /// Values scrubbed from logs (`AchievementsConfig::secrets`).
    secrets: Vec<String>,
    pub local_only: bool,
    pub api: &'a Pacer,
    pub media: &'a Pacer,
    /// First back-off wait (doubled each retry).
    pub backoff: Duration,
    pub sleep: &'a (dyn Fn(Duration) + Sync),
}

fn real_sleep(d: Duration) {
    std::thread::sleep(d);
}

impl<'a> Client<'a> {
    /// The real thing: process-wide pacing, real sleeps, `art::net::local_only`.
    pub fn new(net: &'a dyn Net, key: &str, secrets: Vec<String>) -> Client<'a> {
        Client {
            net,
            key: key.trim().to_string(),
            secrets,
            local_only: crate::art::net::local_only(),
            api: &API_PACER,
            media: &MEDIA_PACER,
            backoff: Duration::from_secs(2),
            sleep: &real_sleep,
        }
    }

    /// No Web API key was given.
    pub fn is_keyless(&self) -> bool {
        self.key.is_empty()
    }

    /// `text` without the key (and the other secrets).
    pub fn redact(&self, text: &str) -> String {
        let mut s = self.secrets.clone();
        if !self.key.is_empty() {
            // The URL carries the key percent-encoded; scrub that form first.
            for k in [encode(&self.key), self.key.clone()] {
                if !s.contains(&k) {
                    s.push(k);
                }
            }
        }
        crate::config::redact(text, &s)
    }

    /// `API_<name>.php?<params>&y=<key>` → JSON.
    pub fn call(&self, name: &str, params: &[(&str, String)]) -> Result<serde_json::Value, ApiErr> {
        let mut url = format!("{API_BASE}API_{name}.php?");
        for (k, v) in params {
            url.push_str(&format!("{k}={}&", encode(v)));
        }
        url.push_str(&format!("y={}", encode(&self.key)));
        let limit = if name == "GetGameList" { MAX_LIST_BYTES } else { MAX_JSON_BYTES };
        let resp = self.get(self.api, &url, limit, true)?;
        serde_json::from_slice(&resp.body).map_err(|e| {
            log::warn!("RetroAchievements: {name}: not JSON ({e})");
            ApiErr::Garbled
        })
    }

    /// A picture from RetroAchievements' media server (None: not there / not reachable).
    pub fn media(&self, path: &str) -> Result<Vec<u8>, ApiErr> {
        let url = format!("{MEDIA_BASE}{path}");
        self.get(self.media, &url, MAX_BADGE_BYTES, false).map(|r| r.body)
    }

    fn get(&self, pacer: &Pacer, url: &str, limit: u64, json: bool) -> Result<HttpResponse, ApiErr> {
        if self.local_only && !crate::art::net::is_local(url) {
            return Err(ApiErr::LocalOnly);
        }
        let mut headers = vec![("User-Agent", USER_AGENT.to_string())];
        if json {
            headers.push(("Accept", "application/json".to_string()));
        }
        let shown = self.redact(url);
        if pacer.resting() {
            return Err(ApiErr::Down(0));
        }
        let mut attempt = 0;
        loop {
            log::debug!("RetroAchievements: GET {shown}");
            let r = pacer.run(self.sleep, || self.net.get_with(url, &headers, limit));
            let resp = match r {
                Ok(r) => r,
                Err(e) => {
                    log::info!("RetroAchievements: {shown}: {}", self.redact(&e));
                    return Err(ApiErr::Offline);
                }
            };
            match resp.status {
                200 => return Ok(resp),
                401 | 403 => return Err(ApiErr::BadKey),
                404 | 422 => return Err(ApiErr::NotFound),
                s @ (429 | 500 | 502 | 503 | 504) => {
                    if attempt >= RETRIES {
                        log::info!("RetroAchievements: {shown}: still HTTP {s}, giving up for now");
                        pacer.rest();
                        return Err(ApiErr::Down(s));
                    }
                    let wait = retry_after(&resp).unwrap_or(self.backoff * 2u32.pow(attempt));
                    if wait > MAX_RETRY_AFTER {
                        pacer.rest();
                        return Err(ApiErr::Down(s));
                    }
                    log::info!("RetroAchievements: HTTP {s}, waiting {} s", wait.as_secs_f32());
                    (self.sleep)(wait);
                    attempt += 1;
                }
                s => {
                    log::info!("RetroAchievements: {shown}: HTTP {s}");
                    return Err(ApiErr::Status(s));
                }
            }
        }
    }
}

/// `Retry-After: <seconds>`.
fn retry_after(r: &HttpResponse) -> Option<Duration> {
    let v = r.headers.iter().find(|(k, _)| k == "retry-after")?;
    v.1.trim().parse::<u64>().ok().map(Duration::from_secs)
}

// ---------------------------------------------------------------------------------------------
// Lenient JSON reading (V1 answers mix numbers and numeric strings)
// ---------------------------------------------------------------------------------------------

/// A number field (also `"12"`).
pub fn num(v: &serde_json::Value, key: &str) -> Option<u64> {
    match v.get(key)? {
        serde_json::Value::Number(n) => n.as_u64().or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64)),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// A text field (None when missing, null or empty).
pub fn text(v: &serde_json::Value, key: &str) -> Option<String> {
    match v.get(key)? {
        serde_json::Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// `"YYYY-MM-DD HH:MM:SS"` (UTC) → unix seconds.
pub fn date(v: &serde_json::Value, key: &str) -> Option<i64> {
    crate::scan::desktop::common::parse_datetime(&text(v, key)?, false)
}
