//! Lodestar's system tray icon: a StatusNotifierItem with a com.canonical.dbusmenu
//! menu, served over the session bus by `ksni` (pure Rust, zbus) on a thread of its own.
//!
//! * Icon: the app icon (`icon::render`) as ARGB32 pixmaps; tooltip "Lodestar" (+ "— playing
//!   <game>").
//! * Left click (Activate) toggles the window; the menu has Show / Hide Lodestar, Stop <game>
//!   while one runs, and Quit Lodestar.
//! * Only shown while a tray host exists: `org.kde.StatusNotifierWatcher` on the bus with
//!   `IsStatusNotifierHostRegistered` true. When there is none at start, the item waits for a
//!   watcher to appear (Plasma restarting, a tray extension turned on later).
//!
//! The tray never touches app state: clicks become [`TrayEvent`]s on a channel (plus a
//! repaint request), and the UI sends the tray what it shows ([`TrayState`]) when that changes.
//!
//! CONTRACT: `start(ctx, events, state) -> TrayHandle`; `TrayHandle::set_state(s)`; dropping the
//! handle removes the icon. Pure helpers (`menu_model`, `tooltip`, `argb32_be`) are tested.

use crossbeam_channel::{Receiver, Sender};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// What the tray shows; sent by the UI whenever it changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayState {
    /// The window is shown (not hidden to the tray).
    pub shown: bool,
    /// The game that runs, if any.
    pub game: Option<String>,
    /// Stop is possible (screenshot runs never stop games).
    pub can_stop: bool,
}

/// What the user did in the tray, and whether a tray host is there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// Left click on the icon.
    Toggle,
    Show,
    Hide,
    StopGame,
    Quit,
    /// The tray host appeared (true) or went away (false).
    Host(bool),
}

/// One menu entry: `action` None is a separator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuEntry {
    pub label: String,
    pub enabled: bool,
    pub action: Option<TrayEvent>,
}

impl MenuEntry {
    fn item(label: impl Into<String>, enabled: bool, action: TrayEvent) -> MenuEntry {
        MenuEntry { label: label.into(), enabled, action: Some(action) }
    }
    fn separator() -> MenuEntry {
        MenuEntry { label: String::new(), enabled: true, action: None }
    }
}

/// The tray menu for `s`.
pub fn menu_model(s: &TrayState) -> Vec<MenuEntry> {
    let mut m = vec![if s.shown {
        MenuEntry::item("Hide Lodestar", true, TrayEvent::Hide)
    } else {
        MenuEntry::item("Show Lodestar", true, TrayEvent::Show)
    }];
    if let Some(g) = &s.game {
        m.push(MenuEntry::item(format!("Stop {g}"), s.can_stop, TrayEvent::StopGame));
    }
    m.push(MenuEntry::separator());
    m.push(MenuEntry::item("Quit Lodestar", true, TrayEvent::Quit));
    m
}

pub fn tooltip(s: &TrayState) -> String {
    match &s.game {
        Some(g) => format!("Lodestar — playing {g}"),
        None => "Lodestar".into(),
    }
}

/// Straight RGBA → ARGB32 in network byte order (what StatusNotifierItem's IconPixmap wants).
pub fn argb32_be(rgba: &[u8]) -> Vec<u8> {
    rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[3], p[0], p[1], p[2]]).collect()
}

/// The icon at the sizes trays ask for (16 – 22 px panels up to HiDPI).
fn pixmaps() -> Vec<ksni::Icon> {
    [16u32, 22, 24, 32, 48, 64, 128]
        .into_iter()
        .filter_map(|n| {
            let rgba = crate::icon::render(n)?;
            Some(ksni::Icon { width: n as i32, height: n as i32, data: argb32_be(&rgba) })
        })
        .collect()
}

struct Item {
    state: TrayState,
    icons: Vec<ksni::Icon>,
    events: Sender<TrayEvent>,
    ctx: egui::Context,
    /// Cleared by `watcher_offline`, set by `watcher_online`.
    online: Arc<AtomicBool>,
    /// The last `Host` told to the app (see [`HostReport`]).
    reported: HostReport,
}

impl Item {
    fn send(&self, ev: TrayEvent) {
        let _ = self.events.send(ev);
        self.ctx.request_repaint();
    }
    fn host(&self, up: bool) {
        if self.reported.changed(up) {
            self.send(TrayEvent::Host(up));
        }
    }
}

/// The last `Host(up)` the app was told, so it hears each change once: with no tray yet, the
/// second registration attempt (waiting for one to appear) would report "no tray" again.
#[derive(Clone, Default)]
struct HostReport(Arc<std::sync::atomic::AtomicU8>);

