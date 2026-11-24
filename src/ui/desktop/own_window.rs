//! The big desktop dialogs — Settings, Add-ons, Edit game details, Emulated games… (the ROM
//! import) and the artwork editor — in their own OS windows (egui immediate viewports), each with
//! its own size, remembered in the config (`[desktop.windows.<key>]`: size, and the position
//! where the session tells it, i.e. X11).
//!
//! How: [`DesktopUi::host`] runs the dialog's usual drawing function inside
//! `ctx.show_viewport_immediate` and marks the viewport as hosting that dialog's modal id; the
//! dialog's own `modal(ctx, id, …)` call (dialogs.rs) then fills the window instead of drawing a
//! modal over the main window, so the dialogs themselves stay as they were. Confirmations and
//! pickers a dialog opens on top of itself stay modal inside its window. Off (Settings → General
//! → Interface), in screenshot runs (unless `LODESTAR_OWN_WINDOWS=1`) or when the backend can't
//! open windows, everything is drawn inside the main window exactly as before.
//!
//! Rules:
//! * One window per dialog; opening it again focuses it (X11; on Wayland, where a program can't
//!   raise its own window, the window is opened anew, which puts it in front).
//! * Esc (nothing on top inside it), Done / Cancel and the window's close button close it, like
//!   the in-window dialog.
//! * While such a window is open the main window waits behind a light veil that says so and
//!   brings the window back on a click, as Playnite's dialogs do: the dialogs keep their
//!   one-at-a-time assumptions (an Edit window never sees its game changed under it).
//! * Hidden to the tray: the windows go with the main window and come back with it (their state
//!   is kept). Controller mode closes Settings and hides the others until desktop mode is back.
//!   Quitting saves their sizes.
//! * Controller input: Lodestar counts as focused when any of its windows has the focus
//!   (input/gate.rs `lodestar_focus`).

use super::{DesktopUi, Pal};
use crate::app::AppState;
use crate::config::WindowGeom;
use egui::{Id, Vec2, ViewportBuilder, ViewportCommand, ViewportId, pos2, vec2};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A dialog that can have its own window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Win {
    Settings,
    Addons,
    Edit,
    Roms,
    Art,
    /// The theme creator (theme_creator.rs).
    Creator,
}

impl Win {
    pub const ALL: [Win; 6] = [Win::Settings, Win::Addons, Win::Edit, Win::Roms, Win::Art, Win::Creator];

    /// Key in `[desktop.windows]`.
    pub fn key(self) -> &'static str {
        match self {
            Win::Settings => "settings",
            Win::Addons => "addons",
            Win::Edit => "edit",
            Win::Roms => "roms",
            Win::Art => "art",
            Win::Creator => "theme-creator",
        }
    }

    /// The id the dialog passes to `modal()`.
    pub fn modal_id(self) -> &'static str {
        match self {
            Win::Settings => "ls_settings",
            Win::Addons => "ls_addons",
            Win::Edit => "ls_edit_dlg",
            Win::Roms => "ls_roms_import",
            Win::Art => "ls_art_dlg",
            Win::Creator => super::theme_creator::MODAL_ID,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Win::Settings => "Settings",
            Win::Addons => "Add-ons",
            Win::Edit => "Edit game details",
            Win::Roms => "Emulated games",
            Win::Art => "Edit artwork",
            Win::Creator => "Theme creator",
        }
    }

    fn title(self) -> String {
        format!("{} — Lodestar", self.name())
    }

    /// Size on first open (shrunk to 80 % of the screen when that is smaller).
    pub fn default_size(self) -> Vec2 {
        match self {
            Win::Settings => vec2(980.0, 720.0),
            Win::Addons => vec2(1040.0, 740.0),
            Win::Edit => vec2(940.0, 760.0),
            Win::Roms => vec2(940.0, 720.0),
            Win::Art => vec2(980.0, 760.0),
            Win::Creator => vec2(1480.0, 900.0),
        }
    }

    /// Where the dialog sits among the others when they are drawn inside the main window (the
    /// order `dialogs()` draws them in: a later one is on top, e.g. "Set up emulation…" opens
    /// Settings over Edit).
    pub fn layer(self) -> u8 {
        match self {
            Win::Edit => 0,
            Win::Roms => 1,
            Win::Art => 2,
            Win::Addons => 3,
            Win::Creator => 4,
            Win::Settings => 5,
        }
    }

    pub fn viewport_id(self) -> ViewportId {
        ViewportId::from_hash_of(("ls_own_window", self.key()))
    }
}

