//! Artwork viewer core, shared by the desktop viewer (ui/desktop/viewer.rs) and controller mode
//! (ui/controller/viewer.rs): which images a game has, their pixel sizes (read from the file
//! headers on a background thread), the texture to show while the full-size one loads, and the
//! zoom / pan math.
//!
//! Zoom state ([`Zoom`]) is in points per image pixel plus the image point shown at the centre of
//! the viewing area, so zooming around the cursor and panning are a few lines each; `one` is the
//! scale at which one image pixel is one screen pixel (`1 / pixels_per_point`), i.e. 100 %.

use crate::art::ArtSlot;
use crate::images::{ImageCache, ImgSize};
use crate::model::Art;
use egui::{Pos2, Rect, TextureHandle, Vec2, vec2};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// One image the viewer can show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtEntry {
    pub slot: ArtSlot,
    pub path: PathBuf,
}

/// The game's artwork in viewer order (cover, background, logo, icon, banner): set slots only,
/// a file used by several slots once (under the first).
pub fn art_list(art: &Art) -> Vec<ArtEntry> {
    let mut out: Vec<ArtEntry> = Vec::new();
    for slot in ArtSlot::ALL {
        if let Some(p) = slot.get(art)
            && !p.as_os_str().is_empty()
            && !out.iter().any(|e| &e.path == p)
        {
            out.push(ArtEntry { slot, path: p.clone() });
        }
    }
    out
}

/// `art_list` without the files that are gone (one `stat` each, when the viewer opens).
pub fn existing_art(art: &Art) -> Vec<ArtEntry> {
    let mut v = art_list(art);
    v.retain(|e| e.path.is_file());
    v
}

/// Where to start: the entry for `slot`, else the first one.
pub fn start_index(entries: &[ArtEntry], slot: ArtSlot) -> usize {
    entries.iter().position(|e| e.slot == slot).unwrap_or(0)
}

/// The slot the game page's background shows (hero, else banner, else cover), as in
/// `paint_hero`: the hero magnifier opens on that one.
pub fn background_slot(art: &Art) -> ArtSlot {
    if art.hero.is_some() {
        ArtSlot::Hero
    } else if art.banner.is_some() {
        ArtSlot::Banner
    } else {
        ArtSlot::Cover
    }
}

/// `i` moved by `delta` with wrap-around in a list of `n`.
pub fn step_index(i: usize, delta: i32, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    (i as i64 + delta as i64).rem_euclid(n as i64) as usize
}

/// Pixel sizes of the viewer's files, read from their headers on one background thread.
#[derive(Clone, Default)]
pub struct Dims(Arc<Mutex<DimsMap>>);

/// File → its (width, height), None when the header couldn't be read.
type DimsMap = HashMap<PathBuf, Option<(u32, u32)>>;

impl Dims {
    pub fn spawn(ctx: &egui::Context, paths: Vec<PathBuf>) -> Dims {
        let d = Dims::default();
        let map = d.0.clone();
        let ctx = ctx.clone();
        let r = std::thread::Builder::new().name("art-dims".into()).spawn(move || {
            for p in paths {
                let dims = crate::images::image_dimensions(&p);
                map.lock().insert(p, dims);
            }
            ctx.request_repaint();
        });
        if let Err(e) = r {
            log::warn!("art viewer: {e}");
        }
        d
    }

    /// None while unknown (not read yet, or not readable: an SVG).
    pub fn get(&self, p: &Path) -> Option<(u32, u32)> {
        self.0.lock().get(p).copied().flatten()
    }
}

/// What to draw for an entry this frame.
pub struct Shown {
    /// The full-size texture, or the medium one while that loads.
    pub tex: Option<TextureHandle>,
    /// The full-size one isn't there yet.
    pub loading: bool,
    /// The full-size one couldn't be decoded (`tex` is the medium one, if that worked).
    pub failed: bool,
}

/// The full-size texture of `path`, with the (usually already cached) medium one standing in
/// while it decodes.
pub fn shown(images: &mut ImageCache, path: &Path) -> Shown {
    if let Some(t) = images.get(path, ImgSize::Original) {
        return Shown { tex: Some(t), loading: false, failed: false };
    }
    if images.is_failed(path, ImgSize::Original) {
        // E.g. a file too large to decode in full: the medium picture is still better than
        // nothing.
        return Shown { tex: images.get(path, ImgSize::Medium), loading: false, failed: true };
    }
    Shown { tex: images.get(path, ImgSize::Medium), loading: true, failed: false }
}

