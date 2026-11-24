//! Worker-side image pipeline: disk thumbnail cache (QOI), raster decode, SVG rasterisation and
//! downscaling. Everything here runs on decode threads, never on the UI thread.
//!
//! Cache format: `<thumbs_dir>/<key:016x>.qoi`, key = fnv1a(path, mtime, file size, target px,
//! format version). Pixels are stored RGB (opaque images) or RGBA with PREMULTIPLIED alpha —
//! egui's native format, so a cache hit is a QOI decode plus a straight copy.

use egui::{Color32, ColorImage};
use image::imageops::FilterType;
use image::{ImageBuffer, ImageEncoder, ImageFormat, Pixel};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// Bump when the cached pixel format changes so old files are ignored (they get pruned).
const CACHE_VERSION: u32 = 1;
/// Source files larger than this are refused.
const MAX_SOURCE_BYTES: u64 = 128 * 1024 * 1024;
/// Images with at most this many pixels that need no downscaling are not cached (decoding the
/// original is already about as fast as reading a QOI).
const SMALL_PIXELS: u64 = 256 * 256;
/// Disk cache pruning thresholds.
pub const DISK_CACHE_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const DISK_CACHE_TARGET_BYTES: u64 = 3 * 512 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// File not found / not readable.
    Missing(String),
    TooLarge,
    /// Not an image format we know.
    Unsupported,
    Decode(String),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Missing(e) => write!(f, "can't read file: {e}"),
            DecodeError::TooLarge => write!(f, "file too large"),
            DecodeError::Unsupported => write!(f, "unsupported image format"),
            DecodeError::Decode(e) => write!(f, "decode failed: {e}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decoded pixels, either opaque RGB or premultiplied RGBA.
pub(crate) struct Pixels {
    pub w: u32,
    pub h: u32,
    /// 3 (RGB) or 4 (premultiplied RGBA).
    pub channels: u8,
    pub data: Vec<u8>,
}

impl Pixels {
    pub fn into_color_image(self) -> ColorImage {
        let size = [self.w as usize, self.h as usize];
        let px: Vec<Color32> = if self.channels == 3 {
            self.data.as_chunks::<3>().0.iter().map(|p| Color32::from_rgb(p[0], p[1], p[2])).collect()
        } else {
            self.data
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| Color32::from_rgba_premultiplied(p[0], p[1], p[2], p[3]))
                .collect()
        };
        if px.len() != size[0] * size[1] {
            // Can't happen for well-formed buffers; never panic in egui on a bad one.
            return ColorImage::filled([1, 1], Color32::TRANSPARENT);
        }
        ColorImage::new(size, px)
    }
}

/// Path of the cached thumbnail for `path` at `target_px`, or None when the file is missing.
#[cfg(test)]
pub fn thumb_cache_path(dir: &Path, path: &Path, target_px: u32) -> Option<PathBuf> {
    let meta = std::fs::metadata(path).ok()?;
    Some(cache_file(dir, path, &meta, target_px))
}

fn cache_file(dir: &Path, path: &Path, meta: &std::fs::Metadata, target_px: u32) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut buf = Vec::with_capacity(path.as_os_str().len() + 24);
    buf.extend_from_slice(path.as_os_str().as_bytes());
    buf.push(0);
    buf.extend_from_slice(&mtime.to_le_bytes());
    buf.extend_from_slice(&meta.len().to_le_bytes());
    buf.extend_from_slice(&target_px.to_le_bytes());
    buf.extend_from_slice(&CACHE_VERSION.to_le_bytes());
    let key = crate::db::fnv1a(&buf);
    dir.join(format!("{key:016x}.qoi"))
}

/// Load `path` downscaled so its longest side is at most `target_px` (raster images are never
/// upscaled; SVGs are rendered with their longest side exactly `target_px`). Uses and fills the
/// disk cache in `cache_dir`.
pub fn load_image(cache_dir: &Path, path: &Path, target_px: u32) -> Result<ColorImage, DecodeError> {
    load_pixels(cache_dir, path, target_px).map(Pixels::into_color_image)
}