/// The smallest a dialog window gets (the pages reflow down to this).
pub(crate) const MIN_SIZE: Vec2 = vec2(600.0, 440.0);

/// A window's first size: the remembered one, else the dialog's default shrunk to 80 % of the
/// monitor; never under [`MIN_SIZE`] nor (when known) over the monitor.
pub(crate) fn first_size(saved: Option<Vec2>, default: Vec2, monitor: Option<Vec2>) -> Vec2 {
    let want = match (saved, monitor) {
        (Some(s), _) => s,
        (None, Some(m)) => default.min(m * 0.8),
        (None, None) => default,
    };
    let max = monitor.map_or(Vec2::splat(f32::INFINITY), |m| m.max(MIN_SIZE));
    want.max(MIN_SIZE).min(max)
}

#[derive(Default)]
struct Slot {
    /// Fixed while the window exists (egui diffs builders frame to frame; resizing by the user
    /// must not be undone).
    builder: Option<ViewportBuilder>,
    /// The main window's pass that last showed the window.
    shown_pass: Option<u64>,
    /// Hosted this frame (else the dialog closed).
    used: bool,
    geom: Option<WindowGeom>,
    focus: bool,
    /// Wayland "focus": leave the window out for one frame so it opens anew, in front.
    skip_once: bool,
    reopened_at: Option<Instant>,
    /// Its focus as its own latest pass saw it (focused and not minimised): fresher than the
    /// report the main window's pass gets (input/gate.rs `OtherWindow`).
    latest_focus: Option<bool>,
}

/// The dialogs' windows.
#[derive(Default)]
pub(crate) struct OwnWindows {
    slots: HashMap<Win, Slot>,
    /// Pass whose bookkeeping ran (`begin`).
    pass: u64,
}

/// "This viewport hosts the modal with this id" (set around the dialog's drawing function).
#[derive(Clone, Copy)]
struct Hosted(ViewportId, &'static str);

fn host_key() -> Id {
    Id::unique("ls_own_window_hosted")
}

/// `modal()` is to fill the current window instead of drawing a modal (dialogs.rs).
pub(crate) fn hosts_modal(ctx: &egui::Context, id: &str) -> bool {
    ctx.data(|d| d.get_temp::<Hosted>(host_key())).is_some_and(|Hosted(v, m)| v == ctx.viewport_id() && m == id)
}

/// Drawing inside one of the dialog windows (not the main window).
pub(crate) fn in_own_window(ctx: &egui::Context) -> bool {
    ctx.viewport_id() != ViewportId::ROOT
}

/// The dialog's content filling its window: same padding as the modal, the window's full width,
/// nothing behind it. Closes like the modal: Esc (when nothing sits on top inside the window)
/// or the window's close button.
pub(crate) fn fill_window<R>(
    ctx: &egui::Context,
    id: &str,
    pal: &Pal,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> (R, bool) {
    let rect = ctx.content_rect();
    let r = egui::Area::new(Id::unique("ls_own_window_body").with(id))
        .order(egui::Order::Background)
        .fixed_pos(rect.min)
        .movable(false)
        .constrain(false)
        .show(ctx, |ui| {
            ui.painter().rect_filled(rect, 0.0, pal.surface);
            let body = rect.shrink(20.0);
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)),
                |ui| {
                    ui.set_width(body.width());
                    add(ui)
                },
            )
            .inner
        })
        .inner;
    let on_top = ctx.memory(|m| m.top_modal_layer().is_some()) || egui::Popup::is_any_open(ctx);
    let esc = !on_top && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let close = esc || ctx.input(|i| i.viewport().close_requested());
    (r, close)
}

/// The window icon, rendered once.
fn icon() -> Option<std::sync::Arc<egui::IconData>> {
    static ICON: std::sync::OnceLock<Option<std::sync::Arc<egui::IconData>>> = std::sync::OnceLock::new();
    ICON.get_or_init(|| crate::icon::window_icon().map(std::sync::Arc::new)).clone()
}

/// Screenshot runs use the windows only when asked (`LODESTAR_OWN_WINDOWS=1`); the main
/// screenshot then shows the main window and each dialog window is saved next to it.
pub(crate) fn screenshot_windows() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LODESTAR_OWN_WINDOWS").is_ok_and(|v| v == "1"))
}