/// The image's size in pixels: the header's, else the texture's (an SVG, or before the header
/// was read; a stand-in medium texture is scaled up to the header size when that is known).
pub fn image_size(header: Option<(u32, u32)>, tex: Option<&TextureHandle>) -> Option<Vec2> {
    match (header, tex) {
        (Some((w, h)), _) if w > 0 && h > 0 => Some(vec2(w as f32, h as f32)),
        (_, Some(t)) => Some(t.size_vec2()).filter(|s| s.x > 0.0 && s.y > 0.0),
        _ => None,
    }
}

/// Fit never enlarges past this many times 100 % (small art stays sharp enough, not mush).
pub const MAX_FIT_UPSCALE: f32 = 2.0;
/// Zooming in stops at this many times 100 % …
pub const MAX_ZOOM: f32 = 16.0;

/// Scale (points per image pixel) that fits `img` into `area`.
pub fn fit_scale(img: Vec2, area: Vec2, one: f32) -> f32 {
    if img.x <= 0.0 || img.y <= 0.0 {
        return one;
    }
    (area.x / img.x).min(area.y / img.y).min(one * MAX_FIT_UPSCALE).max(f32::MIN_POSITIVE)
}

/// Largest scale: 16× 100 %, or twice the fit for huge-window / tiny-image cases.
pub fn max_scale(img: Vec2, area: Vec2, one: f32) -> f32 {
    (one * MAX_ZOOM).max(fit_scale(img, area, one) * 2.0)
}

/// A zoomed view (`None` in the viewers = fit, which follows the window size).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zoom {
    /// Points per image pixel.
    pub scale: f32,
    /// Image point (pixels) shown at the centre of the area.
    pub center: Vec2,
}

impl Zoom {
    pub fn fit(img: Vec2, area: Vec2, one: f32) -> Zoom {
        Zoom { scale: fit_scale(img, area, one), center: img / 2.0 }
    }

    /// The image's rect on screen.
    pub fn image_rect(&self, area: Rect, img: Vec2) -> Rect {
        let min = area.center() - self.center * self.scale;
        Rect::from_min_size(min, img * self.scale)
    }

    /// Keep the image covering the area where it is larger than it, centred where smaller.
    pub fn clamp(&mut self, area: Vec2, img: Vec2) {
        for k in 0..2 {
            let half = area[k] / 2.0 / self.scale;
            self.center[k] =
                if img[k] * self.scale <= area[k] { img[k] / 2.0 } else { self.center[k].clamp(half, img[k] - half) };
        }
    }

    /// Scale by `factor` keeping the image point under `cursor` where it is. None when that
    /// reaches or crosses the fit, i.e. back to fit. Smaller than the fit only goes as far as
    /// 100 % (an image the fit enlarged). `area` is what the fit fills; `view` (centred on it, at
    /// least as large) is what a zoomed image may be panned across.
    pub fn zoom_at(self, factor: f32, cursor: Pos2, area: Rect, view: Vec2, img: Vec2, one: f32) -> Option<Zoom> {
        let fit = fit_scale(img, area.size(), one);
        let scale = (self.scale * factor).min(max_scale(img, area.size(), one)).max(fit.min(one));
        let eps = fit * 1e-4;
        let crossed = (self.scale > fit + eps && scale <= fit + eps) || (self.scale < fit - eps && scale >= fit - eps);
        if crossed || (scale - fit).abs() <= eps {
            return None;
        }
        let off = cursor - area.center();
        let under = self.center + off / self.scale;
        let mut z = Zoom { scale, center: under - off / scale };
        z.clamp(view, img);
        Some(z)
    }

    /// Move the view by a drag of `delta` points.
    pub fn pan(&mut self, delta: Vec2, area: Vec2, img: Vec2) {
        self.center -= delta / self.scale;
        self.clamp(area, img);
    }
}

