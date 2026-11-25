//! Artwork editing: choose local files, search online sources, apply candidates, bulk download.
//!
//! CONTRACT (the desktop UI codes against this; keep signatures, add freely):
//!   * `ArtSlot` — which image of a game.
//!   * `ArtService` — background worker(s). `search` / `apply_url` / `bulk` return at once;
//!     results arrive through `poll()` as `ArtEvent`s (AppState::poll_art turns them into state).
//!   * `import_local` — copy a picked/dropped file into Lodestar's art folder (so moving or
//!     deleting the original doesn't break the game's art) after checking it is an image.
//!   * `auto_fill` / `cancel_auto` — the quiet automatic fill after scans (own thread, same
//!     throttle): `AutoGame` per game, then `AutoFinished`. Files in `<art_dir>/auto/<game>/`,
//!     stored by AppState as fetched art (see art_app.rs).
//!   * Sources are pluggable (`SourceInfo`, `SourceKind`): Steam store, GOG, Lutris.net,
//!     PCGamingWiki, libretro-thumbnails, GameTDB (no key) and SteamGridDB, IGDB, TheGamesDB
//!     (the user's own credentials), in the order and on/off state set in
//!     Settings → Metadata. Unconfigured keyed sources are listed as unavailable with a hint.
//!
//! Files: user-chosen art lives in `<art_dir>/user/<game id>/<slot>.<ext>`; candidate thumbnails
//! in `<cache>/art-thumbs/`.

pub mod limits;
#[cfg(test)]
mod limits_tests;
pub(crate) mod net;
pub mod sources;
pub mod tidy;
mod worker;

use crate::config::MetadataConfig;
use crate::fsutil::write_atomic;
use crate::model::*;
use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::{Condvar, Mutex};
pub use sources::{ArtList, SourceKind};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
pub use worker::AutoRequest;
pub(crate) use worker::gog_product_id;
use worker::{Engine, SearchJob};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ArtSlot {
    Cover,
    Hero,
    Logo,
    Icon,
    Banner,
}

impl ArtSlot {
    pub const ALL: [ArtSlot; 5] = [ArtSlot::Cover, ArtSlot::Hero, ArtSlot::Logo, ArtSlot::Icon, ArtSlot::Banner];

    pub fn label(self) -> &'static str {
        match self {
            ArtSlot::Cover => "Cover",
            ArtSlot::Hero => "Background",
            ArtSlot::Logo => "Logo",
            ArtSlot::Icon => "Icon",
            ArtSlot::Banner => "Banner",
        }
    }

    /// Stable file/key name.
    pub fn key(self) -> &'static str {
        match self {
            ArtSlot::Cover => "cover",
            ArtSlot::Hero => "hero",
            ArtSlot::Logo => "logo",
            ArtSlot::Icon => "icon",
            ArtSlot::Banner => "banner",
        }
    }

    /// Short hint shown in the editor ("portrait 2:3, ~600×900").
    pub fn hint(self) -> &'static str {
        match self {
            ArtSlot::Cover => "Portrait box art, 2:3 (e.g. 600×900)",
            ArtSlot::Hero => "Wide background, e.g. 1920×620 or 3840×1240",
            ArtSlot::Logo => "Transparent title logo (PNG)",
            ArtSlot::Icon => "Small square icon",
            ArtSlot::Banner => "Wide capsule, e.g. 460×215 or 920×430",
        }
    }

    pub fn get(self, a: &Art) -> Option<&PathBuf> {
        match self {
            ArtSlot::Cover => a.cover.as_ref(),
            ArtSlot::Hero => a.hero.as_ref(),
            ArtSlot::Logo => a.logo.as_ref(),
            ArtSlot::Icon => a.icon.as_ref(),
            ArtSlot::Banner => a.banner.as_ref(),
        }
    }

    pub fn set(self, a: &mut Art, v: Option<PathBuf>) {
        match self {
            ArtSlot::Cover => a.cover = v,
            ArtSlot::Hero => a.hero = v,
            ArtSlot::Logo => a.logo = v,
            ArtSlot::Icon => a.icon = v,
            ArtSlot::Banner => a.banner = v,
        }
    }
}

