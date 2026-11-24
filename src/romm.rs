//! RomM server library: the games on the user's own RomM server
//! (github.com/rommapp/romm) as not-installed games of source "RomM"; Download puts one into the
//! matching ROM folder, where the emulation scanner turns it into a normal emulated game.
//!
//! Pieces:
//!   * [`Server`]: address + sign-in (client API token `rmm_…` as `Authorization: Bearer`, or
//!     user name + password as HTTP Basic for older servers), a ureq agent with timeouts.
//!   * [`sync`]: `GET /api/platforms` + `GET /api/roms?limit&offset` (every page; older servers
//!     answer one plain array) → [`Library`], cached in `<data_dir>/romm/library.json`. The
//!     scanner (`scan/romm.rs`) only reads that file, so a scan never waits on the network.
//!   * [`fetch_covers`]: the server's own cover images (`path_cover_large`) into
//!     `<data_dir>/romm/covers/<id>-<crc>.<ext>`; the scanner gives them as the games' covers.
//!   * `romm/download.rs`: where a game goes and the download itself (progress, cancel).
//!   * `romm_app.rs`: the AppState side (worker threads, toasts, rescans).
//!
//! Every network call runs on a worker thread; nothing here touches the UI.

use crate::config::RommConfig;
use crate::emulation::platforms::{self, Platform};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub mod download;

#[cfg(test)]
mod tests;

/// RomM's page on client API tokens (Settings → Libraries → RomM server links to it).
pub const TOKEN_DOCS_URL: &str = "https://docs.romm.app/latest/developers/client-api-tokens/";
/// Games asked for per page.
pub const PAGE_SIZE: usize = 250;
/// Most pages read (a runaway server can't keep a sync going for ever): 250 000 games.
const MAX_PAGES: usize = 1000;
const MAX_LIST_BYTES: u64 = 64 << 20;
const MAX_COVER_BYTES: u64 = 20 << 20;

/// Why a call failed, in words for the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiError {
    /// No address set.
    NotSetUp,
    /// Couldn't connect (wrong address, server down, TLS).
    Unreachable(String),
    /// 401: the token / password was not accepted.
    BadAuth,
    /// 403: signed in, but the token lacks a permission.
    NoPermission,
    /// The address answers, but not like a RomM server.
    NotRomm,
    Http(u16),
    BadReply(String),
    Cancelled,
    /// Writing the download failed (disk full, no permission …).
    Disk(String),
    /// A complete sentence shown as it is.
    Plain(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::NotSetUp => write!(f, "Set the RomM server's address first (Settings → Libraries → RomM server)"),
            ApiError::Unreachable(e) => {
                write!(f, "Couldn't reach the RomM server ({e}). Is the address right and the server running?")
            }
            ApiError::BadAuth => write!(
                f,
                "The RomM server didn't accept the token. Make a new one in RomM (Administration → Client API Tokens) and paste it in Settings → Libraries → RomM server"
            ),
            ApiError::NoPermission => write!(
                f,
                "The token is missing a permission. Give it read access to ROMs and platforms in RomM (Administration → Client API Tokens)"
            ),
            ApiError::NotRomm => write!(f, "That address doesn't look like a RomM server"),
            ApiError::Http(s) => write!(f, "The RomM server answered with an error (HTTP {s})"),
            ApiError::BadReply(e) => write!(f, "The RomM server sent something Lodestar couldn't read ({e})"),
            ApiError::Cancelled => write!(f, "Cancelled"),
            ApiError::Disk(e) => write!(f, "Couldn't save the download: {e}"),
            ApiError::Plain(e) => write!(f, "{e}"),
        }
    }
}

fn status_error(status: u16) -> ApiError {
    match status {
        401 => ApiError::BadAuth,
        403 => ApiError::NoPermission,
        s => ApiError::Http(s),
    }
}

/// "romm.lan:8080/" → "http://romm.lan:8080"; a pasted page or API address keeps only the
/// server part ("https://romm.example.com/api/roms" → "https://romm.example.com").
pub fn normalize_url(raw: &str) -> Option<String> {
    let t = raw.trim().trim_end_matches('/');
    if t.is_empty() || t.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let with_scheme = if t.contains("://") { t.to_string() } else { format!("http://{t}") };
    let (scheme, rest) = with_scheme.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    // Keep a sub-path RomM may be served under ("https://host/romm"), drop UI / API pages.
    let mut rest = rest.trim_end_matches('/').to_string();
    for cut in ["/api/", "/api", "/rom/", "/platform/", "/login", "/#"] {
        if let Some(i) = rest.find(cut)
            && (cut != "/api" || rest[i..].len() == 4)
        {
            rest.truncate(i);
        }
    }
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() || rest.starts_with('/') {
        return None;
    }
    Some(format!("{scheme}://{rest}"))
}

