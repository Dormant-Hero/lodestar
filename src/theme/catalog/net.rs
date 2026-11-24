//! HTTP for the theme catalogue: small GETs (index files, manifests, screenshots) and streamed
//! downloads of theme archives, with size limits, timeouts and a generic User-Agent. `file://`
//! URLs read local files (fixtures for tests and offline screenshot runs).

use std::io::{Read, Write};
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

pub const MAX_INDEX_BYTES: u64 = 16 << 20;
pub const MAX_SHOT_BYTES: u64 = 12 << 20;
/// Theme archives (themes with videos and pictures can be a few hundred MB).
pub const MAX_ARCHIVE_BYTES: u64 = 1 << 30;

/// Screenshot / test runs of the app: only `file://` and this machine's own addresses (local
/// mock servers) may be read, so nothing a click does there goes online (the process-wide
/// switch every HTTP client of Lodestar checks).
pub fn set_local_only() {
    crate::art::net::set_local_only();
}

/// Refuse internet URLs while the process is local-only (loopback hosts stay allowed).
fn check_allowed(url: &str) -> Result<(), String> {
    crate::art::net::check_allowed(url)
}

#[cfg(test)]
fn is_local(url: &str) -> bool {
    crate::art::net::is_local(url)
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let cfg = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_per_read(Some(Duration::from_secs(60)))
            .user_agent(crate::art::net::USER_AGENT)
            .http_status_as_error(false)
            .max_redirects(6)
            .build();
        ureq::Agent::new_with_config(cfg)
    })
}

fn file_path(url: &str) -> Option<&Path> {
    url.strip_prefix("file://").map(Path::new)
}

/// The body of a 200 response (≤ `limit` bytes).
pub fn get(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    if let Some(p) = file_path(url) {
        let m = std::fs::metadata(p).map_err(|e| format!("{}: {e}", p.display()))?;
        if m.len() > limit {
            return Err("the file is too big".into());
        }
        return std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()));
    }
    check_allowed(url)?;
    let mut resp = agent().get(url).call().map_err(friendly)?;
    let status = resp.status().as_u16();
    if status != 200 {
        return Err(status_text(status));
    }
    resp.body_mut().with_config().limit(limit).read_to_vec().map_err(friendly)
}

/// Stream a download into `dest` (written as `<dest>.part`, renamed when complete).
/// `progress` counts the bytes received; `cancel` stops it.
pub fn download(url: &str, dest: &Path, limit: u64, progress: &AtomicU64, cancel: &AtomicBool) -> Result<u64, String> {
    let part = dest.with_extension("part");
    let result = (|| -> Result<u64, String> {
        let mut reader: Box<dyn Read> = if let Some(p) = file_path(url) {
            Box::new(std::fs::File::open(p).map_err(|e| format!("{}: {e}", p.display()))?)
        } else {
            check_allowed(url)?;
            let resp = agent().get(url).call().map_err(friendly)?;
            let status = resp.status().as_u16();
            if status != 200 {
                return Err(status_text(status));
            }
            Box::new(resp.into_body().into_with_config().limit(limit + 1).reader())
        };
        let mut out = std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
        let mut buf = vec![0u8; 256 * 1024];
        let mut total = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            let n = reader.read(&mut buf).map_err(|e| friendly_io(&e))?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > limit {
                return Err(format!("the download is bigger than {} MB", limit >> 20));
            }
            out.write_all(&buf[..n]).map_err(|e| format!("writing the download: {e}"))?;
            progress.store(total, Ordering::Relaxed);
        }
        out.flush().map_err(|e| format!("writing the download: {e}"))?;
        drop(out);
        std::fs::rename(&part, dest).map_err(|e| format!("{e}"))?;
        Ok(total)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

fn status_text(status: u16) -> String {
    match status {
        404 => "not found on the server (404)".into(),
        403 | 429 => format!("the server refused for now ({status}) — try again later"),
        s => format!("the server answered {s}"),
    }
}

fn friendly(e: ureq::Error) -> String {
    match e {
        ureq::Error::Io(io) => friendly_io(&io),
        ureq::Error::BodyExceedsLimit(_) => "the download is too big".into(),
        ureq::Error::HostNotFound => "no internet connection (the server's name couldn't be looked up)".into(),
        ureq::Error::ConnectionFailed => "couldn't connect to the server".into(),
        ureq::Error::Timeout(_) => "the server took too long to answer".into(),
        other => other.to_string(),
    }
}

fn friendly_io(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::TimedOut => "the server took too long to answer".into(),
        std::io::ErrorKind::ConnectionRefused => "couldn't connect to the server".into(),
        _ => {
            let s = e.to_string();
            if s.contains("exceeds") || s.contains("limit") { "the download is too big".into() } else { s }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_local;

    #[test]
    fn local_only_allows_files_and_loopback_hosts() {
        for u in [
            "file:///tmp/x.json",
            "http://127.0.0.1:4000/a",
            "http://localhost/x",
            "http://[::1]:80/y",
            "http://app.localhost/z",
        ] {
            assert!(is_local(u), "{u}");
        }
        for u in [
            "https://example.com/themes/list.json",
            "https://github.com/x/y.git/info/refs?service=git-upload-pack",
            "http://127.0.0.1.example.com/",
            "http://localhost@evil.example/",
            "ftp://localhost/x",
        ] {
            assert!(!is_local(u), "{u}");
        }
    }
}
