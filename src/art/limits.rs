//! The published limits of the artwork / metadata APIs, kept proactively (as scrapers such as
//! Skyscraper do) instead of only reacting to "too many requests":
//!
//! * GitHub (the libretro-thumbnails name lists, 60 an hour without a token):
//!   `X-RateLimit-Remaining` / `-Limit` / `-Reset`. A bulk run waits for the reset (saying so in
//!   the status bar), a search says when it continues.
//! * TheGamesDB: `remaining_monthly_allowance` + `extra_allowance` (+ `allowance_refresh_timer`)
//!   in every reply; bulk runs stop before they are used up.
//! * Any host answering 429 with `Retry-After` (SteamGridDB, also its `X-RateLimit-*`) is left
//!   alone until then.
//! * IGDB's 4 requests a second: `crate::igdb::wait_turn` (one window for artwork + metadata).
//!
//! The counters are saved (`<data>/api-limits.json`, per day / hour / month as the API counts)
//! so a restart doesn't forget a nearly used-up quota. Everything time-related takes `now`
//! (unix seconds) so tests can move the clock.

use super::net::Host;
use super::sources::SourceKind;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Longest a bulk run waits for a limit to reset (GitHub resets within the hour).
pub const MAX_BULK_WAIT: i64 = 65 * 60;
/// Without `Retry-After`, a 429 from SteamGridDB pauses it this long.
const DEFAULT_PAUSE: i64 = 60;
/// Longest pause a `Retry-After` can ask for.
const MAX_PAUSE: i64 = 6 * 3600;
/// Saved at most this often while counting (and at the end of every run).
const SAVE_EVERY: Duration = Duration::from_secs(5);

pub fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TgdbQuota {
    pub remaining: u32,
    pub extra: u32,
    /// When the monthly allowance starts over (unix seconds; 0 = unknown).
    pub refresh_at: i64,
    /// When the numbers were seen.
    pub seen_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GhQuota {
    pub remaining: u32,
    pub limit: u32,
    /// When the hourly window starts over (unix seconds).
    pub reset_at: i64,
}

/// What is saved between sessions.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub thegamesdb: Option<TgdbQuota>,
    pub github: Option<GhQuota>,
    /// Hosts that asked to be left alone (429 + Retry-After): host name → until (unix seconds).
    pub paused: BTreeMap<String, i64>,
}

/// May a request be sent now?
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Go,
    /// Not before this many seconds (a bulk run waits; `why` is shown meanwhile).
    Wait(i64, String),
    /// Not this run / not now: why, and when it continues.
    Stop(String),
}

/// "9 412" (thousands grouped, the way the quota reads best).
pub fn grouped(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// "14:05" (local time).
pub fn clock(ts: i64) -> String {
    let (_, _, _, h, m, _) = crate::theme::render::bind::local_time(ts);
    format!("{h:02}:{m:02}")
}

/// "14:05", or "tomorrow at 00:00" / "on 1 Dec" when it isn't today.
pub fn when(ts: i64, now: i64) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let (y, mo, d, ..) = crate::theme::render::bind::local_time(ts);
    let today = crate::theme::render::bind::local_time(now);
    let tomorrow = crate::theme::render::bind::local_time(now + 86400);
    if (y, mo, d) == (today.0, today.1, today.2) {
        format!("at {}", clock(ts))
    } else if (y, mo, d) == (tomorrow.0, tomorrow.1, tomorrow.2) {
        format!("tomorrow at {}", clock(ts))
    } else {
        format!("on {d} {}", MONTHS[(mo as usize).clamp(1, 12) - 1])
    }
}

fn num(v: Option<&Value>) -> Option<u32> {
    match v? {
        Value::Number(n) => n.as_u64().map(|n| n.min(u32::MAX as u64) as u32),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// TheGamesDB's allowance in a reply: (monthly remaining, extra, seconds to the refresh).
pub fn parse_tgdb_allowance(body: &[u8]) -> Option<(u32, u32, Option<i64>)> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let rem = num(v.get("remaining_monthly_allowance"))?;
    let extra = num(v.get("extra_allowance")).unwrap_or(0);
    let refresh = num(v.get("allowance_refresh_timer")).map(i64::from);
    Some((rem, extra, refresh))
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim())
}

