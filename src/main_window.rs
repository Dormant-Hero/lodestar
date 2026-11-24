//! The main window comes back as it was left: size and maximised state everywhere, and on X11
//! its position (so its monitor) too — never off-screen. Kept in the config as
//! `[desktop.windows.main]` (logical px, the size before maximising).
//!
//! Wayland: a program can't place its own window (the compositor picks position and monitor), so
//! only size and maximised come back; on KDE Plasma a KWin window rule ("Remember" position /
//! screen) keeps the place.
//!
//! Saved a moment after the user stops resizing / moving it, and on quit. Fullscreen (controller
//! mode), minimised and hidden-to-tray states are never saved; screenshot runs and `--size`
//! neither restore nor save.
//!
//! CONTRACT: `restore(builder, saved, wayland, monitors) -> ViewportBuilder` (pure but for what
//! it's given); `x11_monitors()` (one short X connection, empty on failure); `Tracker::track`
//! once per frame (cheap: a few comparisons).

use crate::config::WindowGeom;
use egui::{Pos2, Rect, Vec2, pos2, vec2};
use std::time::{Duration, Instant};

/// Key in `[desktop.windows]`.
pub const KEY: &str = "main";

/// The main window's minimum size (main.rs).
pub const MIN: Vec2 = vec2(900.0, 560.0);

/// The main window's size on first start (main.rs).
pub const DEFAULT: Vec2 = vec2(1480.0, 900.0);

/// Saved this long after the last change (a drag sends many).
const SAVE_AFTER: Duration = Duration::from_millis(1500);

/// Pixels per logical px when the geometry was saved (1 when unknown or nonsense).
fn scale_of(saved: &WindowGeom) -> f32 {
    saved.scale.filter(|k| k.is_finite() && (0.25..=8.0).contains(k)).unwrap_or(1.0)
}

/// The size to start with: the saved one, at least [`MIN`], at most the biggest monitor (when
/// known; monitors are in pixels, the size in logical px).
pub fn restore_size(saved: &WindowGeom, monitors: &[Rect]) -> Vec2 {
    let mut s = vec2(saved.w, saved.h).max(MIN);
    if let Some(big) = monitors.iter().map(|m| m.size()).reduce(|a, b| if a.x * a.y >= b.x * b.y { a } else { b }) {
        s = s.min((big / scale_of(saved)).max(MIN));
    }
    s
}

/// The saved position when the window's title bar would still be on one of the monitors (at
/// least 160×40 logical px of its top edge on one), else None: the window manager places it.
/// `size` and the place are logical px, `monitors` pixels (X11 RandR): compared at the scale
/// the place was saved with.
pub fn restore_position(saved: &WindowGeom, size: Vec2, monitors: &[Rect]) -> Option<Pos2> {
    let p = pos2(saved.x?, saved.y?);
    if !p.x.is_finite() || !p.y.is_finite() {
        return None;
    }
    let k = scale_of(saved);
    let bar = Rect::from_min_size(p, vec2(size.x, 40.0)) * k;
    monitors
        .iter()
        .any(|m| {
            let i = m.intersect(bar);
            i.is_positive() && i.width() >= 160.0_f32.min(size.x) * k && i.height() >= 39.0 * k
        })
        .then_some(p)
}

/// Apply the saved state to the startup builder.
pub fn restore(
    mut b: egui::ViewportBuilder,
    saved: Option<&WindowGeom>,
    wayland: bool,
    monitors: &[Rect],
) -> egui::ViewportBuilder {
    let Some(g) = saved else { return b };
    let size = restore_size(g, monitors);
    b = b.with_inner_size(size);
    if !wayland && let Some(p) = restore_position(g, size, monitors) {
        b = b.with_position(p);
    }
    if g.maximized {
        b = b.with_maximized(true);
    }
    b
}

/// The monitors as read at start (X11; empty on Wayland, where nothing is placed by Lodestar).
static MONITORS: std::sync::OnceLock<Vec<Rect>> = std::sync::OnceLock::new();

/// Remember the monitors read at start (main.rs).
pub fn set_monitors(m: Vec<Rect>) {
    let _ = MONITORS.set(m);
}

/// The monitors read at start (the dialog windows' remembered places are checked against them).
pub fn monitors() -> &'static [Rect] {
    MONITORS.get().map_or(&[], Vec::as_slice)
}