/// Standard base64 (HTTP Basic sign-in).
fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for c in input.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// One RomM server as configured.
#[derive(Clone)]
pub struct Server {
    /// "https://romm.example.com" (no trailing slash).
    pub base: String,
    auth: Option<String>,
    insecure: bool,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never log the token.
        f.debug_struct("Server").field("base", &self.base).field("auth", &self.auth.is_some()).finish()
    }
}

impl Server {
    pub fn from_config(c: &RommConfig) -> Result<Server, ApiError> {
        let base = normalize_url(&c.url).ok_or(ApiError::NotSetUp)?;
        let token = c.token.as_deref().map(str::trim).filter(|t| !t.is_empty());
        let user = c.username.as_deref().map(str::trim).filter(|t| !t.is_empty());
        let auth = match (token, user) {
            (Some(t), _) => Some(format!("Bearer {}", t.strip_prefix("Bearer ").unwrap_or(t).trim())),
            (None, Some(u)) => {
                let pw = c.password.as_deref().unwrap_or("");
                Some(format!("Basic {}", base64(format!("{u}:{pw}").as_bytes())))
            }
            (None, None) => None,
        };
        Ok(Server { base, auth, insecure: c.accept_invalid_certs })
    }

    /// The server's address with sign-in, for a URL path ("/api/roms?…").
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// `long`: downloads (no overall time limit, only per read).
    fn agent(&self, long: bool) -> ureq::Agent {
        let mut b = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(8)))
            .timeout_per_read(Some(Duration::from_secs(if long { 120 } else { 30 })))
            .user_agent(crate::art::net::USER_AGENT)
            .http_status_as_error(false)
            .max_redirects(5);
        if !long {
            b = b.timeout_global(Some(Duration::from_secs(90)));
        }
        if self.insecure {
            b = b.tls_config(ureq::tls::TlsConfig::builder().disable_verification(true).build());
        }
        ureq::Agent::new_with_config(b.build())
    }

    /// `url` is on this server (not merely a host whose name starts the same:
    /// "http://romm.lan" must not match "http://romm.lan.example.com").
    fn on_server(&self, url: &str) -> bool {
        url.strip_prefix(&self.base).is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '?', '#']))
    }

    /// GET with sign-in; the response whatever its status (Err = network failure).
    pub(crate) fn get(&self, path: &str, long: bool) -> Result<ureq::http::Response<ureq::Body>, ApiError> {
        let url = self.url(path);
        crate::art::net::check_allowed(&url).map_err(ApiError::Unreachable)?;
        let mut req = self.agent(long).get(&url).header("Accept", "application/json");
        // Sign-in only goes to the server itself (cover URLs may point elsewhere).
        if let Some(a) = &self.auth
            && self.on_server(&url)
        {
            req = req.header("Authorization", a.as_str());
        }
        req.call().map_err(|e| ApiError::Unreachable(short_err(&e.to_string())))
    }

    /// GET JSON (status 200), else the matching error.
    pub(crate) fn get_json(&self, path: &str) -> Result<Value, ApiError> {
        let mut resp = self.get(path, false)?;
        let status = resp.status().as_u16();
        if status != 200 {
            return Err(status_error(status));
        }
        let bytes = resp
            .body_mut()
            .with_config()
            .limit(MAX_LIST_BYTES)
            .read_to_vec()
            .map_err(|e| ApiError::Unreachable(short_err(&e.to_string())))?;
        serde_json::from_slice(&bytes).map_err(|_| ApiError::NotRomm)
    }

    fn post_json(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
        let url = self.url(path);
        crate::art::net::check_allowed(&url).map_err(ApiError::Unreachable)?;
        let mut resp = self
            .agent(false)
            .post(&url)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .send(body.to_string())
            .map_err(|e| ApiError::Unreachable(short_err(&e.to_string())))?;
        let status = resp.status().as_u16();
        if status != 200 && status != 201 {
            return Err(status_error(status));
        }
        let bytes = resp
            .body_mut()
            .with_config()
            .limit(1 << 20)
            .read_to_vec()
            .map_err(|e| ApiError::Unreachable(short_err(&e.to_string())))?;
        serde_json::from_slice(&bytes).map_err(|_| ApiError::NotRomm)
    }
}