/// Load `path` at its own resolution (only downscaled past `max_px`, the GPU's texture limit),
/// for the artwork viewer. Never reads or writes the disk cache: full-size copies would only
/// fill it, and decoding the original is what this is for. SVGs render at [`SVG_ORIGINAL_PX`].
pub fn load_original(path: &Path, max_px: u32) -> Result<ColorImage, DecodeError> {
    let meta = std::fs::metadata(path).map_err(|e| DecodeError::Missing(e.to_string()))?;
    if !meta.is_file() {
        return Err(DecodeError::Missing("not a file".into()));
    }
    if meta.len() > MAX_SOURCE_BYTES {
        return Err(DecodeError::TooLarge);
    }
    let bytes = std::fs::read(path).map_err(|e| DecodeError::Missing(e.to_string()))?;
    let max_px = max_px.max(1);
    let pixels = if is_svg(path, &bytes) {
        render_svg(path, &bytes, SVG_ORIGINAL_PX.min(max_px))?
    } else {
        decode_raster(path, &bytes, max_px)?.0
    };
    Ok(pixels.into_color_image())
}

/// Longest side an SVG is rendered at for the artwork viewer (it has no pixel size of its own).
pub const SVG_ORIGINAL_PX: u32 = 2048;

/// Pixel size of an image file from its header (no full decode); None for SVGs and files the
/// `image` crate can't read.
pub fn image_dimensions(path: &Path) -> Option<(u32, u32)> {
    image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.into_dimensions().ok()
}

pub(crate) fn load_pixels(cache_dir: &Path, path: &Path, target_px: u32) -> Result<Pixels, DecodeError> {
    let meta = std::fs::metadata(path).map_err(|e| DecodeError::Missing(e.to_string()))?;
    if !meta.is_file() {
        return Err(DecodeError::Missing("not a file".into()));
    }
    if meta.len() > MAX_SOURCE_BYTES {
        return Err(DecodeError::TooLarge);
    }
    let target_px = target_px.max(1);
    let cached = cache_file(cache_dir, path, &meta, target_px);
    if let Ok(bytes) = std::fs::read(&cached) {
        match decode_qoi(&bytes) {
            Some(p) => {
                touch_if_old(&cached);
                return Ok(p);
            }
            None => {
                let _ = std::fs::remove_file(&cached);
            }
        }
    }

    let bytes = std::fs::read(path).map_err(|e| DecodeError::Missing(e.to_string()))?;
    let (pixels, cache_it) = if is_svg(path, &bytes) {
        (render_svg(path, &bytes, target_px)?, true)
    } else {
        decode_raster(path, &bytes, target_px)?
    };
    if cache_it {
        write_qoi_atomic(&cached, &pixels);
    }
    Ok(pixels)
}

fn is_svg(path: &Path, bytes: &[u8]) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
    if matches!(ext.as_deref(), Some("svg" | "svgz")) {
        return true;
    }
    let head = &bytes[..bytes.len().min(512)];
    let head = head.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(head);
    let start = head.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(head.len());
    let head = &head[start..];
    head.starts_with(b"<svg") || (head.starts_with(b"<?xml") && head.windows(4).any(|w| w == b"<svg"))
}

/// Decode a raster image, downscale, convert to RGB / premultiplied RGBA.
/// Returns the pixels and whether they are worth caching.
fn decode_raster(path: &Path, bytes: &[u8], target_px: u32) -> Result<(Pixels, bool), DecodeError> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| DecodeError::Decode(e.to_string()))?;
    if reader.format().is_none() {
        match ImageFormat::from_path(path) {
            Ok(f) => reader.set_format(f),
            Err(_) => return Err(DecodeError::Unsupported),
        }
    }
    let format = reader.format();
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(1024 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| match e {
        image::ImageError::Unsupported(_) => DecodeError::Unsupported,
        e => DecodeError::Decode(e.to_string()),
    })?;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return Err(DecodeError::Decode("empty image".into()));
    }
    let (nw, nh) = fit(w, h, target_px);
    let downscale = (nw, nh) != (w, h);

    let mut px = if img.color().has_alpha() {
        let rgba = img.into_rgba8();
        if rgba.as_raw().as_chunks::<4>().0.iter().all(|p| p[3] == 255) {
            let data: Vec<u8> = rgba.as_raw().as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
            Pixels { w, h, channels: 3, data }
        } else {
            let mut data = rgba.into_raw();
            premultiply(&mut data);
            Pixels { w, h, channels: 4, data }
        }
    } else {
        Pixels { w, h, channels: 3, data: img.into_rgb8().into_raw() }
    };
    if downscale {
        px = resize(px, nw, nh);
    }
    let small = (w as u64) * (h as u64) <= SMALL_PIXELS || format == Some(ImageFormat::Qoi);
    Ok((px, downscale || !small))
}