/// The X11 monitors (RandR), in pixels. Empty when there is no X display or it can't tell.
pub fn x11_monitors() -> Vec<Rect> {
    use x11rb::connection::Connection;
    use x11rb::protocol::randr::ConnectionExt as _;
    let Ok((conn, n)) = x11rb::connect(None) else { return vec![] };
    let root = conn.setup().roots[n].root;
    let Ok(reply) = conn.randr_get_monitors(root, true).map_err(|_| ()).and_then(|c| c.reply().map_err(|_| ())) else {
        // No RandR: the whole screen.
        let s = &conn.setup().roots[n];
        return vec![Rect::from_min_size(
            Pos2::ZERO,
            vec2(f32::from(s.width_in_pixels), f32::from(s.height_in_pixels)),
        )];
    };
    reply
        .monitors
        .iter()
        .map(|m| {
            Rect::from_min_size(pos2(f32::from(m.x), f32::from(m.y)), vec2(f32::from(m.width), f32::from(m.height)))
        })
        .collect()
}

/// What one frame tells about the main window (logical px).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Seen {
    pub size: Vec2,
    pub pos: Option<Pos2>,
    pub maximized: bool,
    /// Fullscreen, minimised or hidden: nothing to remember.
    pub skip: bool,
    /// Pixels per logical px (the desktop's scale).
    pub scale: Option<f32>,
}

/// The state to remember after a frame that saw `s`, given the remembered one.
pub fn next_geom(prev: Option<&WindowGeom>, s: &Seen) -> Option<WindowGeom> {
    if s.skip || s.size.x < 1.0 || s.size.y < 1.0 {
        return prev.copied();
    }
    if s.maximized {
        // Keep the size (and place) it had before maximising.
        // (Maximised from the start: un-maximising later gives the usual first size.)
        let mut g = prev.copied().unwrap_or(WindowGeom {
            w: DEFAULT.x,
            h: DEFAULT.y,
            x: None,
            y: None,
            maximized: false,
            scale: s.scale,
        });
        g.maximized = true;
        return Some(g);
    }
    Some(WindowGeom {
        w: s.size.x.round(),
        h: s.size.y.round(),
        x: s.pos.map(|p| p.x.round()),
        y: s.pos.map(|p| p.y.round()),
        maximized: false,
        scale: s.scale,
    })
}

/// Follows the main window and says when to save.
#[derive(Default)]
pub struct Tracker {
    changed_at: Option<Instant>,
}

impl Tracker {
    /// Once per frame: update `windows[KEY]`; true when it is time to write the config.
    pub fn track(&mut self, windows: &mut std::collections::BTreeMap<String, WindowGeom>, s: &Seen) -> bool {
        let prev = windows.get(KEY);
        let next = next_geom(prev, s);
        if next.as_ref() != prev
            && let Some(g) = next
        {
            windows.insert(KEY.to_string(), g);
            self.changed_at = Some(Instant::now());
        }
        if self.changed_at.is_some_and(|t| t.elapsed() >= SAVE_AFTER) {
            self.changed_at = None;
            return true;
        }
        false
    }

    /// A change not written yet (quit writes it).
    pub fn pending(&self) -> bool {
        self.changed_at.is_some()
    }

    /// How long until a change not written yet is due (the caller wakes up then: without other
    /// events no frame would come to write it).
    pub fn due_in(&self) -> Option<Duration> {
        self.changed_at.map(|t| SAVE_AFTER.saturating_sub(t.elapsed()))
    }
}

