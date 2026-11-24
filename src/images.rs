//! Async image loading with a downscaled thumbnail cache and a GPU texture LRU.
//!
//! CONTRACT:
//!   ImageCache::new(ctx, thumbs_dir)
//!   get(&mut self, path, size) -> Option<TextureHandle>: cheap; None while loading or if the
//!     file is missing/undecodable (then `is_failed` is true). Schedules a background decode on
//!     first request and calls ctx.request_repaint() when ready.
//!   prefetch(&mut self, path, size): schedule without returning (decoded after what is on
//!     screen; a loaded picture counts as used, so it isn't evicted before it is shown).
//!   waiting(&self) -> usize: pictures drawn this frame that are still loading (probes).
//!   tune_allocator(): once at start (see its docs).
//!   begin_frame(&mut self): upload finished decodes (bounded per frame) and evict
//!     least-recently-used textures above the memory budget. Called once per frame by App.
//!   Supports jpg/png/webp/gif(first frame)/bmp/ico/qoi and SVG (rasterised at target size).
//!   `ImgSize::Original` (artwork viewer) decodes the file at its own size, bypassing the disk
//!   cache; `image_dimensions(path)` reads a file's pixel size from its header.
//!   `get_adjusted` / `prefetch_adjusted` / `is_failed_adjusted`: the same with a colour
//!   adjustment (`Adjust`: brightness / saturation) applied on the worker, cached as its own
//!   texture.
//!
//! Design:
//! - `get()` is one hash lookup keyed by `(fnv1a(path bytes), ImgSize, Adjust)`; the stored path is
//!   compared on hit (collision check), nothing is allocated on the hit path.
//! - Decoding happens on 2–4 worker threads fed from two STACKS (latest request first): what
//!   is on screen (`get`) before what is only prefetched (`prefetch`: the items next to the
//!   selection). A prefetched picture that comes on screen while still queued is moved up
//!   (the job is queued again; whichever copy a worker takes first is decoded, once). Every
//!   pending entry shares an atomic "last requested frame" with its job; a worker drops jobs
//!   not re-requested within [`STALE_FRAMES`] frames before decoding them (fast scrolling).
//! - Workers consult a disk cache of downscaled images (QOI, see `images/decode.rs`).
//! - Uploads to egui happen in `begin_frame` under a per-frame budget; textures are
//!   LINEAR + mipmapped and evicted LRU by estimated GPU bytes.

mod decode;

pub use decode::{DecodeError, image_dimensions, load_image, load_original, prune_disk_cache};

use crossbeam_channel::{Receiver, Sender, unbounded};
use egui::{ColorImage, TextureFilter, TextureHandle, TextureOptions};
use parking_lot::{Condvar, Mutex};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ImgSize {
    /// Small list icons (rows of ~24 px): longest side <= 128 px. A list scrolled through
    /// thousands of games keeps a few MB of these instead of hundreds of MB of thumbnails.
    Icon,
    /// Grid covers / carousel items: longest side <= 512 px.
    Thumb,
    /// Detail panel, logos: longest side <= 1024 px.
    Medium,
    /// Backgrounds / heroes: longest side <= 2560 px.
    Full,
    /// Whole-window ambient background: tiny (<= 160 px) and heavily blurred; it is stretched
    /// over the window, so the blur plus GPU filtering gives a smooth soft-focus look for free.
    Backdrop,
    /// The file at its own resolution (artwork viewer): only downscaled past 8192 px (a 256 MB
    /// texture, within the GPU budget) or the GPU's texture limit, never written to the disk cache.
    Original,
}

impl ImgSize {
    pub fn max_px(self) -> u32 {
        match self {
            ImgSize::Icon => 128,
            ImgSize::Thumb => 512,
            ImgSize::Medium => 1024,
            ImgSize::Full => 2560,
            ImgSize::Backdrop => 160,
            ImgSize::Original => 8192,
        }
    }
}

/// Jobs not re-requested within this many frames are dropped before decoding.
pub const STALE_FRAMES: u64 = 30;
/// A finished decode not asked for within this many frames is dropped instead of uploaded
/// (`begin_frame`). Anything on screen is asked for every frame (an extra pass or two per frame
/// stays well inside this).
const UPLOAD_STALE_FRAMES: u64 = 4;
/// Estimated GPU memory budget for textures (bytes).
pub const GPU_BUDGET_BYTES: usize = 384 * 1024 * 1024;
/// At most this many texture uploads per frame …
const MAX_UPLOADS_PER_FRAME: usize = 6;
/// … and at most this many bytes (one upload is always allowed).
const MAX_UPLOAD_BYTES_PER_FRAME: usize = 16 * 1024 * 1024;
/// … and stop once this much time was spent in uploads this frame.
const MAX_UPLOAD_TIME: Duration = Duration::from_millis(4);

