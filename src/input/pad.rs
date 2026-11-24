//! Gamepad -> `Nav` state machine (button mapping, stick-as-d-pad with hysteresis, hold-repeat,
//! the menu button). Pure and clock-injected so it can be unit-tested without a gamepad.
//!
//! The MENU BUTTON (Settings → Input → "Lodestar's menu button") is a button combination or
//! Home; either comes out as `Nav::Guide`. The combination counts only when its buttons go down
//! together (each within `COMBO_WINDOW` of the first) and stay held for `COMBO_HOLD`, so a quick
//! L3 + R3 in a game never opens Lodestar. Its buttons keep their own meaning otherwise (Select,
//! Start …); while only Guide gets through (a game runs), they produce nothing else.
//!
//! RECORDING a combination (Settings → "Record a combination"): while on, and only while all of
//! the pad reaches Lodestar, the mapper owns the pad. Nothing comes out; the most buttons held at
//! once until all are let go is the take (`take_recording`), and recording ends by itself.

use super::Nav;
use crate::config::{MenuCombo, PadButton};
use gilrs::{Axis, Button};
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadConfig {
    pub swap_ab: bool,
    pub repeat_delay: Duration,
    pub repeat_interval: Duration,
    /// The button combination that opens Lodestar's menu (None: Home only).
    pub menu_combo: Option<MenuCombo>,
}

impl PadConfig {
    pub fn from_controller(c: &crate::config::ControllerConfig) -> PadConfig {
        PadConfig {
            swap_ab: c.swap_ab,
            repeat_delay: Duration::from_millis(c.repeat_delay_ms.clamp(50, 5000)),
            repeat_interval: Duration::from_millis(c.repeat_interval_ms.clamp(16, 2000)),
            menu_combo: c.active_menu_combo(),
        }
    }
}

/// Raw input from one pad (`pad` is gilrs' GamepadId as usize).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PadInput {
    Pressed(usize, Button),
    Released(usize, Button),
    Axis(usize, Axis, f32),
    Disconnected(usize),
}

/// Stick thresholds: press beyond 0.5, release below 0.35.
pub const STICK_PRESS: f32 = 0.5;
pub const STICK_RELEASE: f32 = 0.35;
/// The same press arriving from a second pad this soon is a duplicate (e.g. Steam Input's
/// virtual pad mirroring the physical one) and is dropped.
pub const DUPLICATE_WINDOW: Duration = Duration::from_millis(40);
/// The menu combination's buttons all go down within this time of the first one ...
pub const COMBO_WINDOW: Duration = Duration::from_millis(150);
/// ... and stay held this long before it counts.
pub const COMBO_HOLD: Duration = Duration::from_millis(400);

/// The combination button a gilrs button is (gilrs: LeftTrigger = LB, LeftThumb = L3; None:
/// Home and buttons a combination can't use).
pub fn pad_button(b: Button) -> Option<PadButton> {
    Some(match b {
        Button::LeftThumb => PadButton::L3,
        Button::RightThumb => PadButton::R3,
        Button::LeftTrigger => PadButton::Lb,
        Button::RightTrigger => PadButton::Rb,
        Button::LeftTrigger2 => PadButton::Lt,
        Button::RightTrigger2 => PadButton::Rt,
        Button::Select => PadButton::Select,
        Button::Start => PadButton::Start,
        Button::South => PadButton::A,
        Button::East => PadButton::B,
        Button::West => PadButton::X,
        Button::North => PadButton::Y,
        Button::DPadUp => PadButton::DpadUp,
        Button::DPadDown => PadButton::DpadDown,
        Button::DPadLeft => PadButton::DpadLeft,
        Button::DPadRight => PadButton::DpadRight,
        _ => return None,
    })
}

/// The gilrs button of a combination button (the inverse of [`pad_button`]; tests press
/// combinations with it).
#[cfg(test)]
pub fn gilrs_button(b: PadButton) -> Button {
    match b {
        PadButton::L3 => Button::LeftThumb,
        PadButton::R3 => Button::RightThumb,
        PadButton::Lb => Button::LeftTrigger,
        PadButton::Rb => Button::RightTrigger,
        PadButton::Lt => Button::LeftTrigger2,
        PadButton::Rt => Button::RightTrigger2,
        PadButton::Select => Button::Select,
        PadButton::Start => Button::Start,
        PadButton::A => Button::South,
        PadButton::B => Button::East,
        PadButton::X => Button::West,
        PadButton::Y => Button::North,
        PadButton::DpadUp => Button::DPadUp,
        PadButton::DpadDown => Button::DPadDown,
        PadButton::DpadLeft => Button::DPadLeft,
        PadButton::DpadRight => Button::DPadRight,
    }
}

/// One pad's progress on the menu combination.
#[derive(Clone, Debug)]
struct ComboHold {
    /// The combination's buttons held now.
    down: MenuCombo,
    /// When the first of them went down (none was held before).
    first: Instant,
    /// All went down in time: counts at this instant if still held.
    due: Option<Instant>,
    /// Counted already; another needs every button let go first.
    fired: bool,
}

/// A recording's progress (all pads together).
#[derive(Clone, Copy, Debug)]
struct Recorder {
    /// Held now.
    down: MenuCombo,
    /// Home is held now.
    home_down: bool,
    /// The take so far: the most buttons held at once.
    best: MenuCombo,
    /// Home was pressed with them (it can't be part of a combination).
    home: bool,
}

impl Recorder {
    /// Nothing held, nothing taken.
    const START: Recorder =
        Recorder { down: MenuCombo::empty(), home_down: false, best: MenuCombo::empty(), home: false };
}

/// What a recording caught: the buttons held together, and whether Home was among them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recorded {
    pub combo: MenuCombo,
    pub home: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Src {
    Button(Button),
    Stick,
}