/// Wayland re-open ("focus") at most this often (a burst of clicks on the veil).
const REOPEN_EVERY: Duration = Duration::from_millis(800);

impl OwnWindows {
    /// Once per main-window pass, before the dialogs are drawn.
    fn begin(&mut self, pass: u64) {
        if self.pass != pass {
            self.pass = pass;
            for s in self.slots.values_mut() {
                s.used = false;
            }
        }
    }

    /// The open dialog on top (the one that takes input, as when they are drawn inside the main
    /// window), counting a window left out for a frame to be re-opened in front (Wayland).
    fn top(&self) -> Option<Win> {
        self.slots.iter().filter(|(_, s)| s.used).map(|(w, _)| *w).max_by_key(|w| w.layer())
    }

    /// The open dialog over `win`, if any (`win` then waits behind a veil in its window, like
    /// the main window: one dialog at a time takes input, as before they had windows).
    fn covering(&self, win: Win) -> Option<Win> {
        self.slots.keys().copied().filter(|w| w.layer() > win.layer()).max_by_key(|w| w.layer())
    }

    /// Remember the window's size / place / maximised (logical px, like the main window's).
    fn note_geom(&mut self, win: Win, vctx: &egui::Context) {
        let seen = crate::main_window::seen(vctx, false);
        if let Some(s) = self.slots.get_mut(&win) {
            s.geom = crate::main_window::next_geom(s.geom.as_ref(), &seen);
        }
    }
}

impl DesktopUi {
    /// Dialogs go to their own windows now.
    pub(super) fn own_windows_on(&self, ctx: &egui::Context, st: &AppState) -> bool {
        st.config.desktop.own_windows && !ctx.embed_viewports() && (!st.no_launch || screenshot_windows())
    }

    /// Draw `draw` — a dialog that draws itself with `modal(ctx, win.modal_id(), …)` — in the
    /// dialog's own window when those are on, else inside the main window as before. None: not
    /// drawn this frame (Lodestar is hidden to the tray); the dialog stays as it is.
    pub(super) fn host<R>(
        &mut self,
        ctx: &egui::Context,
        st: &mut AppState,
        win: Win,
        mut draw: impl FnMut(&mut DesktopUi, &egui::Context, &mut AppState) -> R,
    ) -> Option<R> {
        if !self.own_windows_on(ctx, st) {
            return Some(draw(self, ctx, st));
        }
        let pass = ctx.cumulative_pass_nr();
        self.own.begin(pass);
        let saved = st.config.desktop.windows.get(win.key()).copied();
        let s = self.own.slots.entry(win).or_insert_with(|| Slot { geom: saved, ..Default::default() });
        s.used = true;
        if st.shell.hidden {
            return None; // not shown: the window goes with the main one, and comes back with it
        }
        if std::mem::take(&mut s.skip_once) {
            ctx.request_repaint();
            return None;
        }
        // New, or back after a frame without it (tray, controller mode, re-open): from the last
        // size and position.
        if s.builder.is_none() || s.shown_pass.is_none_or(|p| p + 1 < pass) {
            let b = self.own_builder(ctx, st, win);
            self.own.slots.get_mut(&win).expect("slot").builder = Some(b);
        }
        let s = self.own.slots.get_mut(&win).expect("slot");
        s.shown_pass = Some(pass);
        s.reopened_at.get_or_insert_with(Instant::now);
        let builder = s.builder.clone().expect("builder");
        if std::mem::take(&mut s.focus) {
            ctx.send_viewport_cmd_to(win.viewport_id(), ViewportCommand::Focus);
        }
        let pal = self.pal;
        let covered = self.own.covering(win);
        // (egui may run the window's pass more than once in a frame: `request_discard`.)
        Some(ctx.show_viewport_immediate(win.viewport_id(), builder, |ui, _class| {
            let vctx = ui.ctx().clone();
            ui.painter().rect_filled(ui.max_rect(), 0.0, pal.surface);
            // (The id is read outside `data_mut`: egui's lock isn't re-entrant.)
            let here = vctx.viewport_id();
            vctx.data_mut(|d| d.insert_temp(host_key(), Hosted(here, win.modal_id())));
            let r = draw(self, &vctx, st);
            vctx.data_mut(|d| d.remove::<Hosted>(host_key()));
            if let Some(top) = covered {
                self.own_veil(&vctx, st, top);
            }
            self.own.note_geom(win, &vctx);
            let focus = vctx
                .input(|i| crate::input::gate::effective_focus(i.viewport().focused, false, i.viewport().minimized));
            if let Some(s) = self.own.slots.get_mut(&win) {
                s.latest_focus = focus;
            }
            r
        }))
    }