/// One image found online.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    /// Source name as in `SourceInfo::name` ("Steam", "SteamGridDB", …).
    pub source: String,
    pub slot: ArtSlot,
    /// Full-size image URL (downloaded by `apply_url`).
    pub url: String,
    /// Smaller preview URL, when the source has one (else `url`).
    pub thumb_url: Option<String>,
    /// Pixel size when known (0 = unknown).
    pub width: u32,
    pub height: u32,
    /// Shown under the thumbnail, e.g. "Steam · ELDEN RING" or "SteamGridDB · alternate".
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ArtEvent {
    /// Search results for (game, slot). May arrive in several batches; `done` on the last one.
    Candidates {
        game: GameId,
        slot: ArtSlot,
        items: Vec<Candidate>,
        done: bool,
        errors: Vec<String>,
    },
    /// A candidate's thumbnail is on disk (load it with the ImageCache).
    ThumbReady {
        url: String,
        path: PathBuf,
    },
    /// Full image downloaded to `path`; AppState stores it as the game's user art for `slot`.
    Applied {
        game: GameId,
        slot: ArtSlot,
        path: PathBuf,
    },
    Failed {
        game: GameId,
        slot: ArtSlot,
        error: String,
    },
    BulkProgress {
        done: usize,
        total: usize,
        current: String,
    },
    BulkFinished {
        changed: usize,
        failed: usize,
        cancelled: bool,
    },
    /// A new search for (game, slot) started: earlier results for it are stale. Sent before
    /// any of the new search's `Candidates`.
    SearchStarted {
        game: GameId,
        slot: ArtSlot,
    },
    /// A candidate's thumbnail couldn't be downloaded.
    ThumbFailed {
        url: String,
    },
    /// Sent right before `BulkFinished`: games with no confident match / no images, and why
    /// the run stopped early (if it did).
    BulkNote {
        not_found: Vec<String>,
        error: Option<String>,
    },
    /// Automatic fill (`auto_fill`): one game's lookup is over. `art` holds the images found
    /// (files in `auto_art_dir`, possibly none); `tried` the sources that answered (empty when
    /// the lookup failed on the network and should be retried another time).
    AutoGame {
        game: GameId,
        art: Art,
        tried: Vec<SourceKind>,
    },
    /// The automatic fill run is over (every `AutoGame` was sent before this).
    AutoFinished {
        changed: usize,
        error: Option<String>,
    },
    /// `check_sgdb_key`'s answer for `key`.
    KeyChecked {
        key: String,
        check: KeyCheck,
    },
}

