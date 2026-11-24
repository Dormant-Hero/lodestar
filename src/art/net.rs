//! HTTP for the artwork service: a mockable transport, a process-wide polite throttle shared by
//! every art thread, and cancellation.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const USER_AGENT: &str = concat!("Lodestar/", env!("CARGO_PKG_VERSION"));
pub const MAX_IMAGE_BYTES: u64 = 40 * 1024 * 1024;
pub const MAX_JSON_BYTES: u64 = 4 * 1024 * 1024;
/// Every request of a bulk download also waits for this gap (≤ 2 requests per second overall).
pub const BULK_GAP: Duration = Duration::from_millis(500);

/// Set for screenshot runs (`--screenshot`): only this machine is ever asked — `file://` and
/// loopback hosts (local mock servers) — so nothing a scripted run does goes online. Every HTTP
/// client in Lodestar checks it ([`check_allowed`]); tests never set it (it is process-wide).
static LOCAL_ONLY: AtomicBool = AtomicBool::new(false);

/// Keep this process off the internet (see [`LOCAL_ONLY`]); there is no way back.
pub fn set_local_only() {
    LOCAL_ONLY.store(true, Ordering::Relaxed);
}

/// This process may only reach this machine (screenshot runs).
pub fn local_only() -> bool {
    LOCAL_ONLY.load(Ordering::Relaxed)
}

/// `url` is a local file or on this machine (`localhost`, `127.0.0.1`, `::1`, `*.localhost`).
pub fn is_local(url: &str) -> bool {
    if url.starts_with("file://") {
        return true;
    }
    let rest = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://")).unwrap_or("");
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = if host.starts_with('[') {
        host.split_once(']').map_or(host, |(h, _)| h).trim_start_matches('[')
    } else {
        host.split(':').next().unwrap_or(host)
    };
    host == "localhost" || host == "127.0.0.1" || host == "::1" || host.ends_with(".localhost")
}

/// Err for an internet URL while the process is local-only (see [`LOCAL_ONLY`]).
pub fn check_allowed(url: &str) -> Result<(), String> {
    allowed_when(local_only(), url)
}

fn allowed_when(local_only: bool, url: &str) -> Result<(), String> {
    if !local_only || is_local(url) {
        return Ok(());
    }
    log::debug!("not going online in this run: {url}");
    Err("going online is off in this run".into())
}

/// A response with any status code (body only read for 200).
#[derive(Clone, Debug, Default)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
    /// The rate-limit headers (`Retry-After`, `X-RateLimit-*`), lower-case names.
    pub headers: Vec<(String, String)>,
}

/// The headers `art_limits` reads.
fn limit_headers(h: &ureq::http::HeaderMap) -> Vec<(String, String)> {
    h.iter()
        .filter(|(k, _)| {
            let k = k.as_str();
            k == "retry-after" || k.starts_with("x-ratelimit-")
        })
        .filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string())))
        .collect()
}

/// Transport (mocked in tests). Err = network-level failure.
pub trait Net: Send + Sync {
    fn get(&self, url: &str, bearer: Option<&str>, limit: u64) -> Result<HttpResponse, String>;
    /// Status of a HEAD request.
    fn head(&self, url: &str) -> Result<u16, String>;
    /// POST `body` with extra headers (IGDB, Twitch). Body only read for 200.
    fn post(&self, url: &str, headers: &[(&str, String)], body: &str, limit: u64) -> Result<HttpResponse, String> {
        let _ = (url, headers, body, limit);
        Err("POST is not supported".into())
    }
    /// GET with extra request headers (a service's own `User-Agent`, `Accept`). Transports that
    /// can't send headers (test fakes) answer like [`Net::get`].
    fn get_with(&self, url: &str, headers: &[(&str, String)], limit: u64) -> Result<HttpResponse, String> {
        let _ = headers;
        self.get(url, None, limit)
    }
}

fn read_body(resp: &mut ureq::http::Response<ureq::Body>, limit: u64) -> Result<Vec<u8>, String> {
    resp.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| e.to_string())
}

pub struct UreqNet {
    agent: ureq::Agent,
}

impl UreqNet {
    pub fn new() -> UreqNet {
        let cfg = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_global(Some(Duration::from_secs(25)))
            .user_agent(USER_AGENT)
            .http_status_as_error(false)
            .max_redirects(5)
            .build();
        UreqNet { agent: ureq::Agent::new_with_config(cfg) }
    }
}

