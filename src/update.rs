//! Lodestar's own updates: asking GitHub for the newest release (once a day, on a background
//! thread) and, for the AppImage, replacing its file with a checked download.
//!
//! * The check reads one public, read-only URL ([`LATEST_API`]) with Lodestar's user agent.
//!   Offline, rate limits or no release yet are quiet: nothing is shown, the next check is a
//!   day later.
//! * Only an AppImage updates itself ([`Install::detect`]): `APPIMAGE` names the file that is
//!   running and its folder is writable. Everything else (install.sh, a build from source, a
//!   future Flatpak) only hears that a new version exists, with the release page.
//! * The download comes only from this repository's release assets on github.com and the
//!   hosts GitHub redirects those to ([`valid_asset_url`], [`valid_redirect`]); each redirect
//!   hop is checked before it is followed. It is streamed into a hidden file in the AppImage's
//!   own folder, checked (the size GitHub gives, its SHA-256 when GitHub gives one, the
//!   AppImage's ELF / `AI 2` magic), made executable and renamed over the AppImage. The old file
//!   stays as `<name>.old` until the next start ([`clean_leftovers`]). Nothing downloaded is ever
//!   run except the new AppImage itself, when the user asks for a restart ([`exec_restart`]).
//!
//! CONTRACT: pure parts (`Version`, `parse_release`, `pick_asset`, URL checks, `check_due`)
//! never touch the network; `fetch_latest` and `download_and_replace` take a [`Net`] so tests
//! use a fake one.

use crate::art::net::{self, Net};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub const OWNER: &str = "Dormant-Hero";
pub const REPO: &str = "lodestar";
/// The newest release (GitHub skips drafts and pre-releases here).
pub const LATEST_API: &str = "https://api.github.com/repos/Dormant-Hero/lodestar/releases/latest";
/// Shown when a release's own page isn't known.
pub const RELEASES_PAGE: &str = "https://github.com/Dormant-Hero/lodestar/releases/latest";
/// Automatic checks: at most one a day.
pub const CHECK_EVERY_SECS: i64 = 24 * 3600;
/// No Lodestar AppImage comes near this; a bigger "size" is not believed.
const MAX_APPIMAGE_BYTES: u64 = 512 * 1024 * 1024;
/// Release notes longer than this are cut (they are shown in a small dialog).
const MAX_NOTES_CHARS: usize = 20_000;
/// The hosts GitHub redirects release downloads to.
const ASSET_HOSTS: [&str; 2] = ["objects.githubusercontent.com", "release-assets.githubusercontent.com"];
const MAX_HOPS: usize = 5;

/// The version this binary is.
pub fn current() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("Cargo version")
}

// ------------------------------------------------------------------------------------ versions

/// A semantic version: `major.minor.patch`, optionally `-pre` (sorts before the release).
/// A leading `v` and a `+build` suffix are ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    nums: [u64; 3],
    pre: Option<String>,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
        let s = s.split('+').next().unwrap_or(s);
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) if !p.is_empty() => (c, Some(p.to_string())),
            Some(_) => return None,
            None => (s, None),
        };
        let parts: Vec<&str> = core.split('.').collect();
        if parts.is_empty() || parts.len() > 3 {
            return None;
        }
        let mut nums = [0u64; 3];
        for (i, p) in parts.iter().enumerate() {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            nums[i] = p.parse().ok()?;
        }
        Some(Version { nums, pre })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [a, b, c] = self.nums;
        write!(f, "{a}.{b}.{c}")?;
        if let Some(p) = &self.pre {
            write!(f, "-{p}")?;
        }
        Ok(())
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        self.nums.cmp(&other.nums).then_with(|| match (&self.pre, &other.pre) {
            (None, None) => Equal,
            (None, Some(_)) => Greater,
            (Some(_), None) => Less,
            (Some(a), Some(b)) => cmp_pre(a, b),
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Semver pre-release order: dot-separated parts, numbers by value and before words.
fn cmp_pre(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut x, mut y) = (a.split('.'), b.split('.'));
    loop {
        match (x.next(), y.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(p), Some(q)) => {
                let o = match (p.parse::<u64>(), q.parse::<u64>()) {
                    (Ok(m), Ok(n)) => m.cmp(&n),
                    (Ok(_), Err(_)) => std::cmp::Ordering::Less,
                    (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
                    (Err(_), Err(_)) => p.cmp(q),
                };
                if o.is_ne() {
                    return o;
                }
            }
        }
    }
}

// ------------------------------------------------------------------------------------ releases

/// The newest release, as far as Lodestar needs it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Release {
    /// "0.17.1" (the tag without its `v`).
    pub version: String,
    pub tag: String,
    /// The release's page on GitHub.
    pub page: String,
    /// Its notes (Markdown), possibly cut.
    pub notes: String,
    /// The AppImage for this machine, when the release has one Lodestar may download.
    pub asset: Option<Asset>,
}