/// The texture options used for every image: smooth minification via trilinear mipmaps.
pub const TEXTURE_OPTIONS: TextureOptions =
    TextureOptions { mipmap_mode: Some(TextureFilter::Linear), ..TextureOptions::LINEAR };

/// A colour change applied to a decoded picture before it is uploaded (a theme's
/// `brightness` / `saturation`). Quantised so that it can key the cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Adjust {
    /// Hundredths of a unit, -200..=200; each unit moves every colour channel by 0.3.
    brightness: i16,
    /// Percent, 0 (grey) ..= 100 (unchanged).
    saturation: u8,
}

impl Adjust {
    pub const NONE: Adjust = Adjust { brightness: 0, saturation: 100 };

    pub fn new(brightness: f32, saturation: f32) -> Adjust {
        let b = if brightness.is_finite() { brightness.clamp(-2.0, 2.0) } else { 0.0 };
        let s = if saturation.is_finite() { saturation.clamp(0.0, 1.0) } else { 1.0 };
        Adjust { brightness: (b * 100.0).round() as i16, saturation: (s * 100.0).round() as u8 }
    }

    pub fn is_none(self) -> bool {
        self == Adjust::NONE
    }

    /// Apply to premultiplied pixels: brightness on the
    /// un-premultiplied colour, then the mix towards grey.
    pub fn apply(self, img: &mut ColorImage) {
        if self.is_none() {
            return;
        }
        let add = self.brightness as f32 / 100.0 * 0.3;
        let sat = self.saturation as f32 / 100.0;
        for px in img.pixels.iter_mut() {
            let [r, g, b, a] = px.to_array();
            if a == 0 {
                continue;
            }
            let af = a as f32 / 255.0;
            let mut c = [r, g, b].map(|v| (v as f32 / 255.0 / af + add).clamp(0.0, 1.0));
            if sat < 1.0 {
                let grey = 0.2125 * c[0] + 0.7154 * c[1] + 0.0721 * c[2];
                c = c.map(|v| grey + (v - grey) * sat);
            }
            let [r, g, b] = c.map(|v| (v * af * 255.0).round().clamp(0.0, 255.0) as u8);
            *px = egui::Color32::from_rgba_premultiplied(r, g, b, a);
        }
    }
}

type Key = (u64, ImgSize, Adjust);

enum State {
    /// Queued or decoding; the atomic is the last frame the image was asked for.
    Pending(Arc<AtomicU64>),
    Ready(TextureHandle),
    Failed,
}

struct Entry {
    path: PathBuf,
    state: State,
    last_used: u64,
    /// Last frame `get` asked for it (drawn, not only prefetched): see `waiting`.
    last_get: u64,
    /// Queued as a prefetch and not moved up yet: the job's "taken" flag (see `Job::taken`).
    low: Option<Arc<AtomicBool>>,
    /// Estimated GPU bytes (0 unless Ready).
    bytes: usize,
}

struct Job {
    key: Key,
    path: PathBuf,
    size: ImgSize,
    adjust: Adjust,
    last_req: Arc<AtomicU64>,
    /// Shared by both copies of a prefetch job that was moved up: the first worker to take
    /// one decodes it, the other skips it.
    taken: Option<Arc<AtomicBool>>,
}

/// The decode queues (see the module docs).
#[derive(Default)]
struct Queues {
    /// Pictures on screen.
    high: Vec<Job>,
    /// Prefetches.
    low: Vec<Job>,
}

impl Queues {
    fn len(&self) -> usize {
        self.high.len() + self.low.len()
    }
    fn pop(&mut self) -> Option<Job> {
        self.high.pop().or_else(|| self.low.pop())
    }
}

/// `req` is the job's request token: a result only lands on the entry that asked for it (an
/// entry invalidated and re-requested meanwhile — the file was replaced — has a new token).
enum Done {
    Ok { key: Key, req: Arc<AtomicU64>, image: ColorImage },
    Failed { key: Key, req: Arc<AtomicU64>, error: DecodeError },
    Dropped { key: Key, req: Arc<AtomicU64> },
}