/// The first line of a ureq error, without the URL (it may be long).
fn short_err(e: &str) -> String {
    let e = e.lines().next().unwrap_or(e);
    e.chars().take(160).collect()
}

// ---------------------------------------------------------------------------------------------
// Test connection / pairing

/// What "Test connection" found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerInfo {
    pub version: Option<String>,
    pub platforms: usize,
    pub games: usize,
}

impl ServerInfo {
    pub fn summary(&self) -> String {
        let v = self.version.as_deref().map(|v| format!("RomM {v}")).unwrap_or_else(|| "RomM".into());
        format!(
            "Connected to {v}: {} on {} console{}",
            count(self.games, "game"),
            self.platforms,
            if self.platforms == 1 { "" } else { "s" }
        )
    }
}

pub(crate) fn count(n: usize, what: &str) -> String {
    if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") }
}

/// `GET /api/heartbeat` (no sign-in; tells a RomM server and its version), then
/// `GET /api/platforms` with sign-in (proves the token can read the library).
pub fn test_connection(server: &Server) -> Result<ServerInfo, ApiError> {
    let hb = match server.get_json("/api/heartbeat") {
        Ok(v) => v,
        Err(ApiError::Http(404)) => return Err(ApiError::NotRomm),
        // Some servers want sign-in for everything: the platforms call tells.
        Err(ApiError::BadAuth | ApiError::NoPermission) => Value::Null,
        Err(e) => return Err(e),
    };
    let version =
        hb.pointer("/SYSTEM/VERSION").or_else(|| hb.get("VERSION")).and_then(Value::as_str).map(str::to_string);
    if hb.is_object() && version.is_none() && hb.get("SYSTEM").is_none() && hb.get("FILESYSTEM").is_none() {
        return Err(ApiError::NotRomm);
    }
    let plats = server.get_json("/api/platforms")?;
    let list = plats.as_array().ok_or(ApiError::NotRomm)?;
    let games = list.iter().filter_map(|p| p.get("rom_count").and_then(Value::as_u64)).sum::<u64>() as usize;
    let platforms = list.iter().filter(|p| p.get("rom_count").and_then(Value::as_u64).unwrap_or(1) > 0).count();
    Ok(ServerInfo { version, platforms, games })
}

/// A pairing code from RomM (8 digits, made in RomM's token list) → the full token
/// (`POST /api/client-tokens/exchange`), so nobody has to type 68 characters on a TV.
pub fn exchange_code(server: &Server, code: &str) -> Result<String, ApiError> {
    let code: String = code.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    if code.is_empty() || code.len() > 32 {
        return Err(ApiError::BadReply("the pairing code looks wrong".into()));
    }
    let v = match server.post_json("/api/client-tokens/exchange", &serde_json::json!({ "code": code })) {
        Ok(v) => v,
        Err(ApiError::Http(400 | 404 | 410 | 422)) | Err(ApiError::BadAuth) => {
            return Err(ApiError::BadReply("the pairing code wasn't accepted — codes work once, for 5 minutes".into()));
        }
        Err(e) => return Err(e),
    };
    v.get("raw_token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| ApiError::BadReply("no token in the answer".into()))
}

// ---------------------------------------------------------------------------------------------
// The library

/// One game on the server, as cached (only what Lodestar uses).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RommGame {
    /// RomM's ROM id.
    pub id: u64,
    pub name: String,
    /// Lodestar platform id ("nintendo_snes"); None = a console Lodestar can't emulate.
    pub platform: Option<String>,
    /// RomM's name for the console ("Super Nintendo Entertainment System").
    pub platform_name: String,
    /// File (or folder) name on the server ("Christmas Craze.smc").
    pub fs_name: String,
    /// A folder of several files (multi-disc, cue + bins …).
    pub multi: bool,
    pub size: u64,
    pub summary: Option<String>,
    pub genres: Vec<String>,
    pub year: Option<i32>,
    pub developer: Option<String>,
    pub publisher: Option<String>,
    pub igdb_id: Option<u64>,
    /// Cover: a path on the server ("/assets/romm/resources/…") or a full URL.
    pub cover: Option<String>,
}