/// Longest side to at most `target`, keeping the aspect ratio. Never upscales.
pub(crate) fn fit(w: u32, h: u32, target: u32) -> (u32, u32) {
    let m = w.max(h);
    if m <= target {
        return (w, h);
    }
    let s = target as f64 / m as f64;
    let nw = ((w as f64 * s).round() as u32).clamp(1, target);
    let nh = ((h as f64 * s).round() as u32).clamp(1, target);
    (nw, nh)
}

fn premultiply(rgba: &mut [u8]) {
    for p in rgba.as_chunks_mut::<4>().0 {
        let a = p[3] as u32;
        if a != 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
        }
    }
}

/// Two-pass downscale: integer box filter to within 2x of the target (cheap, removes aliasing
/// for big ratios), then a Triangle (bilinear) resample to the exact size.
pub(crate) fn resize(px: Pixels, nw: u32, nh: u32) -> Pixels {
    let k = (px.w / nw.max(1)).min(px.h / nh.max(1));
    let px = if k >= 2 { box_down(&px, k) } else { px };
    if (px.w, px.h) == (nw, nh) {
        return px;
    }
    let ch = px.channels;
    let data = if ch == 3 {
        triangle::<image::Rgb<u8>>(px.w, px.h, px.data, nw, nh)
    } else {
        triangle::<image::Rgba<u8>>(px.w, px.h, px.data, nw, nh)
    };
    match data {
        Some(data) => Pixels { w: nw, h: nh, channels: ch, data },
        None => Pixels { w: 1, h: 1, channels: ch, data: vec![0; ch as usize] },
    }
}

fn triangle<P: Pixel<Subpixel = u8> + 'static>(w: u32, h: u32, data: Vec<u8>, nw: u32, nh: u32) -> Option<Vec<u8>> {
    let buf: ImageBuffer<P, Vec<u8>> = ImageBuffer::from_raw(w, h, data)?;
    Some(image::imageops::resize(&buf, nw, nh, FilterType::Triangle).into_raw())
}

/// Average k×k blocks (partial blocks at the right/bottom edges are averaged over what exists).
fn box_down(px: &Pixels, k: u32) -> Pixels {
    let c = px.channels as usize;
    let (w, h) = (px.w as usize, px.h as usize);
    let k = k as usize;
    let (ow, oh) = (w.div_ceil(k), h.div_ceil(k));
    let mut out = vec![0u8; ow * oh * c];
    let mut acc = vec![0u32; ow * c];
    for oy in 0..oh {
        acc.iter_mut().for_each(|a| *a = 0);
        let y0 = oy * k;
        let y1 = (y0 + k).min(h);
        for y in y0..y1 {
            let row = &px.data[y * w * c..(y + 1) * w * c];
            for (x, p) in row.chunks_exact(c).enumerate() {
                let a = &mut acc[(x / k) * c..(x / k) * c + c];
                for i in 0..c {
                    a[i] += p[i] as u32;
                }
            }
        }
        let rows = (y1 - y0) as u32;
        let orow = &mut out[oy * ow * c..(oy + 1) * ow * c];
        for ox in 0..ow {
            let cols = ((ox * k + k).min(w) - ox * k) as u32;
            let n = rows * cols;
            for i in 0..c {
                orow[ox * c + i] = ((acc[ox * c + i] + n / 2) / n) as u8;
            }
        }
    }
    Pixels { w: ow as u32, h: oh as u32, channels: px.channels, data: out }
}