#[derive(Clone, Copy, Debug)]
struct Repeat {
    pad: usize,
    src: Src,
    nav: Nav,
    next: Instant,
}

#[derive(Clone, Copy, Debug, Default)]
struct Stick {
    x: f32,
    y: f32,
    dir: Option<Nav>,
}

pub fn map_button(b: Button, swap_ab: bool) -> Option<Nav> {
    Some(match b {
        Button::South if swap_ab => Nav::Back,
        Button::East if swap_ab => Nav::Accept,
        Button::South => Nav::Accept,
        Button::East => Nav::Back,
        Button::West => Nav::Favorite,
        Button::North => Nav::Search,
        Button::Start => Nav::Menu,
        Button::Select => Nav::Select,
        Button::Mode => Nav::Guide,
        Button::LeftTrigger => Nav::PageUp,
        Button::RightTrigger => Nav::PageDown,
        Button::LeftTrigger2 => Nav::JumpUp,
        Button::RightTrigger2 => Nav::JumpDown,
        Button::DPadUp => Nav::Up,
        Button::DPadDown => Nav::Down,
        Button::DPadLeft => Nav::Left,
        Button::DPadRight => Nav::Right,
        _ => return None,
    })
}

pub fn repeats(n: Nav) -> bool {
    matches!(
        n,
        Nav::Up | Nav::Down | Nav::Left | Nav::Right | Nav::PageUp | Nav::PageDown | Nav::JumpUp | Nav::JumpDown
    )
}

/// How far the stick points in direction `d` (gilrs: +y is up).
fn component(s: &Stick, d: Nav) -> f32 {
    match d {
        Nav::Up => s.y,
        Nav::Down => -s.y,
        Nav::Right => s.x,
        Nav::Left => -s.x,
        _ => 0.0,
    }
}

const DIRS: [Nav; 4] = [Nav::Up, Nav::Down, Nav::Left, Nav::Right];

/// Next stick direction given the current one (dominant axis + hysteresis).
fn stick_dir(s: &Stick) -> Option<Nav> {
    let best = DIRS
        .iter()
        .copied()
        .max_by(|a, b| component(s, *a).total_cmp(&component(s, *b)))
        .filter(|d| component(s, *d) >= STICK_PRESS);
    match s.dir {
        Some(cur) => {
            let c = component(s, cur);
            if c < STICK_RELEASE {
                best
            } else {
                match best {
                    Some(b) if b != cur && component(s, b) > c => Some(b),
                    _ => Some(cur),
                }
            }
        }
        None => best,
    }
}

pub struct PadMapper {
    cfg: PadConfig,
    enabled: bool,
    /// Only the menu button is mapped (a game runs and Lodestar isn't focused).
    guide_only: bool,
    /// Home opens the menu now (see `ControllerConfig::home_opens_menu`); off, Home is ignored.
    home: bool,
    combos: HashMap<usize, ComboHold>,
    /// Recording a combination (see the module docs).
    recorder: Option<Recorder>,
    recorded: Option<Recorded>,
    held: Vec<(usize, Src, Nav)>,
    repeat: Option<Repeat>,
    sticks: HashMap<usize, Stick>,
    last_press: Option<(usize, Nav, Instant)>,
}

impl PadMapper {
    pub fn new(cfg: PadConfig) -> PadMapper {
        PadMapper {
            cfg,
            enabled: true,
            guide_only: false,
            home: true,
            combos: HashMap::new(),
            recorder: None,
            recorded: None,
            held: vec![],
            repeat: None,
            sticks: HashMap::new(),
            last_press: None,
        }
    }