impl Net for UreqNet {
    fn get(&self, url: &str, bearer: Option<&str>, limit: u64) -> Result<HttpResponse, String> {
        check_allowed(url)?;
        let mut req = self.agent.get(url);
        if let Some(k) = bearer {
            req = req.header("Authorization", format!("Bearer {k}"));
        }
        let mut resp = req.call().map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let headers = limit_headers(resp.headers());
        let body = if status == 200 { read_body(&mut resp, limit)? } else { Vec::new() };
        Ok(HttpResponse { status, body, headers })
    }

    fn get_with(&self, url: &str, headers: &[(&str, String)], limit: u64) -> Result<HttpResponse, String> {
        check_allowed(url)?;
        let mut req = self.agent.get(url);
        for (k, v) in headers {
            req = req.header(*k, v.as_str());
        }
        let mut resp = req.call().map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let headers = limit_headers(resp.headers());
        let body = if status == 200 { read_body(&mut resp, limit)? } else { Vec::new() };
        Ok(HttpResponse { status, body, headers })
    }

    fn head(&self, url: &str) -> Result<u16, String> {
        check_allowed(url)?;
        let resp = self.agent.head(url).call().map_err(|e| e.to_string())?;
        Ok(resp.status().as_u16())
    }

    fn post(&self, url: &str, headers: &[(&str, String)], body: &str, limit: u64) -> Result<HttpResponse, String> {
        check_allowed(url)?;
        let mut req = self.agent.post(url).header("Accept", "application/json");
        for (k, v) in headers {
            req = req.header(*k, v.as_str());
        }
        let mut resp = req.send(body).map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let headers = limit_headers(resp.headers());
        let body = if status == 200 { read_body(&mut resp, limit)? } else { Vec::new() };
        Ok(HttpResponse { status, body, headers })
    }
}

/// Host classes with their own minimum request spacing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Host {
    /// store.steampowered.com (storesearch, appdetails): ~200 requests / 5 min per IP.
    Store,
    /// api.steampowered.com (IStoreBrowseService).
    SteamApi,
    /// *.steamstatic.com image CDN.
    SteamCdn,
    /// www.steamgriddb.com API.
    Sgdb,
    /// SteamGridDB image CDN.
    SgdbCdn,
    /// lutris.net (API and images).
    Lutris,
    /// catalog.gog.com / api.gog.com.
    Gog,
    /// GOG's image CDN (*.gog-statics.com).
    GogCdn,
    /// www.pcgamingwiki.com (MediaWiki API).
    Pcgw,
    /// images. / thumbnails.pcgamingwiki.com.
    PcgwCdn,
    /// api.github.com (libretro-thumbnails name lists; 60 requests an hour without a token).
    GitHubApi,
    /// raw.githubusercontent.com (libretro-thumbnails images).
    GitHubRaw,
    /// api.thegamesdb.net.
    Tgdb,
    /// cdn.thegamesdb.net.
    TgdbCdn,
    /// api.igdb.com (4 requests a second, shared with the metadata fetcher).
    Igdb,
    /// images.igdb.com.
    IgdbCdn,
    /// id.twitch.tv (IGDB tokens).
    Twitch,
    /// art.gametdb.com (covers by game ID; one volunteer-run server).
    GameTdb,
    Other,
    /// Pseudo host: every bulk request.
    Bulk,
}

impl Host {
    pub fn of(url: &str) -> Host {
        let host = url.split("://").nth(1).unwrap_or("").split(['/', '?', ':']).next().unwrap_or("");
        let host = host.to_ascii_lowercase();
        if host == "store.steampowered.com" {
            Host::Store
        } else if host == "api.steampowered.com" {
            Host::SteamApi
        } else if host.ends_with(".steamstatic.com") || host == "steamcdn-a.akamaihd.net" {
            Host::SteamCdn
        } else if host == "www.steamgriddb.com" || host == "steamgriddb.com" {
            Host::Sgdb
        } else if host.ends_with(".steamgriddb.com") {
            Host::SgdbCdn
        } else if host == "lutris.net" || host.ends_with(".lutris.net") {
            Host::Lutris
        } else if host == "catalog.gog.com" || host == "api.gog.com" {
            Host::Gog
        } else if host.ends_with(".gog-statics.com") {
            Host::GogCdn
        } else if host == "www.pcgamingwiki.com" || host == "pcgamingwiki.com" {
            Host::Pcgw
        } else if host.ends_with(".pcgamingwiki.com") {
            Host::PcgwCdn
        } else if host == "api.github.com" {
            Host::GitHubApi
        } else if host == "raw.githubusercontent.com" {
            Host::GitHubRaw
        } else if host == "api.thegamesdb.net" {
            Host::Tgdb
        } else if host.ends_with(".thegamesdb.net") {
            Host::TgdbCdn
        } else if host == "api.igdb.com" {
            Host::Igdb
        } else if host == "images.igdb.com" {
            Host::IgdbCdn
        } else if host == "id.twitch.tv" {
            Host::Twitch
        } else if host == "art.gametdb.com" {
            Host::GameTdb
        } else {
            Host::Other
        }
    }