impl Release {
    pub fn parsed_version(&self) -> Option<Version> {
        Version::parse(&self.version)
    }

    /// Newer than the running Lodestar.
    pub fn is_newer_than(&self, v: &Version) -> bool {
        self.parsed_version().is_some_and(|r| r > *v)
    }
}

/// A release file Lodestar can download.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lower-case hex, from GitHub's `digest` ("sha256:…") when it has one.
    pub sha256: Option<String>,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

/// The AppImage file name a release `version` has for this machine.
pub fn asset_name(version: &str) -> String {
    format!("Lodestar-{version}-{}.AppImage", std::env::consts::ARCH)
}

/// `releases/latest`'s JSON.
pub fn parse_release(json: &[u8]) -> Result<Release, String> {
    let r: GhRelease = serde_json::from_slice(json).map_err(|e| format!("unexpected answer: {e}"))?;
    if r.draft || r.prerelease {
        return Err("not a published release".into());
    }
    let v = Version::parse(&r.tag_name).ok_or_else(|| format!("unexpected version {:?}", r.tag_name))?;
    let version = r.tag_name.trim().trim_start_matches(['v', 'V']).to_string();
    let page = r.html_url.filter(|u| valid_release_page(u)).unwrap_or_else(|| RELEASES_PAGE.to_string());
    let mut notes = r.body.unwrap_or_default().replace("\r\n", "\n").trim().to_string();
    if notes.chars().count() > MAX_NOTES_CHARS {
        notes = notes.chars().take(MAX_NOTES_CHARS).collect::<String>() + " …";
    }
    let asset = pick_asset(&r.assets, &version);
    log::debug!("update: newest release {v} ({} assets, AppImage {})", r.assets.len(), asset.is_some());
    Ok(Release { version, tag: r.tag_name, page, notes, asset })
}

/// The release's AppImage for this machine: exactly `Lodestar-<version>-<arch>.AppImage`, fully
/// uploaded, with a believable size and a download URL of this repository.
fn pick_asset(assets: &[GhAsset], version: &str) -> Option<Asset> {
    let want = asset_name(version);
    let a = assets.iter().find(|a| a.name == want)?;
    if a.state.as_deref().is_some_and(|s| s != "uploaded") || a.size == 0 || a.size > MAX_APPIMAGE_BYTES {
        log::warn!("update: {} isn't ready for download (state {:?}, {} bytes)", a.name, a.state, a.size);
        return None;
    }
    if !valid_asset_url(&a.browser_download_url, &a.name) {
        log::warn!("update: refusing the download address {:?}", a.browser_download_url);
        return None;
    }
    Some(Asset {
        name: a.name.clone(),
        url: a.browser_download_url.clone(),
        size: a.size,
        sha256: parse_digest(a.digest.as_deref()),
    })
}

/// GitHub's `digest`: `sha256:<64 hex digits>` → the hex in lower case.
pub fn parse_digest(d: Option<&str>) -> Option<String> {
    let hex = d?.trim().strip_prefix("sha256:")?;
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hex.to_ascii_lowercase())
}

/// `https://<host>/<path…>` with a plain host (no user, no port): (host, path).
fn split_https(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("https://")?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    if host.is_empty() || host.contains(['@', ':', '\\']) || url.contains(char::is_whitespace) {
        return None;
    }
    Some((host, path))
}

/// A path segment of a release URL: a tag or file name, nothing that could climb or add a query.
fn plain_segment(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-+".contains(&b))
}

/// The download address of release file `name` of this repository on github.com:
/// `https://github.com/Dormant-Hero/lodestar/releases/download/<tag>/<name>`.
pub fn valid_asset_url(url: &str, name: &str) -> bool {
    let Some((host, path)) = split_https(url) else { return false };
    let p: Vec<&str> = path.split('/').collect();
    host.eq_ignore_ascii_case("github.com")
        && p.len() == 6
        && p[0].eq_ignore_ascii_case(OWNER)
        && p[1].eq_ignore_ascii_case(REPO)
        && p[2] == "releases"
        && p[3] == "download"
        && plain_segment(p[4])
        && p[5] == name
        && plain_segment(name)
}