impl Entry {
    /// Still waiting for the result of request `req`?
    fn waits_for(&self, req: &Arc<AtomicU64>) -> bool {
        matches!(&self.state, State::Pending(r) if Arc::ptr_eq(r, req))
    }
}

struct Shared {
    stack: Mutex<Queues>,
    cv: Condvar,
    frame: AtomicU64,
    shutdown: AtomicBool,
    max_side: AtomicUsize,
    /// Jobs currently being decoded (for `stats`).
    busy: AtomicUsize,
}

/// Counters for debugging overlays / tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageStats {
    pub textures: usize,
    pub gpu_bytes: usize,
    pub pending: usize,
    pub failed: usize,
    pub queued: usize,
    pub uploads_last_frame: usize,
    pub evicted_total: u64,
}

pub struct ImageCache {
    ctx: egui::Context,
    entries: HashMap<Key, Entry>,
    shared: Arc<Shared>,
    done_rx: Receiver<Done>,
    frame: u64,
    gpu_bytes: usize,
    budget: usize,
    uploads_last_frame: usize,
    evicted_total: u64,
}

impl ImageCache {
    pub fn new(ctx: &egui::Context, thumbs_dir: PathBuf) -> ImageCache {
        Self::with_budget(ctx, thumbs_dir, GPU_BUDGET_BYTES)
    }