    fn gap(self) -> Duration {
        Duration::from_millis(match self {
            Host::Store => 1500,
            Host::SteamApi => 1000,
            Host::SteamCdn => 150,
            Host::Sgdb => 300,
            Host::SgdbCdn => 150,
            Host::Lutris => 500,
            Host::Gog => 400,
            Host::GogCdn => 150,
            Host::Pcgw => 1000,
            Host::PcgwCdn => 200,
            Host::GitHubApi => 2000,
            Host::GitHubRaw => 150,
            Host::Tgdb => 1000,
            Host::TgdbCdn => 150,
            // 4 / s allowed, shared with the metadata fetcher: `igdb::wait_turn` keeps to it.
            Host::Igdb => 0,
            Host::IgdbCdn => 100,
            Host::Twitch => 1000,
            Host::GameTdb => 300,
            Host::Other => 300,
            Host::Bulk => BULK_GAP.as_millis() as u64,
        })
    }
}

/// Reserves request slots per host; threads sleep outside the lock.
pub struct Throttle {
    next: Mutex<HashMap<Host, Instant>>,
    enabled: bool,
}

impl Throttle {
    pub fn polite() -> Throttle {
        Throttle { next: Mutex::new(HashMap::new()), enabled: true }
    }
    #[cfg(test)]
    pub fn none() -> Throttle {
        Throttle { next: Mutex::new(HashMap::new()), enabled: false }
    }

    /// Time until a request to `host` may be sent (and reserve that slot).
    fn reserve(&self, host: Host) -> Duration {
        if !self.enabled {
            return Duration::ZERO;
        }
        let now = Instant::now();
        let mut next = self.next.lock();
        let slot = next.get(&host).copied().filter(|t| *t > now).unwrap_or(now);
        next.insert(host, slot + host.gap());
        slot - now
    }

    /// Wait for a slot; false when cancelled while waiting.
    pub fn wait(&self, url: &str, bulk: bool, cancel: Option<&AtomicBool>) -> bool {
        let host = Host::of(url);
        // IGDB: one window for every thread of the process (artwork and metadata).
        if self.enabled && host == Host::Igdb && !crate::igdb::wait_turn(cancel) {
            return false;
        }
        let mut d = self.reserve(host);
        if bulk {
            d = d.max(self.reserve(Host::Bulk));
        }
        let end = Instant::now() + d;
        loop {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return false;
            }
            let now = Instant::now();
            if now >= end {
                return true;
            }
            std::thread::sleep((end - now).min(Duration::from_millis(100)));
        }
    }
}

/// Why a request produced nothing usable right now.
#[derive(Clone, Debug, PartialEq)]
pub enum NetErr {
    Cancelled,
    /// Network failure, 5xx, 408 or 429: try again later.
    Transient(String),
    /// An API limit is reached (or nearly, for bulk runs): why, and when it continues.
    Limit(String),
}

impl std::fmt::Display for NetErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetErr::Cancelled => write!(f, "cancelled"),
            NetErr::Transient(e) | NetErr::Limit(e) => write!(f, "{e}"),
        }
    }
}

/// 4xx (except 408/429) is a definitive "no".
pub fn is_definitive(status: u16) -> bool {
    (400..500).contains(&status) && status != 408 && status != 429
}

/// Percent-encode a URL path segment or query value (RFC 3986 unreserved stay as they are).
pub fn encode(s: &str) -> String {
    crate::metadata::parse::encode_path_segment(s)
}

#[cfg(test)]
mod local_only_tests {
    use super::*;

    /// Screenshot runs: internet hosts are refused, files and this machine's own addresses (local
    /// mock servers) still work; a normal run asks anything.
    #[test]
    fn local_only_refuses_internet_hosts() {
        for u in [
            "file:///tmp/x.json",
            "http://127.0.0.1:4000/a",
            "http://localhost/x",
            "http://[::1]:80/y",
            "http://app.localhost/z",
        ] {
            assert!(allowed_when(true, u).is_ok(), "{u}");
        }
        for u in [
            "https://store.steampowered.com/api/appdetails?appids=1",
            "https://en.wikipedia.org/w/api.php",
            "http://127.0.0.1.example.com/",
            "http://localhost@evil.example/",
            "romm.lan",
            "",
        ] {
            assert!(allowed_when(true, u).is_err(), "{u}");
            assert!(allowed_when(false, u).is_ok(), "{u}");
        }
    }
}