/// Where a release download may be redirected: GitHub's release file hosts (their paths are
/// signed and opaque), over https.
pub fn valid_redirect(url: &str) -> bool {
    split_https(url).is_some_and(|(host, _)| ASSET_HOSTS.iter().any(|h| host.eq_ignore_ascii_case(h)))
}

/// A release page of this repository (only ever opened in the browser).
fn valid_release_page(url: &str) -> bool {
    let prefix = format!("https://github.com/{OWNER}/{REPO}/releases/");
    url.len() > prefix.len() && url[..prefix.len()].eq_ignore_ascii_case(&prefix) && !url.contains(char::is_whitespace)
}

// ------------------------------------------------------------------------------------ checking

/// Why a check has no answer.
#[derive(Clone, Debug, PartialEq)]
pub enum CheckErr {
    /// No connection, a time-out, a server error.
    Offline(String),
    /// GitHub's limit for unauthenticated requests (60 an hour per address).
    RateLimited,
    /// The repository has no published release yet.
    NoRelease,
    /// GitHub answered something unexpected.
    Bad(String),
}

impl std::fmt::Display for CheckErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckErr::Offline(_) => write!(f, "Couldn't reach GitHub. Try again later."),
            CheckErr::RateLimited => write!(f, "GitHub is busy right now. Try again later."),
            CheckErr::NoRelease => write!(f, "There is no published release yet."),
            CheckErr::Bad(_) => write!(f, "GitHub's answer wasn't understood. Try again later."),
        }
    }
}

/// Ask GitHub for the newest release (one request).
pub fn fetch_latest(net: &dyn Net) -> Result<Release, CheckErr> {
    let headers =
        [("Accept", "application/vnd.github+json".to_string()), ("X-GitHub-Api-Version", "2022-11-28".into())];
    let r = net.get_with(LATEST_API, &headers, net::MAX_JSON_BYTES).map_err(CheckErr::Offline)?;
    match r.status {
        200 => parse_release(&r.body).map_err(CheckErr::Bad),
        404 => Err(CheckErr::NoRelease),
        403 | 429 => Err(CheckErr::RateLimited),
        s if s >= 500 || s == 408 => Err(CheckErr::Offline(format!("HTTP {s}"))),
        s => Err(CheckErr::Bad(format!("HTTP {s}"))),
    }
}

/// An automatic check is due: never checked, a day has passed, or the clock went back.
pub fn check_due(last: Option<i64>, now: i64) -> bool {
    last.is_none_or(|t| now - t >= CHECK_EVERY_SECS || t > now + 3600)
}

/// Whether to run an automatic check now: the setting is on, the run may go online and a check
/// is due.
pub fn auto_check_wanted(enabled: bool, local_only: bool, last: Option<i64>, now: i64) -> bool {
    enabled && !local_only && check_due(last, now)
}

/// The last check, kept in the cache folder: when, and the newest release then (shown again on
/// the next start without asking GitHub).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CheckRecord {
    pub checked_at: Option<i64>,
    pub latest: Option<Release>,
}

impl CheckRecord {
    pub fn load(path: &Path) -> CheckRecord {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        match serde_json::to_vec_pretty(self) {
            Ok(b) => {
                if let Err(e) = crate::fsutil::write_atomic(path, &b) {
                    log::warn!("update: saving {}: {e}", path.display());
                }
            }
            Err(e) => log::warn!("update: {e}"),
        }
    }
}

// ------------------------------------------------------------------------------------ install kind

/// How this Lodestar was installed, as far as updating goes.
#[derive(Clone, Debug, PartialEq)]
pub enum Install {
    /// Running from this AppImage file. `writable`: its folder lets Lodestar replace it.
    AppImage { path: PathBuf, writable: bool },
    /// install.sh, a build from source, a package: Lodestar doesn't replace itself.
    Other,
}

impl Install {
    /// From the environment the AppImage runtime sets: `APPIMAGE` (the file) and `APPDIR` (where
    /// it is mounted, which the running program `exe` must be inside — a Lodestar started from
    /// some other AppImage inherits that one's variables and must never touch its file).
    pub fn detect(env: &dyn Fn(&str) -> Option<OsString>, exe: Option<&Path>) -> Install {
        let (Some(file), Some(dir), Some(exe)) = (env("APPIMAGE"), env("APPDIR"), exe) else {
            return Install::Other;
        };
        let (file, dir) = (PathBuf::from(file), PathBuf::from(dir));
        if !file.is_absolute() || !dir.is_absolute() || dir.as_os_str().len() < 2 || !exe.starts_with(&dir) {
            return Install::Other;
        }
        let path = std::fs::canonicalize(&file).unwrap_or(file);
        if !is_appimage_file(&path) {
            return Install::Other;
        }
        let writable = path.parent().is_some_and(dir_writable);
        Install::AppImage { path, writable }
    }

