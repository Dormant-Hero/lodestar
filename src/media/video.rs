//! Theme videos: an `ffmpeg` process per playing video decodes small frames (scaled to the
//! element, 24 per second, looping, low CPU priority) onto a pipe; a helper thread reads them at
//! that pace and keeps only the newest, which the renderer uploads as a texture.
//!
//! A video plays only while something draws it: not drawn for half a second → the helper stops
//! reading (ffmpeg waits, using no CPU); for 8 seconds → ffmpeg is closed. At most
//! `MAX_PLAYING` decode at once. Until the first frame arrives, or without ffmpeg, the caller
//! draws the element's still picture.

use super::ppm;
use egui::{ColorImage, TextureHandle, TextureOptions};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

/// Videos decoding at the same time.
pub const MAX_PLAYING: usize = 2;
pub const FPS: u32 = 24;
/// Largest decoded frame (the GPU scales it up to the element).
pub const MAX_W: u32 = 960;
pub const MAX_H: u32 = 540;
/// Not drawn for this long: stop reading frames (ffmpeg pauses).
const PAUSE_SECS: f64 = 0.5;
/// Not drawn for this long: close ffmpeg.
const CLOSE_SECS: f64 = 8.0;

/// The size frames are decoded at for an element of `px` physical pixels: at most
/// `MAX_W` × `MAX_H` (keeping the box's shape), in steps of 16 so small layout changes reuse the
/// same decoder.
pub fn decode_size(px: [f32; 2]) -> [u32; 2] {
    let (w, h) = (px[0].max(16.0), px[1].max(16.0));
    let s = (MAX_W as f32 / w).min(MAX_H as f32 / h).min(1.0);
    let q = |v: f32| (((v * s) / 16.0).round().max(1.0) as u32) * 16;
    [q(w), q(h)]
}