fn render_svg(path: &Path, bytes: &[u8], target_px: u32) -> Result<Pixels, DecodeError> {
    use resvg::{tiny_skia, usvg};
    let opt = usvg::Options { resources_dir: path.parent().map(Path::to_path_buf), ..Default::default() };
    let tree = usvg::Tree::from_data(bytes, &opt).map_err(|e| DecodeError::Decode(e.to_string()))?;
    let size = tree.size();
    let (sw, sh) = (size.width(), size.height());
    if !(sw.is_finite() && sh.is_finite()) || sw <= 0.0 || sh <= 0.0 {
        return Err(DecodeError::Decode("svg has no size".into()));
    }
    let scale = target_px as f32 / sw.max(sh);
    let w = ((sw * scale).round() as u32).clamp(1, target_px);
    let h = ((sh * scale).round() as u32).clamp(1, target_px);
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or_else(|| DecodeError::Decode("pixmap".into()))?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    // tiny-skia pixels are premultiplied RGBA already.
    Ok(Pixels { w, h, channels: 4, data: pixmap.take() })
}

fn decode_qoi(bytes: &[u8]) -> Option<Pixels> {
    let img = image::load_from_memory_with_format(bytes, ImageFormat::Qoi).ok()?;
    let (w, h) = (img.width(), img.height());
    match img {
        image::DynamicImage::ImageRgb8(b) => Some(Pixels { w, h, channels: 3, data: b.into_raw() }),
        image::DynamicImage::ImageRgba8(b) => Some(Pixels { w, h, channels: 4, data: b.into_raw() }),
        _ => None,
    }
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write temp file + rename, so readers never see a partial thumbnail.
pub(crate) fn write_qoi_atomic(dest: &Path, px: &Pixels) {
    let Some(dir) = dest.parent() else { return };
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = dest.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.{n}.tmp", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let f = std::fs::File::create(&tmp)?;
        let mut w = std::io::BufWriter::with_capacity(256 * 1024, f);
        let ct = if px.channels == 3 { image::ExtendedColorType::Rgb8 } else { image::ExtendedColorType::Rgba8 };
        image::codecs::qoi::QoiEncoder::new(&mut w)
            .write_image(&px.data, px.w, px.h, ct)
            .map_err(std::io::Error::other)?;
        w.flush()?;
        drop(w);
        std::fs::rename(&tmp, dest)
    })();
    if let Err(e) = result {
        log::debug!("thumb cache write {}: {e}", dest.display());
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Refresh the mtime of a cache file used again (approximate LRU for pruning). Only touches
/// files older than a day so hits normally cost no write.
fn touch_if_old(p: &Path) {
    const DAY: Duration = Duration::from_secs(86_400);
    let Ok(m) = std::fs::metadata(p).and_then(|m| m.modified()) else {
        return;
    };
    if SystemTime::now().duration_since(m).unwrap_or_default() > DAY
        && let Ok(f) = std::fs::File::options().write(true).open(p)
    {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// Delete stale temp files, and the oldest thumbnails when the cache exceeds `max_bytes`
/// (down to `target_bytes`). Returns the number of files removed.
pub fn prune_disk_cache(dir: &Path, max_bytes: u64, target_bytes: u64) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    let now = SystemTime::now();
    let mut files: Vec<(SystemTime, u64, PathBuf)> = vec![];
    let mut total = 0u64;
    let mut removed = 0;
    for e in rd.flatten() {
        let p = e.path();
        let Ok(m) = e.metadata() else { continue };
        if !m.is_file() {
            continue;
        }
        let mtime = m.modified().unwrap_or(now);
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".tmp") {
            if now.duration_since(mtime).unwrap_or_default() > Duration::from_secs(600)
                && std::fs::remove_file(&p).is_ok()
            {
                removed += 1;
            }
            continue;
        }
        if name.ends_with(".qoi") {
            total += m.len();
            files.push((mtime, m.len(), p));
        }
    }
    if total <= max_bytes {
        return removed;
    }
    files.sort_by_key(|f| f.0);
    for (_, len, p) in files {
        if total <= target_bytes {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total = total.saturating_sub(len);
            removed += 1;
        }
    }
    removed
}