impl RommGame {
    pub fn platform_def(&self) -> Option<&'static Platform> {
        self.platform.as_deref().and_then(platforms::by_id)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    /// The server these games are from (normalised address).
    pub server: String,
    /// Unix seconds of the sync.
    pub synced_at: i64,
    pub games: Vec<RommGame>,
}

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("romm")
}

pub fn cache_file(data_dir: &Path) -> PathBuf {
    dir(data_dir).join("library.json")
}

pub fn covers_dir(data_dir: &Path) -> PathBuf {
    dir(data_dir).join("covers")
}

/// The cached library (None: never synced, or unreadable).
pub fn load_cache(data_dir: &Path) -> Option<Library> {
    let bytes = crate::scan::util::read_bytes(cache_file(data_dir)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save_cache(data_dir: &Path, lib: &Library) -> Result<(), ApiError> {
    let p = cache_file(data_dir);
    let json = serde_json::to_vec(lib).map_err(|e| ApiError::Disk(e.to_string()))?;
    crate::fsutil::write_atomic(&p, &json).map_err(|e| ApiError::Disk(e.to_string()))
}

/// Forget the cached library (the address changed or was cleared): the next scan drops the
/// server's games.
pub fn clear_cache(data_dir: &Path) {
    let _ = std::fs::remove_file(cache_file(data_dir));
}

/// A RomM platform → Lodestar's: by IGDB id, then RomM's folder / slug names, then its name.
pub fn map_platform(p: &Value) -> Option<&'static Platform> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).filter(|v| !v.is_empty());
    // PC games on a RomM server (Windows / Linux) aren't ROMs an emulator plays.
    let pc = |x: &Platform| matches!(x.id, "pc_windows" | "pc_linux");
    if let Some(igdb) = p.get("igdb_id").and_then(Value::as_u64)
        && let Some(found) = platforms::all().iter().find(|x| x.igdb == Some(igdb as u32))
    {
        return (!pc(found)).then_some(found);
    }
    let slug_alias = |slug: &str| -> Option<&'static Platform> {
        // RomM slugs that are not one of Lodestar's folder names.
        let id = match slug {
            "ngc" => "nintendo_gamecube",
            "sfam" => "nintendo_super_nes",
            "famicom" => "nintendo_nes",
            "turbografx16--1" | "turbografx-16-slash-pc-engine-cd" => "nec_turbografx_cd",
            "genesis-slash-megadrive" => "sega_genesis",
            _ => return None,
        };
        platforms::by_id(id)
    };
    ["fs_slug", "slug", "igdb_slug"]
        .iter()
        .filter_map(|k| s(k))
        .find_map(|v| slug_alias(v).or_else(|| platforms::by_folder_name(v)))
        .or_else(|| ["name", "display_name", "custom_name"].iter().filter_map(|k| s(k)).find_map(platforms::by_name))
        .filter(|x| !pc(x))
}

/// Unix milliseconds (RomM's `first_release_date`) → year.
fn year_of_ms(ms: i64) -> Option<i32> {
    let days = ms.div_euclid(86_400_000);
    // Civil-from-days (H. Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (1950..=2100).contains(&y).then_some(y as i32)
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().or_else(|| x.get("name").and_then(Value::as_str)))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .take(12)
                .collect()
        })
        .unwrap_or_default()
}

