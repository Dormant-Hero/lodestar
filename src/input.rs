//! Controller + keyboard navigation.
//!
//! CONTRACT:
//!   InputHub::new(ctx, &ControllerConfig): spawns a gilrs thread (keyboard-only if gilrs fails).
//!     Held directions repeat (repeat_delay_ms / repeat_interval_ms), left stick acts as d-pad.
//!   set_config(&self, &ControllerConfig); set_gate(&self, Gate) (gate.rs decides it per frame:
//!     Off / Guide only / All — what isn't let through is dropped, not queued);
//!     set_home_button(&self, bool): whether Home opens Lodestar's menu right now.
//!   record_combo(&self) -> RecordPoll: record a menu button combination (call every frame
//!     while the recording screen waits; it lapses on its own when the calls stop);
//!     stop_recording(&self).
//!   drain(&mut self) -> Vec<Nav>: gamepad navigation since last call.
//!   gamepads(&self) -> Vec<String>: connected pad names.
//!   pad_style(&self) -> Option<ButtonStyle>: prompt style of the pad used last (see input/style.rs).
//!   keyboard_nav(ctx) -> Vec<Nav>: arrows/Enter/Esc/Backspace/Tab/PageUp/PageDown/F/Space/Home...
//!
//! Gamepad mapping: South=Accept, East=Back (swapped with `swap_ab`), West=Favorite, North=Search,
//! Start=Menu, Select=Select, Mode=Guide, LB=PageUp, RB=PageDown, LT=JumpUp, RT=JumpDown, D-pad and
//! left stick = directions. Directions, page and jump repeat while held. The menu button
//! combination (default L3 + R3, held a moment, see input/pad.rs) is Guide too.
//!
//! The gilrs thread blocks in `next_event_blocking` (epoll) and only wakes for input, hotplug or
//! a pending hold-repeat, so it costs nothing while idle. Force feedback is off (see
//! `gamepad_thread`): with it on, gilrs ticks a thread every 50 ms and the pad echoes its writes.

use crate::config::{ControllerConfig, MenuCombo};
use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::time::{Duration, Instant};

mod pad;
pub use pad::{PadConfig, PadInput, PadMapper, Recorded};
pub mod gate;
pub mod style;
pub use gate::Gate;
pub use style::{ButtonStyle, Prompts};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Nav {
    Up,
    Down,
    Left,
    Right,
    /// A (bottom face button) / Enter
    Accept,
    /// B (right face button) / Esc / Backspace
    Back,
    /// Start / F1: quick menu
    Menu,
    /// Back/Select/View button / Tab: secondary menu (sort, filter, game options)
    Select,
    /// X (left face button) / F: toggle favourite
    Favorite,
    /// Y (top face button) / Ctrl+F or '/': search
    Search,
    /// LB / PageUp
    PageUp,
    /// RB / PageDown
    PageDown,
    /// LT / Home: jump to previous letter / start
    JumpUp,
    /// RT / End: jump to next letter / end
    JumpDown,
    /// Guide / home button
    Guide,
}

struct Shared {
    /// `Gate::to_u8`.
    gate: AtomicU8,
    /// Home opens Lodestar's menu (`ControllerConfig::home_opens_menu`).
    home: AtomicBool,
    cfg: Mutex<PadConfig>,
    cfg_gen: AtomicU64,
    pads: Mutex<Vec<String>>,
    /// Connected pads with their ids (for the prompt style) and the pad pressed last.
    pad_info: Mutex<Vec<style::PadInfo>>,
    used_pad: Mutex<Option<usize>>,
    /// gilrs is unavailable (keyboard only).
    failed: AtomicBool,
    /// The hold-repeat running now (a new number for each held input; 0: nothing repeats).
    repeat_id: AtomicU64,
    /// A hold-repeat was sent and the UI hasn't taken it yet: no second one is queued behind it.
    repeat_queued: AtomicBool,
    record: Mutex<Recording>,
}

/// Recording a menu button combination, between the screen and the pad thread.
struct Recording {
    /// The pad records until then (renewed by every `record_combo`; None: not recording).
    until: Option<Instant>,
    /// The buttons held so far.
    live: MenuCombo,
    /// The finished take, until the screen picks it up.
    done: Option<Recorded>,
}

impl Recording {
    /// Not recording, nothing held, no take.
    const OFF: Recording = Recording { until: None, live: MenuCombo::empty(), done: None };
}