/// `Retry-After` in seconds (only the seconds form; an HTTP date counts as unknown).
pub fn retry_after(headers: &[(String, String)]) -> Option<i64> {
    header(headers, "retry-after")?.parse::<i64>().ok().filter(|s| *s >= 0)
}

/// The source a limited host belongs to (for messages).
fn source_of(h: Host) -> &'static str {
    match h {
        Host::Tgdb | Host::TgdbCdn => SourceKind::TheGamesDb.name(),
        Host::GitHubApi | Host::GitHubRaw => "libretro (GitHub)",
        Host::Sgdb | Host::SgdbCdn => SourceKind::SteamGridDb.name(),
        Host::Igdb | Host::IgdbCdn | Host::Twitch => SourceKind::Igdb.name(),
        Host::Store | Host::SteamApi | Host::SteamCdn => SourceKind::Steam.name(),
        Host::Gog | Host::GogCdn => SourceKind::Gog.name(),
        Host::Lutris => SourceKind::Lutris.name(),
        Host::Pcgw | Host::PcgwCdn => SourceKind::PcGamingWiki.name(),
        Host::GameTdb => SourceKind::GameTdb.name(),
        Host::Other | Host::Bulk => "The server",
    }
}

fn host_key(h: Host) -> String {
    format!("{h:?}")
}

struct State {
    saved: Saved,
    dirty: bool,
    last_save: Option<Instant>,
    /// Why a bulk run is waiting right now (status bar).
    waiting: Option<String>,
    /// Every reason a bulk run waited for, in order (tests: a wait may be too short to catch).
    #[cfg(test)]
    waited: Vec<String>,
}

/// The limits of every API, shared by the art threads of one `Engine`.
pub struct Limits {
    st: Mutex<State>,
    /// `<data>/api-limits.json`; None = not saved (tests).
    path: Option<PathBuf>,
}