/// One `SimpleRomSchema` → [`RommGame`] (lenient: fields differ between RomM versions).
pub fn map_rom(r: &Value, platforms: &[(u64, Option<&'static Platform>, String)]) -> Option<RommGame> {
    let id = r.get("id")?.as_u64()?;
    let s = |k: &str| r.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    let fs_name = s("fs_name")?;
    if fs_name.contains('/') || fs_name == "." || fs_name == ".." || fs_name.contains('\0') {
        return None;
    }
    let name = s("name").or_else(|| s("fs_name_no_ext")).unwrap_or_else(|| fs_name.clone());
    let pid = r.get("platform_id").and_then(Value::as_u64);
    let known = pid.and_then(|p| platforms.iter().find(|(id, _, _)| *id == p));
    let plat = known.and_then(|(_, p, _)| *p).or_else(|| {
        // No platform list (older server): the ROM's own slugs.
        let pseudo = serde_json::json!({
            "fs_slug": r.get("platform_fs_slug"), "slug": r.get("platform_slug"),
            "name": r.get("platform_display_name").or_else(|| r.get("platform_name")),
        });
        map_platform(&pseudo)
    });
    let platform_name = s("platform_display_name")
        .or_else(|| s("platform_name"))
        .or_else(|| known.map(|(_, _, n)| n.clone()))
        .or_else(|| s("platform_slug"))
        .unwrap_or_default();
    let meta = r.get("metadatum");
    let genres = {
        let g = str_list(meta.and_then(|m| m.get("genres")));
        if g.is_empty() {
            let top = str_list(r.get("genres"));
            if top.is_empty() { str_list(r.pointer("/igdb_metadata/genres")) } else { top }
        } else {
            g
        }
    };
    let first = |k: &str| str_list(meta.and_then(|m| m.get(k))).into_iter().next();
    let year = meta
        .and_then(|m| m.get("first_release_date"))
        .and_then(Value::as_i64)
        .and_then(|t| year_of_ms(if t < 100_000_000_000 { t.saturating_mul(1000) } else { t }));
    let multi = r.get("has_multiple_files").and_then(Value::as_bool).unwrap_or(false)
        || r.get("has_nested_single_file").and_then(Value::as_bool).unwrap_or(false)
        || r.get("multi").and_then(Value::as_bool).unwrap_or(false);
    let cover = s("path_cover_large")
        .or_else(|| s("path_cover_l"))
        .or_else(|| s("path_cover_small"))
        .or_else(|| s("path_cover_s"))
        .filter(|p| p.starts_with('/'))
        .or_else(|| s("url_cover").filter(|u| u.starts_with("https://") || u.starts_with("http://")));
    Some(RommGame {
        id,
        name,
        platform: plat.map(|p| p.id.to_string()),
        platform_name,
        fs_name,
        multi,
        size: r.get("fs_size_bytes").or_else(|| r.get("file_size_bytes")).and_then(Value::as_u64).unwrap_or(0),
        summary: s("summary"),
        genres,
        year,
        developer: first("developers"),
        publisher: first("publishers"),
        igdb_id: r.get("igdb_id").and_then(Value::as_u64),
        cover,
    })
}

/// Did the user ask to stop?
fn stop(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|c| c.load(Ordering::Relaxed))
}

/// Read the whole library from the server (every page).
pub fn fetch_library(server: &Server, page_size: usize, cancel: Option<&AtomicBool>) -> Result<Library, ApiError> {
    let plats = match server.get_json("/api/platforms") {
        Ok(v) => v,
        Err(ApiError::Http(404)) => return Err(ApiError::NotRomm),
        Err(e) => return Err(e),
    };
    let plat_list: Vec<(u64, Option<&'static Platform>, String)> = plats
        .as_array()
        .ok_or(ApiError::NotRomm)?
        .iter()
        .filter_map(|p| {
            let id = p.get("id")?.as_u64()?;
            let name =
                p.get("display_name").or_else(|| p.get("name")).and_then(Value::as_str).unwrap_or_default().to_string();
            Some((id, map_platform(p), name))
        })
        .collect();
    let mut games: Vec<RommGame> = vec![];
    let mut seen = std::collections::HashSet::new();
    let page_size = page_size.max(1);
    let mut offset = 0usize;
    for _ in 0..MAX_PAGES {
        if stop(cancel) {
            return Err(ApiError::Cancelled);
        }
        let path = format!(
            "/api/roms?limit={page_size}&offset={offset}&with_char_index=false&with_filter_values=false&with_rom_id_index=false"
        );
        let v = server.get_json(&path)?;
        // RomM ≥ 3.5 pages (`items`, `total`); older ones answer one array with everything.
        let (items, total) = match &v {
            Value::Array(a) => (a.as_slice(), Some(a.len())),
            Value::Object(_) => (
                v.get("items").and_then(Value::as_array).map(Vec::as_slice).ok_or(ApiError::NotRomm)?,
                v.get("total").and_then(Value::as_u64).map(|t| t as usize),
            ),
            _ => return Err(ApiError::NotRomm),
        };
        let before = games.len();
        for r in items {
            if let Some(g) = map_rom(r, &plat_list)
                && seen.insert(g.id)
            {
                games.push(g);
            }
        }
        offset += items.len();
        // A server that ignores `offset` repeats its first page: nothing new ends the loop.
        let stuck = total.is_none() && games.len() == before;
        let done = v.is_array() || items.is_empty() || stuck || total.is_some_and(|t| offset >= t);
        if done {
            break;
        }
    }
    games.sort_by_key(|g| g.id);
    Ok(Library { server: server.base.clone(), synced_at: crate::model::now_unix(), games })
}

/// What a sync did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncSummary {
    pub games: usize,
    /// The cached list changed (a rescan shows it).
    pub changed: bool,
}

/// Fetch the library and cache it.
pub fn sync(server: &Server, data_dir: &Path, cancel: Option<&AtomicBool>) -> Result<(Library, SyncSummary), ApiError> {
    let lib = fetch_library(server, PAGE_SIZE, cancel)?;
    let old = load_cache(data_dir);
    let changed = old.as_ref().is_none_or(|o| o.server != lib.server || o.games != lib.games);
    // Written also when unchanged: synced_at says when the list was last checked.
    save_cache(data_dir, &lib)?;
    Ok((lib.clone(), SyncSummary { games: lib.games.len(), changed }))
}

/// Cover file for `g` (`<covers>/<id>-<crc of the cover's path>.<ext>`), if downloaded.
pub fn cover_file(covers: &Path, g: &RommGame) -> Option<PathBuf> {
    let src = g.cover.as_deref()?;
    let stem = format!("{}-{:08x}", g.id, crc32fast::hash(src.as_bytes()));
    ["png", "jpg", "webp", "gif", "bmp", "avif"]
        .iter()
        .map(|e| covers.join(format!("{stem}.{e}")))
        .find(|p| p.is_file())
}

/// The cover's URL: a server path ("/assets/…?ts=2026-07-05 12:22:06") on the server, encoded.
fn cover_url(server: &Server, cover: &str) -> String {
    let encoded: String = cover
        .chars()
        .map(|c| match c {
            ' ' => "%20".to_string(),
            '"' | '<' | '>' | '\\' | '^' | '`' | '{' | '|' | '}' => format!("%{:02X}", c as u32),
            c if c.is_ascii() => c.to_string(),
            c => {
                let mut b = [0u8; 4];
                c.encode_utf8(&mut b).bytes().map(|x| format!("%{x:02X}")).collect()
            }
        })
        .collect();
    if encoded.starts_with('/') { server.url(&encoded) } else { encoded }
}

/// Download the covers not downloaded yet (old versions of a changed cover are removed).
/// Returns how many are new.
pub fn fetch_covers(server: &Server, data_dir: &Path, lib: &Library, cancel: Option<&AtomicBool>) -> usize {
    let dir = covers_dir(data_dir);
    if std::fs::create_dir_all(&dir).is_err() {
        return 0;
    }
    let mut new = 0;
    let mut failures = 0;
    for g in &lib.games {
        if stop(cancel) || failures >= 20 {
            break;
        }
        let Some(src) = g.cover.as_deref() else { continue };
        if cover_file(&dir, g).is_some() {
            continue;
        }
        let url = cover_url(server, src);
        let bytes = (|| -> Option<Vec<u8>> {
            let mut resp = if server.on_server(&url) {
                server.get(&url[server.base.len()..], false).ok()?
            } else {
                crate::art::net::check_allowed(&url).ok()?;
                let agent = server.agent(false);
                agent.get(&url).call().ok()?
            };
            if resp.status().as_u16() != 200 {
                return None;
            }
            resp.body_mut().with_config().limit(MAX_COVER_BYTES).read_to_vec().ok()
        })();
        let Some(bytes) = bytes else {
            failures += 1;
            continue;
        };
        let Ok(ext) = crate::art::validate_image(&bytes) else {
            failures += 1;
            continue;
        };
        let ext = if ext == "jpeg" { "jpg" } else { ext };
        let stem = format!("{}-{:08x}", g.id, crc32fast::hash(src.as_bytes()));
        // Older covers of this game.
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let prefix = format!("{}-", g.id);
            for e in rd.flatten() {
                let n = e.file_name();
                let n = n.to_string_lossy();
                if n.starts_with(&prefix) && !n.starts_with(&stem) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        if crate::fsutil::write_atomic(&dir.join(format!("{stem}.{ext}")), &bytes).is_ok() {
            new += 1;
        }
    }
    new
}