/// A recording lapses this long after the screen last asked for it (closed some other way).
const RECORD_LEASE: Duration = Duration::from_millis(1500);

/// [`InputHub::record_combo`]: still waiting (the buttons held so far), or the take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordPoll {
    Waiting(MenuCombo),
    Done(Recorded),
}

/// A navigation from the pad thread and, for a hold-repeat, the repeat it belongs to (else 0).
type PadMsg = (Nav, u64);

pub struct InputHub {
    rx: Receiver<PadMsg>,
    shared: Arc<Shared>,
}

impl InputHub {
    pub fn new(ctx: &egui::Context, cfg: &ControllerConfig) -> InputHub {
        let (tx, rx) = unbounded();
        let shared = Arc::new(Shared {
            gate: AtomicU8::new(Gate::All.to_u8()),
            home: AtomicBool::new(cfg.home_opens_menu()),
            cfg: Mutex::new(PadConfig::from_controller(cfg)),
            cfg_gen: AtomicU64::new(0),
            pads: Mutex::new(vec![]),
            pad_info: Mutex::new(vec![]),
            used_pad: Mutex::new(None),
            failed: AtomicBool::new(false),
            repeat_id: AtomicU64::new(0),
            repeat_queued: AtomicBool::new(false),
            record: Mutex::new(Recording::OFF),
        });
        let sh = shared.clone();
        let ctx = ctx.clone();
        let r = std::thread::Builder::new().name("gamepad".into()).spawn(move || gamepad_thread(ctx, tx, sh));
        if let Err(e) = r {
            log::warn!("couldn't start the gamepad thread ({e}); keyboard only");
            shared.failed.store(true, Ordering::Relaxed);
        }
        InputHub { rx, shared }
    }

    pub fn set_config(&self, cfg: &ControllerConfig) {
        let new = PadConfig::from_controller(cfg);
        let mut c = self.shared.cfg.lock();
        if *c != new {
            *c = new;
            self.shared.cfg_gen.fetch_add(1, Ordering::Release);
        }
    }

    /// Cheap; call every frame. What the gate doesn't let through is dropped (not queued) and
    /// held buttons are forgotten, so a game running in the background can't drive Lodestar.
    pub fn set_gate(&self, gate: Gate) {
        let was = Gate::from_u8(self.shared.gate.swap(gate.to_u8(), Ordering::AcqRel));
        if was != gate && gate != Gate::All {
            // Drop anything sent just before under the wider gate.
            while self.rx.try_recv().is_ok() {}
        }
    }

    /// Cheap; call every frame: whether a Home press opens Lodestar's menu now (off: the pad
    /// thread ignores Home, e.g. left to Steam's overlay in a Steam game).
    pub fn set_home_button(&self, on: bool) {
        self.shared.home.store(on, Ordering::Release);
    }

    /// Record a menu button combination: the pad (while all of it reaches Lodestar) does nothing
    /// else until the buttons held together are let go, then this returns the take once and the
    /// pad is Lodestar's again. Call it every frame while waiting (with a repaint at least every
    /// second): the recording lapses when the calls stop.
    pub fn record_combo(&self) -> RecordPoll {
        let mut r = self.shared.record.lock();
        if let Some(take) = r.done.take() {
            r.until = None;
            return RecordPoll::Done(take);
        }
        r.until = Some(Instant::now() + RECORD_LEASE);
        RecordPoll::Waiting(r.live)
    }

    pub fn stop_recording(&self) {
        *self.shared.record.lock() = Recording::OFF;
    }

    pub fn gate(&self) -> Gate {
        Gate::from_u8(self.shared.gate.load(Ordering::Acquire))
    }

    /// Gamepad navigation since the last call. Hold-repeats never pile up: at most one waits
    /// at a time, and one whose button was let go (or another input took over) meanwhile is
    /// dropped, so a slow frame never keeps the selection moving after the release.
    pub fn drain(&mut self) -> Vec<Nav> {
        let msgs: Vec<PadMsg> = self.rx.try_iter().collect();
        self.shared.repeat_queued.store(false, Ordering::Release);
        let live = self.shared.repeat_id.load(Ordering::Acquire);
        let mut v = fresh_navs(msgs, live);
        match self.gate() {
            Gate::Off => v.clear(),
            Gate::GuideOnly => v.retain(|n| *n == Nav::Guide),
            Gate::All => {}
        }
        v
    }

    pub fn gamepads(&self) -> Vec<String> {
        self.shared.pads.lock().clone()
    }