    pub fn from_env() -> Install {
        Install::detect(&|k| std::env::var_os(k), std::env::current_exe().ok().as_deref())
    }

    /// Lodestar can update itself in place.
    pub fn can_update(&self) -> bool {
        matches!(self, Install::AppImage { writable: true, .. })
    }

    pub fn appimage(&self) -> Option<&Path> {
        match self {
            Install::AppImage { path, .. } => Some(path),
            Install::Other => None,
        }
    }
}

fn dir_writable(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else { return false };
    // SAFETY: access() on a NUL-terminated path we own.
    unsafe { libc::access(c.as_ptr(), libc::W_OK | libc::X_OK) == 0 }
}

/// An ELF file with the AppImage type 2 magic (`AI\x02` at offset 8).
pub fn is_appimage_magic(head: &[u8]) -> bool {
    head.len() >= 11 && head[..4] == *b"\x7fELF" && head[8..11] == *b"AI\x02"
}

fn is_appimage_file(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 16];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && is_appimage_magic(&head)
}

// ------------------------------------------------------------------------------------ installing

/// `.<name>.update-<pid>.tmp` next to the AppImage: hidden, cleaned up on the next start.
fn temp_name(name: &str) -> String {
    format!(".{name}.update-{}.tmp", std::process::id())
}

/// `<name>.old`: the version replaced, kept until the new one has started.
pub fn old_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().map(OsString::from).unwrap_or_default();
    name.push(".old");
    target.with_file_name(name)
}

/// Writes the download to the file while hashing it, keeping its first bytes, and stops at
/// cancel or when more than the expected size arrives.
struct Sink<'a> {
    file: std::io::BufWriter<std::fs::File>,
    hash: Sha256,
    head: Vec<u8>,
    written: u64,
    limit: u64,
    cancel: &'a AtomicBool,
}

impl Write for Sink<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            // Not `Interrupted`: `write_all` would retry that forever.
            return Err(std::io::Error::other("cancelled"));
        }
        if self.written + buf.len() as u64 > self.limit {
            return Err(std::io::Error::other("bigger than the release says"));
        }
        self.file.write_all(buf)?;
        self.hash.update(buf);
        if self.head.len() < 16 {
            let n = (16 - self.head.len()).min(buf.len());
            self.head.extend_from_slice(&buf[..n]);
        }
        self.written += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

fn mb(b: u64) -> String {
    format!("{:.1} MB", b as f64 / 1_000_000.0)
}

/// Download `asset` and put it in place of the AppImage `target`. `progress(bytes, total)` is
/// told along the way. On any failure the AppImage is untouched and no temporary file is left.
/// Errors are plain sentences for the user.
pub fn download_and_replace(
    net: &dyn Net,
    asset: &Asset,
    target: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), String> {
    if !valid_asset_url(&asset.url, &asset.name) || asset.size == 0 || asset.size > MAX_APPIMAGE_BYTES {
        return Err("This release's download isn't one Lodestar can use.".into());
    }
    let (Some(dir), Some(name)) = (target.parent(), target.file_name().and_then(|n| n.to_str())) else {
        return Err("Lodestar couldn't tell where its AppImage is.".into());
    };
    let tmp = dir.join(temp_name(name));
    let r = fetch_into(net, asset, &tmp, cancel, progress).and_then(|()| {
        let mode = std::fs::metadata(target)
            .map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()))
            .unwrap_or(0o755);
        set_executable(&tmp, mode).map_err(|e| format!("Couldn't make the update executable: {e}"))?;
        replace(target, &tmp).map_err(|e| format!("Couldn't put the update in place of {}: {e}", target.display()))
    });
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