    pub fn set_config(&mut self, cfg: PadConfig) {
        if cfg != self.cfg {
            self.cfg = cfg;
            self.clear();
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Disabled: every input is dropped and held state is forgotten.
    pub fn set_enabled(&mut self, on: bool) {
        if on != self.enabled {
            self.enabled = on;
            self.clear();
        }
    }

    /// Guide only: every input but a Guide press is dropped, held state is forgotten.
    pub fn set_guide_only(&mut self, on: bool) {
        if on != self.guide_only {
            self.guide_only = on;
            self.clear();
        }
    }

    pub fn guide_only(&self) -> bool {
        self.guide_only
    }

    /// Whether Home opens the menu (the combination is set by the config).
    pub fn set_home(&mut self, on: bool) {
        self.home = on;
    }

    /// Record a combination (on) or not; starting forgets what is held, so nothing repeats.
    pub fn set_recording(&mut self, on: bool) {
        if on != self.recorder.is_some() {
            self.clear();
            self.recorder = on.then_some(Recorder::START);
        }
    }

    pub fn recording(&self) -> bool {
        self.recorder.is_some()
    }

    /// The buttons held so far in the recording (for the screen to show live).
    pub fn recording_live(&self) -> MenuCombo {
        self.recorder.map_or(MenuCombo::empty(), |r| r.best)
    }

    /// The finished take, once: all buttons were let go (recording is off again then).
    pub fn take_recording(&mut self) -> Option<Recorded> {
        self.recorded.take()
    }

    /// A finished take waits for [`PadMapper::take_recording`].
    pub fn recorded_pending(&self) -> bool {
        self.recorded.is_some()
    }

    /// The mapper owns the pad for a recording: everything of the pad reaches Lodestar.
    fn recording_owns_input(&self) -> bool {
        self.recorder.is_some() && self.enabled && !self.guide_only
    }

    fn record(&mut self, input: PadInput) {
        let Some(r) = &mut self.recorder else { return };
        match input {
            PadInput::Pressed(_, Button::Mode) => (r.home_down, r.home) = (true, true),
            PadInput::Released(_, Button::Mode) => r.home_down = false,
            PadInput::Pressed(_, b) => {
                if let Some(pb) = pad_button(b) {
                    r.down = r.down.with(pb);
                    if r.down.len() > r.best.len() {
                        r.best = r.down;
                    }
                }
            }
            PadInput::Released(_, b) => {
                if let Some(pb) = pad_button(b) {
                    r.down = r.down.without(pb);
                }
            }
            PadInput::Axis(..) | PadInput::Disconnected(_) => {}
        }
        if r.down.is_empty() && !r.home_down && (!r.best.is_empty() || r.home) {
            self.recorded = Some(Recorded { combo: r.best, home: r.home });
            self.recorder = None;
        }
    }

    fn clear(&mut self) {
        self.held.clear();
        self.repeat = None;
        for s in self.sticks.values_mut() {
            s.dir = None;
        }
        self.last_press = None;
        self.combos.clear();
        if let Some(r) = &mut self.recorder {
            *r = Recorder::START;
        }
    }

    /// The input that repeats while held (None: nothing repeats).
    pub fn repeating(&self) -> Option<Nav> {
        self.repeat.map(|r| r.nav)
    }

    /// When the next hold-repeat or a held menu combination is due (None: nothing held).
    pub fn next_deadline(&self) -> Option<Instant> {
        let combo = self.combos.values().filter_map(|c| c.due).min();
        match (self.repeat.map(|r| r.next), combo) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    pub fn feed(&mut self, input: PadInput, now: Instant, out: &mut Vec<Nav>) {
        match input {
            PadInput::Disconnected(pad) => {
                self.held.retain(|h| h.0 != pad);
                if self.repeat.is_some_and(|r| r.pad == pad) {
                    self.resume_repeat(now);
                }
                self.sticks.remove(&pad);
                self.combos.remove(&pad);
            }
            _ if !self.enabled => {}
            _ if self.recording_owns_input() => self.record(input),
            PadInput::Pressed(pad, b) => {
                let joins_combo = self.combo_press(pad, b, now);
                let home = map_button(b, false) == Some(Nav::Guide);
                if home && !self.home {
                    // Left to Steam (settings): Lodestar ignores it.
                } else if joins_combo {
                    // Part of the menu combination being pressed: not its own meaning too, and
                    // the button that started it stops repeating (LB's page up).
                    self.combo_takes_over(pad, now);
                } else if self.guide_only {
                    if home {
                        out.push(Nav::Guide);
                    }
                } else if let Some(nav) = map_button(b, self.cfg.swap_ab) {
                    self.press(pad, Src::Button(b), nav, now, out);
                }
            }
            PadInput::Released(pad, b) => {
                self.combo_release(pad, b);
                if !self.guide_only {
                    self.release(pad, Src::Button(b), now);
                }
            }
            _ if self.guide_only => {}
            PadInput::Axis(pad, axis, v) => {
                if !matches!(axis, Axis::LeftStickX | Axis::LeftStickY) {
                    return;
                }
                let v = if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 };
                let s = self.sticks.entry(pad).or_default();
                if axis == Axis::LeftStickX {
                    s.x = v;
                } else {
                    s.y = v;
                }
                let old = s.dir;
                let new = stick_dir(s);
                s.dir = new;
                if old != new {
                    if old.is_some() {
                        self.release(pad, Src::Stick, now);
                    }
                    if let Some(n) = new {
                        self.press(pad, Src::Stick, n, now, out);
                    }
                }
            }
        }
    }

    fn press(&mut self, pad: usize, src: Src, nav: Nav, now: Instant, out: &mut Vec<Nav>) {
        if self.held.iter().any(|h| h.0 == pad && h.1 == src) {
            return;
        }
        if let Some((p, n, t)) = self.last_press
            && p != pad
            && n == nav
            && now.saturating_duration_since(t) < DUPLICATE_WINDOW
        {
            return;
        }
        self.last_press = Some((pad, nav, now));
        self.held.push((pad, src, nav));
        out.push(nav);
        if repeats(nav) {
            self.repeat = Some(Repeat { pad, src, nav, next: now + self.cfg.repeat_delay });
        }
    }

    fn release(&mut self, pad: usize, src: Src, now: Instant) {
        self.held.retain(|h| !(h.0 == pad && h.1 == src));
        if self.repeat.is_some_and(|r| r.pad == pad && r.src == src) {
            self.resume_repeat(now);
        }
    }

    /// The repeating input was let go: hand the repeat to the newest input still held (after a
    /// fresh delay), e.g. hold Down, tap Right, and Down keeps scrolling.
    fn resume_repeat(&mut self, now: Instant) {
        self.repeat = self.held.iter().rev().find(|h| repeats(h.2)).map(|&(pad, src, nav)| Repeat {
            pad,
            src,
            nav,
            next: now + self.cfg.repeat_delay,
        });
    }

    /// Track a press for the menu combination. True when `b` joins a combination already being
    /// pressed (in time after its first button).
    fn combo_press(&mut self, pad: usize, b: Button, now: Instant) -> bool {
        let Some(combo) = self.cfg.menu_combo else { return false };
        let Some(pb) = pad_button(b).filter(|pb| combo.contains(*pb)) else { return false };
        let c = self.combos.entry(pad).or_insert(ComboHold {
            down: MenuCombo::empty(),
            first: now,
            due: None,
            fired: false,
        });
        if c.down.contains(pb) {
            return false;
        }
        let joins = !c.down.is_empty();
        c.down = c.down.with(pb);
        let in_time = now.saturating_duration_since(c.first) <= COMBO_WINDOW;
        if !c.fired && c.due.is_none() && in_time && c.down.covers(combo) {
            c.due = Some(now + COMBO_HOLD);
        }
        joins && in_time
    }

    /// A combination is being pressed on `pad`: its buttons held there no longer count as held
    /// inputs of their own, so none of them keeps repeating (another held input takes over).
    fn combo_takes_over(&mut self, pad: usize, now: Instant) {
        let Some(combo) = self.cfg.menu_combo else { return };
        let part = |p: usize, src: Src| {
            p == pad && matches!(src, Src::Button(b) if pad_button(b).is_some_and(|pb| combo.contains(pb)))
        };
        self.held.retain(|h| !part(h.0, h.1));
        if self.repeat.is_some_and(|r| part(r.pad, r.src)) {
            self.resume_repeat(now);
        }
    }

    fn combo_release(&mut self, pad: usize, b: Button) {
        let (Some(c), Some(pb)) = (self.combos.get_mut(&pad), pad_button(b)) else { return };
        if c.down.contains(pb) {
            c.down = c.down.without(pb);
            c.due = None;
            if c.down.is_empty() {
                self.combos.remove(&pad);
            }
        }
    }

    /// Emit the menu button (`Nav::Guide`) for a combination held long enough by `now`.
    pub fn tick_menu_combo(&mut self, now: Instant, out: &mut Vec<Nav>) {
        if !self.enabled {
            return;
        }
        let mut fire = false;
        for c in self.combos.values_mut() {
            if c.due.is_some_and(|d| now >= d) {
                c.due = None;
                c.fired = true;
                fire = true;
            }
        }
        if fire {
            out.push(Nav::Guide);
        }
    }

    /// Emit a hold-repeat if one is due at `now` (at most one per call; no bursts after stalls).
    pub fn tick(&mut self, now: Instant, out: &mut Vec<Nav>) {
        if !self.enabled || self.guide_only {
            return;
        }
        if let Some(r) = &mut self.repeat
            && now >= r.next
        {
            out.push(r.nav);
            r.next += self.cfg.repeat_interval;
            if r.next <= now {
                r.next = now + self.cfg.repeat_interval;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> PadConfig {
        PadConfig {
            swap_ab: false,
            repeat_delay: Duration::from_millis(300),
            repeat_interval: Duration::from_millis(100),
            menu_combo: None,
        }
    }

    fn ms(t0: Instant, n: u64) -> Instant {
        t0 + Duration::from_millis(n)
    }

    #[test]
    fn button_mapping_and_swap() {
        assert_eq!(map_button(Button::South, false), Some(Nav::Accept));
        assert_eq!(map_button(Button::East, false), Some(Nav::Back));
        assert_eq!(map_button(Button::South, true), Some(Nav::Back));
        assert_eq!(map_button(Button::East, true), Some(Nav::Accept));
        assert_eq!(map_button(Button::West, true), Some(Nav::Favorite));
        assert_eq!(map_button(Button::North, false), Some(Nav::Search));
        assert_eq!(map_button(Button::Start, false), Some(Nav::Menu));
        assert_eq!(map_button(Button::Select, false), Some(Nav::Select));
        assert_eq!(map_button(Button::Mode, false), Some(Nav::Guide));
        assert_eq!(map_button(Button::LeftTrigger, false), Some(Nav::PageUp));
        assert_eq!(map_button(Button::RightTrigger, false), Some(Nav::PageDown));
        assert_eq!(map_button(Button::LeftTrigger2, false), Some(Nav::JumpUp));
        assert_eq!(map_button(Button::RightTrigger2, false), Some(Nav::JumpDown));
        assert_eq!(map_button(Button::DPadLeft, false), Some(Nav::Left));
        assert_eq!(map_button(Button::LeftThumb, false), None);
    }

    #[test]
    fn hold_repeat_timing() {
        let t0 = Instant::now();
        let mut m = PadMapper::new(cfg());
        let mut out = vec![];
        m.feed(PadInput::Pressed(0, Button::DPadDown), t0, &mut out);
        assert_eq!(out, vec![Nav::Down]);
        assert_eq!(m.next_deadline(), Some(ms(t0, 300)));
        m.tick(ms(t0, 299), &mut out);
        assert_eq!(out.len(), 1);
        m.tick(ms(t0, 300), &mut out);
        m.tick(ms(t0, 350), &mut out);
        m.tick(ms(t0, 400), &mut out);
        m.tick(ms(t0, 500), &mut out);
        assert_eq!(out, vec![Nav::Down; 4]);
        // A stall emits once, not a burst.
        m.tick(ms(t0, 2000), &mut out);
        m.tick(ms(t0, 2001), &mut out);
        assert_eq!(out.len(), 5);
        assert_eq!(m.next_deadline(), Some(ms(t0, 2100)));
        m.feed(PadInput::Released(0, Button::DPadDown), ms(t0, 2050), &mut out);
        assert_eq!(m.next_deadline(), None);
        m.tick(ms(t0, 5000), &mut out);
        assert_eq!(out.len(), 5);
    }

    #[test]
    fn non_directional_buttons_do_not_repeat() {
        let t0 = Instant::now();
        let mut m = PadMapper::new(cfg());
        let mut out = vec![];
        m.feed(PadInput::Pressed(0, Button::South), t0, &mut out);
        assert_eq!(m.next_deadline(), None);
        m.tick(ms(t0, 1000), &mut out);
        assert_eq!(out, vec![Nav::Accept]);
        // Page/jump do repeat.
        m.feed(PadInput::Pressed(0, Button::RightTrigger2), ms(t0, 1000), &mut out);
        m.tick(ms(t0, 1300), &mut out);
        assert_eq!(out, vec![Nav::Accept, Nav::JumpDown, Nav::JumpDown]);
    }

    #[test]
    fn newest_press_owns_the_repeat() {
        let t0 = Instant::now();
        let mut m = PadMapper::new(cfg());
        let mut out = vec![];
        m.feed(PadInput::Pressed(0, Button::DPadDown), t0, &mut out);
        m.feed(PadInput::Pressed(0, Button::DPadRight), ms(t0, 100), &mut out);
        m.tick(ms(t0, 400), &mut out);
        assert_eq!(out, vec![Nav::Down, Nav::Right, Nav::Right]);
        // Releasing the older one doesn't stop the newer repeat.
        m.feed(PadInput::Released(0, Button::DPadDown), ms(t0, 450), &mut out);
        m.tick(ms(t0, 500), &mut out);
        assert_eq!(out.last(), Some(&Nav::Right));
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn repeat_returns_to_the_input_still_held() {
        let t0 = Instant::now();
        let mut m = PadMapper::new(cfg());
        let mut out = vec![];
        m.feed(PadInput::Pressed(0, Button::DPadDown), t0, &mut out);
        m.feed(PadInput::Pressed(0, Button::DPadRight), ms(t0, 100), &mut out);
        m.feed(PadInput::Released(0, Button::DPadRight), ms(t0, 150), &mut out);
        assert_eq!(m.next_deadline(), Some(ms(t0, 450)), "fresh delay, no instant repeat");
        m.tick(ms(t0, 449), &mut out);
        m.tick(ms(t0, 450), &mut out);
        assert_eq!(out, vec![Nav::Down, Nav::Right, Nav::Down]);
        // Non-repeating buttons still held don't take over.
        m.feed(PadInput::Pressed(0, Button::South), ms(t0, 500), &mut out);
        m.feed(PadInput::Released(0, Button::DPadDown), ms(t0, 510), &mut out);
        assert_eq!(m.next_deadline(), None);
        // Stick released while the d-pad is held: the d-pad repeats again.
        m.feed(PadInput::Pressed(1, Button::DPadUp), ms(t0, 600), &mut out);
        m.feed(PadInput::Axis(1, Axis::LeftStickX, 1.0), ms(t0, 700), &mut out);
        m.feed(PadInput::Axis(1, Axis::LeftStickX, 0.0), ms(t0, 800), &mut out);
        assert_eq!(m.next_deadline(), Some(ms(t0, 1100)));
        m.tick(ms(t0, 1100), &mut out);
        assert_eq!(out.last(), Some(&Nav::Up));
        // A disconnect hands over too (to another pad's held input).
        m.feed(PadInput::Pressed(2, Button::DPadLeft), ms(t0, 1200), &mut out);
        m.feed(PadInput::Disconnected(2), ms(t0, 1250), &mut out);
        assert_eq!(m.next_deadline(), Some(ms(t0, 1550)));
    }

    #[test]
    fn stick_hysteresis_and_dominant_axis() {
        let t0 = Instant::now();
        let mut m = PadMapper::new(cfg());
        let mut out = vec![];
        m.feed(PadInput::Axis(0, Axis::LeftStickY, -0.4), t0, &mut out);
        assert!(out.is_empty(), "below press threshold");
        m.feed(PadInput::Axis(0, Axis::LeftStickY, -0.6), t0, &mut out);
        assert_eq!(out, vec![Nav::Down]);
        // Wobble between release and press thresholds: still held, no new press.
        m.feed(PadInput::Axis(0, Axis::LeftStickY, -0.4), t0, &mut out);
        m.feed(PadInput::Axis(0, Axis::LeftStickY, -0.55), t0, &mut out);
        assert_eq!(out.len(), 1);
        assert!(m.next_deadline().is_some());
        // Small sideways drift doesn't steal the direction.
        m.feed(PadInput::Axis(0, Axis::LeftStickX, 0.3), t0, &mut out);
        assert_eq!(out.len(), 1);
        // Clearly dominant other axis switches.
        m.feed(PadInput::Axis(0, Axis::LeftStickX, 0.8), ms(t0, 10), &mut out);
        assert_eq!(out, vec![Nav::Down, Nav::Right]);
        // Back to centre releases; repeat stops.
        m.feed(PadInput::Axis(0, Axis::LeftStickY, 0.0), ms(t0, 20), &mut out);
        m.feed(PadInput::Axis(0, Axis::LeftStickX, 0.1), ms(t0, 20), &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(m.next_deadline(), None);
        // Up is +y in gilrs.
        m.feed(PadInput::Axis(0, Axis::LeftStickY, 0.9), ms(t0, 30), &mut out);
        assert_eq!(out.last(), Some(&Nav::Up));
        // Right stick is ignored; NaN is harmless.
        m.feed(PadInput::Axis(0, Axis::RightStickX, 1.0), ms(t0, 40), &mut out);
        m.feed(PadInput::Axis(0, Axis::LeftStickX, f32::NAN), ms(t0, 40), &mut out);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn stick_repeat() {
        let t0 = Instant::now();
        let mut m = PadMapper::new(cfg());
        let mut out = vec![];
        m.feed(PadInput::Axis(0, Axis::LeftStickX, -1.0), t0, &mut out);
        m.tick(ms(t0, 300), &mut out);
        m.tick(ms(t0, 400), &mut out);
        assert_eq!(out, vec![Nav::Left; 3]);
    }

    #[test]
    fn disabled_drops_everything_and_clears_held() {
        let t0 = Instant::now();
        let mut m = PadMapper::new(cfg());
        let mut out = vec![];
        m.feed(PadInput::Pressed(0, Button::DPadUp), t0, &mut out);
        m.set_enabled(false);
        assert_eq!(m.next_deadline(), None);
        m.tick(ms(t0, 1000), &mut out);
        m.feed(PadInput::Pressed(0, Button::South), ms(t0, 1000), &mut out);
        m.feed(PadInput::Axis(0, Axis::LeftStickY, 1.0), ms(t0, 1000), &mut out);
        assert_eq!(out, vec![Nav::Up]);
        m.set_enabled(true);
        // The release of the button pressed before disabling is harmless.
        m.feed(PadInput::Released(0, Button::DPadUp), ms(t0, 1100), &mut out);
        m.feed(PadInput::Pressed(0, Button::DPadUp), ms(t0, 1200), &mut out);
        assert_eq!(out, vec![Nav::Up, Nav::Up]);
    }

    /// A game runs and Lodestar isn't focused: only Guide presses come out, nothing repeats,
    /// and nothing held before carries over.
    #[test]
    fn guide_only_maps_nothing_but_guide() {
        let t0 = Instant::now();
        let mut m = PadMapper::new(cfg());
        let mut out = vec![];
        m.feed(PadInput::Pressed(0, Button::DPadDown), t0, &mut out);
        m.set_guide_only(true);
        assert_eq!(m.next_deadline(), None);
        for b in [Button::South, Button::East, Button::Start, Button::Select, Button::North, Button::DPadUp] {
            m.feed(PadInput::Pressed(0, b), ms(t0, 10), &mut out);
            m.feed(PadInput::Released(0, b), ms(t0, 20), &mut out);
        }
        m.feed(PadInput::Axis(0, Axis::LeftStickY, -1.0), ms(t0, 30), &mut out);
        m.tick(ms(t0, 2000), &mut out);
        m.feed(PadInput::Pressed(0, Button::Mode), ms(t0, 2100), &mut out);
        assert_eq!(out, vec![Nav::Down, Nav::Guide]);
        m.set_guide_only(false);
        m.feed(PadInput::Pressed(0, Button::South), ms(t0, 3000), &mut out);
        assert_eq!(out, vec![Nav::Down, Nav::Guide, Nav::Accept]);
    }

    fn combo_cfg(c: MenuCombo) -> PadConfig {
        PadConfig { menu_combo: Some(c), ..cfg() }
    }

    /// Press each of `buttons` at its time (ms), hold, and tick through `until` (ms) the way the
    /// pad thread does (it wakes at `next_deadline`).
    fn hold(
        m: &mut PadMapper,
        t0: Instant,
        presses: &[(u64, Button)],
        releases: &[(u64, Button)],
        until: u64,
    ) -> Vec<Nav> {
        let mut out = vec![];
        for t in 0..=until {
            for &(at, b) in presses.iter().filter(|p| p.0 == t) {
                m.feed(PadInput::Pressed(0, b), ms(t0, at), &mut out);
            }
            for &(at, b) in releases.iter().filter(|p| p.0 == t) {
                m.feed(PadInput::Released(0, b), ms(t0, at), &mut out);
            }
            if m.next_deadline().is_some_and(|d| d <= ms(t0, t)) {
                m.tick_menu_combo(ms(t0, t), &mut out);
            }
        }
        out
    }

    #[test]
    fn menu_combo_counts_once_after_the_hold() {
        use Button::{LeftThumb as L3, RightThumb as R3};
        let t0 = Instant::now();
        let mut m = PadMapper::new(combo_cfg(MenuCombo::STICKS));
        let mut out = vec![];
        m.feed(PadInput::Pressed(0, L3), t0, &mut out);
        m.feed(PadInput::Pressed(0, R3), ms(t0, 120), &mut out);
        assert!(out.is_empty(), "stick clicks map to nothing by themselves");
        assert_eq!(m.next_deadline(), Some(ms(t0, 120) + COMBO_HOLD));
        m.tick_menu_combo(ms(t0, 519), &mut out);
        assert!(out.is_empty(), "not held long enough yet");
        m.tick_menu_combo(ms(t0, 520), &mut out);
        assert_eq!(out, vec![Nav::Guide]);
        // Held on: once only, and nothing is due any more.
        assert_eq!(m.next_deadline(), None);
        m.tick_menu_combo(ms(t0, 5000), &mut out);
        // Let one go and press it again: still once (both must be let go first).
        m.feed(PadInput::Released(0, R3), ms(t0, 5100), &mut out);
        m.feed(PadInput::Pressed(0, R3), ms(t0, 5150), &mut out);
        m.tick_menu_combo(ms(t0, 6000), &mut out);
        assert_eq!(out, vec![Nav::Guide]);
        // Both let go: a new combination counts again.
        m.feed(PadInput::Released(0, R3), ms(t0, 6100), &mut out);
        m.feed(PadInput::Released(0, L3), ms(t0, 6100), &mut out);
        assert_eq!(hold(&mut m, ms(t0, 7000), &[(0, R3), (10, L3)], &[], 500), vec![Nav::Guide]);
    }

    #[test]
    fn menu_combo_tapped_too_slow_or_incomplete_does_nothing() {
        use Button::{LeftThumb as L3, RightThumb as R3};
        let t0 = Instant::now();
        let fresh = || PadMapper::new(combo_cfg(MenuCombo::STICKS));
        // Tapped together (an in-game L3 + R3): let go before the hold.
        assert!(hold(&mut fresh(), t0, &[(0, L3), (30, R3)], &[(300, L3), (300, R3)], 2000).is_empty());
        // The second one too late (sprinting with L3, then R3 to melee).
        assert!(hold(&mut fresh(), t0, &[(0, L3), (151, R3)], &[], 2000).is_empty());
        // One stick held for long.
        assert!(hold(&mut fresh(), t0, &[(0, R3)], &[], 2000).is_empty());
        // Just within the window: counts.
        assert_eq!(hold(&mut fresh(), t0, &[(0, L3), (150, R3)], &[], 600), vec![Nav::Guide]);
        // Another pad's stick doesn't complete this pad's combination.
        let mut m = fresh();
        let mut out = vec![];
        m.feed(PadInput::Pressed(0, L3), t0, &mut out);
        m.feed(PadInput::Pressed(1, R3), ms(t0, 10), &mut out);
        assert_eq!(m.next_deadline(), None);
        // Home only (no combination): nothing.
        assert!(hold(&mut PadMapper::new(cfg()), t0, &[(0, L3), (10, R3)], &[], 2000).is_empty());
    }

    /// Like [`hold`], with the hold-repeats too (the pad thread runs both ticks on every wake).
    fn hold_with_repeats(m: &mut PadMapper, t0: Instant, presses: &[(u64, Button)], until: u64) -> Vec<Nav> {
        let mut out = vec![];
        for t in 0..=until {
            for &(at, b) in presses.iter().filter(|p| p.0 == t) {
                m.feed(PadInput::Pressed(0, b), ms(t0, at), &mut out);
            }
            if m.next_deadline().is_some_and(|d| d <= ms(t0, t)) {
                m.tick_menu_combo(ms(t0, t), &mut out);
                m.tick(ms(t0, t), &mut out);
            }
        }
        out
    }

    #[test]
    fn a_button_that_starts_the_combo_stops_repeating_once_the_rest_join() {
        use Button::{DPadDown, LeftTrigger as Lb, RightTrigger as Rb, Start};
        let t0 = Instant::now();
        // LB (page up) held through the combination: one page, then only the menu button —
        // not pages scrolling under the menu while the bumpers are still held.
        let mut m = PadMapper::new(combo_cfg(MenuCombo::BUMPERS_START));
        let out = hold_with_repeats(&mut m, t0, &[(0, Lb), (40, Rb), (90, Start)], 3000);
        assert_eq!(out, vec![Nav::PageUp, Nav::Guide]);
        assert_eq!(m.repeating(), None);
        // A held direction that isn't part of the combination keeps repeating.
        let mut m = PadMapper::new(combo_cfg(MenuCombo::BUMPERS_START));
        let out = hold_with_repeats(&mut m, t0, &[(0, DPadDown), (10, Lb), (40, Rb), (90, Start)], 1000);
        assert_eq!(out.iter().filter(|n| **n == Nav::PageUp).count(), 1, "{out:?}");
        assert_eq!(out.iter().filter(|n| **n == Nav::Guide).count(), 1, "{out:?}");
        assert_eq!(m.repeating(), Some(Nav::Down), "{out:?}");
        // On its own (no other combination button joins), LB repeats as always.
        let mut m = PadMapper::new(combo_cfg(MenuCombo::BUMPERS_START));
        let out = hold_with_repeats(&mut m, t0, &[(0, Lb)], 1000);
        assert!(out.len() > 2 && out.iter().all(|n| *n == Nav::PageUp), "{out:?}");
    }

    #[test]
    fn three_button_combo_and_its_buttons_keep_their_meaning() {
        use Button::{LeftTrigger as Lb, RightTrigger as Rb, Start};
        let t0 = Instant::now();
        let mut m = PadMapper::new(combo_cfg(MenuCombo::BUMPERS_START));
        // Lodestar in use: the first button does what it always does, the ones joining it in
        // time don't (Start would open a menu), then the hold opens the menu.
        let out = hold(&mut m, t0, &[(0, Lb), (40, Rb), (90, Start)], &[], 600);
        assert_eq!(out, vec![Nav::PageUp, Nav::Guide]);
        // Pressed on their own (or too late to join), they keep their meaning.
        let mut m = PadMapper::new(combo_cfg(MenuCombo::BUMPERS_START));
        let out = hold(&mut m, t0, &[(0, Start), (400, Rb)], &[(200, Start)], 600);
        assert_eq!(out, vec![Nav::Menu, Nav::PageDown]);
        // Two of three: nothing more.
        let mut m = PadMapper::new(combo_cfg(MenuCombo::BUMPERS_START));
        m.set_guide_only(true);
        assert!(hold(&mut m, t0, &[(0, Lb), (40, Rb)], &[], 2000).is_empty());
    }

    #[test]
    fn menu_combo_progress_is_forgotten_when_the_gate_changes() {
        use Button::{Select, Start};
        let t0 = Instant::now();
        let mut m = PadMapper::new(combo_cfg(MenuCombo::SELECT_START));
        m.set_guide_only(true);
        let mut out = vec![];
        m.feed(PadInput::Pressed(0, Select), t0, &mut out);
        m.feed(PadInput::Pressed(0, Start), ms(t0, 20), &mut out);
        m.set_guide_only(false);
        assert_eq!(m.next_deadline(), None, "fail safe: a hold started under the other gate doesn't count");
        m.tick_menu_combo(ms(t0, 2000), &mut out);
        assert!(out.is_empty());
        // Disabled: nothing at all.
        let mut m = PadMapper::new(combo_cfg(MenuCombo::SELECT_START));
        m.set_enabled(false);
        assert!(hold(&mut m, t0, &[(0, Select), (10, Start)], &[], 2000).is_empty());
    }

    #[test]
    fn home_left_to_steam_is_ignored() {
        let t0 = Instant::now();
        for guide_only in [false, true] {
            let mut m = PadMapper::new(cfg());
            m.set_guide_only(guide_only);
            m.set_home(false);
            let mut out = vec![];
            m.feed(PadInput::Pressed(0, Button::Mode), t0, &mut out);
            assert!(out.is_empty(), "guide_only {guide_only}");
            m.set_home(true);
            m.feed(PadInput::Released(0, Button::Mode), ms(t0, 50), &mut out);
            m.feed(PadInput::Pressed(0, Button::Mode), ms(t0, 100), &mut out);
            assert_eq!(out, vec![Nav::Guide], "guide_only {guide_only}");
        }
    }

    #[test]
    fn duplicate_pads_are_collapsed() {
        let t0 = Instant::now();
        let mut m = PadMapper::new(cfg());
        let mut out = vec![];
        m.feed(PadInput::Pressed(0, Button::South), t0, &mut out);
        m.feed(PadInput::Pressed(1, Button::South), ms(t0, 5), &mut out);
        assert_eq!(out, vec![Nav::Accept]);
        // A real second press from another pad later counts.
        m.feed(PadInput::Pressed(1, Button::South), ms(t0, 500), &mut out);
        assert_eq!(out, vec![Nav::Accept, Nav::Accept]);
    }

    #[test]
    fn disconnect_clears_pad() {
        let t0 = Instant::now();
        let mut m = PadMapper::new(cfg());
        let mut out = vec![];
        m.feed(PadInput::Pressed(3, Button::DPadDown), t0, &mut out);
        m.feed(PadInput::Disconnected(3), ms(t0, 10), &mut out);
        assert_eq!(m.next_deadline(), None);
    }

    /// Press and release `(ms, button, down)` on pad 0; what came out and the finished take.
    fn record(m: &mut PadMapper, t0: Instant, steps: &[(u64, Button, bool)]) -> (Vec<Nav>, Option<Recorded>) {
        let mut out = vec![];
        for &(at, b, down) in steps {
            let input = if down { PadInput::Pressed(0, b) } else { PadInput::Released(0, b) };
            m.feed(input, ms(t0, at), &mut out);
            m.tick_menu_combo(ms(t0, at), &mut out);
            m.tick(ms(t0, at), &mut out);
        }
        (out, m.take_recording())
    }

    #[test]
    fn recording_takes_the_buttons_held_together_and_nothing_else_happens() {
        use Button::{LeftTrigger as Lb, Mode as Home, RightTrigger as Rb, South as A, Start};
        let t0 = Instant::now();
        let mut m = PadMapper::new(combo_cfg(MenuCombo::STICKS));
        m.set_recording(true);
        // Held one after another, let go in any order: the most held at once.
        let (out, take) = record(
            &mut m,
            t0,
            &[(0, Lb, true), (300, Rb, true), (900, Start, true), (1000, Rb, false), (1100, Lb, false)],
        );
        assert_eq!(m.recording_live(), MenuCombo::BUMPERS_START, "shown while held");
        let (out2, take2) = record(&mut m, t0, &[(1200, Start, false)]);
        assert!(out.is_empty() && out2.is_empty() && take.is_none(), "the recorder owns the pad: {out:?}");
        assert_eq!(take2, Some(Recorded { combo: MenuCombo::BUMPERS_START, home: false }));
        assert!(!m.recording(), "one take, then the pad is Lodestar's again");
        assert_eq!(record(&mut m, t0, &[(1300, A, true)]).0, vec![Nav::Accept]);
        // Home with them: caught (the screen refuses it); one button alone: a take too small.
        let mut m = PadMapper::new(combo_cfg(MenuCombo::STICKS));
        m.set_recording(true);
        let steps = [(0, A, true), (10, Home, true), (50, Home, false), (60, A, false)];
        assert_eq!(
            record(&mut m, t0, &steps),
            (vec![], Some(Recorded { combo: MenuCombo::of([PadButton::A]), home: true }))
        );
        // A button held from before (the press that started the recording) is not part of it.
        let mut m = PadMapper::new(cfg());
        let mut out = vec![];
        m.feed(PadInput::Pressed(0, A), t0, &mut out);
        m.set_recording(true);
        assert_eq!(out, vec![Nav::Accept]);
        assert_eq!(record(&mut m, t0, &[(100, A, false)]), (vec![], None));
        assert!(m.recording());
    }

    /// REGRESSION (safety rule): a recording never catches a game's presses — while only the
    /// menu button reaches Lodestar, the pad works as without one (and the take starts over).
    #[test]
    fn recording_pauses_while_a_game_has_the_pad() {
        use Button::{LeftThumb as L3, RightThumb as R3, South as A, West as X};
        let t0 = Instant::now();
        let mut m = PadMapper::new(combo_cfg(MenuCombo::STICKS));
        m.set_recording(true);
        m.set_guide_only(true);
        let (out, take) = record(&mut m, t0, &[(0, A, true), (10, X, true), (20, X, false), (30, A, false)]);
        assert!(out.is_empty() && take.is_none());
        // The menu button still works in the game.
        let (mut out, _) = record(&mut m, t0, &[(100, L3, true), (110, R3, true)]);
        m.tick_menu_combo(ms(t0, 600), &mut out);
        assert_eq!(out, vec![Nav::Guide]);
        assert!(m.recording(), "resumes once Lodestar has the pad again");
        m.set_guide_only(false);
        assert_eq!(m.recording_live(), MenuCombo::empty());
        m.set_enabled(false);
        assert!(record(&mut m, t0, &[(700, A, true), (710, A, false)]).1.is_none());
    }

    #[test]
    fn config_from_controller_clamps() {
        let mut c = crate::config::ControllerConfig { repeat_delay_ms: 0, repeat_interval_ms: 0, ..Default::default() };
        c.swap_ab = true;
        let p = PadConfig::from_controller(&c);
        assert!(p.swap_ab);
        assert!(p.repeat_interval >= Duration::from_millis(16));
        assert_eq!(p.menu_combo, Some(MenuCombo::STICKS));
        c.menu_button = crate::config::MenuButton::Home;
        assert_eq!(PadConfig::from_controller(&c).menu_combo, None);
    }
}