impl Limits {
    pub fn new(path: Option<PathBuf>) -> Limits {
        let saved = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Saved>(&b).ok())
            .unwrap_or_default();
        Limits::with_saved(saved, path)
    }

    pub fn with_saved(saved: Saved, path: Option<PathBuf>) -> Limits {
        Limits {
            st: Mutex::new(State {
                saved,
                dirty: false,
                last_save: None,
                waiting: None,
                #[cfg(test)]
                waited: vec![],
            }),
            path,
        }
    }

    #[cfg(test)]
    pub fn saved(&self) -> Saved {
        self.st.lock().saved.clone()
    }

    /// May a request to `host` be sent now? `bulk`: a bulk run / the automatic fill (stops at a
    /// safety margin; may wait), else a search the user asked for (only stops at 0).
    pub fn check(&self, host: Host, bulk: bool, now: i64) -> Verdict {
        let st = self.st.lock();
        let s = &st.saved;
        if let Some(&until) = s.paused.get(&host_key(host))
            && until > now
        {
            let why = format!("{} asked Lodestar to wait — continues {}", source_of(host), when(until, now));
            return if bulk && until - now <= MAX_BULK_WAIT {
                Verdict::Wait(until - now, why)
            } else {
                Verdict::Stop(why)
            };
        }
        match host {
            Host::Tgdb => {
                let Some(q) = s.thegamesdb.as_ref().filter(|q| q.refresh_at == 0 || q.refresh_at > now) else {
                    return Verdict::Go;
                };
                let left = q.remaining.saturating_add(q.extra);
                let keep = if bulk { 20 } else { 0 };
                if left <= keep {
                    let resume = if q.refresh_at > 0 { when(q.refresh_at, now) } else { "next month".into() };
                    return Verdict::Stop(format!(
                        "TheGamesDB's monthly allowance is {} ({} left) — continues {resume}",
                        if left == 0 { "used up" } else { "almost used up" },
                        grouped(left)
                    ));
                }
                Verdict::Go
            }
            Host::GitHubApi => {
                let Some(q) = s.github.as_ref().filter(|q| q.reset_at > now) else {
                    return Verdict::Go;
                };
                let keep = if bulk { 3 } else { 0 };
                if q.remaining <= keep {
                    let why = format!(
                        "GitHub's hourly limit for libretro lookups is {} — continues {}",
                        if q.remaining == 0 { "used up" } else { "almost used up" },
                        when(q.reset_at, now)
                    );
                    // A moment after the reset (clocks differ).
                    let wait = q.reset_at - now + 2;
                    return if bulk && wait <= MAX_BULK_WAIT { Verdict::Wait(wait, why) } else { Verdict::Stop(why) };
                }
                Verdict::Go
            }
            _ => Verdict::Go,
        }
    }

    /// A reply from `host`: read what it says about the limits.
    pub fn observe(&self, host: Host, status: u16, headers: &[(String, String)], body: &[u8], now: i64) {
        let mut st = self.st.lock();
        let before = st.saved.clone();
        let s = &mut st.saved;
        // 429 + Retry-After (any host); SteamGridDB also without one.
        if status == 429 || status == 503 {
            let secs = retry_after(headers).or((status == 429 && matches!(host, Host::Sgdb)).then_some(DEFAULT_PAUSE));
            if let Some(secs) = secs {
                s.paused.insert(host_key(host), now + secs.min(MAX_PAUSE));
            }
        }
        if matches!(host, Host::Sgdb)
            && header(headers, "x-ratelimit-remaining").and_then(|v| v.parse::<u32>().ok()) == Some(0)
            && let Some(reset) = header(headers, "x-ratelimit-reset").and_then(|v| v.parse::<i64>().ok())
        {
            // Either a unix time or seconds from now.
            let until = if reset > 1_000_000_000 { reset } else { now + reset };
            if until > now {
                s.paused.insert(host_key(host), until.min(now + MAX_PAUSE));
            }
        }
        s.paused.retain(|_, until| *until > now);
        match host {
            Host::Tgdb => {
                if let Some((remaining, extra, refresh)) = parse_tgdb_allowance(body) {
                    let old_refresh = s.thegamesdb.as_ref().map_or(0, |q| q.refresh_at);
                    s.thegamesdb = Some(TgdbQuota {
                        remaining,
                        extra,
                        refresh_at: refresh.map_or(old_refresh, |r| now + r),
                        seen_at: now,
                    });
                }
            }
            Host::GitHubApi => {
                let remaining = header(headers, "x-ratelimit-remaining").and_then(|v| v.parse::<u32>().ok());
                let reset = header(headers, "x-ratelimit-reset").and_then(|v| v.parse::<i64>().ok());
                let limit = header(headers, "x-ratelimit-limit").and_then(|v| v.parse::<u32>().ok());
                match (remaining, reset) {
                    (Some(remaining), Some(reset_at)) => {
                        s.github = Some(GhQuota { remaining, limit: limit.unwrap_or(60), reset_at });
                    }
                    // A rate-limit answer without the headers: assume the hour is used up.
                    _ if matches!(status, 403 | 429) => {
                        let old = s.github.clone().filter(|q| q.reset_at > now);
                        s.github = Some(GhQuota {
                            remaining: 0,
                            limit: old.as_ref().map_or(60, |q| q.limit),
                            reset_at: old.map_or(now + 3600, |q| q.reset_at),
                        });
                    }
                    _ => {
                        if let Some(q) = s.github.as_mut().filter(|q| q.reset_at > now) {
                            q.remaining = q.remaining.saturating_sub(1);
                        }
                    }
                }
            }
            _ => {}
        }
        if st.saved != before {
            st.dirty = true;
        }
        let due = st.last_save.is_none_or(|t| t.elapsed() >= SAVE_EVERY);
        if st.dirty && due {
            self.save_locked(&mut st);
        }
    }

    /// Save now if anything changed (end of a run).
    pub fn flush(&self) {
        let mut st = self.st.lock();
        if st.dirty {
            self.save_locked(&mut st);
        }
    }

    fn save_locked(&self, st: &mut State) {
        st.dirty = false;
        st.last_save = Some(Instant::now());
        let Some(p) = &self.path else { return };
        match serde_json::to_vec_pretty(&st.saved) {
            Ok(b) => {
                if let Err(e) = super::write_atomic(p, &b) {
                    log::debug!("api limits: {}: {e}", p.display());
                }
            }
            Err(e) => log::debug!("api limits: {e}"),
        }
    }

    pub fn set_waiting(&self, why: Option<String>) {
        let mut st = self.st.lock();
        #[cfg(test)]
        st.waited.extend(why.clone());
        st.waiting = why;
    }

    /// Every reason given to [`Limits::set_waiting`] so far.
    #[cfg(test)]
    pub fn waited(&self) -> Vec<String> {
        self.st.lock().waited.clone()
    }

    /// Why a bulk run is waiting right now.
    pub fn waiting(&self) -> Option<String> {
        self.st.lock().waiting.clone()
    }

    /// What is left per source, for Settings → Metadata
    /// ("TheGamesDB: 2 912 left this month").
    pub fn summary(&self, now: i64) -> Vec<(SourceKind, String)> {
        let st = self.st.lock();
        let s = &st.saved;
        let mut out = vec![];
        if let Some(q) = &s.github {
            let limit = q.limit.max(1);
            let t = if q.reset_at > now {
                let mut t = format!("{} of {limit} lookups left this hour", q.remaining.min(limit));
                if q.remaining == 0 {
                    t.push_str(&format!(" (continues {})", when(q.reset_at, now)));
                }
                t
            } else {
                format!("{limit} of {limit} lookups left this hour")
            };
            out.push((SourceKind::Libretro, t));
        }
        if let Some(q) = &s.thegamesdb {
            if q.refresh_at == 0 || q.refresh_at > now {
                let mut t = format!("{} left this month", grouped(q.remaining));
                if q.extra > 0 {
                    t.push_str(&format!(" (+ {} extra)", grouped(q.extra)));
                }
                out.push((SourceKind::TheGamesDb, t));
            } else {
                out.push((SourceKind::TheGamesDb, "the monthly allowance has started over".into()));
            }
        }
        for (k, until) in &s.paused {
            if *until <= now {
                continue;
            }
            let kind = match k.as_str() {
                "Sgdb" | "SgdbCdn" => SourceKind::SteamGridDb,
                "Igdb" | "IgdbCdn" | "Twitch" => SourceKind::Igdb,
                "Tgdb" | "TgdbCdn" => SourceKind::TheGamesDb,
                "GitHubApi" | "GitHubRaw" => SourceKind::Libretro,
                "Gog" | "GogCdn" => SourceKind::Gog,
                "Lutris" => SourceKind::Lutris,
                "Pcgw" | "PcgwCdn" => SourceKind::PcGamingWiki,
                "GameTdb" => SourceKind::GameTdb,
                "Store" | "SteamApi" | "SteamCdn" => SourceKind::Steam,
                // Hosts no source owns, or no longer exist (an old saved file).
                _ => continue,
            };
            out.push((kind, format!("asked to wait — continues {}", when(*until, now))));
        }
        out
    }
}

/// Sleep `d`, waking early when cancelled. False when cancelled.
pub fn sleep_cancellable(d: Duration, cancel: Option<&AtomicBool>) -> bool {
    let end = Instant::now() + d;
    loop {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return false;
        }
        let now = Instant::now();
        if now >= end {
            return true;
        }
        std::thread::sleep((end - now).min(Duration::from_millis(200)));
    }
}