    fn own_builder(&self, ctx: &egui::Context, st: &AppState, win: Win) -> ViewportBuilder {
        let (monitor, main) = ctx.input(|i| (i.viewport().monitor_size, i.viewport().outer_rect));
        // Remembered in logical px; the builder takes points (the UI scale applied).
        let zoom = ctx.zoom_factor();
        let saved = self.own.slots.get(&win).and_then(|s| s.geom);
        let size = first_size(saved.map(|g| vec2(g.w, g.h) / zoom), win.default_size(), monitor);
        // A remembered place still on one of the monitors (X11; same rule as the main window).
        let place = saved
            .and_then(|g| crate::main_window::restore_position(&g, size * zoom, crate::main_window::monitors()))
            .map(|p| (p.to_vec2() / zoom).to_pos2());
        let mut b = ViewportBuilder::default()
            .with_title(win.title())
            .with_app_id("lodestar")
            .with_inner_size(size)
            .with_min_inner_size(MIN_SIZE)
            .with_resizable(true);
        if let Some(ic) = icon() {
            b = b.with_icon(ic);
        }
        let pos = if st.no_launch {
            // Screenshot runs (no window manager): beside the main window, so both can be seen,
            // and one under the other when several are open (the X screen is 2560×1600).
            let below = self.own.slots.keys().filter(|w| w.layer() < win.layer()).count() as f32;
            main.map(|m| pos2(m.max.x + 16.0, m.min.y + below * 780.0))
        } else {
            place.or_else(|| {
                // X11 tells where the main window is: centred over it (Wayland places it itself).
                main.map(|m| (m.center() - size / 2.0).max(pos2(0.0, 0.0)))
            })
        };
        if let Some(p) = pos {
            b = b.with_position(p);
        }
        if saved.is_some_and(|g| g.maximized) {
            b = b.with_maximized(true);
        }
        b
    }

    /// What the dialog window `id`'s own latest pass saw of its focus (None: not one of them, or
    /// not drawn yet).
    pub(crate) fn own_window_focus(&self, id: ViewportId) -> Option<bool> {
        self.own.slots.iter().find(|(w, _)| w.viewport_id() == id).and_then(|(_, s)| s.latest_focus)
    }

    /// Bring a dialog's window to the front (it is open; nothing when it isn't in a window).
    pub(super) fn own_focus(&mut self, st: &AppState, win: Win) {
        let Some(s) = self.own.slots.get_mut(&win) else { return };
        if st.window_restorable {
            s.focus = true;
        } else if s.reopened_at.is_none_or(|t| t.elapsed() >= REOPEN_EVERY) {
            s.skip_once = true;
            s.reopened_at = Some(Instant::now());
        }
        st.ctx().request_repaint();
    }

    /// After the dialogs were drawn: forget the windows whose dialog closed (remembering their
    /// size), and while one is open, keep the main window behind a veil that says so.
    pub(super) fn own_end(&mut self, ctx: &egui::Context, st: &mut AppState) {
        let on = self.own_windows_on(ctx, st);
        self.own.begin(ctx.cumulative_pass_nr());
        let closed: Vec<Win> = self.own.slots.iter().filter(|(_, s)| !s.used || !on).map(|(w, _)| *w).collect();
        for w in closed {
            if let Some(g) = self.own.slots.remove(&w).and_then(|s| s.geom)
                && st.config.desktop.windows.get(w.key()) != Some(&g)
            {
                st.config.desktop.windows.insert(w.key().to_string(), g);
                self.mark_config_dirty();
            }
        }
        if let Some(top) = self.own.top() {
            self.own_veil(ctx, st, top);
        }
    }