impl HostReport {
    /// Record `up`; true when it differs from the last report (or there was none).
    fn changed(&self, up: bool) -> bool {
        let v = if up { 2 } else { 1 };
        self.0.swap(v, Ordering::SeqCst) != v
    }
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        "lodestar".into()
    }
    fn title(&self) -> String {
        "Lodestar".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icons.clone()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: tooltip(&self.state), ..Default::default() }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayEvent::Toggle);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        menu_model(&self.state)
            .into_iter()
            .map(|e| match e.action {
                None => ksni::MenuItem::Separator,
                Some(ev) => ksni::menu::StandardItem {
                    label: e.label.replace('_', "__"),
                    enabled: e.enabled,
                    activate: Box::new(move |t: &mut Item| t.send(ev.clone())),
                    ..Default::default()
                }
                .into(),
            })
            .collect()
    }
    fn watcher_online(&self) {
        log::info!("system tray is back");
        self.online.store(true, Ordering::SeqCst);
        self.host(true);
    }
    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        log::info!("system tray went away ({reason:?})");
        self.online.store(false, Ordering::SeqCst);
        self.host(false);
        true
    }
}

/// The running tray (see the module docs). Dropping it removes the icon.
pub struct TrayHandle {
    states: Sender<TrayState>,
}

impl TrayHandle {
    /// Show `s` in the tray (tooltip, menu).
    pub fn set_state(&self, s: TrayState) {
        let _ = self.states.send(s);
    }

    /// A handle with no tray behind it (tests).
    #[cfg(test)]
    pub fn detached() -> TrayHandle {
        TrayHandle { states: crossbeam_channel::unbounded().0 }
    }
}

/// Put the icon in the tray, on a thread of its own (connecting to the bus never holds up the
/// UI). `events` gets `Host(true)` once the icon shows, `Host(false)` when there is no tray.
pub fn start(ctx: &egui::Context, events: Sender<TrayEvent>, state: TrayState) -> TrayHandle {
    let (tx, rx) = crossbeam_channel::unbounded();
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new().name("tray".into()).spawn({
        let events = events.clone();
        move || run(ctx, events, state, rx)
    });
    if let Err(e) = spawned {
        log::warn!("tray: no thread ({e})");
        let _ = events.send(TrayEvent::Host(false));
    }
    TrayHandle { states: tx }
}

/// Says `Host(false)` when the tray thread ends for any reason but the app dropping its handle
/// (the service died, a panic): a window hidden to the tray then comes back.
struct GoneGuard {
    events: Sender<TrayEvent>,
    ctx: egui::Context,
    /// The app dropped its handle: nobody needs telling.
    dropped: bool,
}

impl Drop for GoneGuard {
    fn drop(&mut self) {
        if !self.dropped {
            let _ = self.events.send(TrayEvent::Host(false));
            self.ctx.request_repaint();
        }
    }
}

/// How often the tray thread checks that ksni's service still runs.
const ALIVE_CHECK: std::time::Duration = std::time::Duration::from_secs(2);