/// Wheel / pinch zoom from fit or a zoomed view.
pub fn zoom_by(
    z: Option<Zoom>,
    factor: f32,
    cursor: Pos2,
    area: Rect,
    view: Vec2,
    img: Vec2,
    one: f32,
) -> Option<Zoom> {
    if (factor - 1.0).abs() < 1e-4 {
        return z;
    }
    z.unwrap_or_else(|| Zoom::fit(img, area.size(), one)).zoom_at(factor, cursor, area, view, img, one)
}

/// Double-click: fit → 100 % around `cursor` → fit. When the fit already is about 100 % it goes
/// to 200 % instead, so the click always does something; a small image the fit enlarged goes to
/// its real size. `view` as in [`Zoom::zoom_at`].
pub fn toggle_100(z: Option<Zoom>, cursor: Pos2, area: Rect, view: Vec2, img: Vec2, one: f32) -> Option<Zoom> {
    if z.is_some() {
        return None;
    }
    let fit = Zoom::fit(img, area.size(), one);
    let target = if (fit.scale - one).abs() <= one * 0.05 { one * 2.0 } else { one };
    let off = cursor - area.center();
    let under = fit.center + off / fit.scale;
    let mut z = Zoom { scale: target, center: under - off / target };
    z.clamp(view, img);
    Some(z)
}

/// A drag of `total` points on a fitted (not zoomed) picture in a viewer `width` points wide:
/// -1 = previous picture (dragged to the right, like turning a page back), 1 = next, 0 = not a
/// swipe (too short, or more vertical than horizontal).
pub fn swipe_step(total: Vec2, width: f32) -> i32 {
    let min = (width * 0.06).clamp(40.0, 120.0);
    if total.x.abs() < min || total.x.abs() < total.y.abs() * 1.5 {
        0
    } else if total.x > 0.0 {
        -1
    } else {
        1
    }
}

/// Current scale as a percentage of 100 % ("37 %").
pub fn percent(scale: f32, one: f32) -> String {
    format!("{:.0} %", (scale / one.max(f32::MIN_POSITIVE) * 100.0).round())
}