    /// A controller is connected (cheap: no copy of the names).
    pub fn has_gamepads(&self) -> bool {
        !self.shared.pads.lock().is_empty()
    }

    /// Prompt style of the pad in use (last pressed, else the first connected); None without a pad.
    pub fn pad_style(&self) -> Option<ButtonStyle> {
        let used = *self.shared.used_pad.lock();
        style::pad_style(&self.shared.pad_info.lock(), used)
    }
}

/// The navigations of `msgs` without the hold-repeats of a repeat that isn't `live` any more.
fn fresh_navs(msgs: Vec<PadMsg>, live: u64) -> Vec<Nav> {
    msgs.into_iter().filter(|&(_, rep)| rep == 0 || rep == live).map(|m| m.0).collect()
}

fn gamepad_thread(ctx: egui::Context, tx: Sender<PadMsg>, sh: Arc<Shared>) {
    // No force feedback: gilrs' FF server thread wakes every 50 ms forever and writes an EV_FF
    // "play" to every pad on each tick, which evdev echoes back to all readers — so it also woke
    // this thread ~20×/s (measured: 200 + 100 wakeups per 10 s idle with one DS4 connected).
    let built = gilrs::GilrsBuilder::new().with_force_feedback(false).build();
    let mut gilrs = match built {
        Ok(g) => g,
        Err(e) => {
            log::warn!("gamepad support unavailable ({e}); keyboard only");
            sh.failed.store(true, Ordering::Relaxed);
            return;
        }
    };
    let names = |g: &gilrs::Gilrs| -> Vec<String> { g.gamepads().map(|(_, p)| p.name().to_string()).collect() };
    let refresh = |g: &gilrs::Gilrs| {
        *sh.pads.lock() = names(g);
        *sh.pad_info.lock() = g
            .gamepads()
            .map(|(id, p)| style::PadInfo {
                id: usize::from(id),
                name: p.name().to_string(),
                vendor: p.vendor_id(),
                product: p.product_id(),
            })
            .collect();
    };
    refresh(&gilrs);
    log::info!("gamepads: {:?}", sh.pads.lock());

    let mut mapper = PadMapper::new(*sh.cfg.lock());
    let mut cfg_gen = sh.cfg_gen.load(Ordering::Acquire);
    let mut out = Vec::new();
    let mut idle_spins = 0u32;
    // The hold-repeat running (its input) and its number (see `Shared::repeat_id`).
    let mut repeating: Option<Nav> = None;
    let mut repeat_n = 0u64;
    loop {
        let timeout =
            mapper.next_deadline().map(|d| d.saturating_duration_since(Instant::now()).max(Duration::from_millis(1)));
        let t_wait = Instant::now();
        let ev = gilrs.next_event_blocking(timeout);
        let now = Instant::now();

        // Pick up settings changed while we were blocked.
        let g = sh.cfg_gen.load(Ordering::Acquire);
        if g != cfg_gen {
            cfg_gen = g;
            mapper.set_config(*sh.cfg.lock());
        }
        let gate = Gate::from_u8(sh.gate.load(Ordering::Acquire));
        mapper.set_enabled(gate != Gate::Off);
        mapper.set_guide_only(gate == Gate::GuideOnly);
        mapper.set_home(sh.home.load(Ordering::Acquire));
        mapper.set_recording(sh.record.lock().until.is_some_and(|t| now < t));

        match ev {
            Some(gilrs::Event { id, event, .. }) => {
                idle_spins = 0;
                let pad = usize::from(id);
                use gilrs::EventType as E;
                match event {
                    E::ButtonPressed(b, _) => {
                        // Prompts follow the pad used last (only real input, not while a game runs).
                        if mapper.enabled() && !mapper.guide_only() {
                            let mut used = sh.used_pad.lock();
                            if *used != Some(pad) {
                                *used = Some(pad);
                                ctx.request_repaint();
                            }
                        }
                        mapper.feed(PadInput::Pressed(pad, b), now, &mut out)
                    }
                    E::ButtonReleased(b, _) => mapper.feed(PadInput::Released(pad, b), now, &mut out),
                    E::AxisChanged(a, v, _) => mapper.feed(PadInput::Axis(pad, a, v), now, &mut out),
                    E::Connected => {
                        refresh(&gilrs);
                        ctx.request_repaint(); // prompts may change style
                        log::info!("gamepad connected: {}", gilrs.gamepad(id).name());
                    }
                    E::Disconnected => {
                        mapper.feed(PadInput::Disconnected(pad), now, &mut out);
                        refresh(&gilrs);
                        ctx.request_repaint();
                        log::info!("gamepad disconnected");
                    }
                    _ => {}
                }
            }
            None => {
                // Returned without an event although nothing was due (epoll error): don't spin.
                let woke_early = timeout.is_none_or(|t| t_wait.elapsed() + Duration::from_millis(1) < t / 2);
                if woke_early {
                    idle_spins += 1;
                    if idle_spins > 3 {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                } else {
                    idle_spins = 0;
                }
            }
        }
        // A new held input (or none any more): a new repeat number; repeats of the old one
        // still waiting for the UI are dropped there.
        let now_repeating = mapper.repeating();
        if now_repeating != repeating {
            repeating = now_repeating;
            repeat_n += 1;
            sh.repeat_id.store(if repeating.is_some() { repeat_n } else { 0 }, Ordering::Release);
        }
        if mapper.recording() || mapper.recorded_pending() {
            let mut r = sh.record.lock();
            if let Some(take) = mapper.take_recording() {
                // One take: the pad is Lodestar's again until the screen asks for another.
                (r.done, r.until, r.live) = (Some(take), None, MenuCombo::empty());
                ctx.request_repaint();
            } else if r.live != mapper.recording_live() {
                r.live = mapper.recording_live();
                ctx.request_repaint();
            }
        }
        // A menu combination held long enough counts as a press (not a hold-repeat).
        mapper.tick_menu_combo(now, &mut out);
        let presses = out.len();
        mapper.tick(now, &mut out);
        // At most one hold-repeat waits for the UI.
        if out.len() > presses && sh.repeat_queued.swap(true, Ordering::AcqRel) {
            out.truncate(presses);
        }
        if !out.is_empty() {
            let enabled = mapper.enabled();
            let id = sh.repeat_id.load(Ordering::Relaxed);
            for (i, n) in out.drain(..).enumerate() {
                let msg = (n, if i >= presses { id } else { 0 });
                if enabled && tx.send(msg).is_err() {
                    return; // hub dropped
                }
            }
            if enabled {
                ctx.request_repaint();
            }
        }
    }
}

// ----- keyboard -----

/// Map one key press. `typing`: a text field has focus, so only Escape/Enter navigate.
pub fn map_key(key: egui::Key, m: egui::Modifiers, typing: bool) -> Option<Nav> {
    use egui::Key as K;
    if typing {
        return match key {
            K::Escape => Some(Nav::Back),
            K::Enter => Some(Nav::Accept),
            _ => None,
        };
    }
    if m.alt {
        return None; // Alt+Enter, Alt+F4 … belong to the window manager
    }
    let ctrl = m.ctrl || m.command;
    Some(match key {
        K::F if ctrl => Nav::Search,
        _ if ctrl => return None,
        K::ArrowUp => Nav::Up,
        K::ArrowDown => Nav::Down,
        K::ArrowLeft => Nav::Left,
        K::ArrowRight => Nav::Right,
        K::Enter | K::Space => Nav::Accept,
        K::Escape | K::Backspace => Nav::Back,
        K::F1 | K::Menu => Nav::Menu,
        K::Tab => Nav::Select,
        K::F => Nav::Favorite,
        K::Slash => Nav::Search,
        K::PageUp => Nav::PageUp,
        K::PageDown => Nav::PageDown,
        K::Home => Nav::JumpUp,
        K::End => Nav::JumpDown,
        _ => return None,
    })
}

/// Navigation from this frame's key presses (egui key-repeats included). Only called in
/// controller mode: mapped keys are consumed so egui widgets don't also react to them, and the
/// text those keys would type ("/", "f", " ") is dropped. While a text field has focus, only
/// Escape (Back) and Enter (Accept) are reported and nothing is consumed, so typing works.
pub fn keyboard_nav(ctx: &egui::Context) -> Vec<Nav> {
    let typing = ctx.text_edit_focused();
    ctx.input_mut(|i| {
        let mut out = vec![];
        if typing {
            for e in &i.events {
                if let egui::Event::Key { key, pressed: true, modifiers, .. } = e {
                    out.extend(map_key(*key, *modifiers, true));
                }
            }
            return out;
        }
        let mut swallow_text = false;
        // OS key-repeats that piled up during a slow frame count once (like the pad's
        // hold-repeat), so letting go of a held key stops the selection at once.
        let mut repeated: Vec<Nav> = vec![];
        i.events.retain(|e| match e {
            egui::Event::Key { key, pressed: true, modifiers, repeat, .. } => match map_key(*key, *modifiers, false) {
                Some(n) => {
                    if *repeat {
                        if repeated.contains(&n) {
                            return false;
                        }
                        repeated.push(n);
                    }
                    out.push(n);
                    swallow_text |= matches!(key, egui::Key::Slash | egui::Key::F | egui::Key::Space);
                    false
                }
                None => true,
            },
            _ => true,
        });
        if swallow_text {
            i.events.retain(|e| !matches!(e, egui::Event::Text(t) if matches!(t.as_str(), "/" | "f" | "F" | " ")));
        }
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeats_of_a_released_input_are_dropped() {
        let msgs = vec![(Nav::Right, 0), (Nav::Right, 3), (Nav::Accept, 0), (Nav::Right, 4)];
        assert_eq!(fresh_navs(msgs.clone(), 4), vec![Nav::Right, Nav::Accept, Nav::Right], "repeat 3 ended");
        assert_eq!(fresh_navs(msgs, 0), vec![Nav::Right, Nav::Accept], "let go: no repeat gets through");
    }
    use egui::{Event, Key, Modifiers, RawInput};

    fn key(k: Key, m: Modifiers, repeat: bool) -> Event {
        Event::Key { key: k, physical_key: None, pressed: true, repeat, modifiers: m }
    }

    #[test]
    fn key_mapping() {
        let n = Modifiers::NONE;
        assert_eq!(map_key(Key::ArrowDown, n, false), Some(Nav::Down));
        assert_eq!(map_key(Key::Enter, n, false), Some(Nav::Accept));
        assert_eq!(map_key(Key::Space, n, false), Some(Nav::Accept));
        assert_eq!(map_key(Key::Backspace, n, false), Some(Nav::Back));
        assert_eq!(map_key(Key::F1, n, false), Some(Nav::Menu));
        assert_eq!(map_key(Key::Tab, n, false), Some(Nav::Select));
        assert_eq!(map_key(Key::F, n, false), Some(Nav::Favorite));
        assert_eq!(map_key(Key::F, Modifiers::CTRL, false), Some(Nav::Search));
        assert_eq!(map_key(Key::Slash, n, false), Some(Nav::Search));
        assert_eq!(map_key(Key::Home, n, false), Some(Nav::JumpUp));
        assert_eq!(map_key(Key::End, n, false), Some(Nav::JumpDown));
        assert_eq!(map_key(Key::PageDown, n, false), Some(Nav::PageDown));
        assert_eq!(map_key(Key::Enter, Modifiers::ALT, false), None);
        assert_eq!(map_key(Key::Q, Modifiers::CTRL, false), None);
        assert_eq!(map_key(Key::A, n, false), None);
        // Typing: only Escape / Enter.
        assert_eq!(map_key(Key::Escape, n, true), Some(Nav::Back));
        assert_eq!(map_key(Key::Enter, n, true), Some(Nav::Accept));
        assert_eq!(map_key(Key::Backspace, n, true), None);
        assert_eq!(map_key(Key::F, n, true), None);
        assert_eq!(map_key(Key::ArrowDown, n, true), None);
    }

    #[test]
    fn keyboard_nav_consumes_mapped_keys() {
        let ctx = egui::Context::default();
        let events = vec![
            key(Key::ArrowDown, Modifiers::NONE, false),
            key(Key::ArrowDown, Modifiers::NONE, true), // OS key-repeat counts
            key(Key::A, Modifiers::NONE, false),
            key(Key::Slash, Modifiers::NONE, false),
            Event::Text("/".into()),
            Event::Text("a".into()),
        ];
        ctx.begin_pass(RawInput { events, ..Default::default() });
        let navs = keyboard_nav(&ctx);
        assert_eq!(navs, vec![Nav::Down, Nav::Down, Nav::Search]);
        let left = ctx.input(|i| i.events.clone());
        assert_eq!(left, vec![key(Key::A, Modifiers::NONE, false), Event::Text("a".into())]);
        ctx.end_pass().textures_delta.clear();
    }

    #[test]
    fn piled_up_key_repeats_count_once() {
        let ctx = egui::Context::default();
        // Frame 1: Right goes down.
        ctx.begin_pass(RawInput { events: vec![key(Key::ArrowRight, Modifiers::NONE, false)], ..Default::default() });
        assert_eq!(keyboard_nav(&ctx), vec![Nav::Right]);
        ctx.end_pass().textures_delta.clear();
        // Frame 2 (slow): three repeats of the held key piled up, plus two presses of Down.
        let events = vec![
            key(Key::ArrowRight, Modifiers::NONE, true),
            key(Key::ArrowRight, Modifiers::NONE, true),
            key(Key::ArrowRight, Modifiers::NONE, true),
            key(Key::ArrowDown, Modifiers::NONE, false),
            key(Key::ArrowDown, Modifiers::NONE, false),
        ];
        ctx.begin_pass(RawInput { events, ..Default::default() });
        let navs = keyboard_nav(&ctx);
        assert_eq!(navs, vec![Nav::Right, Nav::Down, Nav::Down], "presses all count, repeats once");
        assert!(ctx.input(|i| i.events.is_empty()), "the dropped repeats are consumed too");
        ctx.end_pass().textures_delta.clear();
    }

    #[test]
    fn keyboard_nav_lets_text_fields_type() {
        let ctx = egui::Context::default();
        let mut text = String::new();
        let id = egui::Id::unique("field");
        // Frame 1: create and focus a text field.
        ctx.run_ui(RawInput::default(), |ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut text).id(id));
            r.request_focus();
        })
        .textures_delta
        .clear();
        ctx.run_ui(RawInput::default(), |ui| {
            ui.add(egui::TextEdit::singleline(&mut text).id(id));
        })
        .textures_delta
        .clear();
        assert!(ctx.text_edit_focused());
        let events =
            vec![key(Key::F, Modifiers::NONE, false), Event::Text("f".into()), key(Key::Enter, Modifiers::NONE, false)];
        ctx.begin_pass(RawInput { events: events.clone(), ..Default::default() });
        let navs = keyboard_nav(&ctx);
        assert_eq!(navs, vec![Nav::Accept]);
        assert_eq!(ctx.input(|i| i.events.clone()), events, "nothing consumed while typing");
        ctx.end_pass().textures_delta.clear();
    }

    #[test]
    fn hub_without_gamepads_is_quiet() {
        let ctx = egui::Context::default();
        let mut hub = InputHub::new(&ctx, &ControllerConfig::default());
        hub.set_gate(Gate::Off);
        assert_eq!(hub.gate(), Gate::Off);
        hub.set_gate(Gate::GuideOnly);
        hub.set_gate(Gate::All);
        hub.set_config(&ControllerConfig { swap_ab: true, ..Default::default() });
        std::thread::sleep(Duration::from_millis(50));
        // A real controller in use on this machine sends events; the test is about a quiet hub.
        if !hub.gamepads().is_empty() {
            eprintln!("a real gamepad is connected; skipping");
            return;
        }
        assert!(hub.drain().is_empty());
    }

    /// Voluntary context switches (≈ wakeups) of this process' gamepad-related threads.
    fn input_thread_wakeups() -> u64 {
        let mut total = 0;
        for t in std::fs::read_dir("/proc/self/task").unwrap().flatten() {
            let comm = std::fs::read_to_string(t.path().join("comm")).unwrap_or_default();
            if !matches!(comm.trim(), "gamepad" | "gilrs") {
                continue;
            }
            let status = std::fs::read_to_string(t.path().join("status")).unwrap_or_default();
            total += status
                .lines()
                .find_map(|l| l.strip_prefix("voluntary_ctxt_switches:"))
                .and_then(|v| v.trim().parse::<u64>().ok())
                .unwrap_or(0);
        }
        total
    }

    #[test]
    fn idle_input_threads_sleep() {
        let ctx = egui::Context::default();
        let hub = InputHub::new(&ctx, &ControllerConfig::default());
        std::thread::sleep(Duration::from_millis(300)); // let gilrs finish starting
        // A real controller in use wakes the threads with its events; idling can't be measured.
        if !hub.gamepads().is_empty() {
            eprintln!("a real gamepad is connected; skipping");
            return;
        }
        let before = input_thread_wakeups();
        std::thread::sleep(Duration::from_millis(1500));
        let woke = input_thread_wakeups().saturating_sub(before);
        // gilrs' force-feedback ticker alone would wake ~30 times in 1.5 s.
        assert!(woke < 8, "input threads woke {woke} times in 1.5 s while idle");
    }
}