    /// Like `new` with a custom GPU byte budget (tests, low-memory setups).
    pub fn with_budget(ctx: &egui::Context, thumbs_dir: PathBuf, budget: usize) -> ImageCache {
        let _ = std::fs::create_dir_all(&thumbs_dir);
        let shared = Arc::new(Shared {
            stack: Mutex::new(Queues::default()),
            cv: Condvar::new(),
            frame: AtomicU64::new(0),
            shutdown: AtomicBool::new(false),
            max_side: AtomicUsize::new(16384),
            busy: AtomicUsize::new(0),
        });
        let (done_tx, done_rx) = unbounded();
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).div_ceil(2).clamp(2, 4);
        for i in 0..n {
            let shared = shared.clone();
            let tx = done_tx.clone();
            let ctx = ctx.clone();
            let dir = thumbs_dir.clone();
            let r =
                std::thread::Builder::new().name(format!("img-decode-{i}")).spawn(move || worker(shared, tx, ctx, dir));
            if let Err(e) = r {
                log::error!("image worker spawn failed: {e}");
            }
        }
        // Keep the disk cache bounded; runs once in the background.
        let _ = std::thread::Builder::new().name("thumb-prune".into()).spawn(move || {
            std::thread::sleep(Duration::from_secs(5));
            prune_disk_cache(&thumbs_dir, decode::DISK_CACHE_MAX_BYTES, decode::DISK_CACHE_TARGET_BYTES);
        });
        ImageCache {
            ctx: ctx.clone(),
            entries: HashMap::with_capacity(1024),
            shared,
            done_rx,
            frame: 0,
            gpu_bytes: 0,
            budget,
            uploads_last_frame: 0,
            evicted_total: 0,
        }
    }

    /// The texture for `path` at `size`, or None while it loads / if it failed.
    pub fn get(&mut self, path: &Path, size: ImgSize) -> Option<TextureHandle> {
        self.get_adjusted(path, size, Adjust::NONE)
    }

    /// `get` with a colour adjustment.
    pub fn get_adjusted(&mut self, path: &Path, size: ImgSize, adjust: Adjust) -> Option<TextureHandle> {
        let key = (path_hash(path), size, adjust);
        let frame = self.frame;
        match self.entries.get_mut(&key) {
            Some(e) if e.path == path => {
                e.last_used = frame;
                e.last_get = frame;
                match &e.state {
                    State::Ready(t) => Some(t.clone()),
                    State::Pending(last) => {
                        last.store(frame, Ordering::Relaxed);
                        // Prefetched and now on screen: decode it before the other prefetches.
                        if let Some(taken) = e.low.take()
                            && !taken.load(Ordering::Relaxed)
                        {
                            let job = Job {
                                key,
                                path: e.path.clone(),
                                size,
                                adjust,
                                last_req: last.clone(),
                                taken: Some(taken),
                            };
                            self.shared.stack.lock().high.push(job);
                            self.shared.cv.notify_one();
                        }
                        None
                    }
                    State::Failed => None,
                }
            }
            // 64-bit hash collision between two different paths: practically never happens;
            // the second path simply doesn't load.
            Some(_) => None,
            None => {
                self.schedule(key, path, size, adjust, false);
                if let Some(e) = self.entries.get_mut(&key) {
                    e.last_get = frame;
                }
                None
            }
        }
    }

    /// Pictures asked for by `get` this frame that are still loading (the screen shows a
    /// placeholder or the previous picture for them). Walks every entry: probes only.
    pub fn waiting(&self) -> usize {
        self.entries.values().filter(|e| e.last_get == self.frame && matches!(e.state, State::Pending(_))).count()
    }

    /// True when the file is missing or could not be decoded (never retried until
    /// `forget_failed`/`invalidate`).
    pub fn is_failed(&self, path: &Path, size: ImgSize) -> bool {
        self.is_failed_adjusted(path, size, Adjust::NONE)
    }

    /// `is_failed` for a colour-adjusted picture.
    pub fn is_failed_adjusted(&self, path: &Path, size: ImgSize, adjust: Adjust) -> bool {
        matches!(
            self.entries.get(&(path_hash(path), size, adjust)),
            Some(Entry { state: State::Failed, path: p, .. }) if p == path
        )
    }

    /// True while a decode is queued or running.
    #[cfg(test)]
    pub fn is_loading(&self, path: &Path, size: ImgSize) -> bool {
        self.is_loading_adjusted(path, size, Adjust::NONE)
    }

    /// `is_loading` for a colour-adjusted picture.
    pub fn is_loading_adjusted(&self, path: &Path, size: ImgSize, adjust: Adjust) -> bool {
        matches!(
            self.entries.get(&(path_hash(path), size, adjust)),
            Some(Entry { state: State::Pending(_), path: p, .. }) if p == path
        )
    }

    /// Schedule a decode without returning anything (e.g. the next game's hero). A picture
    /// already loaded counts as used, so the neighbours of what is on screen aren't evicted
    /// just before they are shown.
    pub fn prefetch(&mut self, path: &Path, size: ImgSize) {
        self.prefetch_adjusted(path, size, Adjust::NONE)
    }

    /// `prefetch` for a colour-adjusted picture.
    pub fn prefetch_adjusted(&mut self, path: &Path, size: ImgSize, adjust: Adjust) {
        let key = (path_hash(path), size, adjust);
        match self.entries.get_mut(&key) {
            Some(e) if e.path == path => {
                e.last_used = e.last_used.max(self.frame.saturating_sub(1));
                if let State::Pending(last) = &e.state {
                    last.store(self.frame, Ordering::Relaxed);
                }
            }
            Some(_) => {}
            None => self.schedule(key, path, size, adjust, true),
        }
    }

    /// Drop every cached size (and colour adjustment) of `path` (the file changed on disk).
    /// The next `get` reloads it.
    pub fn invalidate(&mut self, path: &Path) {
        let h = path_hash(path);
        let mut freed = 0;
        self.entries.retain(|k, e| {
            let drop = k.0 == h && e.path == path;
            if drop {
                freed += e.bytes;
            }
            !drop
        });
        self.gpu_bytes = self.gpu_bytes.saturating_sub(freed);
    }

    /// Forget all failures so missing files are looked for again (e.g. after a rescan).
    pub fn forget_failed(&mut self) {
        self.entries.retain(|_, e| !matches!(e.state, State::Failed));
    }

    pub fn stats(&self) -> ImageStats {
        let mut s = ImageStats {
            gpu_bytes: self.gpu_bytes,
            uploads_last_frame: self.uploads_last_frame,
            evicted_total: self.evicted_total,
            queued: self.shared.stack.lock().len() + self.shared.busy.load(Ordering::Relaxed),
            ..Default::default()
        };
        for e in self.entries.values() {
            match e.state {
                State::Ready(_) => s.textures += 1,
                State::Pending(_) => s.pending += 1,
                State::Failed => s.failed += 1,
            }
        }
        s
    }

    /// Upload finished decodes (bounded per frame) and evict LRU textures over budget.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
        self.shared.frame.store(self.frame, Ordering::Relaxed);
        let max_side = self.ctx.input(|i| i.max_texture_side);
        if max_side > 0 {
            self.shared.max_side.store(max_side, Ordering::Relaxed);
        }

        let t0 = Instant::now();
        let (mut uploads, mut up_bytes) = (0usize, 0usize);
        let mut more = false;
        loop {
            if uploads >= MAX_UPLOADS_PER_FRAME
                || up_bytes >= MAX_UPLOAD_BYTES_PER_FRAME
                || (uploads > 0 && t0.elapsed() >= MAX_UPLOAD_TIME)
            {
                more = !self.done_rx.is_empty();
                break;
            }
            let Ok(msg) = self.done_rx.try_recv() else {
                break;
            };
            match msg {
                Done::Ok { key, req, image } => {
                    let Some(e) = self.entries.get_mut(&key) else {
                        continue;
                    };
                    if !e.waits_for(&req) {
                        continue;
                    }
                    // Scrolled past meanwhile (asked for, but not in the last few frames):
                    // don't upload it. Results arrive in request order, so after a fast scroll
                    // through a long list, uploading every row passed on the way kept the rows
                    // now on screen waiting for seconds (and filled GPU memory). It loads
                    // again (from the disk cache) if it comes back into view.
                    if self.frame.saturating_sub(req.load(Ordering::Relaxed)) > UPLOAD_STALE_FRAMES {
                        self.entries.remove(&key);
                        continue;
                    }
                    let raw = image.size[0] * image.size[1] * 4;
                    let tex = self.ctx.load_texture(tex_name(&e.path), image, TEXTURE_OPTIONS);
                    // + 1/3 for the mip chain.
                    e.bytes = raw + raw / 3;
                    e.state = State::Ready(tex);
                    e.low = None;
                    self.gpu_bytes += e.bytes;
                    uploads += 1;
                    up_bytes += raw;
                }
                Done::Failed { key, req, error } => {
                    if let Some(e) = self.entries.get_mut(&key)
                        && e.waits_for(&req)
                    {
                        log::debug!("image {}: {error}", e.path.display());
                        e.state = State::Failed;
                    }
                }
                Done::Dropped { key, req } => {
                    if self.entries.get(&key).is_some_and(|e| e.waits_for(&req)) {
                        self.entries.remove(&key);
                    }
                }
            }
        }
        self.uploads_last_frame = uploads;
        if more {
            self.ctx.request_repaint();
        }
        if self.gpu_bytes > self.budget {
            self.evict();
        }
    }

    /// Queue a decode; `low`: a prefetch (decoded after everything on screen).
    fn schedule(&mut self, key: Key, path: &Path, size: ImgSize, adjust: Adjust, low: bool) {
        if path.as_os_str().is_empty() {
            self.entries.insert(
                key,
                Entry {
                    path: PathBuf::new(),
                    state: State::Failed,
                    last_used: self.frame,
                    last_get: u64::MAX,
                    low: None,
                    bytes: 0,
                },
            );
            return;
        }
        let last = Arc::new(AtomicU64::new(self.frame));
        let path = path.to_path_buf();
        let taken = low.then(|| Arc::new(AtomicBool::new(false)));
        self.entries.insert(
            key,
            Entry {
                path: path.clone(),
                state: State::Pending(last.clone()),
                last_used: self.frame,
                last_get: u64::MAX,
                low: taken.clone(),
                bytes: 0,
            },
        );
        let job = Job { key, path, size, adjust, last_req: last, taken };
        {
            let mut q = self.shared.stack.lock();
            if low { q.low.push(job) } else { q.high.push(job) }
        }
        self.shared.cv.notify_one();
    }

    /// Evict least-recently-used textures (not used this or the previous frame) down to 90 %
    /// of the budget, so we don't evict on every frame while hovering at the limit.
    fn evict(&mut self) {
        let target = self.budget / 10 * 9;
        let protect = self.frame.saturating_sub(1);
        let mut cands: Vec<(u64, Key)> = self
            .entries
            .iter()
            .filter(|(_, e)| matches!(e.state, State::Ready(_)) && e.last_used < protect)
            .map(|(k, e)| (e.last_used, *k))
            .collect();
        cands.sort_unstable_by_key(|c| c.0);
        for (_, k) in cands {
            if self.gpu_bytes <= target {
                break;
            }
            if let Some(e) = self.entries.remove(&k) {
                self.gpu_bytes = self.gpu_bytes.saturating_sub(e.bytes);
                self.evicted_total += 1;
            }
        }
    }
}