/// This frame's main window, read in its own pass.
pub fn seen(ctx: &egui::Context, hidden: bool) -> Seen {
    let zoom = ctx.zoom_factor();
    ctx.input(|i| {
        let v = i.viewport();
        Seen {
            size: i.content_rect().size() * zoom,
            pos: v.outer_rect.map(|r| (r.min.to_vec2() * zoom).to_pos2()),
            maximized: v.maximized == Some(true),
            skip: hidden || v.fullscreen == Some(true) || v.minimized == Some(true),
            scale: v.native_pixels_per_point,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(w: f32, h: f32, x: Option<f32>, y: Option<f32>, maximized: bool) -> WindowGeom {
        WindowGeom { w, h, x, y, maximized, scale: None }
    }

    fn mons() -> Vec<Rect> {
        // Two monitors side by side: 1920×1080 and a 2560×1440 to its right.
        vec![
            Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0)),
            Rect::from_min_size(pos2(1920.0, 0.0), vec2(2560.0, 1440.0)),
        ]
    }

    #[test]
    fn position_comes_back_on_a_monitor_only() {
        let size = vec2(1480.0, 900.0);
        // On the second monitor: kept (that's "in what screen").
        assert_eq!(
            restore_position(&g(1480.0, 900.0, Some(2100.0), Some(100.0), false), size, &mons()),
            Some(pos2(2100.0, 100.0))
        );
        // That monitor is gone (laptop undocked): off-screen → the window manager places it.
        let one = &mons()[..1];
        assert_eq!(restore_position(&g(1480.0, 900.0, Some(2100.0), Some(100.0), false), size, one), None);
        // Above / left of everything, or far below: never restored off-screen.
        assert_eq!(restore_position(&g(1480.0, 900.0, Some(100.0), Some(-500.0), false), size, &mons()), None);
        assert_eq!(restore_position(&g(1480.0, 900.0, Some(-1470.0), Some(100.0), false), size, &mons()), None);
        assert_eq!(restore_position(&g(1480.0, 900.0, Some(100.0), Some(1075.0), false), size, one), None);
        // Partly over the left edge with most of the title bar visible: kept.
        assert_eq!(
            restore_position(&g(1480.0, 900.0, Some(-200.0), Some(50.0), false), size, &mons()),
            Some(pos2(-200.0, 50.0))
        );
        // No position known (Wayland) or no monitors known: None.
        assert_eq!(restore_position(&g(1480.0, 900.0, None, None, false), size, &mons()), None);
        assert_eq!(restore_position(&g(1480.0, 900.0, Some(10.0), Some(10.0), false), size, &[]), None);
    }

    /// X11 with a desktop scale (Xft.dpi / WINIT_X11_SCALE_FACTOR): monitors come in pixels,
    /// the place in logical px — never restored off-screen because of the mix.
    #[test]
    fn hidpi_x11_place_is_checked_in_pixels() {
        let one = [Rect::from_min_size(pos2(0.0, 0.0), vec2(3840.0, 2160.0))];
        let size = vec2(1480.0, 900.0);
        let mut at = g(1480.0, 900.0, Some(2500.0), Some(100.0), false);
        // Unscaled: x = 2500 px is on the 3840 px monitor.
        assert_eq!(restore_position(&at, size, &one), Some(pos2(2500.0, 100.0)));
        // At scale 2 it is 5000 px: off-screen → the window manager places it.
        at.scale = Some(2.0);
        assert_eq!(restore_position(&at, size, &one), None);
        // A place on the monitor at scale 2 (1000 logical = 2000 px): kept.
        at.x = Some(1000.0);
        assert_eq!(restore_position(&at, size, &one), Some(pos2(1000.0, 100.0)));
        // The bar's last 160 logical px must be on it: 1840 logical = 3680 px, 160 px left: no.
        at.x = Some(1840.0);
        assert_eq!(restore_position(&at, size, &one), None);
        // Size: at most the monitor in logical px.
        let big = WindowGeom { w: 3000.0, h: 2000.0, ..at };
        assert_eq!(restore_size(&big, &one), vec2(1920.0, 1080.0));
        // Nonsense scales count as 1.
        assert_eq!(scale_of(&WindowGeom { scale: Some(f32::NAN), ..at }), 1.0);
        assert_eq!(scale_of(&WindowGeom { scale: Some(0.0), ..at }), 1.0);
        // The scale is remembered with the place.
        let s = Seen {
            size: vec2(1200.0, 800.0),
            pos: Some(pos2(10.0, 20.0)),
            maximized: false,
            skip: false,
            scale: Some(1.5),
        };
        assert_eq!(next_geom(None, &s).and_then(|g| g.scale), Some(1.5));
    }

    #[test]
    fn maximised_from_the_start_keeps_the_usual_size_for_later() {
        let s =
            Seen { size: vec2(1920.0, 1050.0), pos: Some(pos2(0.0, 0.0)), maximized: true, skip: false, scale: None };
        let g = next_geom(None, &s).unwrap();
        assert!(g.maximized);
        assert_eq!(vec2(g.w, g.h), DEFAULT);
    }

    #[test]
    fn tracker_says_when_to_wake_up() {
        let mut w = std::collections::BTreeMap::new();
        let mut t = Tracker::default();
        assert_eq!(t.due_in(), None);
        let s = Seen { size: vec2(1200.0, 800.0), pos: None, maximized: false, skip: false, scale: None };
        t.track(&mut w, &s);
        assert!(t.due_in().is_some_and(|d| d <= SAVE_AFTER && d > Duration::ZERO));
        t.changed_at = Some(Instant::now() - SAVE_AFTER * 2);
        assert_eq!(t.due_in(), Some(Duration::ZERO));
    }

    #[test]
    fn size_within_minimum_and_biggest_monitor() {
        assert_eq!(restore_size(&g(1600.0, 1000.0, None, None, false), &mons()), vec2(1600.0, 1000.0));
        assert_eq!(restore_size(&g(400.0, 300.0, None, None, false), &mons()), MIN);
        assert_eq!(restore_size(&g(5000.0, 3000.0, None, None, false), &mons()), vec2(2560.0, 1440.0));
        assert_eq!(restore_size(&g(5000.0, 3000.0, None, None, false), &[]), vec2(5000.0, 3000.0));
    }

    #[test]
    fn restore_applies_size_maximized_and_x11_position() {
        let base = egui::ViewportBuilder::default().with_inner_size([1480.0, 900.0]);
        let saved = g(1600.0, 950.0, Some(2000.0), Some(40.0), true);
        let b = restore(base.clone(), Some(&saved), false, &mons());
        assert_eq!(b.inner_size, Some(vec2(1600.0, 950.0)));
        assert_eq!(b.position, Some(pos2(2000.0, 40.0)));
        assert_eq!(b.maximized, Some(true));
        // Wayland: size and maximised only.
        let b = restore(base.clone(), Some(&saved), true, &[]);
        assert_eq!(b.inner_size, Some(vec2(1600.0, 950.0)));
        assert_eq!(b.position, None);
        assert_eq!(b.maximized, Some(true));
        // Nothing saved: as given.
        let b = restore(base, None, false, &mons());
        assert_eq!(b.inner_size, Some(vec2(1480.0, 900.0)));
        assert_eq!(b.position, None);
    }

    #[test]
    fn maximising_keeps_the_size_from_before() {
        let prev = g(1500.0, 900.0, Some(30.0), Some(40.0), false);
        let s =
            Seen { size: vec2(1920.0, 1050.0), pos: Some(pos2(0.0, 0.0)), maximized: true, skip: false, scale: None };
        assert_eq!(next_geom(Some(&prev), &s), Some(g(1500.0, 900.0, Some(30.0), Some(40.0), true)));
        // Un-maximised again: what the window is now.
        let s =
            Seen { size: vec2(1500.0, 900.0), pos: Some(pos2(30.0, 40.0)), maximized: false, skip: false, scale: None };
        assert_eq!(next_geom(Some(&g(1500.0, 900.0, Some(30.0), Some(40.0), true)), &s), Some(prev));
        // Fullscreen (controller mode), minimised, hidden: nothing changes.
        let s =
            Seen { size: vec2(2560.0, 1440.0), pos: Some(pos2(0.0, 0.0)), maximized: false, skip: true, scale: None };
        assert_eq!(next_geom(Some(&prev), &s), Some(prev));
        assert_eq!(next_geom(None, &s), None);
        // Wayland (no position): size only.
        let s = Seen { size: vec2(1200.0, 800.0), pos: None, maximized: false, skip: false, scale: None };
        assert_eq!(next_geom(None, &s), Some(g(1200.0, 800.0, None, None, false)));
    }

    #[test]
    fn tracker_saves_after_a_pause() {
        let mut w = std::collections::BTreeMap::new();
        let mut t = Tracker::default();
        let s = Seen { size: vec2(1200.0, 800.0), pos: None, maximized: false, skip: false, scale: None };
        assert!(!t.track(&mut w, &s));
        assert!(t.pending());
        assert_eq!(w.get(KEY), Some(&g(1200.0, 800.0, None, None, false)));
        t.changed_at = Some(Instant::now() - SAVE_AFTER);
        assert!(t.track(&mut w, &s));
        assert!(!t.pending());
        assert!(!t.track(&mut w, &s), "nothing new: nothing to save");
    }
}