/// Controller zoom: fit ↔ a closer look at the centre (100 %, or twice the fit when that is
/// already about 100 %).
pub fn toggle_centered(z: Option<Zoom>, area: Rect, view: Vec2, img: Vec2, one: f32) -> Option<Zoom> {
    toggle_100(z, area.center(), area, view, img, one)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    fn art(paths: [Option<&str>; 5]) -> Art {
        let p = |i: usize| paths[i].map(PathBuf::from);
        Art { cover: p(0), hero: p(1), logo: p(2), icon: p(3), banner: p(4) }
    }

    #[test]
    fn art_list_keeps_order_skips_missing_and_duplicates() {
        let a = art([Some("/c.jpg"), None, Some("/l.png"), Some("/c.jpg"), Some("/b.jpg")]);
        let v = art_list(&a);
        let slots: Vec<ArtSlot> = v.iter().map(|e| e.slot).collect();
        assert_eq!(slots, vec![ArtSlot::Cover, ArtSlot::Logo, ArtSlot::Banner]);
        assert_eq!(v[0].path, PathBuf::from("/c.jpg"));
        assert!(art_list(&Art::default()).is_empty());
        // An empty path is no artwork.
        assert!(art_list(&art([Some(""), None, None, None, None])).is_empty());
    }

    #[test]
    fn existing_art_drops_files_that_are_gone() {
        let tmp = tempfile::tempdir().unwrap();
        let there = tmp.path().join("cover.png");
        std::fs::write(&there, b"x").unwrap();
        let gone = tmp.path().join("hero.png");
        let a = Art { cover: Some(there.clone()), hero: Some(gone), ..Default::default() };
        let v = existing_art(&a);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].path, there);
    }

    #[test]
    fn start_and_step_indices() {
        let a = art([Some("/c"), Some("/h"), Some("/l"), None, None]);
        let v = art_list(&a);
        assert_eq!(start_index(&v, ArtSlot::Logo), 2);
        assert_eq!(start_index(&v, ArtSlot::Icon), 0);
        assert_eq!(step_index(0, -1, 3), 2);
        assert_eq!(step_index(2, 1, 3), 0);
        assert_eq!(step_index(1, 1, 3), 2);
        assert_eq!(step_index(0, 1, 0), 0);
        assert_eq!(background_slot(&a), ArtSlot::Hero);
        assert_eq!(background_slot(&art([Some("/c"), None, None, None, Some("/b")])), ArtSlot::Banner);
        assert_eq!(background_slot(&art([Some("/c"), None, None, None, None])), ArtSlot::Cover);
    }

    #[test]
    fn fit_scale_fits_and_caps_upscaling() {
        // 1920×620 into 960×800 at 1 pt/px: width-bound, half size.
        assert!((fit_scale(vec2(1920.0, 620.0), vec2(960.0, 800.0), 1.0) - 0.5).abs() < 1e-6);
        // A 32 px icon in a big area: at most 2× 100 %.
        assert!((fit_scale(vec2(32.0, 32.0), vec2(1000.0, 1000.0), 1.0) - 2.0).abs() < 1e-6);
        // HiDPI (2 px per point): 100 % is 0.5 pt/px, so the cap is 1 pt/px.
        assert!((fit_scale(vec2(32.0, 32.0), vec2(1000.0, 1000.0), 0.5) - 1.0).abs() < 1e-6);
        // A degenerate image doesn't divide by zero.
        assert_eq!(fit_scale(vec2(0.0, 10.0), vec2(100.0, 100.0), 1.0), 1.0);
    }

    #[test]
    fn zoom_keeps_the_point_under_the_cursor() {
        let area = Rect::from_min_size(pos2(100.0, 50.0), vec2(800.0, 600.0));
        let img = vec2(3000.0, 2000.0);
        let cursor = pos2(300.0, 200.0);
        let fit = Zoom::fit(img, area.size(), 1.0);
        let under = |z: &Zoom| z.center + (cursor - area.center()) / z.scale;
        let before = under(&fit);
        let z = zoom_by(None, 3.0, cursor, area, area.size(), img, 1.0).expect("zoomed in");
        assert!((z.scale - fit.scale * 3.0).abs() < 1e-4);
        let after = under(&z);
        assert!((before - after).length() < 0.01, "{before:?} vs {after:?}");
        // The image point is drawn under the cursor.
        let r = z.image_rect(area, img);
        let drawn = r.min + after * z.scale;
        assert!((drawn - cursor).length() < 0.01);
    }

    #[test]
    fn zooming_out_past_fit_returns_to_fit_and_zoom_in_is_capped() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let img = vec2(1600.0, 1200.0);
        let z = zoom_by(None, 2.0, area.center(), area, area.size(), img, 1.0).unwrap();
        assert_eq!(zoom_by(Some(z), 0.4, area.center(), area, area.size(), img, 1.0), None);
        assert_eq!(zoom_by(None, 0.5, area.center(), area, area.size(), img, 1.0), None);
        let huge = zoom_by(None, 1e6, area.center(), area, area.size(), img, 1.0).unwrap();
        assert!((huge.scale - MAX_ZOOM).abs() < 1e-3);
        // An enlarged small image: zooming out stops at 100 %, zooming back in snaps to fit.
        let small = vec2(100.0, 100.0);
        let real = toggle_100(None, area.center(), area, area.size(), small, 1.0).unwrap();
        let out = zoom_by(Some(real), 0.5, area.center(), area, area.size(), small, 1.0).unwrap();
        assert!((out.scale - 1.0).abs() < 1e-4);
        assert_eq!(zoom_by(Some(real), 4.0, area.center(), area, area.size(), small, 1.0), None);
        // factor 1 changes nothing.
        assert_eq!(zoom_by(Some(z), 1.0, pos2(5.0, 5.0), area, area.size(), img, 1.0), Some(z));
    }

    #[test]
    fn pan_is_clamped_to_the_image_edges() {
        let area = vec2(800.0, 600.0);
        let img = vec2(1600.0, 400.0);
        // At 1 pt/px the image is wider than the area but shorter: x pans, y stays centred.
        let mut z = Zoom { scale: 1.0, center: img / 2.0 };
        z.pan(vec2(10_000.0, 50.0), area, img);
        assert_eq!(z.center, vec2(400.0, 200.0)); // left edge at the area's left edge
        z.pan(vec2(-10_000.0, -50.0), area, img);
        assert_eq!(z.center, vec2(1200.0, 200.0));
        z.pan(vec2(100.0, 0.0), area, img);
        assert_eq!(z.center, vec2(1100.0, 200.0));
        // Panning across a wider view (the viewer's full window width): the image's edge stops
        // at the view's edge, not at the fit area's.
        let view = vec2(1000.0, 600.0);
        z.pan(vec2(10_000.0, 0.0), view, img);
        assert_eq!(z.center, vec2(500.0, 200.0));
        let a = Rect::from_center_size(pos2(500.0, 300.0), area);
        let r = z.image_rect(a, img);
        assert_eq!(r.left(), 0.0);
    }

    #[test]
    fn double_click_toggles_fit_and_100_percent() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let img = vec2(3840.0, 1240.0);
        let z = toggle_100(None, pos2(100.0, 300.0), area, area.size(), img, 1.0).unwrap();
        assert!((z.scale - 1.0).abs() < 1e-4);
        assert_eq!(toggle_100(Some(z), pos2(1.0, 1.0), area, area.size(), img, 1.0), None);
        // HiDPI: 100 % is one image pixel per screen pixel.
        let z = toggle_100(None, area.center(), area, area.size(), img, 0.5).unwrap();
        assert!((z.scale - 0.5).abs() < 1e-4);
        assert_eq!(percent(z.scale, 0.5), "100 %");
        // An image that already fits at 100 % goes to 200 % …
        let exact = vec2(800.0, 400.0);
        let z = toggle_100(None, area.center(), area, area.size(), exact, 1.0).unwrap();
        assert!((z.scale - 2.0).abs() < 1e-4);
        // … one the fit enlarged to its real size (centred: smaller than the area).
        let small = vec2(300.0, 450.0);
        let z = toggle_100(None, pos2(10.0, 10.0), area, area.size(), small, 1.0).unwrap();
        assert!((z.scale - 1.0).abs() < 1e-4);
        assert_eq!(z.center, small / 2.0);
        let c = toggle_centered(None, area, area.size(), img, 1.0).unwrap();
        assert_eq!(c.center, vec2(1920.0, 620.0));
    }

    #[test]
    fn swipe_needs_a_mostly_horizontal_drag_of_some_length() {
        // Dragging the picture to the right shows the previous one, to the left the next.
        assert_eq!(swipe_step(vec2(200.0, 10.0), 1400.0), -1);
        assert_eq!(swipe_step(vec2(-200.0, -30.0), 1400.0), 1);
        // Too short (6 % of the width, at least 40 points) …
        assert_eq!(swipe_step(vec2(80.0, 0.0), 1400.0), 0);
        assert_eq!(swipe_step(vec2(-45.0, 0.0), 500.0), 1);
        assert_eq!(swipe_step(vec2(-39.0, 0.0), 500.0), 0);
        // … or too vertical.
        assert_eq!(swipe_step(vec2(200.0, 150.0), 1400.0), 0);
        assert_eq!(swipe_step(Vec2::ZERO, 1400.0), 0);
    }

    #[test]
    fn a_drag_pans_a_zoomed_hero_across_the_window() {
        // The desktop viewer's geometry: a 1480×900 window, the fit area inset 76 / 64 / 70 pt,
        // the zoomed image panned across the full window width.
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1480.0, 900.0));
        let area = Rect::from_min_max(pos2(76.0, 64.0), pos2(1404.0, 830.0));
        let view = vec2(screen.width(), area.height());
        let img = vec2(1920.0, 620.0);
        let mut z = toggle_100(None, area.center(), area, view, img, 1.0).unwrap();
        let before = z.image_rect(area, img);
        z.pan(vec2(-300.0, -80.0), view, img);
        let after = z.image_rect(area, img);
        // Moved left with the pointer; vertically it fits, so it stays centred.
        assert!((after.left() - (before.left() - 220.0)).abs() < 0.01, "{before:?} → {after:?}");
        assert_eq!(after.center().y, before.center().y);
        // The right edge stops at the window's edge.
        assert!((after.right() - screen.right()).abs() < 0.01);
    }

    #[test]
    fn image_size_prefers_the_header() {
        assert_eq!(image_size(Some((300, 450)), None), Some(vec2(300.0, 450.0)));
        assert_eq!(image_size(None, None), None);
        assert_eq!(image_size(Some((0, 0)), None), None);
        assert_eq!(percent(0.5, 1.0), "50 %");
    }
}