/// ffmpeg's arguments: `path` looped, video only, `fps` frames a second, scaled to fit inside
/// `size` (aspect kept), as PPM pictures on stdout, quiet.
pub fn ffmpeg_args(path: &Path, size: [u32; 2], fps: u32) -> Vec<OsString> {
    let mut v: Vec<OsString> = [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "quiet",
        "-threads",
        "2",
        // Cheaper decoding; invisible at these sizes.
        "-skip_loop_filter",
        "all",
        "-stream_loop",
        "-1",
        "-i",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    v.push(path.as_os_str().to_owned());
    let vf = format!(
        "fps={fps},scale={}:{}:force_original_aspect_ratio=decrease:force_divisible_by=2:flags=fast_bilinear",
        size[0], size[1]
    );
    for a in ["-an", "-sn", "-dn", "-vf", &vf, "-f", "image2pipe", "-c:v", "ppm", "-"] {
        v.push(a.into());
    }
    v
}

/// Shared between a video's helper thread and the renderer.
#[derive(Default)]
struct Shared {
    frame: Mutex<Option<Arc<ColorImage>>>,
    /// Bumped for every new frame.
    serial: AtomicU64,
    /// `super::clock()` milliseconds when the video was last drawn.
    wanted_ms: AtomicU64,
    /// The helper has ended (closed for being unused, or failed).
    done: AtomicBool,
    /// It ended without ever giving a frame: don't try this file again.
    failed: AtomicBool,
    stop: AtomicBool,
}

impl Shared {
    fn want(&self) {
        self.wanted_ms.store((super::clock() * 1000.0) as u64, Ordering::Relaxed);
    }
    fn idle_secs(&self) -> f64 {
        super::clock() - self.wanted_ms.load(Ordering::Relaxed) as f64 / 1000.0
    }
}

/// Where frames come from: ffmpeg, or a generated picture (tests, `LODESTAR_FAKE_VIDEO`).
pub trait Source: Send {
    fn reader(&mut self) -> &mut dyn Read;
}

struct Ffmpeg {
    child: Child,
    out: std::process::ChildStdout,
}

impl Source for Ffmpeg {
    fn reader(&mut self) -> &mut dyn Read {
        &mut self.out
    }
}

impl Drop for Ffmpeg {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Runs on the video's helper thread (starting a process never holds up a frame).
fn spawn_ffmpeg(ffmpeg: &Path, path: &Path, size: [u32; 2]) -> Option<Box<dyn Source>> {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new(ffmpeg);
    cmd.args(ffmpeg_args(path, size, FPS)).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    // SAFETY: setpriority is async-signal-safe and takes no pointers. Lowering the priority
    // before ffmpeg runs means its decoding threads (created later) inherit it too.
    unsafe {
        cmd.pre_exec(|| {
            libc::setpriority(libc::PRIO_PROCESS, 0, 10);
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| log::info!("video: can't start ffmpeg: {e}")).ok()?;
    let out = child.stdout.take()?;
    Some(Box::new(Ffmpeg { child, out }))
}

/// A moving test picture as a PPM stream (never ends).
pub struct FakeSource {
    w: usize,
    h: usize,
    n: usize,
    buf: Vec<u8>,
    at: usize,
}

impl FakeSource {
    pub fn new(size: [u32; 2]) -> FakeSource {
        FakeSource { w: size[0] as usize, h: size[1] as usize, n: 0, buf: vec![], at: 0 }
    }

    /// Frame `n`: a diagonal colour wash with a white bar that moves across.
    pub fn frame(w: usize, h: usize, n: usize) -> ppm::Frame {
        let mut rgb = Vec::with_capacity(w * h * 3);
        let bar = (n * 7) % w.max(1);
        for y in 0..h {
            for x in 0..w {
                if x.abs_diff(bar) < w / 40 + 1 {
                    rgb.extend_from_slice(&[255, 255, 255]);
                } else {
                    let t = (x + y) * 255 / (w + h).max(1);
                    rgb.extend_from_slice(&[t as u8, 60, (255 - t) as u8]);
                }
            }
        }
        ppm::Frame { w, h, rgb }
    }
}

impl Read for FakeSource {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.at >= self.buf.len() {
            self.buf = ppm::encode(&Self::frame(self.w, self.h, self.n));
            self.n += 1;
            self.at = 0;
        }
        let n = out.len().min(self.buf.len() - self.at);
        out[..n].copy_from_slice(&self.buf[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

impl Source for FakeSource {
    fn reader(&mut self) -> &mut dyn Read {
        self
    }
}

/// The helper thread's loop: read frames at `fps`, publish the newest, pause while unused.
fn pump(mut src: Box<dyn Source>, sh: &Shared, fps: u32, on_frame: &dyn Fn()) {
    let period = 1.0 / fps.max(1) as f64;
    let mut r = BufReader::with_capacity(1 << 16, src.reader());
    let mut frames = 0u64;
    let mut next = super::clock();
    loop {
        if sh.stop.load(Ordering::Relaxed) {
            break;
        }
        let idle = sh.idle_secs();
        if idle > CLOSE_SECS {
            break;
        }
        if idle > PAUSE_SECS {
            std::thread::sleep(Duration::from_millis(100));
            next = super::clock();
            continue;
        }
        match ppm::read_frame(&mut r) {
            Ok(Some(f)) => {
                let img = ColorImage::from_rgb([f.w, f.h], &f.rgb);
                *sh.frame.lock() = Some(Arc::new(img));
                sh.serial.fetch_add(1, Ordering::Release);
                frames += 1;
                on_frame();
            }
            Ok(None) | Err(_) => {
                if frames == 0 {
                    sh.failed.store(true, Ordering::Relaxed);
                }
                break;
            }
        }
        next += period;
        let wait = next - super::clock();
        if wait > 0.0 {
            std::thread::sleep(Duration::from_secs_f64(wait));
        } else if wait < -1.0 {
            next = super::clock(); // fell far behind (system busy): don't rush to catch up
        }
    }
    sh.done.store(true, Ordering::Release);
}

struct Slot {
    shared: Arc<Shared>,
    tex: Option<TextureHandle>,
    seen: u64,
}

/// The videos one renderer plays. Dropping it stops them.
#[derive(Default)]
pub struct Videos {
    slots: HashMap<PathBuf, Slot>,
    /// Files that gave no picture (not tried again this session).
    failed: Vec<PathBuf>,
}

impl Drop for Videos {
    fn drop(&mut self) {
        self.stop_all();
    }
}

impl Videos {
    /// Stop every video (theme change, videos turned off).
    pub fn stop_all(&mut self) {
        for s in self.slots.values() {
            s.shared.stop.store(true, Ordering::Relaxed);
        }
        self.slots.clear();
    }

    /// The newest frame of `path` drawn at `px` physical pixels, starting it if needed. None
    /// while it starts, when it can't play, or when `MAX_PLAYING` others are on screen.
    pub fn frame(&mut self, ctx: &egui::Context, path: &Path, px: [f32; 2]) -> Option<TextureHandle> {
        if !super::videos_available() || self.failed.iter().any(|f| f == path) {
            return None;
        }
        // Clear out ended helpers (a failed one is remembered).
        let failed = &mut self.failed;
        self.slots.retain(|p, s| {
            let done = s.shared.done.load(Ordering::Acquire);
            if done && s.shared.failed.load(Ordering::Relaxed) {
                log::info!("video: {} gave no picture (shown as a still instead)", p.display());
                failed.push(p.clone());
            }
            !done
        });
        if self.failed.iter().any(|f| f == path) {
            return None;
        }
        if !self.slots.contains_key(path) {
            if self.slots.len() >= MAX_PLAYING {
                // Replace the one unused longest, if it isn't on screen.
                let stalest = self
                    .slots
                    .iter()
                    .map(|(p, s)| (p.clone(), s.shared.idle_secs()))
                    .max_by(|a, b| a.1.total_cmp(&b.1))?;
                if stalest.1 < PAUSE_SECS {
                    return None;
                }
                if let Some(s) = self.slots.remove(&stalest.0) {
                    s.shared.stop.store(true, Ordering::Relaxed);
                }
            }
            let size = decode_size(px);
            let ffmpeg = if super::fake_video() { None } else { Some(super::ffmpeg()?) };
            let shared = Arc::new(Shared::default());
            shared.want();
            let sh = shared.clone();
            let c = ctx.clone();
            let file = path.to_path_buf();
            let spawned = std::thread::Builder::new().name("lodestar-video".into()).spawn(move || {
                let src: Option<Box<dyn Source>> = match ffmpeg {
                    None => Some(Box::new(FakeSource::new(size))),
                    Some(f) => spawn_ffmpeg(f, &file, size),
                };
                match src {
                    Some(src) => pump(src, &sh, FPS, &|| c.request_repaint()),
                    None => {
                        // ffmpeg didn't start: remembered as failed (the still shows).
                        sh.failed.store(true, Ordering::Relaxed);
                        sh.done.store(true, Ordering::Release);
                    }
                }
            });
            if spawned.is_err() {
                return None;
            }
            log::debug!("video: playing {} at {}x{}", path.display(), size[0], size[1]);
            self.slots.insert(path.to_path_buf(), Slot { shared, tex: None, seen: 0 });
        }
        let slot = self.slots.get_mut(path)?;
        slot.shared.want();
        let serial = slot.shared.serial.load(Ordering::Acquire);
        if serial != slot.seen
            && let Some(img) = slot.shared.frame.lock().take()
        {
            slot.seen = serial;
            match &mut slot.tex {
                Some(t) => t.set(img, TextureOptions::LINEAR),
                None => {
                    slot.tex = Some(ctx.load_texture(format!("video:{}", path.display()), img, TextureOptions::LINEAR))
                }
            }
        }
        slot.tex.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_sizes_are_small_and_keep_the_shape() {
        assert_eq!(decode_size([1920.0, 1080.0]), [960, 544]);
        assert_eq!(decode_size([3840.0, 2160.0]), [960, 544]);
        assert_eq!(decode_size([400.0, 300.0]), [400, 304]);
        assert_eq!(decode_size([0.0, 0.0]), [16, 16]);
        let [w, h] = decode_size([500.0, 1400.0]);
        assert!(h <= MAX_H + 8 && w < h, "{w}x{h}");
    }

    #[test]
    fn ffmpeg_argv() {
        let a = ffmpeg_args(Path::new("/t/my theme/bg.mp4"), [640, 368], 24);
        let s: Vec<String> = a.iter().map(|x| x.to_string_lossy().into_owned()).collect();
        let at = |k: &str| s.iter().position(|x| x == k).unwrap();
        assert_eq!(s[at("-i") + 1], "/t/my theme/bg.mp4", "the path is one argument, spaces kept");
        assert!(at("-stream_loop") < at("-i"), "looping is an input option");
        assert_eq!(s[at("-loglevel") + 1], "quiet");
        assert!(s.contains(&"-an".to_string()), "no audio");
        assert!(s[at("-vf") + 1].starts_with("fps=24,scale=640:368:force_original_aspect_ratio=decrease"));
        assert_eq!(s[s.len() - 5..], ["-f", "image2pipe", "-c:v", "ppm", "-"]);
    }

    #[test]
    fn pump_publishes_frames_and_stops_when_unused() {
        let sh = Arc::new(Shared::default());
        sh.want();
        let count = Arc::new(AtomicU64::new(0));
        let (s2, c2) = (sh.clone(), count.clone());
        let t = std::thread::spawn(move || {
            pump(Box::new(FakeSource::new([32, 16])), &s2, 200, &|| {
                c2.fetch_add(1, Ordering::Relaxed);
            })
        });
        let t0 = std::time::Instant::now();
        while count.load(Ordering::Relaxed) < 3 && t0.elapsed() < Duration::from_secs(5) {
            sh.want();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(count.load(Ordering::Relaxed) >= 3, "frames arrive");
        let img = sh.frame.lock().clone().expect("a frame");
        assert_eq!(img.size, [32, 16]);
        sh.stop.store(true, Ordering::Relaxed);
        t.join().unwrap();
        assert!(sh.done.load(Ordering::Acquire));
        assert!(!sh.failed.load(Ordering::Relaxed));
    }

    #[test]
    fn a_missing_ffmpeg_gives_no_source() {
        assert!(spawn_ffmpeg(Path::new("/nonexistent/ffmpeg"), Path::new("/t/bg.mp4"), [64, 32]).is_none());
    }

    #[test]
    fn a_source_without_frames_fails() {
        struct Empty(std::io::Empty);
        impl Source for Empty {
            fn reader(&mut self) -> &mut dyn Read {
                &mut self.0
            }
        }
        let sh = Shared::default();
        sh.want();
        pump(Box::new(Empty(std::io::empty())), &sh, 24, &|| {});
        assert!(sh.done.load(Ordering::Acquire) && sh.failed.load(Ordering::Relaxed));
    }

    #[test]
    fn unused_videos_pause_without_reading() {
        struct Panics;
        impl Read for Panics {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                panic!("read while paused")
            }
        }
        impl Source for Panics {
            fn reader(&mut self) -> &mut dyn Read {
                self
            }
        }
        while super::super::clock() < PAUSE_SECS + 0.1 {
            std::thread::sleep(Duration::from_millis(20));
        }
        // Last drawn at clock 0: paused, so the source is never read.
        let sh = Arc::new(Shared::default());
        let s2 = sh.clone();
        let t = std::thread::spawn(move || pump(Box::new(Panics), &s2, 24, &|| {}));
        std::thread::sleep(Duration::from_millis(150));
        sh.stop.store(true, Ordering::Relaxed);
        t.join().expect("paused: never reads");
        assert!(!sh.failed.load(Ordering::Relaxed));
    }
}