    /// The main window while a dialog has its own window (or a dialog's window while another one
    /// is open over it): what is open, and a way back to it.
    fn own_veil(&mut self, ctx: &egui::Context, st: &AppState, top: Win) {
        let pal = self.pal;
        let frame = egui::Frame::new()
            .fill(pal.surface)
            .stroke(egui::Stroke::new(1.0, pal.border))
            .corner_radius(10)
            .inner_margin(egui::Margin::same(16));
        let mut show = false;
        let resp = egui::Modal::new(Id::unique("ls_own_window_veil").with(ctx.viewport_id()))
            .frame(frame)
            .backdrop_color(egui::Color32::from_black_alpha(70))
            .show(ctx, |ui| {
                ui.set_width(320.0);
                ui.label(
                    egui::RichText::new(format!("{} is open in its own window", top.name()))
                        .size(15.0)
                        .strong()
                        .color(pal.text),
                );
                ui.add_space(4.0);
                let hint =
                    if in_own_window(ctx) { "Close it to go on here." } else { "Close it to use the library again." };
                super::dialogs::hint_text(ui, hint, &pal);
                ui.add_space(10.0);
                if super::dialogs::primary_button(ui, "Show it", &pal, true).clicked() {
                    show = true;
                }
            });
        // A click beside the card (or Esc in the main window) brings the window back too.
        if show || resp.should_close() {
            self.own_focus(st, top);
        }
    }

    /// Switching to controller mode: Settings closes (its changes saved); the other windows
    /// wait until desktop mode is back (their windows close meanwhile).
    pub(crate) fn leave_desktop(&mut self, st: &mut AppState) {
        if self.dlg.settings.take().is_some() || self.config_dirty.is_some() {
            self.config_dirty = None;
            st.save_config();
        }
    }

    /// Lodestar quits: remember the open windows' sizes, save pending settings.
    pub(crate) fn on_quit(&mut self, st: &mut AppState) {
        if let Some(c) = &self.dlg.creator {
            c.save_now();
        }
        let mut dirty = self.config_dirty.take().is_some();
        for (w, s) in &self.own.slots {
            if let Some(g) = s.geom
                && st.config.desktop.windows.get(w.key()) != Some(&g)
            {
                st.config.desktop.windows.insert(w.key().to_string(), g);
                dirty = true;
            }
        }
        if dirty {
            st.save_config();
        }
    }
}

/// Screenshot runs with `LODESTAR_OWN_WINDOWS=1`: the dialog windows' places on the (virtual)
/// X screen, in physical pixels, read before the main screenshot is taken.
pub(crate) fn window_rects(ctx: &egui::Context) -> Vec<egui::Rect> {
    if !screenshot_windows() {
        return vec![];
    }
    let zoom = ctx.zoom_factor();
    ctx.input(|i| {
        let mut v: Vec<(ViewportId, egui::Rect)> = i
            .raw
            .viewports
            .iter()
            .filter(|(id, _)| **id != ViewportId::ROOT)
            .filter_map(|(id, v)| Some((*id, v.inner_rect? * (v.native_pixels_per_point.unwrap_or(1.0) * zoom))))
            .collect();
        v.sort_by_key(|(id, _)| Win::ALL.iter().position(|w| w.viewport_id() == *id));
        v.into_iter().map(|(_, r)| r).collect()
    })
}