/// Whether a SteamGridDB API key works (`ArtService::check_sgdb_key`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyCheck {
    Checking,
    Works,
    /// SteamGridDB refused it (401 / 403).
    Rejected,
    /// SteamGridDB couldn't be asked (offline, server trouble): why.
    Unreachable(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct BulkOptions {
    pub slots: Vec<ArtSlot>,
    /// Only fill slots that have no art yet (else replace).
    pub only_missing: bool,
    /// Source names to use, in priority order (empty = all available).
    pub sources: Vec<String>,
}

impl Default for BulkOptions {
    fn default() -> Self {
        BulkOptions { slots: ArtSlot::ALL.to_vec(), only_missing: true, sources: vec![] }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceInfo {
    pub name: String,
    pub available: bool,
    /// Why it's unavailable ("needs a SteamGridDB API key in Settings → Metadata").
    pub reason: Option<String>,
    pub kind: SourceKind,
    /// Switched on in Settings → Metadata (it may still lack credentials).
    pub enabled: bool,
}

/// Work for the interactive thread (searches and single-image downloads).
enum Job {
    Apply { game: GameId, slot: ArtSlot, url: String },
    Import { game: GameId, slot: ArtSlot, path: PathBuf },
    Search { job: SearchJob, generation: u64 },
    CheckKey { key: String },
}

#[derive(Default)]
struct ThumbQueue {
    q: VecDeque<String>,
    /// Queued, done or failed this session.
    seen: HashSet<String>,
}

/// Pending thumbnails beyond this are dropped (oldest searches first).
const MAX_THUMB_QUEUE: usize = 400;
/// Thumbnail cache size limits (pruned at startup).
const THUMBS_MAX_BYTES: u64 = 300 * 1024 * 1024;
const THUMBS_TARGET_BYTES: u64 = 200 * 1024 * 1024;
/// Largest local file `import_local` accepts.
const MAX_IMPORT_BYTES: u64 = 64 * 1024 * 1024;

struct Shared {
    eng: Engine,
    tx: Sender<ArtEvent>,
    ctx: egui::Context,
    jobs: Mutex<VecDeque<Job>>,
    jobs_cv: Condvar,
    thumbs: Mutex<ThumbQueue>,
    thumbs_cv: Condvar,
    bulks: Mutex<VecDeque<(Vec<Game>, BulkOptions)>>,
    bulks_cv: Condvar,
    bulk_cancel: AtomicBool,
    autos: Mutex<VecDeque<Vec<AutoRequest>>>,
    autos_cv: Condvar,
    auto_cancel: AtomicBool,
    /// Latest search generation per (game, slot); older searches stop sending.
    gens: Mutex<HashMap<(GameId, ArtSlot), u64>>,
    shutdown: AtomicBool,
}

impl Shared {
    fn send(&self, ev: ArtEvent) {
        let _ = self.tx.send(ev);
        self.ctx.request_repaint();
    }

    fn queue_thumbs(&self, items: &[Candidate]) {
        let mut t = self.thumbs.lock();
        for c in items.iter().rev() {
            let url = c.thumb_url.clone().unwrap_or_else(|| c.url.clone());
            if t.seen.insert(url.clone()) {
                t.q.push_front(url);
            }
        }
        while t.q.len() > MAX_THUMB_QUEUE {
            if let Some(u) = t.q.pop_back() {
                t.seen.remove(&u);
            }
        }
        self.thumbs_cv.notify_one();
    }
}

type ThreadFn = fn(&Shared);

/// Background artwork worker: one thread for searches and downloads the user asked for, one
/// for candidate thumbnails, one for bulk runs. All requests share one polite throttle.
pub struct ArtService {
    sh: Arc<Shared>,
    rx: Receiver<ArtEvent>,
}

impl ArtService {
    pub fn new(ctx: &egui::Context, art_dir: PathBuf, thumbs_dir: PathBuf, cfg: &MetadataConfig) -> ArtService {
        let eng = Engine::new(Box::new(net::UreqNet::new()), net::Throttle::polite(), art_dir, thumbs_dir, cfg);
        ArtService::with_engine(ctx, eng)
    }

    fn with_engine(ctx: &egui::Context, eng: Engine) -> ArtService {
        let (tx, rx) = unbounded();
        let sh = Arc::new(Shared {
            eng,
            tx,
            ctx: ctx.clone(),
            jobs: Mutex::new(VecDeque::new()),
            jobs_cv: Condvar::new(),
            thumbs: Mutex::new(ThumbQueue::default()),
            thumbs_cv: Condvar::new(),
            bulks: Mutex::new(VecDeque::new()),
            bulks_cv: Condvar::new(),
            bulk_cancel: AtomicBool::new(false),
            autos: Mutex::new(VecDeque::new()),
            autos_cv: Condvar::new(),
            auto_cancel: AtomicBool::new(false),
            gens: Mutex::new(HashMap::new()),
            shutdown: AtomicBool::new(false),
        });
        let threads: [(&str, ThreadFn); 4] =
            [("art", run_jobs), ("art-thumbs", run_thumbs), ("art-bulk", run_bulks), ("art-auto", run_autos)];
        for (name, f) in threads {
            let sh = sh.clone();
            if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(move || f(&sh)) {
                log::error!("{name} thread spawn failed: {e}");
            }
        }
        ArtService { sh, rx }
    }

    #[cfg(test)]
    pub(crate) fn with_net(
        ctx: &egui::Context,
        n: Box<dyn net::Net>,
        art_dir: PathBuf,
        thumbs_dir: PathBuf,
        cfg: &MetadataConfig,
    ) -> ArtService {
        ArtService::with_engine(ctx, Engine::new(n, net::Throttle::none(), art_dir, thumbs_dir, cfg))
    }

    pub fn set_config(&mut self, cfg: &MetadataConfig) {
        self.sh.eng.set_config(cfg);
    }

    /// The PC games' source list (`sources_for(ArtList::Pc)`).
    pub fn sources(&self) -> Vec<SourceInfo> {
        self.sh.eng.sources()
    }

    /// One source list of Settings → Metadata, in the user's order (see `Engine::sources_for`).
    pub fn sources_for(&self, list: ArtList) -> Vec<SourceInfo> {
        self.sh.eng.sources_for(list)
    }

    /// What is left of each source's API limits ("TheGamesDB", "2 912 left this month").
    pub fn limits_summary(&self) -> Vec<(SourceKind, String)> {
        self.sh.eng.limits.summary(limits::unix_now())
    }

    /// Why the running bulk download is waiting (an API limit), if it is.
    pub fn limit_wait(&self) -> Option<String> {
        self.sh.eng.limits.waiting()
    }

    /// Where user art is stored (`<art_dir>/user/<game>/…`).
    pub fn art_dir(&self) -> &Path {
        &self.sh.eng.art_dir
    }

    /// Search every available source for `slot` of `game` (by Steam appid when it has one,
    /// else by name). Optional `query` overrides the name searched (digits only = an appid).
    pub fn search(&self, game: &Game, slot: ArtSlot, query: Option<&str>) {
        let job = SearchJob::new(game, slot, query);
        let key = (game.id, slot);
        let generation = {
            let mut g = self.sh.gens.lock();
            let n = g.entry(key).or_insert(0);
            *n += 1;
            // Under the lock: older searches can't send after this. (No repaint request here:
            // this runs on the UI thread, which polls right away.)
            let _ = self.sh.tx.send(ArtEvent::SearchStarted { game: game.id, slot });
            *n
        };
        let mut q = self.sh.jobs.lock();
        q.retain(|j| !matches!(j, Job::Search { job, .. } if (job.game, job.slot) == key));
        q.push_back(Job::Search { job, generation });
        self.sh.jobs_cv.notify_one();
    }

    /// Download `url` (http/https; a local path or file:// URL is imported instead) as the
    /// game's user art for `slot`. Applied / Failed arrives through `poll`.
    pub fn apply_url(&self, game: GameId, slot: ArtSlot, url: &str) {
        self.push_front(Job::Apply { game, slot, url: url.to_string() });
    }

    /// `import_local` on the worker thread (decoding a big image takes a while).
    pub fn import_file(&self, game: GameId, slot: ArtSlot, path: &Path) {
        self.push_front(Job::Import { game, slot, path: path.to_path_buf() });
    }

    fn push_front(&self, job: Job) {
        // Before queued searches, after earlier downloads.
        let mut q = self.sh.jobs.lock();
        let at = q.iter().position(|j| matches!(j, Job::Search { .. })).unwrap_or(q.len());
        q.insert(at, job);
        self.sh.jobs_cv.notify_one();
    }

    /// Ask SteamGridDB whether `key` works; `KeyChecked` arrives through `poll`.
    pub fn check_sgdb_key(&self, key: &str) {
        self.push_front(Job::CheckKey { key: key.to_string() });
    }

    /// Fill `opts.slots` of `games` from the online sources (confident matches only).
    pub fn bulk(&self, games: Vec<Game>, opts: BulkOptions) {
        self.sh.bulks.lock().push_back((games, opts));
        self.sh.bulks_cv.notify_one();
    }

    pub fn cancel_bulk(&self) {
        let mut q = self.sh.bulks.lock();
        q.clear();
        self.sh.bulk_cancel.store(true, Ordering::Relaxed);
    }

    /// Fill missing art of `reqs` in the background, quietly (its own thread, sharing the
    /// polite throttle with everything else). Results: `AutoGame` per game, then `AutoFinished`.
    pub fn auto_fill(&self, reqs: Vec<AutoRequest>) {
        self.sh.autos.lock().push_back(reqs);
        self.sh.autos_cv.notify_one();
    }

    /// Stop the automatic fill (queued runs are dropped; the running one ends soon).
    pub fn cancel_auto(&self) {
        let mut q = self.sh.autos.lock();
        // Every run sends one AutoFinished, also those that never started.
        for _ in q.drain(..) {
            let _ = self.sh.tx.send(ArtEvent::AutoFinished { changed: 0, error: None });
        }
        self.sh.auto_cancel.store(true, Ordering::Relaxed);
    }

    pub fn poll(&mut self) -> Vec<ArtEvent> {
        self.rx.try_iter().collect()
    }
}

impl Drop for ArtService {
    fn drop(&mut self) {
        self.sh.shutdown.store(true, Ordering::Relaxed);
        self.sh.bulk_cancel.store(true, Ordering::Relaxed);
        self.sh.auto_cancel.store(true, Ordering::Relaxed);
        self.sh.jobs.lock().clear();
        self.sh.thumbs.lock().q.clear();
        self.sh.bulks.lock().clear();
        self.sh.autos.lock().clear();
        self.sh.jobs_cv.notify_all();
        self.sh.thumbs_cv.notify_all();
        self.sh.bulks_cv.notify_all();
        self.sh.autos_cv.notify_all();
    }
}

fn wait_pop<T>(sh: &Shared, m: &Mutex<VecDeque<T>>, cv: &Condvar) -> Option<T> {
    let mut q = m.lock();
    loop {
        if sh.shutdown.load(Ordering::Relaxed) {
            return None;
        }
        if let Some(j) = q.pop_front() {
            return Some(j);
        }
        cv.wait(&mut q);
    }
}

fn run_jobs(sh: &Shared) {
    while let Some(job) = wait_pop(sh, &sh.jobs, &sh.jobs_cv) {
        match job {
            Job::Apply { game, slot, url } => match sh.eng.apply(game, slot, &url) {
                Ok(path) => sh.send(ArtEvent::Applied { game, slot, path }),
                Err(error) => sh.send(ArtEvent::Failed { game, slot, error }),
            },
            Job::Import { game, slot, path } => match import_local(&sh.eng.art_dir, game, slot, &path) {
                Ok(path) => sh.send(ArtEvent::Applied { game, slot, path }),
                Err(e) => sh.send(ArtEvent::Failed { game, slot, error: format!("{e:#}") }),
            },
            Job::CheckKey { key } => {
                let check = sh.eng.check_sgdb_key(&key);
                sh.send(ArtEvent::KeyChecked { key, check });
            }
            Job::Search { job, generation } => {
                let key = (job.game, job.slot);
                if sh.gens.lock().get(&key) != Some(&generation) {
                    continue;
                }
                sh.eng.search(&job, |items, done, errors| {
                    let g = sh.gens.lock();
                    if g.get(&key) != Some(&generation) {
                        return false;
                    }
                    sh.queue_thumbs(&items);
                    sh.send(ArtEvent::Candidates { game: job.game, slot: job.slot, items, done, errors });
                    true
                });
            }
        }
    }
}

fn run_thumbs(sh: &Shared) {
    prune_thumbs(&sh.eng.thumbs_dir, THUMBS_MAX_BYTES, THUMBS_TARGET_BYTES);
    loop {
        let url = {
            let mut t = sh.thumbs.lock();
            loop {
                if sh.shutdown.load(Ordering::Relaxed) {
                    return;
                }
                if let Some(u) = t.q.pop_front() {
                    break u;
                }
                sh.thumbs_cv.wait(&mut t);
            }
        };
        match sh.eng.thumb(&url) {
            Ok(path) => sh.send(ArtEvent::ThumbReady { url, path }),
            Err(e) => {
                log::debug!("art thumb {url}: {e}");
                sh.send(ArtEvent::ThumbFailed { url });
            }
        }
    }
}

fn run_bulks(sh: &Shared) {
    loop {
        let (games, opts) = {
            let mut q = sh.bulks.lock();
            loop {
                if sh.shutdown.load(Ordering::Relaxed) {
                    return;
                }
                if let Some(j) = q.pop_front() {
                    sh.bulk_cancel.store(false, Ordering::Relaxed);
                    break j;
                }
                sh.bulks_cv.wait(&mut q);
            }
        };
        let t0 = std::time::Instant::now();
        let rep = sh.eng.bulk(
            &games,
            &opts,
            &sh.bulk_cancel,
            &mut |done, total, current| sh.send(ArtEvent::BulkProgress { done, total, current: current.to_string() }),
            &mut |game, slot, path| sh.send(ArtEvent::Applied { game, slot, path }),
        );
        log::info!(
            "art bulk: {} games, {} changed, {} failed, {} not found, cancelled {} in {:?}",
            games.len(),
            rep.changed,
            rep.failed,
            rep.not_found.len(),
            rep.cancelled,
            t0.elapsed()
        );
        sh.send(ArtEvent::BulkNote { not_found: rep.not_found, error: rep.error });
        sh.send(ArtEvent::BulkFinished { changed: rep.changed, failed: rep.failed, cancelled: rep.cancelled });
    }
}

fn run_autos(sh: &Shared) {
    loop {
        let reqs = {
            let mut q = sh.autos.lock();
            loop {
                if sh.shutdown.load(Ordering::Relaxed) {
                    return;
                }
                if let Some(j) = q.pop_front() {
                    sh.auto_cancel.store(false, Ordering::Relaxed);
                    break j;
                }
                sh.autos_cv.wait(&mut q);
            }
        };
        let t0 = std::time::Instant::now();
        let rep = sh.eng.auto_fill(&reqs, &sh.auto_cancel, &mut |game, art, tried| {
            sh.send(ArtEvent::AutoGame { game, art, tried })
        });
        log::info!(
            "art auto: {} games, {} got art, cancelled {}, error {:?} in {:?}",
            reqs.len(),
            rep.changed,
            rep.cancelled,
            rep.error,
            t0.elapsed()
        );
        sh.send(ArtEvent::AutoFinished { changed: rep.changed, error: rep.error });
    }
}

// ---------------------------------------------------------------------------------------------
// Files

/// The Steam store appid of a game, if it has one (not for non-Steam shortcuts).
pub fn steam_appid(g: &Game) -> Option<u32> {
    let id = match &g.launch {
        Some(LaunchSpec::Steam { appid }) => Some(*appid),
        _ if g.source == Source::Steam => g.source_id.parse().ok(),
        _ => None,
    };
    id.and_then(sources::store_appid)
}

/// `<art_dir>/user/<game>`: the folder holding a game's user-chosen art.
pub fn user_art_dir(art_dir: &Path, game: GameId) -> PathBuf {
    art_dir.join("user").join(game.to_string())
}

/// `<art_dir>/user/<game>/<slot>.<ext>`.
pub fn user_art_path(art_dir: &Path, game: GameId, slot: ArtSlot, ext: &str) -> PathBuf {
    user_art_dir(art_dir, game).join(format!("{}.{ext}", slot.key()))
}

/// Is `path` a file Lodestar stored as user art for `game` (and so may delete)?
pub fn is_user_art_file(art_dir: &Path, game: GameId, path: &Path) -> bool {
    path.parent() == Some(user_art_dir(art_dir, game).as_path())
}

/// Check `bytes` is an image Lodestar can show; returns its file extension.
pub fn validate_image(bytes: &[u8]) -> Result<&'static str, String> {
    let ext = sources::sniff_ext(bytes).ok_or("not a supported image (PNG, JPEG, WebP, GIF, BMP, ICO, QOI or SVG)")?;
    if ext == "svg" {
        let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default())
            .map_err(|e| format!("broken SVG: {e}"))?;
        let s = tree.size();
        if !(s.width() > 0.0 && s.height() > 0.0) {
            return Err("SVG has no size".into());
        }
        return Ok(ext);
    }
    let fmt = image::ImageFormat::from_extension(ext).ok_or("unsupported image format")?;
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), fmt);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(1024 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| format!("can't read the image: {e}"))?;
    if img.width() == 0 || img.height() == 0 {
        return Err("empty image".into());
    }
    Ok(ext)
}