fn fetch_into(
    net: &dyn Net,
    asset: &Asset,
    tmp: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), String> {
    let file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(tmp)
    }
    .map_err(|e| format!("Couldn't save the update in {}: {e}", tmp.parent().unwrap_or(tmp).display()))?;
    let mut sink = Sink {
        file: std::io::BufWriter::with_capacity(256 * 1024, file),
        hash: Sha256::new(),
        head: Vec::new(),
        written: 0,
        limit: asset.size,
        cancel,
    };
    let total = asset.size;
    let mut url = asset.url.clone();
    let mut hops = 0;
    loop {
        let got = net.download(&url, &mut sink, total, &mut |n| progress(n, total));
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled.".into());
        }
        let d = got.map_err(|e| {
            log::warn!("update: downloading {url}: {e}");
            "The download stopped. Check the connection and try again.".to_string()
        })?;
        match d.status {
            200 => break,
            301 | 302 | 303 | 307 | 308 => {
                hops += 1;
                let next = d.location.unwrap_or_default();
                if hops > MAX_HOPS
                    || sink.written > 0
                    || !(valid_redirect(&next) || valid_asset_url(&next, &asset.name))
                {
                    log::warn!("update: refusing the redirect to {next:?}");
                    return Err("GitHub sent the download somewhere Lodestar doesn't trust, so it stopped.".into());
                }
                url = next;
            }
            s => {
                log::warn!("update: {url} answered HTTP {s}");
                return Err(format!("GitHub didn't hand out the download (HTTP {s}). Try again later."));
            }
        }
    }
    sink.flush().map_err(|e| format!("Couldn't save the update: {e}"))?;
    if sink.written != asset.size {
        return Err(format!("The download was incomplete ({} of {}). Try again.", mb(sink.written), mb(asset.size)));
    }
    if !is_appimage_magic(&sink.head) {
        return Err("The download isn't an AppImage, so it wasn't used.".into());
    }
    if let Some(want) = &asset.sha256 {
        let got: String = sink.hash.finalize_reset().iter().map(|b| format!("{b:02x}")).collect();
        if &got != want {
            log::warn!("update: SHA-256 {got}, the release says {want}");
            return Err("The download doesn't match the release's checksum, so it wasn't used.".into());
        }
    }
    let file = sink.file.into_inner().map_err(|e| format!("Couldn't save the update: {}", e.error()))?;
    file.sync_all().map_err(|e| format!("Couldn't save the update: {e}"))?;
    Ok(())
}

/// The old file's permissions, executable wherever it is readable (and always for the owner).
fn set_executable(path: &Path, old_mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let m = old_mode & 0o777;
    let m = m | ((m & 0o444) >> 2) | 0o700;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(m))
}

/// Keep `target` as `<name>.old` and rename `new` over it. With a hard link the AppImage path
/// never goes missing; where links don't work (some USB-stick file systems) the old file is
/// moved aside first and moved back if the second step fails.
fn replace(target: &Path, new: &Path) -> std::io::Result<()> {
    let old = old_path(target);
    match std::fs::remove_file(&old) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    if std::fs::hard_link(target, &old).is_ok() {
        return std::fs::rename(new, target);
    }
    std::fs::rename(target, &old)?;
    if let Err(e) = std::fs::rename(new, target) {
        let _ = std::fs::rename(&old, target);
        return Err(e);
    }
    Ok(())
}

/// After a start: the version an update replaced (`<name>.old`) and unfinished downloads of this
/// AppImage are removed. Returns how many files went.
pub fn clean_leftovers(target: &Path) -> usize {
    let (Some(dir), Some(name)) = (target.parent(), target.file_name().and_then(|n| n.to_str())) else { return 0 };
    let mut n = 0;
    if std::fs::remove_file(old_path(target)).is_ok() {
        n += 1;
    }
    let prefix = format!(".{name}.update-");
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let f = e.file_name();
            let Some(f) = f.to_str() else { continue };
            if f.starts_with(&prefix) && f.ends_with(".tmp") && std::fs::remove_file(e.path()).is_ok() {
                n += 1;
            }
        }
    }
    n
}

// ------------------------------------------------------------------------------------ restart

static RESTART: parking_lot::Mutex<Option<PathBuf>> = parking_lot::Mutex::new(None);

/// Start `appimage` again (with the same arguments) once the window has closed.
pub fn request_restart(appimage: &Path) {
    *RESTART.lock() = Some(appimage.to_path_buf());
}

/// The restart asked for, if any (main.rs, after the window closed).
pub fn take_restart() -> Option<PathBuf> {
    RESTART.lock().take()
}

/// Replace this process with the new AppImage (same arguments, same process id, so a session
/// script waiting on Lodestar keeps waiting). Returns only when that failed.
pub fn exec_restart(appimage: &Path) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    log::info!("update: starting {} again", appimage.display());
    std::process::Command::new(appimage).args(std::env::args_os().skip(1)).exec()
}

#[cfg(test)]
mod tests;