fn run(ctx: egui::Context, events: Sender<TrayEvent>, state: TrayState, states: Receiver<TrayState>) {
    use crossbeam_channel::RecvTimeoutError;
    use ksni::blocking::TrayMethods as _;
    let mut guard = GoneGuard { events: events.clone(), ctx: ctx.clone(), dropped: false };
    let online = Arc::new(AtomicBool::new(true));
    let reported = HostReport::default();
    let item = |state: TrayState| Item {
        state,
        icons: pixmaps(),
        events: events.clone(),
        ctx: ctx.clone(),
        online: online.clone(),
        reported: reported.clone(),
    };
    let host = |up: bool| {
        if reported.changed(up) {
            let _ = events.send(TrayEvent::Host(up));
            ctx.request_repaint();
        }
    };
    // First as the spec has it: a watcher with a host registered, or no icon.
    let handle = match item(state.clone()).spawn() {
        Ok(h) => {
            log::info!("tray icon shown");
            host(true);
            h
        }
        Err(e @ (ksni::Error::Watcher(_) | ksni::Error::WontShow)) => {
            log::info!("no system tray ({e}); Lodestar's icon shows when one appears");
            host(false);
            // A watcher without a host: registering works, but nothing says when a host comes,
            // so the icon never counts as shown (closing minimises instead of hiding).
            let wont_show = matches!(e, ksni::Error::WontShow);
            // Then wait for a watcher to appear (`watcher_online` says so). One may have appeared
            // just now: then registering succeeds at once (`watcher_offline` isn't called).
            online.store(true, Ordering::SeqCst);
            match item(state).assume_sni_available(true).spawn() {
                Ok(h) => {
                    if online.load(Ordering::SeqCst) && !wont_show {
                        log::info!("tray icon shown");
                        host(true);
                    }
                    h
                }
                Err(e) => {
                    log::info!("tray: {e}");
                    return;
                }
            }
        }
        Err(e) => {
            log::info!("no system tray: {e}");
            host(false);
            return;
        }
    };
    // Until the app drops its handle (or ksni's service ends: then the guard says so).
    loop {
        let mut s = match states.recv_timeout(ALIVE_CHECK) {
            Ok(s) => s,
            Err(RecvTimeoutError::Timeout) if handle.is_closed() => break,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                guard.dropped = true;
                break;
            }
        };
        while let Ok(newer) = states.try_recv() {
            s = newer;
        }
        if handle.update(|t| t.state = s).is_none() {
            break;
        }
    }
    if !guard.dropped {
        log::warn!("the tray icon's service ended; Lodestar is no longer in the tray");
    }
    handle.shutdown().wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(m: &[MenuEntry]) -> Vec<(&str, bool)> {
        m.iter().map(|e| (if e.action.is_none() { "--" } else { e.label.as_str() }, e.enabled)).collect()
    }

    #[test]
    fn menu_without_a_game() {
        let s = TrayState { shown: true, game: None, can_stop: true };
        let m = menu_model(&s);
        assert_eq!(labels(&m), [("Hide Lodestar", true), ("--", true), ("Quit Lodestar", true)]);
        assert_eq!(m[0].action, Some(TrayEvent::Hide));
        assert_eq!(m[2].action, Some(TrayEvent::Quit));
        let m = menu_model(&TrayState { shown: false, ..s.clone() });
        assert_eq!((m[0].label.as_str(), m[0].action.clone()), ("Show Lodestar", Some(TrayEvent::Show)));
        assert_eq!(tooltip(&s), "Lodestar");
    }

    #[test]
    fn menu_while_a_game_runs() {
        let s = TrayState { shown: false, game: Some("Hades II".into()), can_stop: true };
        let m = menu_model(&s);
        assert_eq!(
            labels(&m),
            [("Show Lodestar", true), ("Stop Hades II", true), ("--", true), ("Quit Lodestar", true)]
        );
        assert_eq!(m[1].action, Some(TrayEvent::StopGame));
        assert_eq!(tooltip(&s), "Lodestar — playing Hades II");
        // Screenshot runs: Stop is there but greyed out.
        let m = menu_model(&TrayState { can_stop: false, ..s });
        assert_eq!(labels(&m)[1], ("Stop Hades II", false));
    }

    #[test]
    fn a_tray_thread_that_ends_says_so_unless_the_app_dropped_it() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let ctx = egui::Context::default();
        drop(GoneGuard { events: tx.clone(), ctx: ctx.clone(), dropped: false });
        assert_eq!(rx.try_recv(), Ok(TrayEvent::Host(false)));
        drop(GoneGuard { events: tx, ctx, dropped: true });
        assert!(rx.try_recv().is_err());
        // A panic on the tray thread counts too.
        let (tx, rx) = crossbeam_channel::unbounded();
        let t = std::thread::spawn(move || {
            let _g = GoneGuard { events: tx, ctx: egui::Context::default(), dropped: false };
            panic!("tray thread panics (expected in this test)");
        });
        assert!(t.join().is_err());
        assert_eq!(rx.try_recv(), Ok(TrayEvent::Host(false)));
    }

    #[test]
    fn host_changes_are_reported_once() {
        let r = HostReport::default();
        assert!(r.changed(false), "the first report is news");
        assert!(!r.changed(false), "no tray, again (the second registration attempt)");
        assert!(r.changed(true));
        assert!(!r.changed(true));
        assert!(r.changed(false));
    }

    #[test]
    fn pixmaps_are_argb_big_endian() {
        assert_eq!(argb32_be(&[1, 2, 3, 4, 10, 20, 30, 255]), [4, 1, 2, 3, 255, 10, 20, 30]);
        assert!(argb32_be(&[1, 2, 3]).is_empty(), "a partial pixel is dropped");
        let icons = pixmaps();
        assert_eq!(icons.iter().map(|i| i.width).collect::<Vec<_>>(), [16, 22, 24, 32, 48, 64, 128]);
        for i in &icons {
            assert_eq!((i.height, i.data.len()), (i.width, (i.width * i.width * 4) as usize));
            // Some opaque pixels, alpha first.
            assert!(i.data.as_chunks::<4>().0.iter().any(|p| p[0] == 255));
        }
        let rgba = crate::icon::render(16).unwrap();
        assert_eq!(icons[0].data, argb32_be(&rgba));
    }
}

#[cfg(test)]
mod dbus_tests;