pub(crate) fn write_user_art(
    art_dir: &Path,
    game: GameId,
    slot: ArtSlot,
    ext: &str,
    bytes: &[u8],
) -> std::io::Result<PathBuf> {
    let dest = user_art_path(art_dir, game, slot, ext);
    write_atomic(&dest, bytes)?;
    Ok(dest)
}

/// Copy a local image into `<art_dir>/user/<game>/<slot>.<ext>` after checking it decodes.
/// The file's own extension is kept when it names the format the content really is.
pub fn import_local(art_dir: &Path, game: GameId, slot: ArtSlot, src: &Path) -> anyhow::Result<PathBuf> {
    use anyhow::Context as _;
    let meta = std::fs::metadata(src).with_context(|| format!("can't open {}", src.display()))?;
    if !meta.is_file() {
        anyhow::bail!("{} is not a file", src.display());
    }
    if meta.len() > MAX_IMPORT_BYTES {
        anyhow::bail!("{} is too large ({} MB)", src.display(), meta.len() / (1024 * 1024));
    }
    let bytes = std::fs::read(src).with_context(|| format!("can't read {}", src.display()))?;
    let sniffed = validate_image(&bytes).map_err(|e| anyhow::anyhow!("{}: {e}", src.display()))?;
    let own = src.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    let ext = match own.as_deref() {
        Some(e) if e == sniffed => e.to_string(),
        Some("jpeg" | "jpe") if sniffed == "jpg" => own.clone().unwrap_or_default(),
        Some("svgz") if sniffed == "svg" => "svg".to_string(),
        _ => sniffed.to_string(),
    };
    let dest = user_art_path(art_dir, game, slot, &ext);
    if std::fs::canonicalize(src).ok().is_some_and(|s| std::fs::canonicalize(&dest).ok() == Some(s)) {
        return Ok(dest);
    }
    write_atomic(&dest, &bytes).with_context(|| format!("can't write {}", dest.display()))?;
    Ok(dest)
}

/// Delete stale temp files, and the oldest thumbnails when over `max_bytes` (down to `target`).
fn prune_thumbs(dir: &Path, max_bytes: u64, target: u64) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    let mut files = vec![];
    let mut total = 0u64;
    for e in rd.flatten() {
        let Ok(m) = e.metadata() else { continue };
        if !m.is_file() {
            continue;
        }
        let mtime = m.modified().unwrap_or(now);
        if e.file_name().to_string_lossy().ends_with(".tmp") {
            if now.duration_since(mtime).unwrap_or_default().as_secs() > 600 {
                let _ = std::fs::remove_file(e.path());
            }
            continue;
        }
        total += m.len();
        files.push((mtime, m.len(), e.path()));
    }
    if total <= max_bytes {
        return;
    }
    files.sort_by_key(|f| f.0);
    for (_, len, p) in files {
        if total <= target {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

#[cfg(test)]
mod more_tests;
#[cfg(test)]
mod tests;