impl Drop for ImageCache {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Relaxed);
        *self.shared.stack.lock() = Queues::default();
        self.shared.cv.notify_all();
    }
}

/// Large allocations (a decoded picture is 1–15 MB) go straight to the system and back when
/// freed. glibc otherwise raises that limit each time such a block is freed, after which
/// decoded pictures come from its heaps and are kept there: scrolling through a theme's
/// full-screen art left about 600 MB more memory in use than the pictures shown. Blocks below
/// the limit (per-frame buffers) keep the heap's speed. Call once at start; the variable
/// `MALLOC_MMAP_THRESHOLD_` still wins when set.
pub fn tune_allocator() {
    const LIMIT: libc::c_int = 2 * 1024 * 1024;
    if std::env::var_os("MALLOC_MMAP_THRESHOLD_").is_none() {
        // SAFETY: mallopt only changes glibc's allocator settings; called before other threads start.
        unsafe {
            libc::mallopt(libc::M_MMAP_THRESHOLD, LIMIT);
        }
    }
}

fn tex_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// FNV-1a over the path's bytes.
pub fn path_hash(p: &Path) -> u64 {
    use std::os::unix::ffi::OsStrExt;
    crate::db::fnv1a(p.as_os_str().as_bytes())
}

fn worker(shared: Arc<Shared>, tx: Sender<Done>, ctx: egui::Context, dir: PathBuf) {
    loop {
        let job = {
            let mut st = shared.stack.lock();
            loop {
                if shared.shutdown.load(Ordering::Relaxed) {
                    return;
                }
                if let Some(j) = st.pop() {
                    break j;
                }
                shared.cv.wait(&mut st);
            }
        };
        // The other copy of a moved-up prefetch was taken already (and answers for both).
        if job.taken.as_ref().is_some_and(|t| t.swap(true, Ordering::AcqRel)) {
            continue;
        }
        let now = shared.frame.load(Ordering::Relaxed);
        if now.saturating_sub(job.last_req.load(Ordering::Relaxed)) > STALE_FRAMES {
            if tx.send(Done::Dropped { key: job.key, req: job.last_req }).is_err() {
                return;
            }
            continue;
        }
        shared.busy.fetch_add(1, Ordering::Relaxed);
        let max_side = shared.max_side.load(Ordering::Relaxed) as u32;
        let target = job.size.max_px().min(max_side.max(64));
        let loaded = if job.size == ImgSize::Original {
            load_original(&job.path, target)
        } else {
            load_image(&dir, &job.path, target)
        };
        let msg = match loaded {
            Ok(mut image) => Done::Ok {
                image: {
                    if job.size == ImgSize::Backdrop {
                        box_blur(&mut image, 6, 3);
                    }
                    job.adjust.apply(&mut image);
                    image
                },
                key: job.key,
                req: job.last_req,
            },
            Err(error) => Done::Failed { key: job.key, req: job.last_req, error },
        };
        shared.busy.fetch_sub(1, Ordering::Relaxed);
        if tx.send(msg).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

/// In-place separable box blur (`passes` passes approximate a gaussian). Works on premultiplied
/// colours, which is the correct space for blurring. Cheap at Backdrop sizes (<= 160 px).
pub fn box_blur(img: &mut egui::ColorImage, radius: usize, passes: usize) {
    let [w, h] = img.size;
    if w == 0 || h == 0 || radius == 0 {
        return;
    }
    let mut buf = vec![[0u32; 4]; w.max(h)];
    let px = &mut img.pixels;
    let mut blur_line = |get: &dyn Fn(usize) -> usize, len: usize, px: &mut Vec<egui::Color32>| {
        for (i, b) in buf.iter_mut().enumerate().take(len) {
            let c = px[get(i)].to_array();
            *b = [c[0] as u32, c[1] as u32, c[2] as u32, c[3] as u32];
        }
        let r = radius.min(len.saturating_sub(1)).max(1);
        let n = (2 * r + 1) as u32;
        let at = |i: isize| buf[i.clamp(0, len as isize - 1) as usize];
        let mut sum = [0u32; 4];
        for k in -(r as isize)..=(r as isize) {
            let v = at(k);
            for c in 0..4 {
                sum[c] += v[c];
            }
        }
        for i in 0..len {
            let o = [sum[0] / n, sum[1] / n, sum[2] / n, sum[3] / n];
            px[get(i)] = egui::Color32::from_rgba_premultiplied(o[0] as u8, o[1] as u8, o[2] as u8, o[3] as u8);
            let (add, sub) = (at(i as isize + r as isize + 1), at(i as isize - r as isize));
            for c in 0..4 {
                sum[c] = sum[c] + add[c] - sub[c];
            }
        }
    };
    for _ in 0..passes {
        for y in 0..h {
            blur_line(&|x| y * w + x, w, px);
        }
        for x in 0..w {
            blur_line(&|y| y * w + x, h, px);
        }
    }
}

#[cfg(test)]
mod tests;