/// Read a part of the X screen (Xvfb in screenshot runs: no compositor, the windows' pixels are
/// on the root window) as RGBA.
pub(crate) fn capture_x11(r: egui::Rect) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat};
    let (conn, n) = x11rb::connect(None)?;
    let root = conn.setup().roots[n].root;
    let (x, y) = (r.min.x.round().max(0.0) as i16, r.min.y.round().max(0.0) as i16);
    let (w, h) = (r.width().round().max(1.0) as u16, r.height().round().max(1.0) as u16);
    let img = conn.get_image(ImageFormat::Z_PIXMAP, root, x, y, w, h, !0)?.reply()?;
    let px = usize::from(w) * usize::from(h);
    anyhow::ensure!(img.data.len() == px * 4, "unexpected X image format ({} bytes for {px} pixels)", img.data.len());
    // 24-bit depth, 32 bits per pixel, little endian: B G R x.
    let rgba = img.data.as_chunks::<4>().0.iter().flat_map(|p| [p[2], p[1], p[0], 255]).collect();
    Ok((u32::from(w), u32::from(h), rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sizes() {
        let d = vec2(980.0, 720.0);
        // Big screen: the default.
        assert_eq!(first_size(None, d, Some(vec2(2560.0, 1440.0))), d);
        // Small screen: 80 % of it.
        assert_eq!(first_size(None, d, Some(vec2(1024.0, 768.0))), vec2(819.2, 614.4));
        // Tiny screen: the minimum, but never bigger than the screen.
        assert_eq!(first_size(None, d, Some(vec2(700.0, 500.0))), vec2(600.0, 440.0));
        assert_eq!(first_size(None, d, Some(vec2(500.0, 400.0))), MIN_SIZE);
        // Unknown screen: the default.
        assert_eq!(first_size(None, d, None), d);
        // Remembered: as it was, within the minimum and the screen.
        assert_eq!(first_size(Some(vec2(640.0, 900.0)), d, Some(vec2(1920.0, 1080.0))), vec2(640.0, 900.0));
        assert_eq!(first_size(Some(vec2(300.0, 200.0)), d, None), MIN_SIZE);
        assert_eq!(first_size(Some(vec2(4000.0, 3000.0)), d, Some(vec2(1920.0, 1080.0))), vec2(1920.0, 1080.0));
    }

    #[test]
    fn window_ids_and_keys_are_distinct() {
        for (i, a) in Win::ALL.iter().enumerate() {
            for b in &Win::ALL[i + 1..] {
                assert_ne!(a.key(), b.key());
                assert_ne!(a.modal_id(), b.modal_id());
                assert_ne!(a.viewport_id(), b.viewport_id());
            }
            assert_ne!(a.viewport_id(), ViewportId::ROOT);
        }
    }

    /// A dialog window opened from another one (Edit → "Set up emulation…" opens Settings) sits
    /// over it as the in-window dialogs did: the lower one waits behind a veil, the main
    /// window's veil points at the top one — one dialog takes input at a time.
    #[test]
    fn the_top_dialog_window_covers_the_others() {
        let mut o = OwnWindows::default();
        assert_eq!(o.top(), None);
        o.slots.insert(Win::Edit, Slot { used: true, ..Default::default() });
        assert_eq!(o.top(), Some(Win::Edit));
        assert_eq!(o.covering(Win::Edit), None);
        o.slots.insert(Win::Settings, Slot { used: true, ..Default::default() });
        assert_eq!(o.top(), Some(Win::Settings));
        assert_eq!(o.covering(Win::Edit), Some(Win::Settings));
        assert_eq!(o.covering(Win::Settings), None);
        o.slots.insert(Win::Art, Slot { used: true, ..Default::default() });
        assert_eq!(o.covering(Win::Edit), Some(Win::Settings));
        assert_eq!(o.covering(Win::Art), Some(Win::Settings));
        // Settings closed: the artwork editor (opened over Edit) is on top.
        o.slots.remove(&Win::Settings);
        assert_eq!(o.top(), Some(Win::Art));
        assert_eq!(o.covering(Win::Edit), Some(Win::Art));
        // A window left out for one frame (re-opened in front on Wayland) still counts: the
        // main window's veil doesn't blink away.
        o.slots.get_mut(&Win::Art).unwrap().shown_pass = None;
        assert_eq!(o.top(), Some(Win::Art));
        // Layers are distinct and Settings is always on top, as `dialogs()` draws it last.
        let mut layers: Vec<u8> = Win::ALL.iter().map(|w| w.layer()).collect();
        layers.sort_unstable();
        layers.dedup();
        assert_eq!(layers.len(), Win::ALL.len());
        assert!(Win::ALL.iter().all(|w| w.layer() <= Win::Settings.layer()));
    }

    #[test]
    fn geometry_round_trips_through_the_config() {
        let mut c = crate::config::Config::default();
        assert!(c.desktop.own_windows, "on by default");
        c.desktop.windows.insert(
            "settings".into(),
            WindowGeom { w: 640.0, h: 720.0, x: Some(10.0), y: None, maximized: true, scale: Some(1.25) },
        );
        let text = toml::to_string(&c).unwrap();
        let back: crate::config::Config = toml::from_str(&text).unwrap();
        assert_eq!(back.desktop.windows, c.desktop.windows);
        // Old config files (no such keys) load with the windows on.
        let old: crate::config::Config = toml::from_str("[desktop]\nview = \"grid\"\n").unwrap();
        assert!(old.desktop.own_windows);
        assert!(old.desktop.windows.is_empty());
    }
}
